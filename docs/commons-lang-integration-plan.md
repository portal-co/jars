# Apache Commons Lang integration shadow plan

Status: initial `StringUtils.isEmpty` slice complete.

## Artifact and reproducibility

- Pinned input: `org.apache.commons:commons-lang3:3.14.0`.
- Vendored JAR: `crates/jars-core/tests/jar-fixtures/third-party/commons-lang3-3.14.0.jar`.
- SHA-256: `7b96bf3ee68949abb5bc465559ac270e0551596fa34523fddf890ec418dde13c`.
- The archive is Apache-2.0, retains its LICENSE/NOTICE, and declares `Multi-Release: true`.
- Fixture apps compile with `/opt/homebrew/opt/openjdk@21/bin/javac --release 8`; JAR import selects Java 21 overlays deterministically.

## Completed foundation

- `jars-triage` uses the owned class-file parser and the same JAR-selection/member-closure logic as compilation. It reports selected members and complete class-wide JDK/non-JDK references before more support is added.
- `JarImportOptions` selects the highest eligible multi-release entry; module descriptors are metadata, never generated classes.
- JAR lowering is member-reachable, with every active class's `<clinit>` retained. Unselected methods may contain unlowered bytecode without expanding the generated program.
- The supported platform slice now includes immutable String-backed `CharSequence.length`, `PrintStream.println(boolean)`, and initialization-only `Pattern.compile(String)` for the `StringUtils` initializer.

## Verified integration slice

The real-JAR fixture compiles an application whose static entry invokes:

```java
StringUtils.isEmpty(null); // true
StringUtils.isEmpty("");   // true
StringUtils.isEmpty(" ");  // false
```

The selected closure is `app/CommonsLangApp.run`, `StringUtils.isEmpty(CharSequence)`, and `StringUtils.<clinit>`. It runs as generated Rust and prints `true`, `true`, `false` on separate lines. No class file, JAR data, dynamic loading, reflection, or bytecode interpreter is emitted.

Verification completed: `cargo test -p jars-core --lib`, `cargo test -p jars-core --test jar_harness`, `cargo check --workspace`, and focused existing e2e actor/static-initialization tests all pass.

## Expansion gates

Before each added Commons API or JAR dependency, run:

```sh
cargo run -p jars-core --bin jars-triage -- \
  21 app/CommonsLangApp run '()V' app.jar commons-lang3-3.14.0.jar
```

Record the artifact checksum, selected closure, platform members, and first unsupported operation here. Next candidates are `StringUtils.isBlank` (character access plus Unicode whitespace), selected string transformations, then separately triaged Commons modules. Pattern matching remains unsupported: the current `JavaPattern` value exists only to preserve the required, deterministic class initialization path.
