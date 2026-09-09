use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use jars_core::{
    CompileError, JarEntrypoint, JarImportOptions, compile_jars, compile_jars_with_options,
    triage_jars,
};
use serde::Deserialize;
use tempfile::TempDir;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    name: String,
    entry_class: String,
    entry_method: String,
    entry_descriptor: String,
    expected_stdout: Option<String>,
    expected_exit: Option<i32>,
    expected_error: Option<String>,
    target_release: Option<u16>,
    jars: Vec<JarFixture>,
}

#[derive(Debug, Deserialize)]
struct JarFixture {
    name: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    sources: Vec<PathBuf>,
    artifact: Option<PathBuf>,
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("jar-fixtures")
}

fn manifest(name: &str) -> FixtureManifest {
    let path = fixture_root().join(format!("{name}.toml"));
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn add_class_files(writer: &mut ZipWriter<File>, directory: &Path, prefix: &Path) {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for class in fs::read_dir(directory).unwrap() {
        let class = class.unwrap();
        let name = prefix.join(class.file_name());
        if class.path().is_dir() {
            add_class_files(writer, &class.path(), &name);
            continue;
        }
        if class.path().extension() != Some(OsStr::new("class")) {
            continue;
        }
        writer.start_file(name.to_string_lossy(), options).unwrap();
        writer.write_all(&fs::read(class.path()).unwrap()).unwrap();
    }
}

fn create_jar(classes: &Path, output: &Path) {
    let file = File::create(output).unwrap();
    let mut writer = ZipWriter::new(file);
    add_class_files(&mut writer, classes, Path::new(""));
    writer.finish().unwrap();
}

fn homebrew_javac() -> PathBuf {
    let path = PathBuf::from("/opt/homebrew/opt/openjdk@21/bin/javac");
    assert!(
        path.is_file(),
        "Homebrew OpenJDK 21 is required for JAR fixtures: expected {}",
        path.display()
    );
    path
}

fn minimal_class(name: &str) -> Vec<u8> {
    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_be_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend(0xCAFE_BABEu32.to_be_bytes());
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 52);
    push_u16(&mut bytes, 5);
    bytes.push(1);
    push_u16(&mut bytes, name.len().try_into().unwrap());
    bytes.extend(name.as_bytes());
    bytes.extend([7, 0, 1]);
    bytes.push(1);
    push_u16(&mut bytes, "java/lang/Object".len().try_into().unwrap());
    bytes.extend(b"java/lang/Object");
    bytes.extend([7, 0, 3]);
    push_u16(&mut bytes, 0x0021);
    push_u16(&mut bytes, 2);
    push_u16(&mut bytes, 4);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    bytes
}

fn static_void_class(name: &str, method: &str, code: &[u8]) -> Vec<u8> {
    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_be_bytes());
    }
    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_be_bytes());
    }
    fn push_utf8(bytes: &mut Vec<u8>, value: &str) {
        bytes.push(1);
        push_u16(bytes, value.len().try_into().unwrap());
        bytes.extend(value.as_bytes());
    }

    let mut bytes = Vec::new();
    bytes.extend(0xCAFE_BABEu32.to_be_bytes());
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 52);
    push_u16(&mut bytes, 8);
    push_utf8(&mut bytes, name);
    bytes.extend([7, 0, 1]);
    push_utf8(&mut bytes, "java/lang/Object");
    bytes.extend([7, 0, 3]);
    push_utf8(&mut bytes, method);
    push_utf8(&mut bytes, "()V");
    push_utf8(&mut bytes, "Code");
    push_u16(&mut bytes, 0x0021);
    push_u16(&mut bytes, 2);
    push_u16(&mut bytes, 4);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 0x0009);
    push_u16(&mut bytes, 5);
    push_u16(&mut bytes, 6);
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 7);
    push_u32(&mut bytes, (12 + code.len()).try_into().unwrap());
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u32(&mut bytes, code.len().try_into().unwrap());
    bytes.extend(code);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    bytes
}

fn executable_class(name: &str) -> Vec<u8> {
    static_void_class(name, "run", &[0xb1])
}

