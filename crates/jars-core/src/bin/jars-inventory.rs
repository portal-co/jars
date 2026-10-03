//! Writes a checked-in worklist of natives and builtins for a goal binary.
//!
//! Platform JARs and jmods are optional and are used only to tell `ACC_NATIVE`
//! methods from builtins. The goal archive is not copied into the output.

use std::{env, path::PathBuf, process::ExitCode};

use jars_core::{JarEntrypoint, JarImportOptions, inventory_jars, write_inventory};

fn usage() -> ! {
    eprintln!(
        "usage: jars-inventory --name <goal> --out <dir> [--platform <jar-or-jmod>]... <target-release> <entry-class> <entry-method> <descriptor> <jar>..."
    );
    std::process::exit(2);
}

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    let mut name = None;
    let mut out = None;
    let mut platforms = Vec::new();
    let mut positional = Vec::new();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--name" => name = Some(arguments.next().unwrap_or_else(|| usage())),
            "--out" => out = Some(PathBuf::from(arguments.next().unwrap_or_else(|| usage()))),
            "--platform" => {
                platforms.push(PathBuf::from(arguments.next().unwrap_or_else(|| usage())));
            }
            "-h" | "--help" => usage(),
            _ => positional.push(argument),
        }
    }
    let name = name.unwrap_or_else(|| usage());
    let out = out.unwrap_or_else(|| usage());
    let mut values = positional.into_iter();
    let target_release = values.next().unwrap_or_else(|| usage());
    let target_release = target_release.parse::<u16>().unwrap_or_else(|_| usage());
    let class = values.next().unwrap_or_else(|| usage());
    let method = values.next().unwrap_or_else(|| usage());
    let descriptor = values.next().unwrap_or_else(|| usage());
    let jars = values.map(PathBuf::from).collect::<Vec<_>>();
    if jars.is_empty() {
        usage();
    }
    let entry = JarEntrypoint::new(class, method, descriptor);
    match inventory_jars(
        &jars,
        &platforms,
        &entry,
        JarImportOptions::for_release(target_release),
    ) {
        Ok(report) => match write_inventory(&out, &name, &entry, target_release, &report) {
            Ok(()) => {
                println!(
                    "wrote {} open classes ({} open members, {} blocked) to {}",
                    report.open_classes(),
                    report.open_members(),
                    report.blocked_members(),
                    out.display()
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("jars-inventory: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("jars-inventory: {error}");
            ExitCode::FAILURE
        }
    }
}
