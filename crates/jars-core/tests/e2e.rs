use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use jars_core::{compile_class, compile_classes};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.java"))
}

fn compile_java(name: &str, temp: &TempDir) -> Vec<u8> {
    let classes = temp.path().join("classes");
    fs::create_dir(&classes).unwrap();
    let output = Command::new("javac")
        .arg("-d")
        .arg(&classes)
        .arg(fixture(name))
        .output()
        .expect("JDK 21 javac should be available for the e2e fixtures");
    assert!(
        output.status.success(),
        "javac failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read(classes.join(format!("{name}.class"))).unwrap()
}

fn compile_and_run(name: &str) -> (String, String) {
    let temp = tempfile::tempdir().unwrap();
    let generated = compile_class(&compile_java(name, &temp)).unwrap();
    let package = temp.path().join("generated");
    fs::create_dir_all(package.join("src")).unwrap();
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("jars-runtime");
    fs::write(
        package.join("Cargo.toml"),
        format!(
            "[package]\nname = \"generated-{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\njars-runtime = {{ path = {:?} }}\n",
            runtime
        ),
    )
    .unwrap();
    fs::write(package.join("src/main.rs"), &generated).unwrap();
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--offline", "--manifest-path"])
        .arg(package.join("Cargo.toml"))
        .output()
        .expect("cargo should compile generated Rust");
    assert!(
        output.status.success(),
        "generated Rust failed for {name}: {}\n--- source ---\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
    (String::from_utf8(output.stdout).unwrap(), generated)
}

fn compile_set_and_run(names: &[&str]) -> (String, String) {
    let temp = tempfile::tempdir().unwrap();
    let classes = temp.path().join("classes");
    fs::create_dir(&classes).unwrap();
    let mut command = Command::new("javac");
    command.arg("-d").arg(&classes);
    for name in names {
        command.arg(fixture(name));
    }
    let output = command.output().expect("JDK 21 javac should be available");
    assert!(
        output.status.success(),
        "javac failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let class_bytes = names
        .iter()
        .map(|name| fs::read(classes.join(format!("{name}.class"))).unwrap())
        .collect::<Vec<_>>();
    let inputs = class_bytes.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let generated = compile_classes(&inputs).unwrap();
    let package = temp.path().join("generated");
    fs::create_dir_all(package.join("src")).unwrap();
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("jars-runtime");
    fs::write(
        package.join("Cargo.toml"),
        format!("[package]\nname = \"generated-set\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\njars-runtime = {{ path = {:?} }}\n", runtime),
    ).unwrap();
    fs::write(package.join("src/main.rs"), &generated).unwrap();
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--offline", "--manifest-path"])
        .arg(package.join("Cargo.toml"))
        .output()
        .expect("cargo should compile generated Rust");
    assert!(
        output.status.success(),
        "generated Rust failed: {}\n--- source ---\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
    (String::from_utf8(output.stdout).unwrap(), generated)
}

#[test]
fn hello_world_runs_after_java_to_rust_compilation() {
    assert_eq!(compile_and_run("Hello").0, "Hello, world!\n");
}

#[test]
fn static_add_is_public_and_runs() {
    let (stdout, generated) = compile_and_run("Add");
    assert_eq!(stdout, "42\n");
    assert!(generated.contains("pub async fn add"));
}

#[test]
fn add_factory_is_an_actor_backed_object() {
    let (stdout, generated) = compile_and_run("AddFactory");
    assert_eq!(stdout, "42\n");
    assert!(generated.contains("pub struct AddFactory"));
    assert!(generated.contains("pub async fn new"));
    assert!(generated.contains("pub async fn add"));
}

#[test]
fn accumulator_serializes_private_state_through_its_actor() {
    let (stdout, generated) = compile_and_run("Accumulator");
    assert_eq!(stdout, "42\n");
    assert!(generated.contains("total: i32") || generated.contains("total : i32"));
    assert!(generated.contains("state.total ="));
}

#[test]
fn arithmetic_branches_and_back_edges_are_emitted_as_aot_state_machines() {
    let (stdout, generated) = compile_and_run("Arithmetic");
    assert_eq!(stdout, "63\n");
    assert!(generated.contains("loop {"));
    assert!(generated.contains("match pc"));
    assert!(!generated.contains("RawInstruction"));
}

#[test]
fn closed_class_set_links_cross_class_static_calls_aot() {
    let (stdout, generated) = compile_set_and_run(&["CrossMain", "Helper"]);
    assert_eq!(stdout, "42\n");
    assert!(generated.contains("pub mod CrossMain"));
    assert!(generated.contains("pub mod Helper"));
    assert!(generated.contains("super::Helper::add"));
}

#[test]
fn closed_class_set_constructs_and_calls_a_cross_class_actor() {
    let (stdout, generated) = compile_set_and_run(&["CrossObjectMain", "Counter"]);
    assert_eq!(stdout, "42\n");
    assert!(generated.contains("super::Counter::Counter::new"));
    assert!(generated.contains("pub struct Counter"));
}

#[test]
fn unsupported_bytecode_has_a_source_location() {
    let temp = tempfile::tempdir().unwrap();
    let error = compile_class(&compile_java("Unsupported", &temp)).unwrap_err();
    assert!(matches!(
        error,
        jars_core::CompileError::Unsupported { class, method, .. }
            if class == "Unsupported" && method == "main"
    ));
}

#[test]
fn missing_entry_point_is_reported() {
    let temp = tempfile::tempdir().unwrap();
    assert!(matches!(
        compile_class(&compile_java("MissingMain", &temp)),
        Err(jars_core::CompileError::MissingEntryPoint { class }) if class == "MissingMain"
    ));
}
