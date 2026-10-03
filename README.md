# jars

`jars` translates a deliberately small Java bytecode subset from Java class
files through Java 26 (non-preview class-file version 70.0) into Rust.  Its
owned class-file reader validates the current constant-pool and attribute
layout directly; it does not depend on a JVM, reflection, or a bytecode
interpreter.  Format acceptance is intentionally broader than lowering:
valid Java 26 instructions or JDK APIs outside the modeled AOT subset receive
compiler diagnostics rather than being interpreted at runtime.

`jars-core::compile_class` accepts one default-package `.class` file and emits
a complete Rust binary source file. The default object model is mailbox-backed
actors (`ObjectModel::Task`). `ObjectModel::Object` emits synchronous
`Rc<RefCell<_>>` calls, and `ObjectModel::Entity` uses the same calls with a
Bevy entity id stored by `jars-bevy`. Each generated binary creates a shared
`Program` context; static methods take that context, and it owns static storage
plus lazy `<clinit>` initialization. The task host uses `jars_runtime::Runtime`,
a deterministic single-thread executor.

`jars-core::compile_jars` accepts an explicitly ordered JAR classpath and a
`JarEntrypoint` (`class`, `method`, and JVM descriptor). It indexes archive
entries, follows the reachable closed class set from that entrypoint, and emits
only the selected classes. Duplicate and missing classes, malformed
multi-release layouts, and unmodeled Java platform classes produce structured
compiler diagnostics.
`compile_jars_with_options` accepts an explicit Java feature release and uses
it to choose the highest eligible multi-release class entry deterministically;
the default importer retains Java 8/base-entry selection. Member reachability
is at method granularity, while required class initialization remains explicit.
The JAR is an import format only: neither the generated Rust nor its runtime
retains archive/class-file data or dynamically loads classes. Package-qualified
class names are retained as JVM identities and encoded only for Rust symbols;
the modeled platform surface is intentionally limited to the APIs already
lowered by the compiler.

Supported generated programs include `System.out.println`, primitive arithmetic,
cross-class static fields, actor-backed object construction, cross-object field
access, virtual calls, integer branch state machines, table/lookup switches, and
the current exception-table state-machine subset (typed catches, catch-all
handlers used for `finally`, rethrow, and failures returned from generated static
calls). Exception tables are retained only while compiling; emitted Rust retains
no class-file or bytecode representation.

Control-flow and exception-table bodies share an emitted typed activation frame
for supported numeric values, strings, closed-world object references, recursive
array references, and throwable/null operands. Framed field reads/writes and
virtual calls relay through the target actor; a state lock is acquired only for
direct local access and is never held over that relay. The frame also preserves
actor-address identity for supported object/array reference comparisons. It is
reused by static methods, actor method handlers, and supported `<clinit>` bodies;
its state remains local to one generated invocation and does not expose actor
state. Every array descriptor is recursive. On the task host, generated arrays
are typed mailbox actors, so array aliases can cross object actors without
exposing mutable elements. The object and entity hosts use the synchronous
`JavaArray` store instead.

Generated Java entry points and public calls return
`jars_runtime::JavaResult<T>` (an `anyhow::Result<T>`). The runtime exposes
typed `Error` wrappers for Java-visible arithmetic, null, array, cast, actor,
and class-initialization failures. The compiler remains closed-world and
AOT-only: generated Rust contains no class-file data or bytecode interpreter.

## Generated JDK standard library

`jars-stdlib::java_stdlib!` is the single declaration of modeled JDK classes.
The runtime macro consumer emits Rust implementations; the compiler consumer
emits exact JVM member descriptors, lowering expressions, Rust
representations, constructors, and closed reference coercions. A declaration
can contain ordinary Rust items for a concrete JDK class, so adding one cannot
silently leave its runtime and compiler registrations out of sync.

The initial concrete example is `java.lang.StringBuilder`. Its generated Rust
implementation is mailbox-backed and implements the generated
`CharSequenceValue` interface, allowing only declaration-approved AOT
coercions to `CharSequence`. Java object state never crosses that interface or
escapes the actor. `java.lang.String` values are likewise owned runtime
`JavaString`s rather than only compile-time literals, so computed strings can
flow through the same declarations.

Run the verification suite with:

```sh
cargo test --workspace
```

The JAR fixture harness compiles source fixtures into separate archives, imports
them through the public API, then compiles and runs the generated Rust. Run it
directly with:

```sh
cargo test -p jars-core --test jar_harness
```

Use the parser-backed triage helper before adding a real third-party JAR or a
new language/runtime dependency. It reports the selected member closure and
class-wide JDK/non-JDK references using the same importer as compilation:

```sh
cargo run -p jars-core --bin jars-triage -- \
  21 app/Entry run '()V' app.jar library.jar
```

`jars-inventory` writes the open native and builtin members of that same
entrypoint closure. Finished `java_stdlib!` members drop out on regeneration.
`goals/minecraft/README.md` is the command for a local client JAR.

```sh
cargo run -p jars-core --bin jars-inventory -- \
  --name example --out goals/example \
  --platform /path/to/java.base.jmod \
  21 app/Entry main '([Ljava/lang/String;)V' app.jar
```

Generated Rust needs a `jars-runtime` dependency, for example:

```toml
[dependencies]
jars-runtime = { path = "../crates/jars-runtime" }
```
