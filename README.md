# jars

`jars` translates a deliberately small Java class-file subset into Rust.

`jars-core::compile_class` accepts one default-package `.class` file and emits
a complete Rust binary source file. Generated Java objects are mailbox-backed
actors. Each generated binary creates a shared `Program<S: Spawner>` context;
static methods take that context, and it owns static storage plus lazy
`<clinit>` initialization. The emitted executable uses `jars_runtime::Runtime`,
a deterministic single-thread executor.

Supported generated programs include `System.out.println`, primitive arithmetic,
cross-class static fields, actor-backed object construction, cross-object field
access, virtual calls, integer branch state machines, table/lookup switches, and
the current exception-table state-machine subset (typed catches, catch-all
handlers used for `finally`, rethrow, and failures returned from generated static
calls). Exception tables are retained only while compiling; emitted Rust retains
no class-file or bytecode representation.
Every array descriptor is recursive; generated arrays are typed mailbox actors,
so array aliases can cross object actors without exposing mutable elements.

Generated Java entry points and public calls return
`jars_runtime::JavaResult<T>` (an `anyhow::Result<T>`). The runtime exposes
typed `Error` wrappers for Java-visible arithmetic, null, array, cast, actor,
and class-initialization failures. The compiler remains closed-world and
AOT-only: generated Rust contains no class-file data or bytecode interpreter.
Run the verification suite with:

```sh
cargo test --workspace
```

Generated Rust needs a `jars-runtime` dependency, for example:

```toml
[dependencies]
jars-runtime = { path = "../crates/jars-runtime" }
```
