use std::{env, path::PathBuf};

/// Find the Java compiler supplied by the agent runner or the host JDK.
pub fn javac() -> PathBuf {
    let mut candidates = Vec::new();

    for variable in ["JARS_JAVAC", "JAVAC"] {
        if let Some(path) = env::var_os(variable).map(PathBuf::from) {
            if path.components().count() > 1 || path.is_absolute() {
                candidates.push(path);
            } else if let Some(search_path) = env::var_os("PATH") {
                candidates
                    .extend(env::split_paths(&search_path).map(|directory| directory.join(&path)));
            }
        }
    }

    for variable in ["JAVA_HOME", "JDK_HOME", "OPENJDK_HOME"] {
        if let Some(home) = env::var_os(variable).map(PathBuf::from) {
            candidates.push(home.join("bin/javac"));
        }
    }

    candidates.extend([
        PathBuf::from("/opt/homebrew/opt/openjdk@25/bin/javac"),
        PathBuf::from("/usr/local/opt/openjdk@25/bin/javac"),
        PathBuf::from("/opt/homebrew/opt/openjdk@21/bin/javac"),
        PathBuf::from("/usr/local/opt/openjdk@21/bin/javac"),
    ]);
    if let Some(search_path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&search_path).map(|directory| directory.join("javac")));
    }
    candidates.push(PathBuf::from("/usr/bin/javac"));

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .expect("OpenJDK javac is required; set JAVAC or JAVA_HOME")
}
