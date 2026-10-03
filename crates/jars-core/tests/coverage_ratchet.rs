//! Coverage ratchet: whole-JAR compile metrics may only improve.
//!
//! The snapshot records the number of classes whose representative method
//! compiles across the real commons-lang3 JAR. When an agent loop expands
//! the stdlib and the number grows, refresh it with:
//!
//! ```text
//! UPDATE_COVERAGE_SNAPSHOT=1 cargo test -p jars-core --test coverage_ratchet
//! ```
//!
//! A shrink fails the test: removing modeled JDK surface is a regression.

use std::fs;
use std::path::{Path, PathBuf};

use jars_core::{JarImportOptions, coverage_jars};

const VENDORED_JAR: &str = "tests/jar-fixtures/third-party/commons-lang3-3.14.0.jar";
const SNAPSHOT: &str = "tests/coverage-snapshot/commons-lang3-3.14.0.txt";

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[test]
fn whole_jar_compile_coverage_never_regresses() {
    let root = crate_root();
    let jar = root.join(VENDORED_JAR);
    let coverage = coverage_jars(&[&jar], JarImportOptions::for_release(21)).unwrap();
    let unmodeled = coverage.unmodeled_platform_classes();
    assert!(!unmodeled.iter().any(|class| class == "java/lang/String"));
    assert!(!unmodeled.iter().any(|class| class == "java/lang/Integer"));
    assert!(!unmodeled.iter().any(|class| class == "java/lang/Long"));
    let actual = format!(
        "classes {}\nok_classes {}\ntotal_methods {}\nok_methods {}\nunmodeled {}\n",
        coverage.classes.len(),
        coverage.ok_classes(),
        coverage.total_methods(),
        coverage.ok_methods(),
        unmodeled.len(),
    );
    let snapshot_path = root.join(SNAPSHOT);
    if std::env::var("UPDATE_COVERAGE_SNAPSHOT").is_ok() {
        fs::create_dir_all(snapshot_path.parent().unwrap()).unwrap();
        fs::write(&snapshot_path, &actual).unwrap();
        println!("coverage snapshot updated:\n{actual}");
        return;
    }
    let expected = fs::read_to_string(&snapshot_path).unwrap_or_else(|error| {
        panic!(
            "missing coverage snapshot {}: {error} (set UPDATE_COVERAGE_SNAPSHOT=1 to create it)",
            snapshot_path.display()
        )
    });
    let parse = |text: &str, key: &str| -> usize {
        text.lines()
            .find(|line| line.starts_with(key))
            .and_then(|line| line.split_whitespace().next_back())
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("snapshot line `{key}` missing"))
    };
    let (expected_ok_classes, actual_ok_classes) =
        (parse(&expected, "ok_classes"), parse(&actual, "ok_classes"));
    let (expected_ok_methods, actual_ok_methods) =
        (parse(&expected, "ok_methods"), parse(&actual, "ok_methods"));
    let (expected_unmodeled, actual_unmodeled) =
        (parse(&expected, "unmodeled"), parse(&actual, "unmodeled"));
    assert!(
        actual_ok_classes >= expected_ok_classes,
        "compiled-class coverage regressed: {actual_ok_classes} < {expected_ok_classes}\n{actual}"
    );
    assert!(
        actual_ok_methods >= expected_ok_methods,
        "compiled-method coverage regressed: {actual_ok_methods} < {expected_ok_methods}\n{actual}"
    );
    assert!(
        actual_unmodeled <= expected_unmodeled,
        "unmodeled platform classes grew: {actual_unmodeled} > {expected_unmodeled}\n{actual}"
    );
}
