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
                // The shared superclass slot. Platform members such as
                // `Objects.toString(Object, String)` accept widened reference
                // values; the closed-world enum grows with triaged members.
                name: "java/lang/Object",
                rust_type: Some("jars_runtime::JavaObject"),
                coercions: [
                    "java/lang/String" => |value| {
                        format!("{value}.map(jars_runtime::JavaObject::from_string)")
                    },
                ],
                runtime: {
                    /// The closed-world Java `Object` reference. Java code
                    /// widens `String` values into `Object`-typed positions;
                    /// each newly supported kind adds a variant here.
                    #[derive(Clone, Debug)]
                    pub enum JavaObject {
                        Str(JavaString),
                    }

                    impl JavaObject {
                        #[must_use]
                        pub fn from_string(value: JavaString) -> Self {
                            Self::Str(value)
                        }

                        /// Java `Object.toString()` for the supported kinds.
                        #[must_use]
                        pub fn to_string_value(&self) -> JavaString {
                            match self {
                                Self::Str(value) => value.clone(),
                            }
                        }

                        /// Java reference identity for the closed world.
                        #[must_use]
                        pub fn __same(&self, other: &Self) -> bool {
                            match (self, other) {
                                (Self::Str(left), Self::Str(right)) => left.__same(right),
                            }
                        }
                    }
                },
                members: [
                    constructor "<init>" "()V" |_args| None;
                ],
            }
            class {
                // `String` has a dedicated compiler representation, but its
                // runtime values are the owned `JavaString` below and follow
                // the same member-table lowering as every other class.
                name: "java/lang/String",
                rust_type: Some("jars_runtime::JavaString"),
                coercions: [],
                runtime: {
                    /// An owned Java `String` value.  Values are computed at
                    /// runtime (literals, later transformations), so the
                    /// characters live in a shared `Rc<String>` rather than
                    /// only in `'static` literals.  Nullable string slots are
                    /// the compiler's `Option<JavaString>`; non-null slots hold
                    /// the value directly, so this type itself is never null.
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

                        /// Java `String.trim()`: strips code units `<= ' '`
                        /// (`U+0020`) from both ends, matching the JDK rather
                        /// than Rust's broader Unicode whitespace.
                        #[must_use]
                        pub fn trim(&self) -> Self {
                            Self::new(
                                self.as_str()
                                    .trim_matches(|c: char| c <= '\u{20}'),
                            )
                        }

                        /// Java `String.toUpperCase()` under the root locale.
                        /// Rust's `to_uppercase` is the closest available
                        /// full-mapping equivalent; locale-sensitive overrides
                        /// are outside the modeled subset.
                        #[must_use]
                        pub fn to_upper_case(&self) -> Self {
                            Self::new(self.as_str().to_uppercase())
                        }

                        /// Java `String.toLowerCase()` under the root locale.
                        #[must_use]
                        pub fn to_lower_case(&self) -> Self {
                            Self::new(self.as_str().to_lowercase())
                        }

                        /// Java `String.length()`: the UTF-16 code-unit count.
                        #[must_use]
                        pub fn length(&self) -> i32 {
                            self.as_str().encode_utf16().count() as i32
                        }

                        /// Java `String.isEmpty()`.
                        #[must_use]
                        pub fn is_empty(&self) -> bool {
                            self.0.is_empty()
                        }

                        /// Java `String.charAt(int)`: the UTF-16 code unit at
                        /// `index`.
                        pub fn char_at(&self, index: i32) -> JavaResult<u16> {
                            char_sequence_char_at(self.as_str(), index)
                        }

                        fn utf16_units(&self) -> Vec<u16> {
                            self.as_str().encode_utf16().collect()
                        }

                        /// Java `String.indexOf(int)`: the first UTF-16 index
                        /// of the code unit.
                        #[must_use]
                        pub fn index_of_unit(&self, unit: i32) -> i32 {
                            let wanted = unit as u16;
                            self.as_str()
                                .encode_utf16()
                                .position(|unit| unit == wanted)
                                .map_or(-1, |index| index as i32)
                        }

                        /// Java `String.indexOf(String)`: the first UTF-16
                        /// index of the substring.
                        #[must_use]
                        pub fn index_of(&self, needle: &Self) -> i32 {
                            if needle.is_empty() {
                                return 0;
                            }
                            let needle_units = needle.utf16_units();
                            self.utf16_units()
                                .windows(needle_units.len())
                                .position(|window| window == needle_units.as_slice())
                                .map_or(-1, |index| index as i32)
                        }

                        /// Java `String.lastIndexOf(String)`: the last UTF-16
                        /// index of the substring.
                        #[must_use]
                        pub fn last_index_of(&self, needle: &Self) -> i32 {
                            if needle.is_empty() {
                                return self.length();
                            }
                            let needle_units = needle.utf16_units();
                            self.utf16_units()
                                .windows(needle_units.len())
                                .rposition(|window| window == needle_units.as_slice())
                                .map_or(-1, |index| index as i32)
                        }

                        /// Java `String.substring(int)`: the tail from a
                        /// UTF-16 index.
                        pub fn substring(&self, start: i32) -> JavaResult<Self> {
                            self.substring_range(start, self.length())
                        }

                        /// Java `String.substring(int, int)`.
                        pub fn substring_range(&self, start: i32, end: i32) -> JavaResult<Self> {
                            let length = self.length();
                            let start = start.clamp(0, length);
                            let end = end.clamp(0, length);
                            if start > end {
                                return Err(string_index_out_of_bounds());
                            }
                            let units: Vec<u16> = self
                                .as_str()
                                .encode_utf16()
                                .skip(start as usize)
                                .take((end - start) as usize)
                                .collect();
                            Ok(Self::new(String::from_utf16(&units).expect(
                                "sliced UTF-16 stays valid",
                            )))
                        }

                        /// Java `String.compareTo(String)`: UTF-16 code-unit
                        /// lexicographic order, then the length difference.
                        #[must_use]
                        pub fn compare_to(&self, other: &Self) -> i32 {
                            let left = self.utf16_units();
                            let right = other.utf16_units();
                            for (left_unit, right_unit) in left.iter().zip(right.iter()) {
                                if left_unit != right_unit {
                                    return i32::from(*left_unit) - i32::from(*right_unit);
                                }
                            }
                            left.len() as i32 - right.len() as i32
                        }

                        /// Java `String.concat(String)`.
                        #[must_use]
                        pub fn concat(&self, other: &Self) -> Self {
                            Self::new(format!("{}{}", self.0, other.0))
                        }

                        /// Java `String.startsWith(String)`.
                        #[must_use]
                        pub fn starts_with(&self, prefix: &Self) -> bool {
                            self.index_of(prefix) == 0
                        }

                        /// Java `String.endsWith(String)`.
                        #[must_use]
                        pub fn ends_with(&self, suffix: &Self) -> bool {
                            if suffix.is_empty() {
                                return true;
                            }
                            let needle_units = suffix.utf16_units();
                            let haystack = self.utf16_units();
                            haystack.len() >= needle_units.len()
                                && haystack[haystack.len() - needle_units.len()..]
                                    == needle_units[..]
                        }

                        /// Java `String.codePointAt(int)`: the code point at a
                        /// UTF-16 index, combining a surrogate pair when the
                        /// index points at a high surrogate.
                        pub fn code_point_at(&self, index: i32) -> JavaResult<i32> {
                            let units: Vec<u16> = self.as_str().encode_utf16().collect();
                            let index = usize::try_from(index)
                                .map_err(|_| string_index_out_of_bounds())?;
                            let unit = *units
                                .get(index)
                                .ok_or_else(string_index_out_of_bounds)?;
                            if (0xd800..0xdc00).contains(&unit) {
                                if let Some(low) = units.get(index + 1) {
                                    if (0xdc00..0xe000).contains(low) {
                                        let combined = 0x1_0000
                                            + ((u32::from(unit) - 0xd800) << 10)
                                            + (u32::from(*low) - 0xdc00);
                                        return Ok(i32::try_from(combined)
                                            .expect("code point fits in i32"));
                                    }
                                }
                            }
                            Ok(i32::from(unit))
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

                        fn length_value(&self) -> JavaResult<i32> {
                            Ok(self.length())
                        }

                        fn char_at_value(&self, index: i32) -> JavaResult<u16> {
                            self.char_at(index)
                        }

                        fn length(
                            &self,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<i32>>>> {
                            let value = self.length_value();
                            Box::pin(async move { value })
                        }

                        fn char_at(
                            &self,
                            index: i32,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<u16>>>> {
                            let value = self.char_at_value(index);
                            Box::pin(async move { value })
                        }
                    }
                },
                members: [
                    // Instance methods on Java strings. The compiler unwraps
                    // the nullable `Option<JavaString>` receiver before these
                    // lowerings run.
                    instance "trim" "()Ljava/lang/String;" |receiver, _args| {
                        format!("Some({receiver}.trim())")
                    };
                    instance "toUpperCase" "()Ljava/lang/String;" |receiver, _args| {
                        format!("Some({receiver}.to_upper_case())")
                    };
                    instance "toLowerCase" "()Ljava/lang/String;" |receiver, _args| {
                        format!("Some({receiver}.to_lower_case())")
                    };
                    instance "codePointAt" "(I)I" |receiver, args| {
                        format!("{receiver}.code_point_at({})?", args[0])
                    };
                    instance "length" "()I" |receiver, _args| {
                        format!("{receiver}.length() as i32")
                    };
                    instance "isEmpty" "()Z" |receiver, _args| {
                        format!("{receiver}.is_empty()")
                    };
                    instance "charAt" "(I)C" |receiver, args| {
                        format!("{receiver}.char_at({})?", args[0])
                    };
                    instance "indexOf" "(I)I" |receiver, args| {
                        format!("{receiver}.index_of_unit({})", args[0])
                    };
                    instance "indexOf" "(Ljava/lang/String;)I" |receiver, args| {
                        format!(
                            "{receiver}.index_of(&{}.ok_or_else(jars_runtime::null_pointer)?)",
                            args[0]
                        )
                    };
                    instance "lastIndexOf" "(Ljava/lang/String;)I" |receiver, args| {
                        format!(
                            "{receiver}.last_index_of(&{}.ok_or_else(jars_runtime::null_pointer)?)",
                            args[0]
                        )
                    };
                    instance "substring" "(I)Ljava/lang/String;" |receiver, args| {
                        format!("Some({receiver}.substring({})?)", args[0])
                    };
                    instance "substring" "(II)Ljava/lang/String;" |receiver, args| {
                        format!("Some({receiver}.substring_range({}, {})?)", args[0], args[1])
                    };
                    instance "compareTo" "(Ljava/lang/String;)I" |receiver, args| {
                        format!(
                            "{receiver}.compare_to(&{}.ok_or_else(jars_runtime::null_pointer)?)",
                            args[0]
                        )
                    };
                    instance "concat" "(Ljava/lang/String;)Ljava/lang/String;" |receiver, args| {
                        format!(
                            "Some({receiver}.concat(&{}.ok_or_else(jars_runtime::null_pointer)?))",
                            args[0]
                        )
                    };
                    instance "startsWith" "(Ljava/lang/String;)Z" |receiver, args| {
                        format!(
                            "{receiver}.starts_with(&{}.ok_or_else(jars_runtime::null_pointer)?)",
                            args[0]
                        )
                    };
                    instance "endsWith" "(Ljava/lang/String;)Z" |receiver, args| {
                        format!(
                            "{receiver}.ends_with(&{}.ok_or_else(jars_runtime::null_pointer)?)",
                            args[0]
                        )
                    };
                    constructor "<init>" "([III)V" |args| {
                        // Java's String(int[], int, int) validates the range
                        // and throws StringIndexOutOfBoundsException otherwise.
                        Some(format!(
                            "Some(jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {}, {}){}?)",
                            crate::model::runtime_method_name("java_string_from_code_points"),
                            args[0], args[1], args[2],
                            crate::model::await_token()
                        ))
                    };
                    constructor "<init>" "([C)V" |args| {
                        // Java's String(char[]) copies the array contents.
                        Some(format!(
                            "Some(jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?){}?)",
                            crate::model::runtime_method_name("java_string_from_chars"),
                            args[0],
                            crate::model::await_token()
                        ))
                    };
                    constructor "<init>" "([CII)V" |args| {
                        // Java's String(char[], int, int) validates the range.
                        Some(format!(
                            "Some(jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {}, {}){}?)",
                            crate::model::runtime_method_name("java_string_from_char_range"),
                            args[0], args[1], args[2],
                            crate::model::await_token()
                        ))
                    };
                ],
            }
            class {
                name: "java/lang/CharSequence",
                rust_type: Some("jars_runtime::CharSequence"),
                coercions: [
                    "java/lang/String" => |value| {
                        format!("{value}.map(jars_runtime::CharSequence::from_java_string)")
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

                        fn length_value(&self) -> JavaResult<i32>;

                        fn char_at_value(&self, index: i32) -> JavaResult<u16>;

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

                        fn length_value(&self) -> JavaResult<i32> {
                            Ok(char_sequence_length(self.0))
                        }

                        fn char_at_value(&self, index: i32) -> JavaResult<u16> {
                            char_sequence_char_at(self.0, index)
                        }

                        fn length(
                            &self,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<i32>>>> {
                            let length = self.length_value();
                            Box::pin(async move { length })
                        }

                        fn char_at(
                            &self,
                            index: i32,
                        ) -> std::pin::Pin<Box<dyn Future<Output = JavaResult<u16>>>> {
                            let value = self.char_at_value(index);
                            Box::pin(async move { value })
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

                        pub fn length_sync(&self) -> JavaResult<i32> {
                            self.value.length_value()
                        }

                        pub async fn char_at(&self, index: i32) -> JavaResult<u16> {
                            self.value.char_at(index).await
                        }

                        pub fn char_at_sync(&self, index: i32) -> JavaResult<u16> {
                            self.value.char_at_value(index)
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

                    /// Java's `String(int[], int, int)` constructor. Copies
                    /// `count` valid code points starting at `offset`; range
                    /// violations and surrogate code points throw the Java
                    /// string range failure.
                    pub async fn java_string_from_code_points(
                        array: JavaArray<i32>,
                        offset: i32,
                        count: i32,
                    ) -> JavaResult<JavaString> {
                        let length = array.length().await?;
                        let end = i64::from(offset) + i64::from(count);
                        if offset < 0 || count < 0 || end > i64::from(length) {
                            return Err(string_index_out_of_bounds());
                        }
                        let mut result = String::new();
                        for index in offset..offset + count {
                            let point = array.get(index).await?;
                            let ch = u32::try_from(point).ok().and_then(char::from_u32);
                            result.push(ch.ok_or_else(string_index_out_of_bounds)?);
                        }
                        Ok(JavaString::new(result))
                    }

                    /// Java's `String(char[])` constructor. Copies the UTF-16
                    /// code units from the array.
                    pub async fn java_string_from_chars(array: JavaArray<u16>) -> JavaResult<JavaString> {
                        let length = array.length().await?;
                        let mut units = Vec::with_capacity(length as usize);
                        for index in 0..length {
                            units.push(array.get(index).await?);
                        }
                        Ok(JavaString::new(String::from_utf16_lossy(&units)))
                    }

                    /// Java's `String(char[], int, int)` constructor: copies
                    /// `count` code units starting at `offset`.
                    pub async fn java_string_from_char_range(
                        array: JavaArray<u16>,
                        offset: i32,
                        count: i32,
                    ) -> JavaResult<JavaString> {
                        let length = array.length().await?;
                        let end = i64::from(offset) + i64::from(count);
                        if offset < 0 || count < 0 || end > i64::from(length) {
                            return Err(string_index_out_of_bounds());
                        }
                        let mut units = Vec::with_capacity(count as usize);
                        for index in offset..offset + count {
                            units.push(array.get(index).await?);
                        }
                        Ok(JavaString::new(String::from_utf16_lossy(&units)))
                    }

                    pub fn java_string_from_code_points_sync(
                        array: JavaArray<i32>,
                        offset: i32,
                        count: i32,
                    ) -> JavaResult<JavaString> {
                        let length = array.length_sync()?;
                        let end = i64::from(offset) + i64::from(count);
                        if offset < 0 || count < 0 || end > i64::from(length) {
                            return Err(string_index_out_of_bounds());
                        }
                        let mut result = String::new();
                        for index in offset..offset + count {
                            let point = array.get_sync(index)?;
                            let ch = u32::try_from(point).ok().and_then(char::from_u32);
                            result.push(ch.ok_or_else(string_index_out_of_bounds)?);
                        }
                        Ok(JavaString::new(result))
                    }

                    pub fn java_string_from_chars_sync(array: JavaArray<u16>) -> JavaResult<JavaString> {
                        let length = array.length_sync()?;
                        let mut units = Vec::with_capacity(length as usize);
                        for index in 0..length {
                            units.push(array.get_sync(index)?);
                        }
                        Ok(JavaString::new(String::from_utf16_lossy(&units)))
                    }

                    pub fn java_string_from_char_range_sync(
                        array: JavaArray<u16>,
                        offset: i32,
                        count: i32,
                    ) -> JavaResult<JavaString> {
                        let length = array.length_sync()?;
                        let end = i64::from(offset) + i64::from(count);
                        if offset < 0 || count < 0 || end > i64::from(length) {
                            return Err(string_index_out_of_bounds());
                        }
                        let mut units = Vec::with_capacity(count as usize);
                        for index in offset..offset + count {
                            units.push(array.get_sync(index)?);
                        }
                        Ok(JavaString::new(String::from_utf16_lossy(&units)))
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
                        format!(
                            "{receiver}.{}(){}?",
                            crate::model::runtime_method_name("length"),
                            crate::model::await_token()
                        )
                    };
                    instance "charAt" "(I)C" |receiver, args| {
                        format!(
                            "{receiver}.{}({}){}?",
                            crate::model::runtime_method_name("char_at"),
                            args[0],
                            crate::model::await_token()
                        )
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
                    enum StringBuilderStore {
                        Actor(ActorRef<StringBuilderMessage>),
                        Direct(std::rc::Rc<std::cell::RefCell<JavaString>>),
                    }

                    #[derive(Clone)]
                    pub struct JavaStringBuilder {
                        store: StringBuilderStore,
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
                        Reverse {
                            reply: Reply<JavaResult<()>>,
                        },
                        ToStringValue {
                            reply: Reply<JavaResult<JavaString>>,
                        },
                        AppendChar {
                            unit: u16,
                            reply: Reply<JavaResult<()>>,
                        },
                        AppendString {
                            text: JavaString,
                            reply: Reply<JavaResult<()>>,
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
                                                    StringBuilderMessage::Reverse { reply } => {
                                                        // Java reverses by UTF-16 code
                                                        // units while keeping surrogate
                                                        // pairs together, which matches
                                                        // reversing Unicode scalar values.
                                                        let reversed: String = state
                                                            .lock()
                                                            .expect("string builder state mutex")
                                                            .as_str()
                                                            .chars()
                                                            .rev()
                                                            .collect();
                                                        *state
                                                            .lock()
                                                            .expect("string builder state mutex") =
                                                            JavaString::new(reversed);
                                                        let _ = reply.send(Ok(()));
                                                    }
                                                    StringBuilderMessage::ToStringValue { reply } => {
                                                        let value = state
                                                            .lock()
                                                            .expect("string builder state mutex")
                                                            .clone();
                                                        let _ = reply.send(Ok(value));
                                                    }
                                                    StringBuilderMessage::AppendChar { unit, reply } => {
                                                        let mut next = state
                                                            .lock()
                                                            .expect("string builder state mutex")
                                                            .as_str()
                                                            .to_owned();
                                                        next.push(char::from_u32(u32::from(unit)).expect("valid UTF-16 append unit"));
                                                        *state
                                                            .lock()
                                                            .expect("string builder state mutex") =
                                                            JavaString::new(next);
                                                        let _ = reply.send(Ok(()));
                                                    }
                                                    StringBuilderMessage::AppendString { text, reply } => {
                                                        let mut next = state
                                                            .lock()
                                                            .expect("string builder state mutex")
                                                            .as_str()
                                                            .to_owned();
                                                        next.push_str(text.as_str());
                                                        *state
                                                            .lock()
                                                            .expect("string builder state mutex") =
                                                            JavaString::new(next);
                                                        let _ = reply.send(Ok(()));
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
                                store: StringBuilderStore::Actor(actor),
                                identity: CharSequenceIdentity::object(),
                            })
                        }

                        /// Java's `new StringBuilder()`: an empty builder.
                        pub fn empty<S: Spawner>(spawner: S) -> JavaResult<Self> {
                            Self::new(spawner, JavaString::new(""))
                        }

                        /// Synchronous `StringBuilder` for the object and entity hosts.
                        pub fn direct(source: JavaString) -> JavaResult<Self> {
                            Ok(Self {
                                store: StringBuilderStore::Direct(std::rc::Rc::new(
                                    std::cell::RefCell::new(source),
                                )),
                                identity: CharSequenceIdentity::object(),
                            })
                        }

                        pub fn empty_direct() -> JavaResult<Self> {
                            Self::direct(JavaString::new(""))
                        }

                        #[must_use]
                        pub fn __same(&self, other: &Self) -> bool {
                            match (&self.store, &other.store) {
                                (
                                    StringBuilderStore::Actor(left),
                                    StringBuilderStore::Actor(right),
                                ) => left.same(right),
                                (
                                    StringBuilderStore::Direct(left),
                                    StringBuilderStore::Direct(right),
                                ) => std::rc::Rc::ptr_eq(left, right),
                                _ => false,
                            }
                        }

                        pub async fn length(&self) -> JavaResult<i32> {
                            if let StringBuilderStore::Direct(state) = &self.store {
                                return Ok(char_sequence_length(state.borrow().as_str()));
                            }
                            let StringBuilderStore::Actor(actor) = &self.store else {
                                return Err(direct_call_on_actor());
                            };
                            let (reply, response) = reply();
                            actor.send(StringBuilderMessage::Length { reply }).await?;
                            response.recv().await?
                        }

                        pub fn length_sync(&self) -> JavaResult<i32> {
                            match &self.store {
                                StringBuilderStore::Direct(state) => {
                                    Ok(char_sequence_length(state.borrow().as_str()))
                                }
                                StringBuilderStore::Actor(_) => Err(direct_call_on_actor()),
                            }
                        }

                        fn mailbox(&self) -> JavaResult<&ActorRef<StringBuilderMessage>> {
                            match &self.store {
                                StringBuilderStore::Actor(actor) => Ok(actor),
                                StringBuilderStore::Direct(_) => Err(direct_call_on_actor()),
                            }
                        }

                        pub async fn char_at(&self, index: i32) -> JavaResult<u16> {
                            if matches!(self.store, StringBuilderStore::Direct(_)) {
                                return self.char_at_sync(index);
                            }
                            let (reply, response) = reply();
                            self.mailbox()?
                                .send(StringBuilderMessage::CharAt { index, reply })
                                .await?;
                            response.recv().await?
                        }

                        pub fn char_at_sync(&self, index: i32) -> JavaResult<u16> {
                            match &self.store {
                                StringBuilderStore::Direct(state) => {
                                    char_sequence_char_at(state.borrow().as_str(), index)
                                }
                                StringBuilderStore::Actor(_) => Err(direct_call_on_actor()),
                            }
                        }

                        /// Java `StringBuilder.reverse()`: mutates the actor's
                        /// state and returns this same handle.
                        pub async fn reverse(&self) -> JavaResult<Self> {
                            if matches!(self.store, StringBuilderStore::Direct(_)) {
                                return self.reverse_sync();
                            }
                            let (reply, response) = reply();
                            self.mailbox()?
                                .send(StringBuilderMessage::Reverse { reply })
                                .await?;
                            response.recv().await??;
                            Ok(self.clone())
                        }

                        pub fn reverse_sync(&self) -> JavaResult<Self> {
                            let StringBuilderStore::Direct(state) = &self.store else {
                                return Err(direct_call_on_actor());
                            };
                            let reversed: String = state.borrow().as_str().chars().rev().collect();
                            *state.borrow_mut() = JavaString::new(reversed);
                            Ok(self.clone())
                        }

                        /// Java `StringBuilder.toString()`: a Java string with
                        /// the builder's current characters.
                        pub async fn to_string_value(&self) -> JavaResult<JavaString> {
                            if matches!(self.store, StringBuilderStore::Direct(_)) {
                                return self.to_string_value_sync();
                            }
                            let (reply, response) = reply();
                            self.mailbox()?
                                .send(StringBuilderMessage::ToStringValue { reply })
                                .await?;
                            response.recv().await?
                        }

                        pub fn to_string_value_sync(&self) -> JavaResult<JavaString> {
                            match &self.store {
                                StringBuilderStore::Direct(state) => Ok(state.borrow().clone()),
                                StringBuilderStore::Actor(_) => Err(direct_call_on_actor()),
                            }
                        }

                        /// Java `StringBuilder.append(char)`.
                        pub async fn append_char(&self, unit: u16) -> JavaResult<Self> {
                            if matches!(self.store, StringBuilderStore::Direct(_)) {
                                return self.append_char_sync(unit);
                            }
                            let (reply, response) = reply();
                            self.mailbox()?
                                .send(StringBuilderMessage::AppendChar { unit, reply })
                                .await?;
                            response.recv().await??;
                            Ok(self.clone())
                        }

                        pub fn append_char_sync(&self, unit: u16) -> JavaResult<Self> {
                            let StringBuilderStore::Direct(state) = &self.store else {
                                return Err(direct_call_on_actor());
                            };
                            let mut next = state.borrow().as_str().to_owned();
                            next.push(
                                char::from_u32(u32::from(unit)).expect("valid UTF-16 append unit"),
                            );
                            *state.borrow_mut() = JavaString::new(next);
                            Ok(self.clone())
                        }

                        /// Java `StringBuilder.append(String)`. Java throws
                        /// NPE for a null argument.
                        pub async fn append_string(&self, text: JavaString) -> JavaResult<Self> {
                            if matches!(self.store, StringBuilderStore::Direct(_)) {
                                return self.append_string_sync(text);
                            }
                            let (reply, response) = reply();
                            self.mailbox()?
                                .send(StringBuilderMessage::AppendString { text, reply })
                                .await?;
                            response.recv().await??;
                            Ok(self.clone())
                        }

                        pub fn append_string_sync(&self, text: JavaString) -> JavaResult<Self> {
                            let StringBuilderStore::Direct(state) = &self.store else {
                                return Err(direct_call_on_actor());
                            };
                            let mut next = state.borrow().as_str().to_owned();
                            next.push_str(text.as_str());
                            *state.borrow_mut() = JavaString::new(next);
                            Ok(self.clone())
                        }

                        /// Java `StringBuilder.append(int)`.
                        pub async fn append_int(&self, value: i32) -> JavaResult<Self> {
                            self.append_string(JavaString::new(value.to_string()))
                                .await
                        }

                        pub fn append_int_sync(&self, value: i32) -> JavaResult<Self> {
                            self.append_string_sync(JavaString::new(value.to_string()))
                        }

                        /// Java `StringBuilder.substring(int, int)`: the
                        /// current characters between the two UTF-16 indices.
                        pub async fn substring_range(
                            &self,
                            start: i32,
                            end: i32,
                        ) -> JavaResult<JavaString> {
                            let text = self.to_string_value().await?;
                            text.substring_range(start, end)
                        }

                        pub fn substring_range_sync(
                            &self,
                            start: i32,
                            end: i32,
                        ) -> JavaResult<JavaString> {
                            self.to_string_value_sync()?
                                .substring_range(start, end)
                        }
                    }

                    impl CharSequenceValue for JavaStringBuilder {
                        fn identity(&self) -> CharSequenceIdentity {
                            self.identity.clone()
                        }

                        fn length_value(&self) -> JavaResult<i32> {
                            self.length_sync()
                        }

                        fn char_at_value(&self, index: i32) -> JavaResult<u16> {
                            self.char_at_sync(index)
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
                            "Some({})",
                            crate::model::string_builder_new(&format!(
                                "{}.ok_or_else(jars_runtime::null_pointer)?",
                                args[0]
                            ))
                        ))
                    };
                    instance "length" "()I" |receiver, _args| {
                        format!(
                            "{receiver}.{}(){}?",
                            crate::model::runtime_method_name("length"),
                            crate::model::await_token()
                        )
                    };
                    instance "charAt" "(I)C" |receiver, args| {
                        format!(
                            "{receiver}.{}({}){}?",
                            crate::model::runtime_method_name("char_at"),
                            args[0],
                            crate::model::await_token()
                        )
                    };
                    instance "reverse" "()Ljava/lang/StringBuilder;" |receiver, _args| {
                        format!(
                            "Some({receiver}.{}(){}?)",
                            crate::model::runtime_method_name("reverse"),
                            crate::model::await_token()
                        )
                    };
                    instance "toString" "()Ljava/lang/String;" |receiver, _args| {
                        format!(
                            "Some({receiver}.{}(){}?)",
                            crate::model::runtime_method_name("to_string_value"),
                            crate::model::await_token()
                        )
                    };
                    instance "append" "(C)Ljava/lang/StringBuilder;" |receiver, args| {
                        format!(
                            "Some({receiver}.{}({} as u16){}?)",
                            crate::model::runtime_method_name("append_char"),
                            args[0],
                            crate::model::await_token()
                        )
                    };
                    instance "append" "(Ljava/lang/String;)Ljava/lang/StringBuilder;" |receiver, args| {
                        format!(
                            "Some({receiver}.{}({}.ok_or_else(jars_runtime::null_pointer)?){}?)",
                            crate::model::runtime_method_name("append_string"),
                            args[0],
                            crate::model::await_token()
                        )
                    };
                    instance "append" "(I)Ljava/lang/StringBuilder;" |receiver, args| {
                        format!(
                            "Some({receiver}.{}({}){}?)",
                            crate::model::runtime_method_name("append_int"),
                            args[0],
                            crate::model::await_token()
                        )
                    };
                    instance "substring" "(II)Ljava/lang/String;" |receiver, args| {
                        format!(
                            "Some({receiver}.{}({}, {}){}?)",
                            crate::model::runtime_method_name("substring_range"),
                            args[0],
                            args[1],
                            crate::model::await_token()
                        )
                    };
                    constructor "<init>" "()V" |_args| {
                        Some(format!("Some({})", crate::model::string_builder_empty()))
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

                        /// Java `Character.toTitleCase(int)`. Java only applies
                        /// one-to-one mappings, so a full mapping that expands
                        /// (like `ß` to `Ss`) leaves the code point unchanged.
                        /// Rust's `to_titlecase` is unstable, so the titlecase
                        /// mappings come from `to_uppercase` plus the four
                        /// Latin titlecase digraph letters, whose single-char
                        /// titlecase is the dedicated Lt code point.
                        #[must_use]
                        pub fn to_title_case(value: i32) -> i32 {
                            const DIGRAPHS: [(i32, i32); 8] = [
                                (0x01c4, 0x01c5), // DŽ -> Dž
                                (0x01c5, 0x01c5),
                                (0x01c6, 0x01c5), // dž -> Dž
                                (0x01c7, 0x01c8), // LJ -> Lj
                                (0x01c9, 0x01c8), // lj -> Lj
                                (0x01ca, 0x01cb), // NJ -> Nj
                                (0x01cb, 0x01cb),
                                (0x01cc, 0x01cb), // nj -> Nj
                            ];
                            if let Some((_, title)) =
                                DIGRAPHS.iter().find(|(from, _)| *from == value)
                            {
                                return i32::try_from(*title)
                                    .expect("scalar values fit in i32");
                            }
                            match value {
                                0x01f1 => 0x01f2, // DZ -> Dz
                                0x01f2 => 0x01f2,
                                0x01f3 => 0x01f2, // dz -> Dz
                                _ => single_char_mapping(value, char::to_uppercase),
                            }
                        }

                        /// Java `Character.toLowerCase(int)` with the same
                        /// one-to-one mapping rule.
                        #[must_use]
                        pub fn to_lower_case(value: i32) -> i32 {
                            single_char_mapping(value, char::to_lowercase)
                        }

                        /// Java `Character.charCount(int)`: 2 UTF-16 code
                        /// units for a supplementary code point, otherwise 1.
                        #[must_use]
                        pub fn char_count(value: i32) -> i32 {
                            1 + i32::from(value >= 0x1_0000)
                        }

                        /// Java `Character.isLowerCase(char)` under the
                        /// documented Rust property approximation; the fixture
                        /// avoids the Nl/No divergence ranges.
                        #[must_use]
                        pub fn is_lower_case(value: u16) -> bool {
                            char::from_u32(u32::from(value))
                                .is_some_and(char::is_lowercase)
                        }

                        /// Java `Character.isUpperCase(char)` under the same
                        /// approximation.
                        #[must_use]
                        pub fn is_upper_case(value: u16) -> bool {
                            char::from_u32(u32::from(value))
                                .is_some_and(char::is_uppercase)
                        }

                        /// Java `Character.isLetter(char)` via Rust's
                        /// alphabetic property (documented approximation of
                        /// Java's Letter categories).
                        #[must_use]
                        pub fn is_letter(value: u16) -> bool {
                            char::from_u32(u32::from(value))
                                .is_some_and(char::is_alphabetic)
                        }

                        /// Java `Character.isDigit(char)`: ASCII digits plus
                        /// Rust's numeric property (Java is exactly Nd; the
                        /// fixture avoids the No/Nl divergence ranges).
                        #[must_use]
                        pub fn is_digit(value: u16) -> bool {
                            matches!(value, 0x30..=0x39)
                                || char::from_u32(u32::from(value))
                                    .is_some_and(char::is_numeric)
                        }

                        /// Java `Character.isLetterOrDigit(char)`.
                        #[must_use]
                        pub fn is_letter_or_digit(value: u16) -> bool {
                            is_letter(value) || is_digit(value)
                        }

                        /// Applies `map` when it yields exactly one code point;
                        /// multi-character mappings and invalid code points
                        /// return the input unchanged, like the JDK.
                        fn single_char_mapping<I: Iterator<Item = char>>(
                            value: i32,
                            map: impl Fn(char) -> I,
                        ) -> i32 {
                            let Ok(point) = u32::try_from(value) else {
                                return value;
                            };
                            match char::from_u32(point) {
                                Some(c) => {
                                    let mut mapped = map(c);
                                    match (mapped.next(), mapped.next()) {
                                        (Some(single), None) => i32::try_from(u32::from(single))
                                            .expect("scalar values fit in i32"),
                                        _ => value,
                                    }
                                }
                                None => value,
                            }
                        }
                    }
                },
                members: [
                    static_method "isWhitespace" "(C)Z" |args| {
                        format!("jars_runtime::character::is_whitespace({} as u16)", args[0])
                    };
                    static_method "toTitleCase" "(I)I" |args| {
                        format!("jars_runtime::character::to_title_case({})", args[0])
                    };
                    static_method "toLowerCase" "(I)I" |args| {
                        format!("jars_runtime::character::to_lower_case({})", args[0])
                    };
                    static_method "charCount" "(I)I" |args| {
                        format!("jars_runtime::character::char_count({})", args[0])
                    };
                    static_method "isLowerCase" "(C)Z" |args| {
                        format!("jars_runtime::character::is_lower_case({} as u16)", args[0])
                    };
                    static_method "isUpperCase" "(C)Z" |args| {
                        format!("jars_runtime::character::is_upper_case({} as u16)", args[0])
                    };
                    static_method "isLetter" "(C)Z" |args| {
                        format!("jars_runtime::character::is_letter({} as u16)", args[0])
                    };
                    static_method "isDigit" "(C)Z" |args| {
                        format!("jars_runtime::character::is_digit({} as u16)", args[0])
                    };
                    static_method "isLetterOrDigit" "(C)Z" |args| {
                        format!("jars_runtime::character::is_letter_or_digit({} as u16)", args[0])
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
                        // Java prints the literal `null` for a null argument.
                        format!(
                            "jars_runtime::println_string({})",
                            args[0]
                        )
                    };
                ],
            }
            class {
                name: "org/apache/commons/logging/LogFactory",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [
                    static_method "getLog"
                    "(Ljava/lang/String;)Lorg/apache/commons/logging/Log;" |_args| {
                        "Some(jars_runtime::JavaLog::new())".to_owned()
                    };
                ],
            }
            class {
                // A stateless no-op logger handle. Backend discovery is outside
                // the closed AOT surface; each call returns a distinct Java
                // reference without relying on process-global logger caches.
                name: "org/apache/commons/logging/Log",
                rust_type: Some("jars_runtime::JavaLog"),
                coercions: [],
                runtime: {
                    #[derive(Clone, Debug)]
                    pub struct JavaLog {
                        identity: std::rc::Rc<()>,
                    }

                    impl JavaLog {
                        #[must_use]
                        pub fn new() -> Self {
                            Self {
                                identity: std::rc::Rc::new(()),
                            }
                        }

                        /// Compares Java logger references without exposing
                        /// any logger implementation state.
                        #[must_use]
                        pub fn __same(&self, other: &Self) -> bool {
                            std::rc::Rc::ptr_eq(&self.identity, &other.identity)
                        }
                    }
                },
                members: [],
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
                        format!(
                            "Some(jars_runtime::JavaPattern::compile({}.ok_or_else(jars_runtime::null_pointer)?))",
                            args[0]
                        )
                    };
                ],
            }
            class {
                name: "java/util/Arrays",
                rust_type: None,
                coercions: [],
                runtime: {
                    /// Java `Arrays.fill(T[], T)` for the modeled element
                    /// values. Array elements stay inside the array's
                    /// mailbox; only the length and written slots cross it.
                    pub async fn java_arrays_fill<T: Clone + 'static>(
                        array: JavaArray<T>,
                        value: T,
                    ) -> JavaResult<()> {
                        let length = array.length().await?;
                        for index in 0..length {
                            array.set(index, value.clone()).await?;
                        }
                        Ok(())
                    }

                    pub fn java_arrays_fill_sync<T: Clone + 'static>(
                        array: JavaArray<T>,
                        value: T,
                    ) -> JavaResult<()> {
                        let length = array.length_sync()?;
                        for index in 0..length {
                            array.set_sync(index, value.clone())?;
                        }
                        Ok(())
                    }
                },
                members: [
                    static_method "fill" "([CC)V" |args| {
                        format!(
                            "jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {} as u16){}?",
                            crate::model::runtime_method_name("java_arrays_fill"),
                            args[0], args[1],
                            crate::model::await_token()
                        )
                    };
                    static_method "fill" "([BB)V" |args| {
                        format!(
                            "jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {} as i8){}?",
                            crate::model::runtime_method_name("java_arrays_fill"),
                            args[0], args[1],
                            crate::model::await_token()
                        )
                    };
                    static_method "fill" "([SS)V" |args| {
                        format!(
                            "jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {} as i16){}?",
                            crate::model::runtime_method_name("java_arrays_fill"),
                            args[0], args[1],
                            crate::model::await_token()
                        )
                    };
                    static_method "fill" "([II)V" |args| {
                        format!(
                            "jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {}){}?",
                            crate::model::runtime_method_name("java_arrays_fill"),
                            args[0], args[1],
                            crate::model::await_token()
                        )
                    };
                    static_method "fill" "([JJ)V" |args| {
                        format!(
                            "jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {}){}?",
                            crate::model::runtime_method_name("java_arrays_fill"),
                            args[0], args[1],
                            crate::model::await_token()
                        )
                    };
                    static_method "fill" "([FF)V" |args| {
                        format!(
                            "jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {}){}?",
                            crate::model::runtime_method_name("java_arrays_fill"),
                            args[0], args[1],
                            crate::model::await_token()
                        )
                    };
                    static_method "fill" "([DD)V" |args| {
                        format!(
                            "jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {}){}?",
                            crate::model::runtime_method_name("java_arrays_fill"),
                            args[0], args[1],
                            crate::model::await_token()
                        )
                    };
                    // Java `Arrays.fill(T[], T)` accepts a null fill value, so
                    // the widened reference argument keeps its `Option`.
                    static_method "fill"
                    "([Ljava/lang/Object;Ljava/lang/Object;)V" |args| {
                        format!(
                            "jars_runtime::{}({}.ok_or_else(jars_runtime::null_pointer)?, {}){}?",
                            crate::model::runtime_method_name("java_arrays_fill"),
                            args[0], args[1],
                            crate::model::await_token()
                        )
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
                // Type-only utility holder. Its members consume widened
                // `Object` references produced by the `java/lang/Object`
                // coercion table.
                name: "java/util/Objects",
                rust_type: None,
                coercions: [],
                runtime: {},
                members: [
                    static_method "toString"
                    "(Ljava/lang/Object;Ljava/lang/String;)Ljava/lang/String;" |args| {
                        // Java: obj != null ? obj.toString() : default. Both
                        // arms produce the nullable string result directly.
                        format!(
                            "match {} {{ Some(object) => Some(object.to_string_value()), None => {} }}",
                            args[0], args[1]
                        )
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
