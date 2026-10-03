//! Whole-JAR compile coverage: per-class, per-method diagnostics for an
//! entire JAR, one target method per class.
//!
//! This is the machine-readable surface used by coverage ratchets and
//! agentic API-expansion loops. It answers "which `java/…` classes and
//! members does the compiler still reject, and with what error", so a loop
//! can pick the next stdlib slice without inspecting bytecode by hand.

use std::path::Path;

use crate::{
    CompileError, JarClass, JarEntrypoint, JarImportOptions, Method, Type, parse_program_custom,
    parse_signature, program_dependencies, read_jar_classpath, render_jar_program,
};
use CompileError::UnsupportedPlatformClass;

/// One method's compile outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodCoverage {
    /// Binary method name (e.g. `isEmpty`).
    pub name: String,
    /// Raw descriptor (e.g. `(Ljava/lang/CharSequence;)Z`).
    pub descriptor: String,
    /// `ok` when the method compiles, otherwise the failure reason.
    pub status: MethodStatus,
}

/// Why a method failed to compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MethodStatus {
    Ok,
    /// A `java/…` class the compiler does not model.
    UnsupportedPlatformClass {
        class: String,
    },
    /// Any other compile failure (unsupported opcode, invalid stack, …).
    /// The string is the `Display` of the underlying [`CompileError`].
    Error {
        detail: String,
    },
}

impl MethodStatus {
    fn from_error(error: &CompileError) -> Self {
        match error {
            UnsupportedPlatformClass { class, .. } => Self::UnsupportedPlatformClass {
                class: class.clone(),
            },
            other => Self::Error {
                detail: other.to_string(),
            },
        }
    }

    /// `true` when the method compiled.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// One class's compile coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassCoverage {
    /// Binary class name (e.g. `org/apache/commons/lang3/StringUtils`).
    pub name: String,
    pub methods: Vec<MethodCoverage>,
    /// Distinct `java/…` classes referenced anywhere in the class (member
    /// refs, signatures, supertypes), sorted.
    pub jdk_dependencies: Vec<String>,
}

/// Whole-JAR coverage for every class on a JAR classpath.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JarCoverage {
    pub classes: Vec<ClassCoverage>,
}

impl JarCoverage {
    /// Count of classes whose selected method compiles.
    #[must_use]
    pub fn ok_classes(&self) -> usize {
        self.classes
            .iter()
            .filter(|class| class.methods.iter().all(|method| method.status.is_ok()))
            .count()
    }

    /// Total number of selected methods.
    #[must_use]
    pub fn total_methods(&self) -> usize {
        self.classes.iter().map(|class| class.methods.len()).sum()
    }

    /// Count of methods that compile.
    #[must_use]
    pub fn ok_methods(&self) -> usize {
        self.classes
            .iter()
            .flat_map(|class| &class.methods)
            .filter(|method| method.status.is_ok())
            .count()
    }

    /// Distinct unmodeled `java/…` classes across the JAR, sorted.
    #[must_use]
    pub fn unmodeled_platform_classes(&self) -> Vec<String> {
        let mut classes: Vec<String> = self
            .classes
            .iter()
            .flat_map(|class| &class.jdk_dependencies)
            .cloned()
            .collect();
        classes.sort();
        classes.dedup();
        classes
    }

    /// Deterministic Markdown report for humans and agent loops.
    #[must_use]
    pub fn to_report(&self, target_release: u16) -> String {
        let mut report = format!(
            "# JAR compile coverage\n\nTarget Java release: {}\n\nClasses: {}, compiled {}, methods compiled {}/{}\n\n",
            target_release,
            self.classes.len(),
            self.ok_classes(),
            self.ok_methods(),
            self.total_methods(),
        );
        report.push_str("## Unmodeled platform classes\n\n");
        let unmodeled = self.unmodeled_platform_classes();
        if unmodeled.is_empty() {
            report.push_str("(none)\n");
        } else {
            for class in &unmodeled {
                report.push_str(&format!("- `{class}`\n"));
            }
        }
        report.push_str("\n## Per-class results\n\n");
        for class in &self.classes {
            report.push_str(&format!("### `{}`\n\n", class.name));
            for method in &class.methods {
                let outcome = match &method.status {
                    MethodStatus::Ok => "ok".to_owned(),
                    MethodStatus::UnsupportedPlatformClass { class } => {
                        format!("unmodeled platform class `{class}`")
                    }
                    MethodStatus::Error { detail } => format!("error: {detail}"),
                };
                report.push_str(&format!(
                    "- `{}`{} — {}\n",
                    method.name, method.descriptor, outcome
                ));
            }
            report.push('\n');
        }
        report
    }
}

