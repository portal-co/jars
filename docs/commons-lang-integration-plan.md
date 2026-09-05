# Apache Commons Lang integration shadow plan

Status: `StringUtils.isEmpty`, `isBlank`, the first string-transformation
slice (`isNotEmpty`, `trim`, `trimToNull`, `trimToEmpty`, `upperCase`,
`lowerCase`), the code-point slice (`capitalize`, `uncapitalize`,
`reverse`, `defaultString(String, String)`), and the CharSequence-predicate
plus substring slice (`isAlpha` family, `isNumericSpace`, `isAlphaSpace`,
`substring`, `substringBefore/After(+Last)`, `chop`, `chomp`, `compare`,
`countMatches`, `wrap`, `deleteWhitespace`, `repeat(char, int)`,
`join(int[], char, int, int)`) complete; the modeled JDK surface is generated
from one shared declaration and `Type::String` values are nullable runtime
`JavaString`s.

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
- Java strings are owned runtime `JavaString` values, not only `'static` literals: the compiler keeps a dedicated non-null `Type::String` representation, while the runtime representation participates in the same class table, `CharSequence` coercion, and reference-identity rules as other Rust-written JDK classes.
- The supported platform slice now includes generated `CharSequence.length`/UTF-16 `charAt`, Java `Character.isWhitespace(char)`, `PrintStream.println(boolean)`, and initialization-only `Pattern.compile(String)` for the `StringUtils` initializer. `java.lang.StringBuilder` is the first Rust-written concrete JDK class: its state remains a mailbox actor and it coerces to `CharSequence` through the generated table.
- String transformations: `java/lang/String` declares instance members
  (`trim`, `toUpperCase`, `toLowerCase`, `codePointAt`), the `String(int[],
  int, int)` constructor, and frame slots hold `Option<JavaString>`, so
  `aconst_null`, `ifnull`/`ifnonnull`, and null-return paths compile.
  `Type::Class("java/lang/String")` normalizes onto the `Type::String` slot,
  letting one frame slot carry both descriptor spellings. Stdlib lowerings
  produce the full frame value themselves (nullable strings are `Some(...)`;
  class results are already `Option`), so the compiler pushes them unwrapped.
- Code-point slice: `Character.toTitleCase(int)` (one-to-one mappings with a
  Latin digraph table; Rust's `to_titlecase` is unstable),
  `Character.toLowerCase(int)`, `Character.charCount`, typed-frame
  `newarray int`/`iaload`/`iastore`, `StringBuilder.reverse()`/`toString()`
  as mailbox messages, and a closed-world `jars_runtime::JavaObject` with a
  verifier-approved `String`-to-`Object` widening coercion feeding
  `Objects.toString(Object, String)`. This unblocked `capitalize`,
  `uncapitalize`, `reverse`, and `defaultString(String, String)`. Overload
  name collisions (the generated Rust methods share one name per class, e.g.
  `defaultString(String)` vs `defaultString(String, String)`) remain a
  known limitation, so the fixture exercises the two-argument form.
- Predicate/substring slice: `Character` gains `isLowerCase`,
  `isUpperCase`, `isLetter`, `isDigit`, and `isLetterOrDigit` statics;
  `java/lang/String` declares `length`, `isEmpty`, `charAt`, `indexOf(int)`,
  `indexOf(String)`, `lastIndexOf(String)`, `substring(I)`, `substring(II)`,
  `compareTo`, `concat`, `startsWith`, and `endsWith` with UTF-16 index
  semantics; `String(char[])` and `String(char[], int, int)` constructors,
  typed-frame `newarray char`/`caload`/`castore`, `java/util/Arrays.fill
  ([CC)V`, `StringBuilder.append(char/String/int)` and
  `StringBuilder.substring(II)` unblock the `repeat` and `join` closure.
  Boolean-valued stack operands are now a distinct `Value::Boolean` whose
  text is a Rust `bool`: boolean stdlib lowerings, boolean known-class
  returns, and `println(Z)` all speak the same language, and argument
  binding casts narrow/boolean operands at typed-frame call sites.
  `invokestatic` now also resolves `InterfaceMethodref` constants (JDK
  interfaces expose static methods), and `dup` of array operands emits
  independent handle text instead of cloning a moveable `Option` binding.

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

The string-transformation slice additionally exercises:

```java
StringUtils.isNotEmpty(null);        // false
StringUtils.isNotEmpty("jars");      // true
StringUtils.trim(null);              // null
StringUtils.trim("  jars  ");        // "jars"
StringUtils.trim("\u00a0 x \u00a0");  // unchanged: NBSP > U+0020
StringUtils.trimToNull(null);        // null
StringUtils.trimToNull("  ");        // null
StringUtils.trimToNull(" jars ");    // "jars"
StringUtils.trimToEmpty(null);       // ""
StringUtils.trimToEmpty("  ");       // ""
StringUtils.upperCase(null);         // null
StringUtils.upperCase("jars");       // "JARS"
StringUtils.lowerCase("JaRs");       // "jars"
StringUtils.capitalize(null);       // null
StringUtils.capitalize("jars");     // "Jars"
StringUtils.capitalize("Jars");     // "Jars" (already title-cased)
StringUtils.capitalize("\u01c6ars"); // "\u01c5ars" (dž -> Dž titlecase)
StringUtils.uncapitalize("Jars");   // "jars"
StringUtils.uncapitalize("jars");   // "jars"
StringUtils.uncapitalize("\u01c5ars"); // "\u01c6ars"
StringUtils.reverse(null);          // null
StringUtils.reverse("jars");        // "sraj"
StringUtils.reverse("\ud83d\ude00a"); // "a\ud83d\ude00" (surrogate pair kept)
StringUtils.defaultString(null, "def"); // "def"
StringUtils.defaultString("jars", "def"); // "jars"
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
