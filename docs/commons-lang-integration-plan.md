# Apache Commons Lang integration shadow plan

Status: `StringUtils.isEmpty` and `StringUtils.isBlank` slices complete; the
modeled JDK surface is now generated from one shared declaration.

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
- `jars-stdlib::java_stdlib!` is the single description for supported JDK classes. It generates runtime Rust implementations and the compiler's exact-member table, representations, constructors, and verifier-approved reference coercions. The declaration can include ordinary Rust items for a concrete JDK class; interface implementers use generated traits such as `CharSequenceValue` and carry a Java identity without exposing state.
- The supported platform slice now includes generated `CharSequence.length`/UTF-16 `charAt`, Java `Character.isWhitespace(char)`, `PrintStream.println(boolean)`, and initialization-only `Pattern.compile(String)` for the `StringUtils` initializer. `java.lang.StringBuilder` is the first Rust-written concrete JDK class: its state remains a mailbox actor and it coerces to `CharSequence` through the generated table.

## Verified integration slice

The real-JAR fixture compiles an application whose static entry invokes:

```java
StringUtils.isEmpty(null); // true
StringUtils.isEmpty("");   // true
StringUtils.isEmpty(" ");  // false
StringUtils.isBlank(null);  // true
StringUtils.isBlank(" \t");  // true
StringUtils.isBlank("\u00a0"); // false
StringUtils.isBlank("\u1680"); // true
StringUtils.isBlank(" jars "); // false
StringUtils.isBlank(new StringBuilder(" \t")); // true
StringUtils.isBlank(new StringBuilder("jars")); // false
new StringBuilder("😀").length(); // 2 UTF-16 code units
```

The selected closure is `app/CommonsLangApp.run`, its private branchy `builderLength(int)` helper, `StringUtils.isEmpty(CharSequence)`, `StringUtils.isBlank(CharSequence)`, its private `length(CharSequence)` helper, and `StringUtils.<clinit>`. It runs as generated Rust and verifies null, empty, ASCII and Unicode whitespace, Java's non-breaking-space exception, non-blank input, and concrete `StringBuilder` values crossing the `CharSequence` interface. No class file, JAR data, dynamic loading, reflection, or bytecode interpreter is emitted.

Verification completed: `cargo test -p jars-runtime`, `cargo test -p jars-core --lib`, `cargo test -p jars-core --test jar_harness`, `cargo check --workspace`, and focused existing e2e actor/static-initialization tests all pass. The real-JAR harness observes twelve expected boolean lines and two UTF-16 length results, including the typed-frame constructor path.

## Expansion gates

Before each added Commons API or JAR dependency, run:

```sh
cargo run -p jars-core --bin jars-triage -- \
  21 app/CommonsLangApp run '()V' app.jar commons-lang3-3.14.0.jar
```

Record the artifact checksum, selected closure, platform members, and first unsupported operation here. Next candidates are selected string transformations, then separately triaged Commons modules. Pattern matching remains unsupported: the current `JavaPattern` value exists only to preserve the required, deterministic class initialization path.
