use std::{env, path::PathBuf, process::ExitCode};

use jars_core::{JarEntrypoint, JarImportOptions, triage_jars};

fn usage() -> ! {
    eprintln!(
        "usage: jars-triage <target-release> <entry-class> <entry-method> <descriptor> <jar>..."
    );
    std::process::exit(2);
}

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    let target_release = arguments.next().unwrap_or_else(|| usage());
    let target_release = target_release.parse::<u16>().unwrap_or_else(|_| usage());
    let class = arguments.next().unwrap_or_else(|| usage());
    let method = arguments.next().unwrap_or_else(|| usage());
    let descriptor = arguments.next().unwrap_or_else(|| usage());
    let jars = arguments.map(PathBuf::from).collect::<Vec<_>>();
    if jars.is_empty() {
        usage();
    }
    match triage_jars(
        &jars,
        &JarEntrypoint::new(class, method, descriptor),
        JarImportOptions::for_release(target_release),
    ) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("jars-triage: {error}");
            ExitCode::FAILURE
        }
    }
}
