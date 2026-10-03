//! Whole-JAR compile coverage CLI.
//!
//! Prints a deterministic Markdown report of which classes and entry methods
//! across the entire JAR compile, and which unmodeled `java/…` classes block
//! the rest. Agentic loops and coverage ratchets consume this report.

use std::{path::PathBuf, process::ExitCode};

use jars_core::{JarImportOptions, coverage_jars};

fn usage() -> ! {
    eprintln!("usage: jars-coverage [--json] <target-release> <jar>...");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1);
    let mut json = false;
    let mut positional = Vec::new();
    for argument in arguments {
        match argument.as_str() {
            "--json" => json = true,
            "-h" | "--help" => usage(),
            _ => positional.push(argument),
        }
    }
    let mut values = positional.into_iter();
    let target_release = values.next().unwrap_or_else(|| usage());
    let target_release = target_release.parse::<u16>().unwrap_or_else(|_| usage());
    let jars = values.map(PathBuf::from).collect::<Vec<_>>();
    if jars.is_empty() {
        usage();
    }
    match coverage_jars(&jars, JarImportOptions::for_release(target_release)) {
        Ok(coverage) => {
            if json {
                print!("{}", to_json(&coverage));
            } else {
                print!("{}", coverage.to_report(target_release));
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("jars-coverage: {error}");
            ExitCode::FAILURE
        }
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn to_json(coverage: &jars_core::JarCoverage) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"classes\": [");
    for (class_index, class) in coverage.classes.iter().enumerate() {
        if class_index > 0 {
            out.push(',');
        }
        out.push_str("\n    {\n      \"name\": ");
        out.push_str(&json_string(&class.name));
        out.push_str(",\n      \"jdkDependencies\": [");
        for (index, dependency) in class.jdk_dependencies.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&json_string(dependency));
        }
        out.push_str("],\n      \"methods\": [");
        for (method_index, method) in class.methods.iter().enumerate() {
            if method_index > 0 {
                out.push(',');
            }
            out.push_str("\n        {\n          \"name\": ");
            out.push_str(&json_string(&method.name));
            out.push_str(",\n          \"descriptor\": ");
            out.push_str(&json_string(&method.descriptor));
            out.push_str(",\n          \"status\": ");
            match &method.status {
                jars_core::MethodStatus::Ok => out.push_str("\"ok\""),
                jars_core::MethodStatus::UnsupportedPlatformClass { class } => {
                    out.push_str("{\"kind\":\"unmodeled\",\"class\":");
                    out.push_str(&json_string(class));
                    out.push('}');
                }
                jars_core::MethodStatus::Error { detail } => {
                    out.push_str("{\"kind\":\"error\",\"detail\":");
                    out.push_str(&json_string(detail));
                    out.push('}');
                }
            }
            out.push_str("\n        }");
        }
        out.push_str("\n      ]\n    }");
    }
    out.push_str("\n  ]\n}");
    out
}