fn invokes_static_void_class(name: &str, owner: &str, method: &str) -> Vec<u8> {
    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_be_bytes());
    }
    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_be_bytes());
    }
    fn push_utf8(bytes: &mut Vec<u8>, value: &str) {
        bytes.push(1);
        push_u16(bytes, value.len().try_into().unwrap());
        bytes.extend(value.as_bytes());
    }

    let mut bytes = Vec::new();
    bytes.extend(0xCAFE_BABEu32.to_be_bytes());
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 52);
    push_u16(&mut bytes, 13);
    push_utf8(&mut bytes, name);
    bytes.extend([7, 0, 1]);
    push_utf8(&mut bytes, "java/lang/Object");
    bytes.extend([7, 0, 3]);
    push_utf8(&mut bytes, "run");
    push_utf8(&mut bytes, "()V");
    push_utf8(&mut bytes, "Code");
    push_utf8(&mut bytes, owner);
    bytes.extend([7, 0, 8]);
    push_utf8(&mut bytes, method);
    bytes.extend([12, 0, 10, 0, 6]);
    bytes.extend([10, 0, 9, 0, 11]);
    push_u16(&mut bytes, 0x0021);
    push_u16(&mut bytes, 2);
    push_u16(&mut bytes, 4);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 0x0009);
    push_u16(&mut bytes, 5);
    push_u16(&mut bytes, 6);
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 7);
    push_u32(&mut bytes, 16);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u32(&mut bytes, 4);
    bytes.extend([0xb8, 0, 12, 0xb1]);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    bytes
}

fn write_class_jar(path: &Path, class: &str, bytes: &[u8]) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    writer
        .start_file(
            format!("{class}.class"),
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap();
}

fn write_multi_release_jar(path: &Path, multi_release: bool) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file("META-INF/MANIFEST.MF", options).unwrap();
    let manifest = if multi_release {
        "Manifest-Version: 1.0\nMulti-Release: true\n\n"
    } else {
        "Manifest-Version: 1.0\n\n"
    };
    writer.write_all(manifest.as_bytes()).unwrap();
    writer.start_file("Entry.class", options).unwrap();
    writer.write_all(&executable_class("Entry")).unwrap();
    let mut overlay = executable_class("Entry");
    overlay[6..8].copy_from_slice(&53u16.to_be_bytes());
    writer
        .start_file("META-INF/versions/9/Entry.class", options)
        .unwrap();
    writer.write_all(&overlay).unwrap();
    writer.finish().unwrap();
}

fn write_single_class_jar(path: &Path, class: &str) {
    write_class_jar(path, class, &minimal_class(class));
}

fn build_jars(manifest: &FixtureManifest, temp: &TempDir) -> Vec<PathBuf> {
    let root = fixture_root();
    let mut class_dirs = HashMap::new();
    let mut jars = Vec::new();
    for jar in &manifest.jars {
        if let Some(artifact) = &jar.artifact {
            assert!(
                jar.sources.is_empty(),
                "artifact JAR fixtures cannot also list sources"
            );
            let artifact = root.join(artifact);
            assert!(
                artifact.is_file(),
                "missing vendored JAR fixture {}",
                artifact.display()
            );
            class_dirs.insert(jar.name.clone(), artifact.clone());
            jars.push(artifact);
            continue;
        }
        let classes = temp.path().join(format!("{}-classes", jar.name));
        fs::create_dir(&classes).unwrap();
        let mut javac = Command::new(homebrew_javac());
        javac.args(["--release", "8", "-d"]).arg(&classes);
        if !jar.classpath.is_empty() {
            let entries = jar
                .classpath
                .iter()
                .map(|name| class_dirs.get(name).unwrap())
                .collect::<Vec<_>>();
            javac
                .arg("-classpath")
                .arg(std::env::join_paths(entries).unwrap());
        }
        javac.args(jar.sources.iter().map(|source| root.join(source)));
        let result = javac.output().expect("JDK 21 javac should be available");
        assert!(
            result.status.success(),
            "javac failed for {}: {}",
            jar.name,
            String::from_utf8_lossy(&result.stderr)
        );
        let output = temp.path().join(format!("{}.jar", jar.name));
        create_jar(&classes, &output);
        class_dirs.insert(jar.name.clone(), classes);
        jars.push(output);
    }
    jars
}

