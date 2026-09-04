//! The single source of truth for the modeled Java standard-library surface.
//!
//! [`java_stdlib!`] is deliberately an X-macro: `jars-runtime` consumes the
//! Rust implementation blocks while `jars-core` consumes the same classes,
//! descriptors, coercions, and lowering expressions.  Adding a Rust-written
//! JDK class therefore does not require separately synchronizing a runtime
//! type and a compiler registry entry.

/// Passes the modeled JDK class declarations to a local code-generation
/// macro.
///
/// The consumer owns how it materializes the declaration.  The runtime emits
/// each `runtime` block; the compiler emits its exact-member lookup table.
/// This keeps the declaration crate dependency-free and prevents either side
/// from depending on the other.
#[macro_export]
macro_rules! java_stdlib {
    ($consumer:ident) => {
        $consumer! {
            class {
                name: "java/lang/Object",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [
                    constructor "<init>" "()V" |_args| None;
                ],
            }
            class {
                // `String` has a dedicated, non-null compiler representation,
                // but its runtime values are the owned `JavaString` below.
                name: "java/lang/String",
                rust_type: Some("jars_runtime::JavaString"),
                coercions: [],
                runtime: {
                    /// An owned Java `String` value.  Values are computed at
                    /// runtime (literals, later transformations), so the
                    /// characters live in a shared `Rc<String>` rather than
                    /// only in `'static` literals.  Strings are never null in
                    /// the modeled subset, so the representation is not
                    /// optional; actors receive clones that share characters.
                    #[derive(Clone, Debug)]
                    pub struct JavaString(std::rc::Rc<String>);

                    impl JavaString {
                        #[must_use]
                        pub fn new(value: impl Into<String>) -> Self {
                            Self(std::rc::Rc::new(value.into()))
                        }

                        #[must_use]
                        pub fn as_str(&self) -> &str {
                            &self.0
                        }

                        /// Java reference identity for the modeled slice.  The
                        /// modeled strings are interned literals, so the
                        /// compiler lowers `if_acmpeq` by content; distinct
                        /// clones of one value are also the same reference.
                        #[must_use]
                        pub fn __same(&self, other: &Self) -> bool {
                            std::rc::Rc::ptr_eq(&self.0, &other.0) || **self.0 == **other.0
                        }
                    }

                    impl std::fmt::Display for JavaString {
                        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                            f.write_str(&self.0)
                        }
                    }

                    impl From<JavaString> for String {
                        fn from(value: JavaString) -> String {
                            (*value.0).clone()
                        }
                    }

                    impl CharSequenceValue for JavaString {
                        fn identity(&self) -> CharSequenceIdentity {
                            CharSequenceIdentity::JavaString(std::rc::Rc::clone(&self.0))
                        }

                        fn length(
                            &self,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<i32>>>> {
                            let value = std::rc::Rc::clone(&self.0);
                            Box::pin(async move { Ok(char_sequence_length(&value)) })
                        }

                        fn char_at(
                            &self,
                            index: i32,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<u16>>>> {
                            let value = std::rc::Rc::clone(&self.0);
                            Box::pin(async move { char_sequence_char_at(&value, index) })
                        }
                    }
                },
                members: [],
            }
            class {
                name: "java/lang/CharSequence",
                rust_type: Some("jars_runtime::CharSequence"),
                coercions: [
                    "java/lang/String" => |value| {
                        format!(
                            "Some(jars_runtime::CharSequence::from_java_string({value}))"
                        )
                    },
                    "java/lang/StringBuilder" => |value| {
                        format!(
                            "{value}.map(jars_runtime::CharSequence::from_string_builder)"
                        )
                    },
                ],
                runtime: {
                    /// A closed, dynamically dispatched Java `CharSequence`.
                    /// Rust-written JDK classes implement
                    /// [`CharSequenceValue`] and add a coercion above; calls
                    /// remain explicit Rust, never JDK reflection or bytecode
                    /// interpretation.
                    #[derive(Clone)]
                    pub struct CharSequence {
                        value: Rc<dyn CharSequenceValue>,
                        identity: CharSequenceIdentity,
                    }

                    /// A Java reference identity usable by Rust-written
                    /// interface implementations without exposing their state.
                    #[derive(Clone)]
                    pub enum CharSequenceIdentity {
                        StaticString { address: usize, length: usize },
                        JavaString(std::rc::Rc<String>),
                        Object(Rc<()>),
                    }

                    impl CharSequenceIdentity {
                        #[must_use]
                        pub fn static_string(value: &'static str) -> Self {
                            Self::StaticString {
                                address: value.as_ptr() as usize,
                                length: value.len(),
                            }
                        }

                        #[must_use]
                        pub fn object() -> Self {
                            Self::Object(Rc::new(()))
                        }

                        #[must_use]
                        pub fn same(&self, other: &Self) -> bool {
                            match (self, other) {
                                (
                                    Self::StaticString {
                                        address: left_address,
                                        length: left_length,
                                    },
                                    Self::StaticString {
                                        address: right_address,
                                        length: right_length,
                                    },
                                ) => left_address == right_address && left_length == right_length,
                                (Self::JavaString(left), Self::JavaString(right)) => {
                                    std::rc::Rc::ptr_eq(left, right)
                                }
                                (Self::Object(left), Self::Object(right)) => Rc::ptr_eq(left, right),
                                _ => false,
                            }
                        }
                    }

                    /// The runtime seam a Rust-written class implements when
                    /// Java verification permits it as a `CharSequence`.
                    /// Implementations choose their own actor protocol; this
                    /// interface never exposes mutable Java object state.
                    pub trait CharSequenceValue {
                        fn identity(&self) -> CharSequenceIdentity;

                        fn length(
                            &self,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<i32>>>>;

                        fn char_at(
                            &self,
                            index: i32,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<u16>>>>;
                    }

                    #[derive(Clone)]
                    struct StringCharSequence(&'static str);

                    impl CharSequenceValue for StringCharSequence {
                        fn identity(&self) -> CharSequenceIdentity {
                            CharSequenceIdentity::static_string(self.0)
                        }

                        fn length(
                            &self,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<i32>>>> {
                            let length = char_sequence_length(self.0);
                            Box::pin(async move { Ok(length) })
                        }

                        fn char_at(
                            &self,
                            index: i32,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<u16>>>> {
                            let value = self.0;
                            Box::pin(async move { char_sequence_char_at(value, index) })
                        }
                    }

                    impl CharSequence {
                        #[must_use]
                        pub fn from_string(value: &'static str) -> Self {
                            Self::from_rust(StringCharSequence(value))
                        }

                        /// Wraps any Rust-written class that implements this
                        /// Java interface. The implementation remains closed
                        /// at AOT compile time because the compiler emits only
                        /// conversions declared in this same macro input.
                        #[must_use]
                        pub fn from_rust<T>(value: T) -> Self
                        where
                            T: CharSequenceValue + 'static,
                        {
                            Self {
                                identity: value.identity(),
                                value: Rc::new(value),
                            }
                        }

                        /// Wraps an owned Java string as a `CharSequence`.
                        /// The wrapper shares the string's characters and its
                        /// Java reference identity.
                        #[must_use]
                        pub fn from_java_string(value: JavaString) -> Self {
                            Self::from_rust(value)
                        }

                        #[must_use]
                        pub fn from_string_builder(value: JavaStringBuilder) -> Self {
                            Self::from_rust(value)
                        }

                        pub async fn length(&self) -> JavaResult<i32> {
                            self.value.length().await
                        }

                        pub async fn char_at(&self, index: i32) -> JavaResult<u16> {
                            self.value.char_at(index).await
                        }

                        /// Java reference identity without exposing a
                        /// Rust-written class's private state.
                        #[must_use]
                        pub fn __same(&self, other: &Self) -> bool {
                            self.identity.same(&other.identity)
                        }
                    }

                    /// Java's UTF-16 `CharSequence.length()` value for the
                    /// supported immutable String-backed representation.
                    #[must_use]
                    pub fn char_sequence_length(value: &str) -> i32 {
                        value.encode_utf16().count() as i32
                    }

                    /// Java `CharSequence.charAt` for the supported immutable
                    /// String-backed representation. It indexes UTF-16 code
                    /// units, not Rust Unicode scalar values.
                    pub fn char_sequence_char_at(value: &str, index: i32) -> JavaResult<u16> {
                        let index = usize::try_from(index)
                            .map_err(|_| string_index_out_of_bounds())?;
                        value
                            .encode_utf16()
                            .nth(index)
                            .ok_or_else(string_index_out_of_bounds)
                    }
                },
                members: [
                    instance "length" "()I" |receiver, _args| {
                        format!("{receiver}.length().await?")
                    };
                    instance "charAt" "(I)C" |receiver, args| {
                        format!("{receiver}.char_at({}).await?", args[0])
                    };
                ],
            }
            class {
                // A concrete, Rust-written JDK class. Its mutable Java state
                // remains in the mailbox actor below; the currently modeled
                // constructor and CharSequence reads never expose it.
                name: "java/lang/StringBuilder",
                rust_type: Some("jars_runtime::JavaStringBuilder"),
                coercions: [],
                runtime: {
                    #[derive(Clone)]
                    pub struct JavaStringBuilder {
                        actor: ActorRef<StringBuilderMessage>,
                        identity: CharSequenceIdentity,
                    }

                    enum StringBuilderMessage {
                        Length {
                            reply: Reply<JavaResult<i32>>,
                        },
                        CharAt {
                            index: i32,
                            reply: Reply<JavaResult<u16>>,
                        },
                    }

                    impl JavaStringBuilder {
                        pub fn new<S: Spawner>(
                            spawner: S,
                            source: JavaString,
                        ) -> JavaResult<Self> {
                            let (actor, mailbox) = actor_channel();
                            spawner.spawn(async move {
                                use futures::{FutureExt as _, StreamExt as _};

                                // This is an actor even though no mutator is
                                // in the supported slice yet. Future mutable
                                // methods add message variants; aliases never
                                // receive the state directly.  The characters
                                // are shared through the source string's `Rc`.
                                let state = Rc::new(std::sync::Mutex::new(source));
                                let mut in_flight: FuturesUnordered<
                                    std::pin::Pin<Box<dyn Future<Output = ()>>>,
                                > = FuturesUnordered::new();
                                loop {
                                    let spawn_handler =
                                        |message: StringBuilderMessage,
                                         state: Rc<std::sync::Mutex<JavaString>>| {
                                            Box::pin(async move {
                                                match message {
                                                    StringBuilderMessage::Length { reply } => {
                                                        let length = char_sequence_length(
                                                            state
                                                                .lock()
                                                                .expect("string builder state mutex")
                                                                .as_str(),
                                                        );
                                                        let _ = reply.send(Ok(length));
                                                    }
                                                    StringBuilderMessage::CharAt { index, reply } => {
                                                        let value = char_sequence_char_at(
                                                            state
                                                                .lock()
                                                                .expect("string builder state mutex")
                                                                .as_str(),
                                                            index,
                                                        );
                                                        let _ = reply.send(value);
                                                    }
                                                }
                                            })
                                                as std::pin::Pin<Box<dyn Future<Output = ()>>>
                                        };
                                    if in_flight.is_empty() {
                                        let message = match mailbox.recv().await {
                                            Ok(message) => message,
                                            Err(_) => break,
                                        };
                                        in_flight.push(spawn_handler(message, state.clone()));
                                    } else {
                                        select_biased! {
                                            message = mailbox.recv().fuse() => match message {
                                                Ok(message) => in_flight.push(spawn_handler(message, state.clone())),
                                                Err(_) => break,
                                            },
                                            _ = in_flight.next().fuse() => {},
                                        }
                                    }
                                }
                            });
                            Ok(Self {
                                actor,
                                identity: CharSequenceIdentity::object(),
                            })
                        }

                        #[must_use]
                        pub fn __same(&self, other: &Self) -> bool {
                            self.actor.same(&other.actor)
                        }

                        pub async fn length(&self) -> JavaResult<i32> {
                            let (reply, response) = reply();
                            self.actor.send(StringBuilderMessage::Length { reply }).await?;
                            response.recv().await?
                        }

                        pub async fn char_at(&self, index: i32) -> JavaResult<u16> {
                            let (reply, response) = reply();
                            self.actor
                                .send(StringBuilderMessage::CharAt { index, reply })
                                .await?;
                            response.recv().await?
                        }
                    }

                    impl CharSequenceValue for JavaStringBuilder {
                        fn identity(&self) -> CharSequenceIdentity {
                            self.identity.clone()
                        }

                        fn length(
                            &self,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<i32>>>> {
                            let value = self.clone();
                            Box::pin(async move { value.length().await })
                        }

                        fn char_at(
                            &self,
                            index: i32,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<u16>>>> {
                            let value = self.clone();
                            Box::pin(async move { value.char_at(index).await })
                        }
                    }
                },
                members: [
                    constructor "<init>" "(Ljava/lang/String;)V" |args| {
                        Some(format!(
                            "Some(jars_runtime::JavaStringBuilder::new(program.spawner.clone(), {})?)",
                            args[0]
                        ))
                    };
                    instance "length" "()I" |receiver, _args| {
                        format!("{receiver}.length().await?")
                    };
                    instance "charAt" "(I)C" |receiver, args| {
                        format!("{receiver}.char_at({}).await?", args[0])
                    };
                ],
            }
            class {
                name: "java/lang/Character",
                rust_type: None,
                coercions: [],
                runtime: {
                    /// Java's `Character.isWhitespace(char)` rule for BMP code
                    /// units. The three non-breaking spaces and NEXT LINE are
                    /// deliberately excluded, matching Java rather than
                    /// Rust's broader Unicode whitespace property.
                    pub mod character {
                        #[must_use]
                        pub fn is_whitespace(value: u16) -> bool {
                            match value {
                                0x0009..=0x000d | 0x001c..=0x001f => true,
                                0x0085 | 0x00a0 | 0x2007 | 0x202f => false,
                                _ => char::from_u32(u32::from(value))
                                    .is_some_and(char::is_whitespace),
                            }
                        }
                    }
                },
                members: [
                    static_method "isWhitespace" "(C)Z" |args| {
                        format!("jars_runtime::character::is_whitespace({} as u16)", args[0])
                    };
                ],
            }
            class {
                name: "java/lang/System",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [
                    static_field "out" "Ljava/io/PrintStream;" || {
                        "Some(jars_runtime::PrintStream)".to_owned()
                    };
                ],
            }
            class {
                name: "java/io/PrintStream",
                rust_type: Some("jars_runtime::PrintStream"),
                coercions: [],
                runtime: {
                    /// The Rust representation of `java.io.PrintStream` values
                    /// such as `System.out`. It has no mutable object state.
                    #[derive(Debug, Clone, Copy)]
                    pub struct PrintStream;

                    impl PrintStream {
                        #[must_use]
                        pub fn __same(&self, _other: &Self) -> bool {
                            true
                        }
                    }
                },
                members: [
                    instance "println" "(Z)V" |receiver, args| {
                        format!("jars_runtime::println({})", args[0])
                    };
                    instance "println" "(I)V" |receiver, args| {
                        format!("jars_runtime::println({})", args[0])
                    };
                    instance "println" "(J)V" |receiver, args| {
                        format!("jars_runtime::println({})", args[0])
                    };
                    instance "println" "(F)V" |receiver, args| {
                        format!("jars_runtime::println({})", args[0])
                    };
                    instance "println" "(D)V" |receiver, args| {
                        format!("jars_runtime::println({})", args[0])
                    };
                    instance "println" "(Ljava/lang/String;)V" |receiver, args| {
                        format!("jars_runtime::println({})", args[0])
                    };
                ],
            }
            class {
                name: "java/util/regex/Pattern",
                rust_type: Some("jars_runtime::JavaPattern"),
                coercions: [],
                runtime: {
                    /// An immutable Java regular-expression value retained only
                    /// for supported class initialization. No matching API is
                    /// exposed until a separately triaged slice needs it.
                    #[derive(Clone, Debug)]
                    pub struct JavaPattern {
                        source: JavaString,
                    }

                    impl JavaPattern {
                        #[must_use]
                        pub fn compile(source: JavaString) -> Self {
                            Self { source }
                        }

                        #[must_use]
                        pub fn source(&self) -> JavaString {
                            self.source.clone()
                        }

                        #[must_use]
                        pub fn __same(&self, other: &Self) -> bool {
                            std::ptr::eq(self, other)
                        }
                    }
                },
                members: [
                    static_method "compile" "(Ljava/lang/String;)Ljava/util/regex/Pattern;" |args| {
                        format!("Some(jars_runtime::JavaPattern::compile({}))", args[0])
                    };
                ],
            }
            class {
                name: "java/lang/Math",
                rust_type: None,
                coercions: [],
                runtime: {
                    /// Rust implementations backing `java.lang.Math` static
                    /// methods in the modeled subset.
                    pub mod math {
                        #[must_use]
                        pub fn min(a: i32, b: i32) -> i32 {
                            a.min(b)
                        }
                    }
                },
                members: [
                    static_method "min" "(II)I" |args| {
                        format!("jars_runtime::math::min({}, {})", args[0], args[1])
                    };
                ],
            }
            class {
                name: "java/lang/Throwable",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/Exception",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/RuntimeException",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/ArithmeticException",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/NullPointerException",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/ClassCastException",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/ArrayIndexOutOfBoundsException",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/StringIndexOutOfBoundsException",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/NegativeArraySizeException",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/ArrayStoreException",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/lang/Cloneable",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
            class {
                name: "java/io/Serializable",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [],
            }
        }
    };
}
