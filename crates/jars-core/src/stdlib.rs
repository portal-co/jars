//! Compiler-side code generated from [`jars_stdlib::java_stdlib`].
//!
//! The declaration itself lives in the dependency-free `jars-stdlib` crate.
//! Its runtime consumer emits the Rust implementations while this consumer
//! emits exact JVM-member lowering metadata. No class needs a second,
//! hand-maintained registration here.

/// How a registered member is lowered to a Rust expression or statement.
pub(crate) enum StdMember {
    /// `None` means the receiver's existing storage already suffices (the
    /// `Object.<init>` shape). `Some(expr)` creates a standard-library object
    /// whose runtime representation is already wrapped in `Option`.
    Constructor {
        lower: fn(args: &[String]) -> Option<String>,
    },
    StaticField {
        lower: fn() -> String,
    },
    StaticMethod {
        lower: fn(args: &[String]) -> String,
    },
    InstanceMethod {
        lower: fn(receiver: &str, args: &[String]) -> String,
    },
}

pub(crate) struct StdEntry {
    pub name: &'static str,
    /// Exact JVM descriptor. The class file has already resolved overloads.
    pub descriptor: &'static str,
    pub member: StdMember,
}

/// A closed conversion accepted when Java verification assigns one reference
/// class to another. The expression is already an `Option<source-rust-type>`
/// for class sources, or a `jars_runtime::JavaString` for `java/lang/String`.
pub(crate) struct StdCoercion {
    pub from: &'static str,
    pub lower: fn(&str) -> String,
}

pub(crate) struct StdClass {
    /// JVM binary name, e.g. `java/lang/CharSequence`.
    pub name: &'static str,
    /// The `java/lang/String` entry now carries its `JavaString` runtime
    /// representation like any other Rust-written class, while the compiler
    /// keeps a dedicated non-null `Type::String` representation. Type-only
    /// classes remain `None`.
    pub rust_type: Option<&'static str>,
    pub coercions: &'static [StdCoercion],
    pub members: &'static [StdEntry],
}

macro_rules! std_member {
    (constructor, $lower:expr) => {
        StdMember::Constructor { lower: $lower }
    };
    (static_field, $lower:expr) => {
        StdMember::StaticField { lower: $lower }
    };
    (static_method, $lower:expr) => {
        StdMember::StaticMethod { lower: $lower }
    };
    (instance, $lower:expr) => {
        StdMember::InstanceMethod { lower: $lower }
    };
}

/// Materializes the compiler half of the shared declaration.
macro_rules! compile_stdlib {
    (
        $(
            class {
                name: $name:literal,
                rust_type: $rust_type:expr,
                coercions: [$( $from:literal => $coercion:expr, )*],
                runtime: { $($runtime:tt)* },
                members: [$( $kind:ident $member_name:literal $descriptor:literal $lower:expr; )*],
            }
        )*
    ) => {
        static CLASSES: &[StdClass] = &[
            $(
                StdClass {
                    name: $name,
                    rust_type: $rust_type,
                    coercions: &[
                        $(StdCoercion { from: $from, lower: $coercion },)*
                    ],
                    members: &[
                        $(StdEntry {
                            name: $member_name,
                            descriptor: $descriptor,
                            member: std_member!($kind, $lower),
                        },)*
                    ],
                },
            )*
        ];
    };
}

jars_stdlib::java_stdlib!(compile_stdlib);

/// Registry membership check for JAR-import dependency validation.
pub(crate) fn is_known_type(class: &str) -> bool {
    CLASSES.iter().any(|entry| entry.name == class)
}

/// Class-level lookup used by `Type::rust`.
pub(crate) fn class(class: &str) -> Option<&'static StdClass> {
    CLASSES.iter().find(|entry| entry.name == class)
}

/// Member lookup by exact owner/name/descriptor, used by both compiler bodies.
pub(crate) fn member(class: &str, name: &str, descriptor: &str) -> Option<&'static StdMember> {
    self::class(class)?
        .members
        .iter()
        .find(|entry| entry.name == name && entry.descriptor == descriptor)
        .map(|entry| &entry.member)
}

/// Lowers a verifier-approved reference conversion from `from` to `to`.
pub(crate) fn coerce(from: &str, to: &str, expression: &str) -> Option<String> {
    self::class(to)?
        .coercions
        .iter()
        .find(|coercion| coercion.from == from)
        .map(|coercion| (coercion.lower)(expression))
}

/// A standard-library class may appear at a Java `new` instruction only when
/// the shared declaration contains a constructor member.
pub(crate) fn is_constructible(class: &str) -> bool {
    self::class(class).is_some_and(|entry| {
        entry
            .members
            .iter()
            .any(|entry| matches!(entry.member, StdMember::Constructor { .. }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_table_matches_members_by_exact_descriptor() {
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
    fn generated_table_exposes_runtime_classes_and_type_only_classes() {
        assert_eq!(
            class("java/lang/CharSequence").and_then(|entry| entry.rust_type),
            Some("jars_runtime::CharSequence")
        );
        assert_eq!(
            class("java/lang/Throwable").and_then(|entry| entry.rust_type),
            None
        );
    }

    #[test]
    fn generated_table_coerces_strings_and_rust_written_classes() {
        assert_eq!(
            coerce("java/lang/String", "java/lang/CharSequence", "value"),
            Some("value.map(jars_runtime::CharSequence::from_java_string)".to_owned())
        );
        assert_eq!(
            coerce("java/lang/StringBuilder", "java/lang/CharSequence", "value"),
            Some("value.map(jars_runtime::CharSequence::from_string_builder)".to_owned())
        );
        assert!(is_constructible("java/lang/StringBuilder"));
    }

    #[test]
    fn flite_voice_registration_is_an_exact_native_shim_entry() {
        let owner = "com/mojang/text2speech/NarratorLinux$FliteLibrary$CmuUsKal16";
        let descriptor = "(Ljava/lang/String;)Lcom/sun/jna/Pointer;";
        let Some(StdMember::InstanceMethod { lower }) =
            member(owner, "register_cmu_us_kal16", descriptor)
        else {
            panic!("expected the Flite native shim to be registered");
        };
        assert_eq!(
            lower("_receiver", &["name".to_owned()]),
            "jars_runtime::register_cmu_us_kal16(name)?"
        );
        assert_eq!(
            class("com/sun/jna/Pointer").and_then(|entry| entry.rust_type),
            Some("jars_runtime::NativePointer")
        );
    }

    #[test]
    fn object_init_remains_a_no_op_constructor() {
        let Some(StdMember::Constructor { lower }) = member("java/lang/Object", "<init>", "()V")
        else {
            panic!("expected a generated Object.<init> entry");
        };
        assert_eq!(lower(&[]), None);
    }
}
