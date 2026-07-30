# Jars design constraints

- Compile a closed set of default-package Java class files ahead of time. The
  generated Rust must not retain class files, interpret bytecode, load classes
  dynamically, or use reflection.
- Preserve the actor model. Java object state is private to its mailbox-backed
  actor; cross-object field and method access must use messages rather than
  exposing mutable state.
- Static state belongs to the generated program context. Keep initialization
  explicit and deterministic, including supported `<clinit>` behavior and
  cached initialization failure.
- When an actor operation needs another actor, lower it as async message work.
  Generated actor loops use `FuturesUnordered` to retain multiple in-flight
  handlers. Their `State` is an internal `Rc<Mutex<_>>`, shared only among that
  actor's handlers; acquire a guard only for an individual state access and
  never hold it across an `await`. Do not introduce shared mutable object state
  outside the actor or thread-local Java globals.
- Extend the supported bytecode subset by emitting more Rust, never by adding
  a runtime bytecode interpreter.
