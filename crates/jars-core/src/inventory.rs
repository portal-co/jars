//! Member-level inventory of the natives and builtins a goal binary needs.
//!
//! The walker follows the same entrypoint closure as JAR compilation, but it
//! records platform and native-binding members instead of failing when they
//! are unmodeled. Platform archives (JARs or `jmod` files) are consulted only
//! to see `ACC_NATIVE`; they are never lowered.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs,
    io::Read,
    path::Path,
};

use crate::{
    CompileError, JarEntrypoint, JarImportOptions, classfile, read_jar_classpath, stdlib,
};

const ACC_NATIVE: u16 = 0x0100;
const ACC_ABSTRACT: u16 = 0x0400;

/// Whether an open inventory member is a Rust builtin or a native shim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryKind {
    Native,
    Builtin,
}

impl InventoryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Builtin => "builtin",
        }
    }
}

/// One referenced member that is not already declared in `java_stdlib!`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryMember {
    pub name: String,
    pub descriptor: String,
    pub kind: InventoryKind,
    /// `Some("closed-world")` for reflection, method handles, and
    /// `invokedynamic`. Those rows are visible and are not implementation
    /// subtasks.
    pub reason: Option<&'static str>,
    pub references: u32,
}

impl InventoryMember {
    /// `true` when a contributor can implement this member.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.reason.is_none()
    }
}

/// One class that still has non-`done` members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryClass {
    pub name: String,
    pub members: Vec<InventoryMember>,
}

impl InventoryClass {
    #[must_use]
    pub fn references(&self) -> u32 {
        self.members.iter().map(|member| member.references).sum()
    }

    #[must_use]
    pub fn open_members(&self) -> usize {
        self.members.iter().filter(|member| member.is_open()).count()
    }

    #[must_use]
    pub fn blocked_members(&self) -> usize {
        self.members
            .iter()
            .filter(|member| member.reason.is_some())
            .count()
    }

    /// Checked-in file name: `java/lang/System` becomes `java.lang.System.toml`.
    #[must_use]
    pub fn file_name(&self) -> String {
        format!("{}.toml", self.name.replace('/', "."))
    }
}

/// Deterministic inventory for one goal entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryReport {
    pub classes: Vec<InventoryClass>,
}

impl InventoryReport {
    #[must_use]
    pub fn open_classes(&self) -> usize {
        self.classes
            .iter()
            .filter(|class| class.open_members() > 0)
            .count()
    }

    #[must_use]
    pub fn open_members(&self) -> usize {
        self.classes.iter().map(InventoryClass::open_members).sum()
    }

    #[must_use]
    pub fn blocked_members(&self) -> usize {
        self.classes
            .iter()
            .map(InventoryClass::blocked_members)
            .sum()
    }
}

