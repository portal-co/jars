# jars

`jars` translates a deliberately small Java class-file subset into Rust.

`jars-core::compile_class` accepts one default-package `.class` file and emits
a complete Rust binary source file. Generated Java objects are mailbox-backed
actors and static entry points are generic over `jars_runtime::Spawner`.
The emitted executable uses `jars_runtime::Runtime`, a deterministic
single-thread executor.

The initial supported programs cover `System.out.println`, integer addition,
same-class static calls, actor-backed object construction and virtual calls,
and `int` instance fields. Run the verification suite with:

```sh
cargo test --workspace
```

Generated Rust needs a `jars-runtime` dependency, for example:

```toml
[dependencies]
jars-runtime = { path = "../crates/jars-runtime" }
```
