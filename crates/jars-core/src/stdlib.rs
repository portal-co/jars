//! A small, data-driven registry of JDK stdlib classes/members the compiler
//! knows how to lower to Rust.  Both codegen engines in `lib.rs` consult this
//! registry before falling back to the closed-world `known_classes` path, so
//! expanding JDK support is "add a `StdEntry`", not "add a match arm".

/// How a registered member is lowered to a Rust expression/statement.
///
/// `Constructor`, `StaticMethod`, and `InstanceMethod` closures receive
/// already-lowered Rust argument expressions and return the full call
/// expression text (never a bare `receiver.method(args)` template — the
/// closure owns the entire expression, matching the shape the previous
/// hand-written special cases already produced).
pub(crate) enum StdMember {
    /// `lower(args)` returns `None` when the receiver's existing storage
    /// already suffices and no allocation/statement should be emitted (this
    /// is exactly `Object.<init>`'s shape); `Some(expr)` when `expr`
    /// constructs the new instance.
    Constructor {
        lower: fn(args: &[String]) -> Option<String>,
    },
    /// Static fields take no arguments and have no receiver.
    StaticField { lower: fn() -> String },
    StaticMethod {
        lower: fn(args: &[String]) -> String,
    },
    InstanceMethod {
        lower: fn(receiver: &str, args: &[String]) -> String,
    },
}

pub(crate) struct StdEntry {
    pub name: &'static str,
    /// Exact JVM member descriptor, e.g. `"(I)V"`.  The class file has
    /// already resolved overloads by the time a `MemberRef` reaches the
    /// compiler, so exact matching is sufficient — no Java-level overload
    /// resolution is needed here.
    pub descriptor: &'static str,
    pub member: StdMember,
}

pub(crate) struct StdClass {
    /// JVM binary name, e.g. `"java/io/PrintStream"`.
    pub name: &'static str,
    /// The Rust path instances are represented as, when this class is ever
    /// held as a value (e.g. `System.out`).  `None` for classes that are
    /// only ever legal as a type/dependency/catch-type name (the exception
    /// hierarchy, `Cloneable`, `Serializable`, `String` — which keeps its
    /// own dedicated `Type::String` representation instead).
    pub rust_type: Option<&'static str>,
    pub members: &'static [StdEntry],
}

static OBJECT_INIT: &[StdEntry] = &[StdEntry {
    name: "<init>",
    descriptor: "()V",
    member: StdMember::Constructor {
        lower: |_args| None,
    },
}];

static SYSTEM_OUT: &[StdEntry] = &[StdEntry {
    name: "out",
    descriptor: "Ljava/io/PrintStream;",
    member: StdMember::StaticField {
        lower: || "Some(jars_runtime::PrintStream)".to_owned(),
    },
}];

fn println_lower(_receiver: &str, args: &[String]) -> String {
    format!("jars_runtime::println({})", args[0])
}

static PRINTLN_ENTRIES: &[StdEntry] = &[
    StdEntry {
        name: "println",
        descriptor: "(Z)V",
        member: StdMember::InstanceMethod {
            lower: println_lower,
        },
    },
    StdEntry {
        name: "println",
        descriptor: "(I)V",
        member: StdMember::InstanceMethod {
            lower: println_lower,
        },
    },
    StdEntry {
        name: "println",
        descriptor: "(J)V",
        member: StdMember::InstanceMethod {
            lower: println_lower,
        },
    },
    StdEntry {
        name: "println",
        descriptor: "(F)V",
        member: StdMember::InstanceMethod {
            lower: println_lower,
        },
    },
    StdEntry {
        name: "println",
        descriptor: "(D)V",
        member: StdMember::InstanceMethod {
            lower: println_lower,
        },
    },
    StdEntry {
        name: "println",
        descriptor: "(Ljava/lang/String;)V",
        member: StdMember::InstanceMethod {
            lower: println_lower,
        },
    },
];

static MATH_MIN: &[StdEntry] = &[StdEntry {
    name: "min",
    descriptor: "(II)I",
    member: StdMember::StaticMethod {
        lower: |args| format!("jars_runtime::math::min({}, {})", args[0], args[1]),
    },
}];

