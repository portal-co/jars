# Minecraft client worklist

The client JAR and `java.base` jmod stay on the machine that runs this command.
This directory is where the generated API worklist is checked in. Do not commit
the JAR, the jmod, or decompiled game source.

This tree was generated from Prism Launcher's Minecraft 26.3 client,
`minecraft-26.3-client.jar`, class-file version 69. The entrypoint is
`net/minecraft/client/main/Main.main([Ljava/lang/String;)V` (the manifest's
`Main-Class` names `net.minecraft.client.Main`, which is not the class in the
JAR). The goal classpath also includes the 26.3 library JARs that are present
locally, so calls into Gson, Guava, Brigadier, and the other libraries are
walked instead of recorded as builtins. The 26.3 metadata does not list LWJGL.
Eight OS-specific native JARs were not downloaded. Platform classification uses
the local Temurin 21 jmods; the Java 25 runtime shipped for this version has
no jmods. `index.toml` currently reports 71 open classes, 148 open members, and
29 blocked members.

Find the jar's public static `main(String[])`, official or obfuscated, and pass
that binary name. JDK descriptors are read from the constant pool, so the
backlog does not need mappings. `--platform` is what distinguishes JDK natives
from builtins. Without it, every platform member is recorded as a builtin.

```sh
cargo run -p jars-core --bin jars-inventory -- \
  --name minecraft \
  --out goals/minecraft \
  --platform /path/to/java.base.jmod \
  21 <entry-class> main '([Ljava/lang/String;)V' \
  /path/to/client.jar
```

`<entry-class>` uses the JAR's binary name, such as
`net/minecraft/client/main/Main` when that class is the one that actually
declares `main`. Add further goal JARs, including LWJGL, after the client JAR.
Regenerate after implementing a class in `java_stdlib!` or as a runtime native
shim. Members already in the stdlib table leave the open files. Entries with
`reason = "closed-world"` are listed so the gap is visible; they are not
implementation subtasks.

Each remaining `<class>.toml` is one independent subtask. This inventory does
not run the game, link GLFW, or replace Minecraft's loop with Bevy.