/// Compiles one entry method per class across the whole JAR classpath and
/// reports per-class outcomes. The selected entry per class is the first
/// method in sorted order among `()V` or `([Ljava/lang/String;)V` methods,
/// preferring `public static`, then `static`, then any visibility.
///
/// This never executes generated code; each class is compiled in isolation so
/// one class's failure does not hide the others.
pub fn coverage_jars(
    paths: &[impl AsRef<Path>],
    options: JarImportOptions,
) -> Result<JarCoverage, CompileError> {
    let classes = read_jar_classpath(paths, options)?;
    let mut classes_sorted: Vec<(String, &JarClass)> = classes
        .iter()
        .map(|(name, class)| (name.clone(), class))
        .collect();
    classes_sorted.sort_by(|left, right| left.0.cmp(&right.0));
    let mut coverage = Vec::new();
    for (name, class) in &classes_sorted {
        coverage.push(class_coverage(name, &class.bytes)?);
    }
    Ok(JarCoverage { classes: coverage })
}

fn class_coverage(name: &str, bytes: &[u8]) -> Result<ClassCoverage, CompileError> {
    let program = parse_program_custom(bytes)?;
    let jdk_dependencies = {
        let mut dependencies = program_dependencies(&program)?;
        dependencies.sort();
        dependencies.dedup();
        dependencies
            .into_iter()
            .filter(|dependency| dependency.starts_with("java/"))
            .collect()
    };
    let void = parse_signature("()V")
        .expect("()V is a valid descriptor")
        .returns;
    let entry_shape = |method: &Method| {
        (method.signature.returns == void && method.signature.parameters.is_empty())
            || is_string_array_void(method)
    };
    // Coverage target: the first `public static` method (main preferred,
    // then alphabetical) with a signature the compiler can lower. This is
    // the class's representative; running it lowers the class module, its
    // static state, and every platform member the method touches.
    let mut candidates: Vec<Method> = program
        .methods
        .iter()
        .filter(|method| method.is_public && method.is_static && method.name != "<init>")
        .cloned()
        .collect();
    candidates.sort_by_key(|method| {
        (
            method.name != "main",
            !entry_shape(method),
            method.name.clone(),
            method.descriptor.clone(),
        )
    });
    let mut results = Vec::new();
    if let Some(method) = candidates.first() {
        let entry = JarEntrypoint {
            class: name.to_owned(),
            method: method.name.clone(),
            descriptor: method.descriptor.clone(),
        };
        let status = match render_jar_program(&[program], &entry) {
            Ok(_) => MethodStatus::Ok,
            Err(error) => MethodStatus::from_error(&error),
        };
        results.push(MethodCoverage {
            name: method.name.clone(),
            descriptor: method.descriptor.clone(),
            status,
        });
    }
    if results.is_empty() {
        results.push(MethodCoverage {
            name: "(no public static method)".to_owned(),
            descriptor: String::new(),
            status: MethodStatus::Error {
                detail: "class declares no public static method".to_owned(),
            },
        });
    }
    Ok(ClassCoverage {
        name: name.to_owned(),
        methods: results,
        jdk_dependencies,
    })
}

fn is_string_array_void(method: &Method) -> bool {
    method.signature.returns == Type::Void
        && method.signature.parameters.len() == 1
        && matches!(&method.signature.parameters[0], Type::Array(element) if **element == Type::String)
}
