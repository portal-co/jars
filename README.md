# jars

`jars` translates a deliberately small Java class-file subset into Rust.

`jars-core::compile_class` accepts one default-package `.class` file and emits
a complete Rust binary source file. Generated Java objects are mailbox-backed
actors. Each generated binary creates a shared `Program<S: Spawner>` context;
static methods take that context, and it owns static storage plus lazy
`<clinit>` initialization. The emitted executable uses `jars_runtime::Runtime`,
a deterministic single-thread executor.

The initial supported programs cover `System.out.println`, integer addition,
cross-class static fields, actor-backed object construction, cross-object field
access, and virtual calls from static or instance code. The compiler remains
closed-world and AOT-only: generated Rust contains no class-file data or
bytecode interpreter. Run the verification suite with:

```sh
cargo test --workspace
```

Generated Rust needs a `jars-runtime` dependency, for example:

```toml
[dependencies]
jars-runtime = { path = "../crates/jars-runtime" }
```