fn run_generated(name: &str, generated: &str, temp: &TempDir) -> std::process::Output {
    let package = temp.path().join("generated");
    fs::create_dir_all(package.join("src")).unwrap();
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("jars-runtime");
    fs::write(
        package.join("Cargo.toml"),
        format!(
            "[package]\nname = \"jar-fixture-{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\njars-runtime = {{ path = {:?} }}\n",
            runtime
        ),
    )
    .unwrap();
    fs::write(package.join("src/main.rs"), generated).unwrap();
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--offline", "--manifest-path"])
        .arg(package.join("Cargo.toml"))
        .output()
        .expect("cargo should compile generated Rust");
    output
}

fn run_fixture(manifest_name: &str) {
    let manifest = manifest(manifest_name);
    let temp = tempfile::tempdir().unwrap();
    let jars = build_jars(&manifest, &temp);
    let entry = JarEntrypoint::new(
        &manifest.entry_class,
        &manifest.entry_method,
        &manifest.entry_descriptor,
    );
    match manifest.expected_error.as_deref() {
        Some("unsupported_platform") => {
            assert!(matches!(
                compile_jars(&jars, &entry),
                Err(CompileError::UnsupportedPlatformClass { class, .. }) if class == "java/util/stream/Stream"
            ));
        }
        None => {
            let generated = match manifest.target_release {
                Some(target_release) => compile_jars_with_options(
                    &jars,
                    &entry,
                    JarImportOptions::for_release(target_release),
                )
                .unwrap(),
                None => compile_jars(&jars, &entry).unwrap(),
            };
            if std::env::var("JARS_DUMP").is_ok() {
                std::fs::write(format!("/tmp/{}.rs", manifest.name), &generated).unwrap();
            }
            assert!(!generated.contains("ZipArchive"));
            assert!(!generated.contains("RawInstruction"));
            let output = run_generated(&manifest.name, &generated, &temp);
            assert_eq!(
                output.status.code(),
                Some(manifest.expected_exit.unwrap_or(0)),
                "generated Rust failed for {}: {}\n--- source ---\n{generated}",
                manifest.name,
                String::from_utf8_lossy(&output.stderr),
            );
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                manifest.expected_stdout.unwrap()
            );
        }
        Some(other) => panic!("unknown expected error category `{other}`"),
    }
}

#[test]
fn commons_lang_fixture_has_a_real_jar_parser_triage_snapshot() {
    let manifest = manifest("commons-lang-predicates");
    let temp = tempfile::tempdir().unwrap();
    let jars = build_jars(&manifest, &temp);
    let entry = JarEntrypoint::new(
        &manifest.entry_class,
        &manifest.entry_method,
        &manifest.entry_descriptor,
    );
    let report = triage_jars(
        &jars,
        &entry,
        JarImportOptions::for_release(manifest.target_release.unwrap()),
    )
    .unwrap();
    assert!(report.contains("org/apache/commons/lang3/StringUtils"));
    assert!(report.contains("isEmpty(Ljava/lang/CharSequence;)Z"));
    assert!(report.contains("isBlank(Ljava/lang/CharSequence;)Z"));
    assert!(report.contains("length(Ljava/lang/CharSequence;)I"));
    assert!(report.contains("java/lang/Character"));
    assert!(report.contains("java/lang/StringBuilder"));
    assert!(report.contains("selected JAR release 0"));
    assert!(report.contains("java/util/regex/Pattern"));
}

#[test]
fn manifest_harness_imports_a_reachable_cross_jar_closure_and_runs_it() {
    run_fixture("cross-jar");
}

#[test]
fn manifest_harness_imports_a_real_commons_lang_jar_and_runs_it() {
    run_fixture("commons-lang-empty");
}

#[test]
fn manifest_harness_runs_the_commons_lang_trim_slice() {
    run_fixture("commons-lang-trim");
}

#[test]
fn manifest_harness_runs_the_commons_lang_codepoint_slice() {
    run_fixture("commons-lang-codepoint");
}

#[test]
fn manifest_harness_runs_the_commons_lang_predicates_slice() {
    run_fixture("commons-lang-predicates");
}

#[test]
fn manifest_harness_runs_the_commons_lang_substring_slice() {
    run_fixture("commons-lang-substring");
}

#[test]
fn manifest_harness_runs_the_commons_lang_search_slice() {
    run_fixture("commons-lang-search");
}

#[test]
fn manifest_harness_runs_the_commons_lang_charsequence_slice() {
    run_fixture("commons-lang-charsequence");
}