/// Walks `jars` from `entry` and classifies referenced natives and builtins.
///
/// `platforms` are JARs or jmods used only for `ACC_NATIVE` classification.
/// A jmod's `classes/` prefix is stripped. Without a platform class, a
/// platform member is recorded as a builtin.
pub fn inventory_jars(
    jars: &[impl AsRef<Path>],
    platforms: &[impl AsRef<Path>],
    entry: &JarEntrypoint,
    options: JarImportOptions,
) -> Result<InventoryReport, CompileError> {
    let goal = read_jar_classpath(jars, options)?;
    let mut platform_classes = BTreeMap::new();
    for platform in platforms {
        read_platform_archive(platform.as_ref(), options, &mut platform_classes)?;
    }
    let mut counts: HashMap<(String, String, String), (InventoryKind, Option<&'static str>, u32)> =
        HashMap::new();
    walk_closure(&goal, &platform_classes, entry, &mut counts)?;
    Ok(report_from_counts(counts))
}

/// Writes `index.toml` and one TOML file per class that still has work.
///
/// Members already in `java_stdlib!` are omitted. Regeneration deletes stale
/// class TOML files in `directory` so finished classes leave the open set.
pub fn write_inventory(
    directory: &Path,
    goal_name: &str,
    entry: &JarEntrypoint,
    target_release: u16,
    report: &InventoryReport,
) -> Result<(), CompileError> {
    fs::create_dir_all(directory).map_err(|error| CompileError::InvalidClass(error.to_string()))?;
    let mut keep = HashSet::from(["index.toml".to_owned()]);
    for class in &report.classes {
        let file_name = class.file_name();
        keep.insert(file_name.clone());
        let path = directory.join(&file_name);
        fs::write(&path, class_toml(class))
            .map_err(|error| CompileError::InvalidClass(error.to_string()))?;
    }
    for entry in fs::read_dir(directory)
        .map_err(|error| CompileError::InvalidClass(error.to_string()))?
    {
        let entry = entry.map_err(|error| CompileError::InvalidClass(error.to_string()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.ends_with(".toml") && !keep.contains(name) {
            fs::remove_file(entry.path())
                .map_err(|error| CompileError::InvalidClass(error.to_string()))?;
        }
    }
    fs::write(directory.join("index.toml"), index_toml(goal_name, entry, target_release, report))
        .map_err(|error| CompileError::InvalidClass(error.to_string()))?;
    Ok(())
}

fn report_from_counts(
    counts: HashMap<(String, String, String), (InventoryKind, Option<&'static str>, u32)>,
) -> InventoryReport {
    let mut by_class: BTreeMap<String, Vec<InventoryMember>> = BTreeMap::new();
    for ((class, name, descriptor), (kind, reason, references)) in counts {
        by_class.entry(class).or_default().push(InventoryMember {
            name,
            descriptor,
            kind,
            reason,
            references,
        });
    }
    let mut classes = by_class
        .into_iter()
        .map(|(name, mut members)| {
            members.sort_by(|left, right| {
                left.descriptor
                    .cmp(&right.descriptor)
                    .then_with(|| left.name.cmp(&right.name))
            });
            InventoryClass { name, members }
        })
        .collect::<Vec<_>>();
    classes.sort_by(|left, right| {
        right
            .references()
            .cmp(&left.references())
            .then_with(|| left.name.cmp(&right.name))
    });
    InventoryReport { classes }
}

fn note(
    counts: &mut HashMap<(String, String, String), (InventoryKind, Option<&'static str>, u32)>,
    class: String,
    name: String,
    descriptor: String,
    kind: InventoryKind,
    reason: Option<&'static str>,
) {
    let slot = counts
        .entry((class, name, descriptor))
        .or_insert((kind, reason, 0));
    slot.2 += 1;
}

fn walk_closure(
    goal: &BTreeMap<String, crate::JarClass>,
    platforms: &BTreeMap<String, Vec<u8>>,
    entry: &JarEntrypoint,
    counts: &mut HashMap<(String, String, String), (InventoryKind, Option<&'static str>, u32)>,
) -> Result<(), CompileError> {
    let mut queue = VecDeque::from([(
        entry.class.clone(),
        entry.method.clone(),
        entry.descriptor.clone(),
    )]);
    let mut seen = HashSet::<(String, String, String)>::new();
    let mut activated = HashSet::<String>::new();
    while let Some((class_name, method_name, descriptor)) = queue.pop_front() {
        if !seen.insert((
            class_name.clone(),
            method_name.clone(),
            descriptor.clone(),
        )) {
            continue;
        }
        let Some(bytes) = goal.get(&class_name).map(|class| class.bytes.as_slice()) else {
            return Err(CompileError::MissingClass {
                class: class_name,
                referenced_by: "the inventory closure".to_owned(),
            });
        };
        let class = classfile::parse(bytes).map_err(|error| CompileError::Parse(error.to_string()))?;
        if activated.insert(class_name.clone()) {
            activate_class(&class, goal, &mut queue);
        }
        let Some(member) = find_member(&class, &method_name, &descriptor) else {
            if class_name == entry.class
                && method_name == entry.method
                && descriptor == entry.descriptor
            {
                return Err(CompileError::MissingEntryMethod {
                    class: class_name,
                    method: method_name,
                    descriptor,
                });
            }
            if let Some(owner) = resolve_method(goal, platforms, &class_name, &method_name, &descriptor)
                && owner != class_name
            {
                queue.push_back((owner, method_name, descriptor));
            }
            continue;
        };
        if member.access_flags & ACC_NATIVE != 0 {
            note(
                counts,
                class_name,
                method_name,
                descriptor,
                InventoryKind::Native,
                None,
            );
            continue;
        }
        let Some(code) = member
            .attributes
            .iter()
            .find_map(|attribute| classfile::code(attribute, &class.constant_pool).transpose())
            .transpose()
            .map_err(|error| CompileError::Parse(error.to_string()))?
        else {
            continue;
        };
        let instructions = classfile::instructions(&code.code)
            .map_err(|error| CompileError::Parse(error.to_string()))?;
        let pool = &class.constant_pool;
        for (_, instruction) in instructions {
            match instruction {
                classfile::RawInstruction::GetStatic { index }
                | classfile::RawInstruction::PutStatic { index }
                | classfile::RawInstruction::GetField { index }
                | classfile::RawInstruction::PutField { index } => {
                    let reference = pool
                        .field_ref(index)
                        .map_err(|error| CompileError::Parse(error.to_string()))?;
                    classify_field(&reference, platforms, counts);
                }
                classfile::RawInstruction::InvokeSpecial { index }
                | classfile::RawInstruction::InvokeStatic { index }
                | classfile::RawInstruction::InvokeVirtual { index } => {
                    let reference = pool
                        .method_ref(index)
                        .map_err(|error| CompileError::Parse(error.to_string()))?;
                    consider_method(&reference, goal, platforms, counts, &mut queue);
                }
                classfile::RawInstruction::InvokeInterface { index } => {
                    let reference = pool
                        .interface_method_ref(index)
                        .map_err(|error| CompileError::Parse(error.to_string()))?;
                    consider_method(&reference, goal, platforms, counts, &mut queue);
                }
                classfile::RawInstruction::InvokeDynamic { index } => {
                    let (name, descriptor) = class
                        .constant_pool
                        .invoke_dynamic(index)
                        .map_err(|error| CompileError::Parse(error.to_string()))?;
                    note(
                        counts,
                        "java/lang/invoke/Invokedynamic".to_owned(),
                        name.to_owned(),
                        descriptor.to_owned(),
                        InventoryKind::Builtin,
                        Some("closed-world"),
                    );
                }
                classfile::RawInstruction::New { index } => {
                    let owner = class
                        .constant_pool
                        .class_name(index)
                        .map_err(|error| CompileError::Parse(error.to_string()))?;
                    enqueue_goal_clinit(&owner, goal, &mut queue);
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn activate_class(
    class: &classfile::ClassFile,
    goal: &BTreeMap<String, crate::JarClass>,
    queue: &mut VecDeque<(String, String, String)>,
) {
    let pool = &class.constant_pool;
    if class.super_class != 0
        && let Ok(parent) = pool.class_name(class.super_class)
    {
        enqueue_goal_clinit(&parent, goal, queue);
    }
    for interface in &class.interfaces {
        if let Ok(name) = pool.class_name(*interface) {
            enqueue_goal_clinit(&name, goal, queue);
        }
    }
    for field in &class.fields {
        if let Ok(descriptor) = pool.utf8(field.descriptor_index) {
            enqueue_descriptor_classes(descriptor, goal, queue);
        }
    }
    if let Some(clinit) = class.methods.iter().find(|method| {
        pool.utf8(method.name_index).ok() == Some("<clinit>")
    }) && let Ok(descriptor) = pool.utf8(clinit.descriptor_index)
    {
        let name = pool
            .class_name(class.this_class)
            .unwrap_or_default();
        if goal.contains_key(&name) {
            queue.push_back((name, "<clinit>".to_owned(), descriptor.to_owned()));
        }
    }
}

fn enqueue_descriptor_classes(
    descriptor: &str,
    goal: &BTreeMap<String, crate::JarClass>,
    queue: &mut VecDeque<(String, String, String)>,
) {
    let bytes = descriptor.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'L' {
            if let Some(end) = descriptor[index + 1..].find(';') {
                let name = &descriptor[index + 1..index + 1 + end];
                enqueue_goal_clinit(name, goal, queue);
                index += end + 2;
                continue;
            }
        }
        index += 1;
    }
}

fn enqueue_goal_clinit(
    class_name: &str,
    goal: &BTreeMap<String, crate::JarClass>,
    queue: &mut VecDeque<(String, String, String)>,
) {
    if is_platform(class_name) || !goal.contains_key(class_name) {
        return;
    }
    let Some(bytes) = goal.get(class_name).map(|class| class.bytes.as_slice()) else {
        return;
    };
    let Ok(class) = classfile::parse(bytes) else {
        return;
    };
    if class.methods.iter().any(|method| {
        class.constant_pool.utf8(method.name_index).ok() == Some("<clinit>")
    }) {
        queue.push_back((
            class_name.to_owned(),
            "<clinit>".to_owned(),
            "()V".to_owned(),
        ));
    } else if class.super_class != 0
        && let Ok(parent) = class.constant_pool.class_name(class.super_class)
    {
        enqueue_goal_clinit(&parent, goal, queue);
    }
}

fn consider_method(
    reference: &classfile::MemberRef,
    goal: &BTreeMap<String, crate::JarClass>,
    platforms: &BTreeMap<String, Vec<u8>>,
    counts: &mut HashMap<(String, String, String), (InventoryKind, Option<&'static str>, u32)>,
    queue: &mut VecDeque<(String, String, String)>,
) {
    if stdlib::member(&reference.class, &reference.name, &reference.descriptor).is_some() {
        return;
    }
    if is_platform(&reference.class) || !goal.contains_key(&reference.class) {
        let kind = platform_kind(
            platforms,
            &reference.class,
            &reference.name,
            &reference.descriptor,
        );
        note(
            counts,
            reference.class.clone(),
            reference.name.clone(),
            reference.descriptor.clone(),
            kind,
            closed_world_reason(&reference.class, &reference.name),
        );
        return;
    }
    let Some(bytes) = goal.get(&reference.class).map(|class| class.bytes.as_slice()) else {
        return;
    };
    let Ok(class) = classfile::parse(bytes) else {
        queue.push_back((
            reference.class.clone(),
            reference.name.clone(),
            reference.descriptor.clone(),
        ));
        return;
    };
    let Some(member) = find_member(&class, &reference.name, &reference.descriptor) else {
        let Some(owner) = resolve_method(
            goal,
            platforms,
            &reference.class,
            &reference.name,
            &reference.descriptor,
        ) else {
            return;
        };
        if is_platform(&owner) || !goal.contains_key(&owner) {
            let kind = platform_kind(platforms, &owner, &reference.name, &reference.descriptor);
            note(
                counts,
                owner.clone(),
                reference.name.clone(),
                reference.descriptor.clone(),
                kind,
                closed_world_reason(&owner, &reference.name),
            );
            return;
        }
        queue.push_back((owner, reference.name.clone(), reference.descriptor.clone()));
        return;
    };
    if member.access_flags & ACC_NATIVE != 0 {
        note(
            counts,
            reference.class.clone(),
            reference.name.clone(),
            reference.descriptor.clone(),
            InventoryKind::Native,
            closed_world_reason(&reference.class, &reference.name),
        );
        return;
    }
    if member.access_flags & ACC_ABSTRACT != 0 {
        return;
    }
    queue.push_back((
        reference.class.clone(),
        reference.name.clone(),
        reference.descriptor.clone(),
    ));
}

fn classify_field(
    reference: &classfile::MemberRef,
    platforms: &BTreeMap<String, Vec<u8>>,
    counts: &mut HashMap<(String, String, String), (InventoryKind, Option<&'static str>, u32)>,
) {
    if stdlib::member(&reference.class, &reference.name, &reference.descriptor).is_some() {
        return;
    }
    if !is_platform(&reference.class) {
        return;
    }
    let _ = platforms;
    note(
        counts,
        reference.class.clone(),
        reference.name.clone(),
        reference.descriptor.clone(),
        InventoryKind::Builtin,
        closed_world_reason(&reference.class, &reference.name),
    );
}

fn platform_kind(
    platforms: &BTreeMap<String, Vec<u8>>,
    class: &str,
    name: &str,
    descriptor: &str,
) -> InventoryKind {
    let Some(bytes) = platforms.get(class) else {
        return InventoryKind::Builtin;
    };
    let Ok(parsed) = classfile::parse(bytes) else {
        return InventoryKind::Builtin;
    };
    let Some(member) = find_member(&parsed, name, descriptor) else {
        return InventoryKind::Builtin;
    };
    if member.access_flags & ACC_NATIVE != 0 && !method_has_code(member) {
        InventoryKind::Native
    } else if member.access_flags & ACC_NATIVE != 0 {
        InventoryKind::Native
    } else {
        InventoryKind::Builtin
    }
}

fn method_has_code(member: &classfile::MemberInfo) -> bool {
    member.attributes.iter().any(|attribute| attribute.name == "Code")
}

/// Call sites name the compile-time owner. The method body may live on a
/// superclass or interface in the goal classpath or the platform.
fn resolve_method(
    goal: &BTreeMap<String, crate::JarClass>,
    platforms: &BTreeMap<String, Vec<u8>>,
    class_name: &str,
    name: &str,
    descriptor: &str,
) -> Option<String> {
    let mut pending = vec![class_name.to_owned()];
    let mut seen = HashSet::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        let Some(bytes) = goal
            .get(&current)
            .map(|class| class.bytes.as_slice())
            .or_else(|| platforms.get(&current).map(Vec::as_slice))
        else {
            continue;
        };
        let Ok(class) = classfile::parse(bytes) else {
            continue;
        };
        if find_member(&class, name, descriptor).is_some() {
            return Some(current);
        }
        if class.super_class != 0
            && let Ok(parent) = class.constant_pool.class_name(class.super_class)
        {
            pending.push(parent);
        }
        for interface in &class.interfaces {
            if let Ok(parent) = class.constant_pool.class_name(*interface) {
                pending.push(parent);
            }
        }
    }
    None
}

fn find_member<'a>(
    class: &'a classfile::ClassFile,
    name: &str,
    descriptor: &str,
) -> Option<&'a classfile::MemberInfo> {
    class.methods.iter().find(|method| {
        class.constant_pool.utf8(method.name_index).ok() == Some(name)
            && class.constant_pool.utf8(method.descriptor_index).ok() == Some(descriptor)
    })
}

fn is_platform(class: &str) -> bool {
    class.starts_with("java/")
        || class.starts_with("javax/")
        || class.starts_with("jdk/")
        || class.starts_with("sun/")
        || class.starts_with("com/sun/")
}

fn closed_world_reason(class: &str, name: &str) -> Option<&'static str> {
    if class.starts_with("java/lang/reflect/")
        || class.starts_with("java/lang/invoke/")
        || class.starts_with("jdk/internal/reflect/")
        || class.starts_with("sun/reflect/")
    {
        return Some("closed-world");
    }
    if class == "java/lang/Class"
        && matches!(
            name,
            "forName"
                | "getMethod"
                | "getDeclaredMethod"
                | "getField"
                | "getDeclaredField"
                | "getConstructor"
                | "getDeclaredConstructor"
                | "newInstance"
        )
    {
        return Some("closed-world");
    }
    if class == "java/lang/ClassLoader" && matches!(name, "loadClass" | "defineClass" | "findClass")
    {
        return Some("closed-world");
    }
    None
}

fn read_platform_archive(
    path: &Path,
    options: JarImportOptions,
    classes: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), CompileError> {
    let file = std::fs::File::open(path).map_err(|error| CompileError::Jar {
        jar: path.to_owned(),
        detail: error.to_string(),
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| CompileError::Jar {
        jar: path.to_owned(),
        detail: error.to_string(),
    })?;
    let mut selected: BTreeMap<String, (String, u16)> = BTreeMap::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| CompileError::Jar {
                jar: path.to_owned(),
                detail: error.to_string(),
            })?;
        let raw_name = entry.name().to_owned();
        if !raw_name.ends_with(".class") || raw_name.ends_with('/') {
            continue;
        }
        let entry_name = raw_name
            .strip_prefix("classes/")
            .unwrap_or(raw_name.as_str())
            .to_owned();
        let (logical_name, release) = if let Some(rest) = entry_name.strip_prefix("META-INF/versions/")
        {
            let Some((release, logical_name)) = rest.split_once('/') else {
                continue;
            };
            let Ok(release) = release.parse::<u16>() else {
                continue;
            };
            if release > options.target_release || logical_name.is_empty() {
                continue;
            }
            (logical_name.to_owned(), release)
        } else {
            (entry_name.clone(), 0)
        };
        if logical_name == "module-info.class" {
            continue;
        }
        match selected.get(&logical_name) {
            Some((_, previous)) if *previous > release => {}
            _ => {
                selected.insert(logical_name, (raw_name, release));
            }
        }
    }
    for (logical_name, (entry_name, _)) in selected {
        let class_name = logical_name
            .trim_end_matches(".class")
            .to_owned();
        let mut entry = archive
            .by_name(&entry_name)
            .map_err(|error| CompileError::Jar {
                jar: path.to_owned(),
                detail: error.to_string(),
            })?;
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| CompileError::Jar {
                jar: path.to_owned(),
                detail: error.to_string(),
            })?;
        classes.insert(class_name, bytes);
    }
    Ok(())
}

fn index_toml(
    goal_name: &str,
    entry: &JarEntrypoint,
    target_release: u16,
    report: &InventoryReport,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("name = {}\n", toml_string(goal_name)));
    out.push_str(&format!(
        "entrypoint_class = {}\n",
        toml_string(&entry.class)
    ));
    out.push_str(&format!(
        "entrypoint_method = {}\n",
        toml_string(&entry.method)
    ));
    out.push_str(&format!(
        "entrypoint_descriptor = {}\n",
        toml_string(&entry.descriptor)
    ));
    out.push_str(&format!("target_release = {target_release}\n"));
    out.push_str(&format!("open_classes = {}\n", report.open_classes()));
    out.push_str(&format!("open_members = {}\n", report.open_members()));
    out.push_str(&format!(
        "blocked_members = {}\n",
        report.blocked_members()
    ));
    for class in &report.classes {
        out.push_str("\n[[class]]\n");
        out.push_str(&format!("name = {}\n", toml_string(&class.name)));
        out.push_str(&format!("file = {}\n", toml_string(&class.file_name())));
        out.push_str(&format!("references = {}\n", class.references()));
        out.push_str(&format!("open_members = {}\n", class.open_members()));
        out.push_str(&format!("blocked_members = {}\n", class.blocked_members()));
    }
    out
}

fn class_toml(class: &InventoryClass) -> String {
    let mut out = format!("class = {}\n", toml_string(&class.name));
    for member in &class.members {
        out.push_str("\n[[member]]\n");
        out.push_str(&format!("name = {}\n", toml_string(&member.name)));
        out.push_str(&format!(
            "descriptor = {}\n",
            toml_string(&member.descriptor)
        ));
        out.push_str(&format!("kind = {}\n", toml_string(member.kind.as_str())));
        if let Some(reason) = member.reason {
            out.push_str(&format!("reason = {}\n", toml_string(reason)));
        }
        out.push_str(&format!("references = {}\n", member.references));
    }
    out
}

fn toml_string(value: &str) -> String {
    let mut out = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    #[test]
    fn inventory_splits_native_builtin_done_and_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let goal = temp.path().join("goal.jar");
        let platform = temp.path().join("java.base.jmod");
        write_zip(
            &goal,
            &[
                ("InventoryApp.class", &inventory_app_class()),
                ("app/Glue.class", &glue_class()),
            ],
        );
        write_zip(
            &platform,
            &[
                (
                    "classes/java/lang/InventoryNative.class",
                    &native_class("java/lang/InventoryNative", "meaning", "()I"),
                ),
                (
                    "classes/java/lang/InventoryBuiltin.class",
                    &builtin_class(),
                ),
                (
                    "classes/java/lang/Class.class",
                    &native_class(
                        "java/lang/Class",
                        "forName",
                        "(Ljava/lang/String;)Ljava/lang/Class;",
                    ),
                ),
            ],
        );
        let entry = JarEntrypoint::main("InventoryApp");
        let report = inventory_jars(
            &[goal],
            &[platform],
            &entry,
            JarImportOptions::for_release(21),
        )
        .unwrap();
        let names = report
            .classes
            .iter()
            .map(|class| class.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"java/lang/InventoryNative"));
        assert!(names.contains(&"java/lang/InventoryBuiltin"));
        assert!(names.contains(&"java/lang/Class"));
        assert!(names.contains(&"app/Glue"));
        assert!(names.contains(&"java/lang/invoke/Invokedynamic"));
        assert!(!names.iter().any(|name| *name == "java/lang/Math"));
        assert!(!names.iter().any(|name| *name == "java/lang/System"));

        let native = report
            .classes
            .iter()
            .find(|class| class.name == "java/lang/InventoryNative")
            .unwrap();
        assert_eq!(native.members[0].kind, InventoryKind::Native);
        assert!(native.members[0].is_open());

        let builtin = report
            .classes
            .iter()
            .find(|class| class.name == "java/lang/InventoryBuiltin")
            .unwrap();
        assert!(builtin.members.iter().any(|member| {
            member.name == "pure" && member.kind == InventoryKind::Builtin && member.is_open()
        }));
        assert!(builtin.members.iter().any(|member| {
            member.name == "FLAG" && member.kind == InventoryKind::Builtin
        }));

        let blocked = report
            .classes
            .iter()
            .find(|class| class.name == "java/lang/Class")
            .unwrap();
        assert_eq!(blocked.members[0].kind, InventoryKind::Native);
        assert_eq!(blocked.members[0].reason, Some("closed-world"));
        assert!(!blocked.members[0].is_open());

        let directory = temp.path().join("goal");
        fs::write(directory.join("stale.toml"), "stale").unwrap_or_else(|_| {
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("stale.toml"), "stale").unwrap();
        });
        write_inventory(&directory, "fixture", &entry, 21, &report).unwrap();
        assert!(!directory.join("stale.toml").exists());
        let index = fs::read_to_string(directory.join("index.toml")).unwrap();
        assert!(index.contains("name = \"fixture\""));
        assert!(directory.join("java.lang.InventoryNative.toml").is_file());
        assert!(directory.join("app.Glue.toml").is_file());
        let glue = fs::read_to_string(directory.join("app.Glue.toml")).unwrap();
        assert!(glue.contains("kind = \"native\""));
        assert!(!glue.contains("reason"));
    }

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn inventory_app_class() -> Vec<u8> {
        let mut pool = Pool::new();
        let this = pool.class("InventoryApp");
        let object = pool.class("java/lang/Object");
        let helper = pool.method(this, "helper", "()V");
        let native_owner = pool.class("java/lang/InventoryNative");
        let meaning = pool.method(native_owner, "meaning", "()I");
        let builtin_owner = pool.class("java/lang/InventoryBuiltin");
        let pure = pool.method(builtin_owner, "pure", "()V");
        let math = pool.class("java/lang/Math");
        let min = pool.method(math, "min", "(II)I");
        let system = pool.class("java/lang/System");
        let out = pool.field(system, "out", "Ljava/io/PrintStream;");
        let builtin_owner = pool.class("java/lang/InventoryBuiltin");
        let flag = pool.field(builtin_owner, "FLAG", "I");
        let class_owner = pool.class("java/lang/Class");
        let for_name = pool.method(
            class_owner,
            "forName",
            "(Ljava/lang/String;)Ljava/lang/Class;",
        );
        let glue_owner = pool.class("app/Glue");
        let glue = pool.method(glue_owner, "glfw", "()I");
        let dynamic = pool.invoke_dynamic("bootstrap", "()Ljava/lang/Object;");
        let mut main = vec![0xb8];
        main.extend(helper.to_be_bytes());
        main.push(0xb1);
        let mut helper_code = Vec::new();
        helper_code.push(0xb8);
        helper_code.extend(meaning.to_be_bytes());
        helper_code.push(0x57);
        helper_code.push(0xb8);
        helper_code.extend(pure.to_be_bytes());
        helper_code.extend([0x04, 0x05, 0xb8]);
        helper_code.extend(min.to_be_bytes());
        helper_code.push(0x57);
        helper_code.push(0xb2);
        helper_code.extend(out.to_be_bytes());
        helper_code.push(0x57);
        helper_code.push(0xb2);
        helper_code.extend(flag.to_be_bytes());
        helper_code.push(0x57);
        helper_code.push(0x01);
        helper_code.push(0xb8);
        helper_code.extend(for_name.to_be_bytes());
        helper_code.push(0x57);
        helper_code.push(0xba);
        helper_code.extend(dynamic.to_be_bytes());
        helper_code.extend([0x00, 0x00, 0x57]);
        helper_code.push(0xb8);
        helper_code.extend(glue.to_be_bytes());
        helper_code.push(0x57);
        helper_code.push(0xb1);
        pool.finish_class(
            this,
            object,
            &[
                method_bytes("main", "([Ljava/lang/String;)V", 0x0009, 1, &main),
                method_bytes("helper", "()V", 0x0009, 2, &helper_code),
            ],
        )
    }

    fn glue_class() -> Vec<u8> {
        native_class("app/Glue", "glfw", "()I")
    }

    fn native_class(name: &str, method: &str, descriptor: &str) -> Vec<u8> {
        let mut pool = Pool::new();
        let this = pool.class(name);
        let object = pool.class("java/lang/Object");
        pool.finish_class(
            this,
            object,
            &[method_bytes(method, descriptor, 0x0109, 0, &[])],
        )
    }

    fn builtin_class() -> Vec<u8> {
        let mut pool = Pool::new();
        let this = pool.class("java/lang/InventoryBuiltin");
        let object = pool.class("java/lang/Object");
        let flag_name = pool.utf8("FLAG");
        let flag_desc = pool.utf8("I");
        let method_name = pool.utf8("pure");
        let method_desc = pool.utf8("()V");
        let code_name = pool.utf8("Code");
        let mut bytes = pool.finish_header(this, object);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 0x0009);
        push_u16(&mut bytes, flag_name);
        push_u16(&mut bytes, flag_desc);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 0x0009);
        push_u16(&mut bytes, method_name);
        push_u16(&mut bytes, method_desc);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, code_name);
        let mut info = Vec::new();
        push_u16(&mut info, 0);
        push_u16(&mut info, 0);
        push_u32(&mut info, 1);
        info.push(0xb1);
        push_u16(&mut info, 0);
        push_u16(&mut info, 0);
        push_u32(&mut bytes, info.len() as u32);
        bytes.extend(info);
        push_u16(&mut bytes, 0);
        bytes
    }

    fn method_bytes(name: &str, descriptor: &str, flags: u16, max_locals: u16, code: &[u8]) -> MethodSpec {
        MethodSpec {
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
            flags,
            max_locals,
            code: code.to_owned(),
        }
    }

    struct MethodSpec {
        name: String,
        descriptor: String,
        flags: u16,
        max_locals: u16,
        code: Vec<u8>,
    }

    struct Pool {
        entries: Vec<Cp>,
    }

    enum Cp {
        Utf8(String),
        Class(u16),
        NameAndType(u16, u16),
        Fieldref(u16, u16),
        Methodref(u16, u16),
        InvokeDynamic(u16, u16),
    }

    impl Pool {
        fn new() -> Self {
            Self { entries: Vec::new() }
        }

        fn utf8(&mut self, value: &str) -> u16 {
            if let Some(index) = self.entries.iter().position(|entry| {
                matches!(entry, Cp::Utf8(existing) if existing == value)
            }) {
                return (index + 1) as u16;
            }
            self.entries.push(Cp::Utf8(value.to_owned()));
            self.entries.len() as u16
        }

        fn class(&mut self, name: &str) -> u16 {
            let name_index = self.utf8(name);
            if let Some(index) = self
                .entries
                .iter()
                .position(|entry| matches!(entry, Cp::Class(existing) if *existing == name_index))
            {
                return (index + 1) as u16;
            }
            self.entries.push(Cp::Class(name_index));
            self.entries.len() as u16
        }

        fn field(&mut self, class: u16, name: &str, descriptor: &str) -> u16 {
            let name_and_type = self.name_and_type(name, descriptor);
            self.entries.push(Cp::Fieldref(class, name_and_type));
            self.entries.len() as u16
        }

        fn method(&mut self, class: u16, name: &str, descriptor: &str) -> u16 {
            let name_and_type = self.name_and_type(name, descriptor);
            self.entries.push(Cp::Methodref(class, name_and_type));
            self.entries.len() as u16
        }

        fn name_and_type(&mut self, name: &str, descriptor: &str) -> u16 {
            let name = self.utf8(name);
            let descriptor = self.utf8(descriptor);
            self.entries.push(Cp::NameAndType(name, descriptor));
            self.entries.len() as u16
        }

        fn invoke_dynamic(&mut self, name: &str, descriptor: &str) -> u16 {
            let name_and_type = self.name_and_type(name, descriptor);
            self.entries.push(Cp::InvokeDynamic(0, name_and_type));
            self.entries.len() as u16
        }

        fn finish_header(&self, this: u16, super_class: u16) -> Vec<u8> {
            let mut bytes = Vec::new();
            bytes.extend(0xcafe_babe_u32.to_be_bytes());
            push_u16(&mut bytes, 0);
            push_u16(&mut bytes, 65);
            push_u16(&mut bytes, (self.entries.len() + 1) as u16);
            for entry in &self.entries {
                match entry {
                    Cp::Utf8(value) => {
                        bytes.push(1);
                        push_u16(&mut bytes, value.len() as u16);
                        bytes.extend(value.as_bytes());
                    }
                    Cp::Class(name) => {
                        bytes.push(7);
                        push_u16(&mut bytes, *name);
                    }
                    Cp::NameAndType(name, descriptor) => {
                        bytes.push(12);
                        push_u16(&mut bytes, *name);
                        push_u16(&mut bytes, *descriptor);
                    }
                    Cp::Fieldref(class, name_and_type) => {
                        bytes.push(9);
                        push_u16(&mut bytes, *class);
                        push_u16(&mut bytes, *name_and_type);
                    }
                    Cp::Methodref(class, name_and_type) => {
                        bytes.push(10);
                        push_u16(&mut bytes, *class);
                        push_u16(&mut bytes, *name_and_type);
                    }
                    Cp::InvokeDynamic(bootstrap, name_and_type) => {
                        bytes.push(18);
                        push_u16(&mut bytes, *bootstrap);
                        push_u16(&mut bytes, *name_and_type);
                    }
                }
            }
            push_u16(&mut bytes, 0x0021);
            push_u16(&mut bytes, this);
            push_u16(&mut bytes, super_class);
            push_u16(&mut bytes, 0);
            bytes
        }

        fn finish_class(&mut self, this: u16, super_class: u16, methods: &[MethodSpec]) -> Vec<u8> {
            let mut specs = Vec::new();
            for method in methods {
                let name = self.utf8(&method.name);
                let descriptor = self.utf8(&method.descriptor);
                let code = if method.flags & ACC_NATIVE != 0 {
                    None
                } else {
                    Some(self.utf8("Code"))
                };
                specs.push((method, name, descriptor, code));
            }
            let mut bytes = self.finish_header(this, super_class);
            push_u16(&mut bytes, 0);
            push_u16(&mut bytes, specs.len() as u16);
            for (method, name, descriptor, code_name) in specs {
                push_u16(&mut bytes, method.flags);
                push_u16(&mut bytes, name);
                push_u16(&mut bytes, descriptor);
                if let Some(code_name) = code_name {
                    push_u16(&mut bytes, 1);
                    push_u16(&mut bytes, code_name);
                    let mut info = Vec::new();
                    push_u16(&mut info, 4);
                    push_u16(&mut info, method.max_locals);
                    push_u32(&mut info, method.code.len() as u32);
                    info.extend(&method.code);
                    push_u16(&mut info, 0);
                    push_u16(&mut info, 0);
                    push_u32(&mut bytes, info.len() as u32);
                    bytes.extend(info);
                } else {
                    push_u16(&mut bytes, 0);
                }
            }
            push_u16(&mut bytes, 0);
            bytes
        }
    }

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_be_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_be_bytes());
    }
}