pub(crate) static CLASSES: &[StdClass] = &[
    StdClass {
        name: "java/lang/Object",
        rust_type: None,
        members: OBJECT_INIT,
    },
    // Type-only: `Type::String` already models String values directly.
    StdClass {
        name: "java/lang/String",
        rust_type: None,
        members: &[],
    },
    // The first Commons Lang slice treats CharSequence values as immutable
    // Strings.  Other CharSequence implementations remain outside the AOT
    // subset until their concrete representation is modeled.
    StdClass {
        name: "java/lang/CharSequence",
        rust_type: Some("&'static str"),
        members: &[
            StdEntry {
                name: "length",
                descriptor: "()I",
                member: StdMember::InstanceMethod {
                    lower: |receiver, _args| format!("{receiver}.len() as i32"),
                },
            },
            StdEntry {
                name: "charAt",
                descriptor: "(I)C",
                member: StdMember::InstanceMethod {
                    lower: |receiver, args| {
                        format!(
                            "jars_runtime::char_sequence_char_at({receiver}, {})?",
                            args[0]
                        )
                    },
                },
            },
        ],
    },
    StdClass {
        name: "java/lang/Character",
        rust_type: None,
        members: &[StdEntry {
            name: "isWhitespace",
            descriptor: "(C)Z",
            member: StdMember::StaticMethod {
                lower: |args| format!("jars_runtime::character::is_whitespace({} as u16)", args[0]),
            },
        }],
    },
    StdClass {
        name: "java/lang/System",
        rust_type: None,
        members: SYSTEM_OUT,
    },
    StdClass {
        name: "java/io/PrintStream",
        rust_type: Some("jars_runtime::PrintStream"),
        members: PRINTLN_ENTRIES,
    },
    // This is deliberately initialization-only support.  Commons Lang's
    // StringUtils eagerly creates one immutable Pattern in <clinit>; no regex
    // matching API is exposed until a separately triaged slice needs it.
    StdClass {
        name: "java/util/regex/Pattern",
        rust_type: Some("jars_runtime::JavaPattern"),
        members: &[StdEntry {
            name: "compile",
            descriptor: "(Ljava/lang/String;)Ljava/util/regex/Pattern;",
            member: StdMember::StaticMethod {
                lower: |args| format!("Some(jars_runtime::JavaPattern::compile({}))", args[0]),
            },
        }],
    },
    StdClass {
        name: "java/lang/Math",
        rust_type: None,
        members: MATH_MIN,
    },
    // Type-only classes: legal as dependency/catch-type names, no modeled
    // members yet.  Kept so `is_known_type` matches the previous
    // `modeled_platform_class` allowlist exactly.
    StdClass {
        name: "java/lang/Throwable",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/Exception",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/RuntimeException",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/ArithmeticException",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/NullPointerException",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/ClassCastException",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/ArrayIndexOutOfBoundsException",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/StringIndexOutOfBoundsException",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/NegativeArraySizeException",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/ArrayStoreException",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/lang/Cloneable",
        rust_type: None,
        members: &[],
    },
    StdClass {
        name: "java/io/Serializable",
        rust_type: None,
        members: &[],
    },
];

/// Registry membership check — replaces the previous flat
/// `modeled_platform_class` allowlist used by the JAR-import dependency walk.
pub(crate) fn is_known_type(class: &str) -> bool {
    CLASSES.iter().any(|entry| entry.name == class)
}

/// Class-level lookup, used by `Type::rust()` to find a registered class's
/// Rust representation.
pub(crate) fn class(class: &str) -> Option<&'static StdClass> {
    CLASSES.iter().find(|entry| entry.name == class)
}

/// Member lookup by exact owner/name/descriptor, tried by both codegen
/// engines before falling back to the closed-world `known_classes` path.
pub(crate) fn member(class: &str, name: &str, descriptor: &str) -> Option<&'static StdMember> {
    self::class(class)?
        .members
        .iter()
        .find(|entry| entry.name == name && entry.descriptor == descriptor)
        .map(|entry| &entry.member)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn println_overloads_match_by_exact_descriptor() {
        assert!(matches!(
            member("java/io/PrintStream", "println", "(I)V"),
            Some(StdMember::InstanceMethod { .. })
        ));
        assert!(matches!(
            member("java/io/PrintStream", "println", "(Z)V"),
            Some(StdMember::InstanceMethod { .. })
        ));
    }

    #[test]
    fn object_init_is_a_no_op_constructor() {
        let Some(StdMember::Constructor { lower }) = member("java/lang/Object", "<init>", "()V")
        else {
            panic!("expected a registered Object.<init> constructor");
        };
        assert_eq!(lower(&[]), None);
    }

    #[test]
    fn type_only_classes_have_no_members() {
        assert!(member("java/lang/String", "length", "()I").is_none());
    }

    #[test]
    fn is_known_type_matches_the_previous_allowlist() {
        assert!(is_known_type("java/io/PrintStream"));
        assert!(is_known_type("java/lang/Throwable"));
        assert!(!is_known_type("java/util/Objects"));
    }

    #[test]
    fn rust_type_is_set_only_for_representable_classes() {
        assert_eq!(
            class("java/io/PrintStream").and_then(|entry| entry.rust_type),
            Some("jars_runtime::PrintStream")
        );
        assert_eq!(
            class("java/lang/Throwable").and_then(|entry| entry.rust_type),
            None
        );
    }

    #[test]
    fn math_min_is_a_static_method() {
        let Some(StdMember::StaticMethod { lower }) = member("java/lang/Math", "min", "(II)I")
        else {
            panic!("expected a registered Math.min static method");
        };
        assert_eq!(
            lower(&["1".to_owned(), "2".to_owned()]),
            "jars_runtime::math::min(1, 2)"
        );
    }
}
