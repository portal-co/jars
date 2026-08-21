use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use jars_core::{CompileError, JarEntrypoint, compile_jars};
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
    jars: Vec<JarFixture>,
}

#[derive(Debug, Deserialize)]
struct JarFixture {
    name: String,
    #[serde(default)]
    classpath: Vec<String>,
    sources: Vec<PathBuf>,
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

fn create_jar(classes: &Path, output: &Path) {
    let file = File::create(output).unwrap();
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for class in fs::read_dir(classes).unwrap() {
        let class = class.unwrap();
        if class.path().extension() != Some(OsStr::new("class")) {
            continue;
        }
        writer
            .start_file(class.file_name().to_string_lossy(), options)
            .unwrap();
        writer.write_all(&fs::read(class.path()).unwrap()).unwrap();
    }
    writer.finish().unwrap();
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

fn write_single_class_jar(path: &Path, class: &str) {
    write_class_jar(path, class, &minimal_class(class));
}

fn build_jars(manifest: &FixtureManifest, temp: &TempDir) -> Vec<PathBuf> {
    let root = fixture_root();
    let mut class_dirs = HashMap::new();
    let mut jars = Vec::new();
    for jar in &manifest.jars {
        let classes = temp.path().join(format!("{}-classes", jar.name));
        fs::create_dir(&classes).unwrap();
        let mut javac = Command::new("javac");
        javac.arg("-d").arg(&classes);
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
                Err(CompileError::UnsupportedPlatformClass { class, .. }) if class == "java/util/Objects"
            ));
        }
        None => {
            let generated = compile_jars(&jars, &entry).unwrap();
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
fn manifest_harness_imports_a_reachable_cross_jar_closure_and_runs_it() {
    run_fixture("cross-jar");
}

#[test]
fn manifest_harness_classifies_unmodeled_platform_dependencies() {
    run_fixture("unsupported-platform");
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
fn jar_reader_rejects_future_class_file_versions() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("future.jar");
    let mut class = minimal_class("Future");
    class[6..8].copy_from_slice(&66u16.to_be_bytes());
    write_class_jar(&jar, "Future", &class);
    assert!(matches!(
        compile_jars(&[jar], &JarEntrypoint::new("Future", "run", "()V")),
        Err(CompileError::Jar { detail, .. }) if detail.contains("unsupported class-file version 66.0")
    ));
}