#[test]
fn manifest_harness_runs_the_commons_lang_arrayfill_slice() {
    run_fixture("commons-lang-arrayfill");
}

#[test]
fn manifest_harness_classifies_unmodeled_platform_dependencies() {
    run_fixture("unsupported-platform");
}

#[test]
fn multi_release_selection_uses_the_explicit_target_release() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("multi-release.jar");
    write_multi_release_jar(&jar, true);
    let entry = JarEntrypoint::new("Entry", "run", "()V");
    let base = triage_jars(&[&jar], &entry, JarImportOptions::for_release(8)).unwrap();
    let overlay = triage_jars(&[&jar], &entry, JarImportOptions::for_release(9)).unwrap();
    assert!(base.contains("Java class-file 52.0, selected JAR release 0"));
    assert!(overlay.contains("Java class-file 53.0, selected JAR release 9"));
}

#[test]
fn multi_release_entries_require_the_manifest_declaration() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("not-multi-release.jar");
    write_multi_release_jar(&jar, false);
    assert!(matches!(
        triage_jars(
            &[&jar],
            &JarEntrypoint::new("Entry", "run", "()V"),
            JarImportOptions::for_release(21),
        ),
        Err(CompileError::Jar { detail, .. })
            if detail == "multi-release class entries require Multi-Release: true"
    ));
}

#[test]
fn jar_reader_reaches_a_declared_entry_without_retaining_the_archive() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("empty.jar");
    write_single_class_jar(&jar, "Empty");
    assert!(matches!(
        compile_jars(&[jar], &JarEntrypoint::new("Empty", "run", "()V")),
        Err(CompileError::MissingEntryMethod {
            class,
            method,
            descriptor,
        }) if class == "Empty" && method == "run" && descriptor == "()V"
    ));
}

#[test]
fn jar_reader_accepts_a_package_qualified_binary_name() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("packaged.jar");
    write_single_class_jar(&jar, "app/Empty");
    assert!(matches!(
        compile_jars(&[jar], &JarEntrypoint::new("app/Empty", "run", "()V")),
        Err(CompileError::MissingEntryMethod { class, .. }) if class == "app/Empty"
    ));
}

#[test]
fn package_qualified_static_entry_is_emitted_with_a_rust_safe_symbol() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("packaged-entry.jar");
    write_class_jar(&jar, "app/Entry", &executable_class("app/Entry"));
    let generated = compile_jars(&[jar], &JarEntrypoint::new("app/Entry", "run", "()V")).unwrap();
    assert!(generated.contains("pub mod __jars_class_6170702f456e747279"));
    assert!(!generated.contains("app/Entry"));
}

#[test]
fn package_qualified_static_calls_link_through_the_closed_jar_classpath() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app.jar");
    let library = temp.path().join("library.jar");
    write_class_jar(
        &app,
        "app/Entry",
        &invokes_static_void_class("app/Entry", "library/Library", "touch"),
    );
    write_class_jar(
        &library,
        "library/Library",
        &static_void_class("library/Library", "touch", &[0xb1]),
    );
    let generated = compile_jars(
        &[app, library],
        &JarEntrypoint::new("app/Entry", "run", "()V"),
    )
    .unwrap();
    assert!(generated.contains("pub mod __jars_class_6c6962726172792f4c696272617279"));
    assert!(generated.contains("super::__jars_class_6c6962726172792f4c696272617279::touch"));
}

#[test]
fn jar_reader_rejects_future_class_file_versions() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("future.jar");
    let mut class = minimal_class("Future");
    class[6..8].copy_from_slice(&71u16.to_be_bytes());
    write_class_jar(&jar, "Future", &class);
    assert!(matches!(
        compile_jars(&[jar], &JarEntrypoint::new("Future", "run", "()V")),
        Err(CompileError::Jar { detail, .. }) if detail.contains("unsupported class-file version 71.0")
    ));
}

#[test]
fn jar_reader_accepts_java_26_class_file_versions() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("java26.jar");
    let mut class = static_void_class("Java26Entry", "run", &[0xb1]);
    class[6..8].copy_from_slice(&70u16.to_be_bytes());
    write_class_jar(&jar, "Java26Entry", &class);
    let generated = compile_jars(&[jar], &JarEntrypoint::new("Java26Entry", "run", "()V")).unwrap();
    assert!(generated.contains("pub mod Java26Entry"));
}
