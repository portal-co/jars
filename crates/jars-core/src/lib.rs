//! A deliberately small Java class-file to actor-oriented Rust compiler.
//!
//! The public surface is intentionally narrow: `compile_class` accepts one
//! class, while `compile_jars` imports a reachable closed set from explicit
//! JARs. Both emit complete Rust binaries which depend on `jars-runtime`.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fmt,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

mod classfile;
mod coverage;
mod inventory;
mod model;
mod stdlib;

pub use coverage::{ClassCoverage, JarCoverage, MethodCoverage, MethodStatus, coverage_jars};
pub use inventory::{
    InventoryClass, InventoryKind, InventoryMember, InventoryReport, inventory_jars,
    write_inventory,
};
pub use model::ObjectModel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    Parse(String),
    InvalidClass(String),
    MissingEntryPoint {
        class: String,
    },
    Jar {
        jar: PathBuf,
        detail: String,
    },
    DuplicateClass {
        class: String,
        first: PathBuf,
        second: PathBuf,
    },
    MissingClass {
        class: String,
        referenced_by: String,
    },
    MissingEntryMethod {
        class: String,
        method: String,
        descriptor: String,
    },
    UnsupportedPlatformClass {
        class: String,
        referenced_by: String,
    },
    Unsupported {
        class: String,
        method: String,
        offset: u32,
        operation: String,
    },
    InvalidStack {
        class: String,
        method: String,
        offset: u32,
        detail: String,
    },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "could not parse class file: {message}"),
            Self::InvalidClass(message) => write!(f, "invalid v1 class: {message}"),
            Self::MissingEntryPoint { class } => {
                write!(f, "{class} does not define public static main(String[])")
            }
            Self::Jar { jar, detail } => {
                write!(f, "could not import JAR {}: {detail}", jar.display())
            }
            Self::DuplicateClass {
                class,
                first,
                second,
            } => write!(
                f,
                "class `{class}` is present in both {} and {}",
                first.display(),
                second.display()
            ),
            Self::MissingClass {
                class,
                referenced_by,
            } => write!(
                f,
                "class `{class}` referenced by `{referenced_by}` is not present in the JAR classpath"
            ),
            Self::MissingEntryMethod {
                class,
                method,
                descriptor,
            } => write!(
                f,
                "{class} does not define public static {method}{descriptor}"
            ),
            Self::UnsupportedPlatformClass {
                class,
                referenced_by,
            } => write!(
                f,
                "Java platform class `{class}` referenced by `{referenced_by}` is not modeled by jars-runtime"
            ),
            Self::Unsupported {
                class,
                method,
                offset,
                operation,
            } => write!(
                f,
                "unsupported operation {operation} in {class}.{method} at bytecode offset {offset}"
            ),
            Self::InvalidStack {
                class,
                method,
                offset,
                detail,
            } => write!(
                f,
                "invalid operand stack in {class}.{method} at bytecode offset {offset}: {detail}"
            ),
        }
    }
}

impl std::error::Error for CompileError {}

/// A static method used as the root of a closed JAR compilation.
///
/// Class names use JVM binary-name syntax, such as `com/example/Entry`. The
/// importer deliberately does not infer an entry point from a manifest in the
/// input archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JarEntrypoint {
    pub class: String,
    pub method: String,
    pub descriptor: String,
}

/// Controls deterministic selection of classes from a multi-release JAR.
///
/// The target is an import decision, not the host JVM.  Keeping it explicit
/// makes an AOT build repeatable on machines with different JDKs installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JarImportOptions {
    /// Java feature release used when selecting `META-INF/versions/N` entries.
    pub target_release: u16,
}

impl JarImportOptions {
    #[must_use]
    pub const fn for_release(target_release: u16) -> Self {
        Self { target_release }
    }
}

impl Default for JarImportOptions {
    fn default() -> Self {
        // Java 8 was the original, non-multi-release JAR contract of this
        // importer.  Callers that want overlays opt in explicitly.
        Self::for_release(8)
    }
}

impl JarEntrypoint {
    #[must_use]
    pub fn new(
        class: impl Into<String>,
        method: impl Into<String>,
        descriptor: impl Into<String>,
    ) -> Self {
        Self {
            class: class.into(),
            method: method.into(),
            descriptor: descriptor.into(),
        }
    }

    #[must_use]
    pub fn main(class: impl Into<String>) -> Self {
        Self::new(class, "main", "([Ljava/lang/String;)V")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Type {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
    Void,
    String,
    Class(String),
    Array(Box<Type>),
}

impl Type {
    fn rust(&self, current_class: &str) -> Result<String, CompileError> {
        match self {
            Self::Boolean => Ok("bool".to_owned()),
            Self::Byte => Ok("i8".to_owned()),
            Self::Char => Ok("u16".to_owned()),
            Self::Short => Ok("i16".to_owned()),
            Self::Int => Ok("i32".to_owned()),
            Self::Long => Ok("i64".to_owned()),
            Self::Float => Ok("f32".to_owned()),
            Self::Double => Ok("f64".to_owned()),
            Self::Void => Ok("()".to_owned()),
            // String values are owned runtime `JavaString`s and participate in
            // Java's null model, so every Rust signature is `Option`.
            Self::String => Ok("Option<jars_runtime::JavaString>".to_owned()),
            Self::Class(name) if name == current_class => {
                Ok(format!("Option<{}>", class_ident(name)?))
            }
            Self::Class(name) if stdlib::class(name).is_some() => {
                match stdlib::class(name).and_then(|entry| entry.rust_type) {
                    Some(rust_type) => Ok(format!("Option<{rust_type}>")),
                    None => Err(invalid(format!(
                        "stdlib class `{name}` has no Rust representation"
                    ))),
                }
            }
            Self::Class(name) => Ok(format!(
                "Option<super::{}::{}>",
                class_ident(name)?,
                class_ident(name)?
            )),
            Self::Array(element) => Ok(format!(
                "Option<jars_runtime::JavaArray<{}>>",
                element.array_element_rust(current_class)?
            )),
        }
    }

    fn array_element_rust(&self, current_class: &str) -> Result<String, CompileError> {
        match self {
            Self::String => Ok("Option<jars_runtime::JavaString>".to_owned()),
            _ => self.rust(current_class),
        }
    }
}

#[derive(Clone, Debug)]
struct Signature {
    parameters: Vec<Type>,
    returns: Type,
}

#[derive(Clone, Debug)]
struct MemberRef {
    class: String,
    name: String,
    descriptor: String,
}

#[derive(Clone, Debug)]
enum Op {
    IConst(i32),
    LConst(i64),
    FConst(f32),
    DConst(f64),
    ILoad(usize),
    IStore(usize),
    ALoad(usize),
    AStore(usize),
    LLoad(usize),
    LStore(usize),
    FLoad(usize),
    FStore(usize),
    DLoad(usize),
    DStore(usize),
    IAdd,
    ISub,
    IMul,
    IDiv,
    IRem,
    INeg,
    IAnd,
    IOr,
    IXor,
    IShl,
    IShr,
    IUshr,
    LAdd,
    LSub,
    LMul,
    LDiv,
    LRem,
    LNeg,
    FAdd,
    FSub,
    FMul,
    FDiv,
    FRem,
    FNeg,
    DAdd,
    DSub,
    DMul,
    DDiv,
    DRem,
    DNeg,
    IInc(usize, i32),
    Goto(i32),
    If(IfKind, i32),
    LookupSwitch {
        default: i32,
        cases: Vec<(i32, i32)>,
    },
    TableSwitch {
        default: i32,
        cases: Vec<(i32, i32)>,
    },
    AConstNull,
    NewArray(Type),
    MultiNewArray(Type, u8),
    ArrayLength,
    IALoad,
    IAStore,
    LALoad,
    LAStore,
    FALoad,
    FAStore,
    DALoad,
    DAStore,
    BALoad,
    BAStore,
    CALoad,
    CAStore,
    SALoad,
    SAStore,
    AALoad,
    AAStore,
    AThrow,
    IReturn,
    LReturn,
    FReturn,
    DReturn,
    AReturn,
    Return,
    LdcString(String),
    GetStatic(MemberRef),
    GetField(MemberRef),
    PutField(MemberRef),
    PutStatic(MemberRef),
    New(String),
    Dup,
    Pop,
    InvokeSpecial(MemberRef),
    InvokeStatic(MemberRef),
    InvokeVirtual(MemberRef),
    InvokeInterface(MemberRef),
    /// A structurally valid bytecode operation that has no AOT lowering yet.
    /// It is retained while discovering the class graph, then diagnosed only
    /// if the selected member-reachable closure actually executes it.
    Unsupported(String),
}

#[derive(Clone, Copy, Debug)]
enum IfKind {
    Eq,
    Ne,
    Lt,
    Ge,
    Gt,
    Le,
    ICmpEq,
    ICmpNe,
    ICmpLt,
    ICmpGe,
    ICmpGt,
    ICmpLe,
    ACmpEq,
    ACmpNe,
    Null,
    NonNull,
}

#[derive(Clone, Debug)]
struct Instruction {
    offset: u32,
    op: Op,
}

#[derive(Clone, Debug)]
struct ExceptionHandler {
    start: u32,
    end: u32,
    handler: u32,
    catch_type: Option<String>,
}

#[derive(Clone, Debug)]
struct Method {
    name: String,
    descriptor: String,
    signature: Signature,
    is_static: bool,
    is_public: bool,
    instructions: Vec<Instruction>,
    handlers: Vec<ExceptionHandler>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct MethodKey {
    name: String,
    descriptor: String,
}

impl MethodKey {
    fn new(name: impl Into<String>, descriptor: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            descriptor: descriptor.into(),
        }
    }
}

#[derive(Clone, Debug)]
struct Field {
    name: String,
    ty: Type,
    is_static: bool,
    is_final: bool,
    is_private: bool,
    constant: Option<Constant>,
}

#[derive(Clone, Debug)]
enum Constant {
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
}

#[derive(Clone, Debug)]
struct Program {
    name: String,
    superclass: Option<String>,
    interfaces: Vec<String>,
    is_interface: bool,
    fields: Vec<Field>,
    methods: Vec<Method>,
    clinit: Option<Method>,
}

fn invalid(message: impl Into<String>) -> CompileError {
    CompileError::InvalidClass(message.into())
}

fn parse_type(input: &str, cursor: &mut usize) -> Result<Type, CompileError> {
    let rest = &input[*cursor..];
    let ty = match rest.as_bytes().first() {
        Some(b'Z') => Type::Boolean,
        Some(b'B') => Type::Byte,
        Some(b'C') => Type::Char,
        Some(b'S') => Type::Short,
        Some(b'I') => Type::Int,
        Some(b'J') => Type::Long,
        Some(b'F') => Type::Float,
        Some(b'D') => Type::Double,
        Some(b'V') => Type::Void,
        Some(b'[') => {
            *cursor += 1;
            return Ok(Type::Array(Box::new(parse_type(input, cursor)?)));
        }
        Some(b'L') => {
            let end = rest
                .find(';')
                .ok_or_else(|| invalid("unterminated reference descriptor"))?;
            let name = &rest[1..end];
            if name == "java/lang/String" {
                *cursor += end + 1;
                return Ok(Type::String);
            }
            if !is_binary_class_name(name) {
                return Err(invalid("invalid reference descriptor"));
            }
            *cursor += end + 1;
            return Ok(Type::Class(name.to_owned()));
        }
        _ => return Err(invalid(format!("unsupported descriptor fragment `{rest}`"))),
    };
    *cursor += 1;
    Ok(ty)
}

/// Produces a generated-Rust default argument expression for a parameter
/// type, used by harness entrypoints (main and coverage compilation) that
/// call a Java method with placeholder values. Reference types pass `None`
/// (Java null); primitives pass zero.
pub(crate) fn default_argument(ty: &Type) -> Option<String> {
    let value = match ty {
        Type::Boolean => "false".to_owned(),
        Type::Byte => "0_i8".to_owned(),
        Type::Char => "0_u16".to_owned(),
        Type::Short => "0_i16".to_owned(),
        Type::Int => "0_i32".to_owned(),
        Type::Long => "0_i64".to_owned(),
        Type::Float => "0.0_f32".to_owned(),
        Type::Double => "0.0_f64".to_owned(),
        Type::String | Type::Class(_) | Type::Array(_) => "None".to_owned(),
        Type::Void => return None,
    };
    Some(value)
}

fn parse_signature(descriptor: &str) -> Result<Signature, CompileError> {
    let mut cursor = 0;
    if !descriptor.starts_with('(') {
        return Err(invalid(format!("invalid method descriptor `{descriptor}`")));
    }
    cursor += 1;
    let mut parameters = Vec::new();
    while descriptor.as_bytes().get(cursor) != Some(&b')') {
        parameters.push(parse_type(descriptor, &mut cursor)?);
    }
    cursor += 1;
    let returns = parse_type(descriptor, &mut cursor)?;
    if cursor != descriptor.len() {
        return Err(invalid(format!("invalid method descriptor `{descriptor}`")));
    }
    Ok(Signature {
        parameters,
        returns,
    })
}

fn array_component_type(name: &str) -> Result<Type, CompileError> {
    if name == "java/lang/String" {
        return Ok(Type::String);
    }
    if name.starts_with('[') {
        let mut cursor = 0;
        let ty = parse_type(name, &mut cursor)?;
        if cursor == name.len() {
            return Ok(ty);
        }
    }
    if is_binary_class_name(name) {
        Ok(Type::Class(name.to_owned()))
    } else {
        Err(invalid("invalid array component"))
    }
}

fn parser_error<T>(value: Result<T, classfile::Error>) -> Result<T, CompileError> {
    value.map_err(|error| CompileError::Parse(error.to_string()))
}
fn classfile_member(value: classfile::MemberRef) -> MemberRef {
    MemberRef {
        class: value.class,
        name: value.name,
        descriptor: value.descriptor,
    }
}

fn parse_custom_op(
    pool: &classfile::ConstantPool,
    raw: classfile::RawInstruction,
) -> Result<Op, CompileError> {
    use classfile::RawInstruction as R;
    Ok(match raw {
        R::AConstNull => Op::AConstNull,
        R::IConst(v) => Op::IConst(v),
        R::LConst(v) => Op::LConst(v),
        R::FConst(v) => Op::FConst(v),
        R::DConst(v) => Op::DConst(v),
        R::ILoad { index } => Op::ILoad(index.into()),
        R::IStore { index } => Op::IStore(index.into()),
        R::ALoad { index } => Op::ALoad(index.into()),
        R::AStore { index } => Op::AStore(index.into()),
        R::LLoad { index } => Op::LLoad(index.into()),
        R::LStore { index } => Op::LStore(index.into()),
        R::FLoad { index } => Op::FLoad(index.into()),
        R::FStore { index } => Op::FStore(index.into()),
        R::DLoad { index } => Op::DLoad(index.into()),
        R::DStore { index } => Op::DStore(index.into()),
        R::IAdd => Op::IAdd,
        R::ISub => Op::ISub,
        R::IMul => Op::IMul,
        R::IDiv => Op::IDiv,
        R::IRem => Op::IRem,
        R::INeg => Op::INeg,
        R::IAnd => Op::IAnd,
        R::IOr => Op::IOr,
        R::IXor => Op::IXor,
        R::IShl => Op::IShl,
        R::IShr => Op::IShr,
        R::IUshr => Op::IUshr,
        R::LAdd => Op::LAdd,
        R::LSub => Op::LSub,
        R::LMul => Op::LMul,
        R::LDiv => Op::LDiv,
        R::LRem => Op::LRem,
        R::LNeg => Op::LNeg,
        R::FAdd => Op::FAdd,
        R::FSub => Op::FSub,
        R::FMul => Op::FMul,
        R::FDiv => Op::FDiv,
        R::FRem => Op::FRem,
        R::FNeg => Op::FNeg,
        R::DAdd => Op::DAdd,
        R::DSub => Op::DSub,
        R::DMul => Op::DMul,
        R::DDiv => Op::DDiv,
        R::DRem => Op::DRem,
        R::DNeg => Op::DNeg,
        R::IInc { index, value } => Op::IInc(index.into(), value.into()),
        R::Goto { offset } => Op::Goto(offset),
        R::LookupSwitch { default, cases } => Op::LookupSwitch { default, cases },
        R::TableSwitch { default, cases } => Op::TableSwitch { default, cases },
        R::If { kind, offset } => Op::If(
            match kind {
                classfile::IfKind::Eq => IfKind::Eq,
                classfile::IfKind::Ne => IfKind::Ne,
                classfile::IfKind::Lt => IfKind::Lt,
                classfile::IfKind::Ge => IfKind::Ge,
                classfile::IfKind::Gt => IfKind::Gt,
                classfile::IfKind::Le => IfKind::Le,
                classfile::IfKind::ICmpEq => IfKind::ICmpEq,
                classfile::IfKind::ICmpNe => IfKind::ICmpNe,
                classfile::IfKind::ICmpLt => IfKind::ICmpLt,
                classfile::IfKind::ICmpGe => IfKind::ICmpGe,
                classfile::IfKind::ICmpGt => IfKind::ICmpGt,
                classfile::IfKind::ICmpLe => IfKind::ICmpLe,
                classfile::IfKind::ACmpEq => IfKind::ACmpEq,
                classfile::IfKind::ACmpNe => IfKind::ACmpNe,
                classfile::IfKind::Null => IfKind::Null,
                classfile::IfKind::NonNull => IfKind::NonNull,
            },
            offset,
        ),
        R::IReturn => Op::IReturn,
        R::LReturn => Op::LReturn,
        R::FReturn => Op::FReturn,
        R::DReturn => Op::DReturn,
        R::AReturn => Op::AReturn,
        R::Return => Op::Return,
        R::Dup => Op::Dup,
        R::Pop => Op::Pop,
        R::AThrow => Op::AThrow,
        R::MultiANewArray { index, dimensions } => {
            let descriptor = parser_error(pool.class_name(index))?;
            let mut cursor = 0;
            let ty = parse_type(&descriptor, &mut cursor)?;
            if cursor != descriptor.len() || !matches!(ty, Type::Array(_)) {
                return Err(invalid("invalid multianewarray descriptor"));
            }
            Op::MultiNewArray(ty, dimensions)
        }
        R::IALoad => Op::IALoad,
        R::IAStore => Op::IAStore,
        R::LALoad => Op::LALoad,
        R::LAStore => Op::LAStore,
        R::FALoad => Op::FALoad,
        R::FAStore => Op::FAStore,
        R::DALoad => Op::DALoad,
        R::DAStore => Op::DAStore,
        R::BALoad => Op::BALoad,
        R::BAStore => Op::BAStore,
        R::CALoad => Op::CALoad,
        R::CAStore => Op::CAStore,
        R::SALoad => Op::SALoad,
        R::SAStore => Op::SAStore,
        R::AALoad => Op::AALoad,
        R::AAStore => Op::AAStore,
        R::ArrayLength => Op::ArrayLength,
        R::New { index } => Op::New(parser_error(pool.class_name(index))?),
        R::ANewArray { index } => Op::NewArray(array_component_type(&parser_error(
            pool.class_name(index),
        )?)?),
        R::NewArray { atype } => Op::NewArray(match atype {
            classfile::ArrayType::Boolean => Type::Boolean,
            classfile::ArrayType::Byte => Type::Byte,
            classfile::ArrayType::Char => Type::Char,
            classfile::ArrayType::Short => Type::Short,
            classfile::ArrayType::Int => Type::Int,
            classfile::ArrayType::Long => Type::Long,
            classfile::ArrayType::Float => Type::Float,
            classfile::ArrayType::Double => Type::Double,
        }),
        R::GetStatic { index } => {
            Op::GetStatic(classfile_member(parser_error(pool.field_ref(index))?))
        }
        R::PutStatic { index } => {
            Op::PutStatic(classfile_member(parser_error(pool.field_ref(index))?))
        }
        R::GetField { index } => {
            Op::GetField(classfile_member(parser_error(pool.field_ref(index))?))
        }
        R::PutField { index } => {
            Op::PutField(classfile_member(parser_error(pool.field_ref(index))?))
        }
        R::InvokeSpecial { index } => {
            Op::InvokeSpecial(classfile_member(parser_error(pool.method_ref(index))?))
        }
        R::InvokeStatic { index } => {
            Op::InvokeStatic(classfile_member(parser_error(pool.method_ref(index))?))
        }
        R::InvokeVirtual { index } => {
            Op::InvokeVirtual(classfile_member(parser_error(pool.method_ref(index))?))
        }
        R::InvokeInterface { index } => Op::InvokeInterface(classfile_member(parser_error(
            pool.interface_method_ref(index),
        )?)),
        R::InvokeDynamic { .. } => Op::Unsupported("invokedynamic".to_owned()),
        R::Skipped { opcode } => {
            return Err(invalid(format!(
                "unsupported bytecode opcode 0x{opcode:02x}"
            )));
        }
        R::Ldc { index } => match parser_error(pool.constant(index))? {
            classfile::ConstantValue::String(v) => Op::LdcString(v.to_owned()),
            classfile::ConstantValue::Integer(v) => Op::IConst(v),
            classfile::ConstantValue::Float(v) => Op::FConst(v),
            _ => return Err(invalid("unsupported ldc literal")),
        },
        R::Ldc2 { index } => match parser_error(pool.constant(index))? {
            classfile::ConstantValue::Long(v) => Op::LConst(v),
            classfile::ConstantValue::Double(v) => Op::DConst(v),
            _ => return Err(invalid("unsupported ldc2 literal")),
        },
        R::Unsupported { opcode } => {
            return Err(invalid(format!(
                "unsupported bytecode opcode 0x{opcode:02x}"
            )));
        }
    })
}

fn parse_program_custom(bytes: &[u8]) -> Result<Program, CompileError> {
    let class = parser_error(classfile::parse(bytes))?;
    let pool = &class.constant_pool;
    let name = parser_error(pool.class_name(class.this_class))?;
    if !is_binary_class_name(&name) {
        return Err(invalid("invalid class binary name"));
    }
    let superclass = if class.super_class == 0 {
        None
    } else {
        Some(parser_error(pool.class_name(class.super_class))?)
    };
    let interfaces = class
        .interfaces
        .iter()
        .map(|i| parser_error(pool.class_name(*i)))
        .collect::<Result<Vec<_>, _>>()?;
    let fields = class
        .fields
        .iter()
        .map(|field| {
            let descriptor = parser_error(pool.utf8(field.descriptor_index))?;
            let mut cursor = 0;
            let ty = parse_type(descriptor, &mut cursor)?;
            if ty == Type::Void || cursor != descriptor.len() {
                return Err(invalid("invalid field descriptor"));
            }
            let mut constant = None;
            for attribute in &field.attributes {
                if let Some(index) = parser_error(classfile::constant_value(attribute))? {
                    if constant.is_some() {
                        return Err(invalid("field has multiple ConstantValue attributes"));
                    }
                    constant = Some(match parser_error(pool.constant(index))? {
                        classfile::ConstantValue::Integer(value) => Constant::Int(value),
                        classfile::ConstantValue::Long(value) => Constant::Long(value),
                        classfile::ConstantValue::Float(value) => Constant::Float(value),
                        classfile::ConstantValue::Double(value) => Constant::Double(value),
                        classfile::ConstantValue::String(value) => {
                            Constant::String(value.to_owned())
                        }
                        // ConstantValue attributes only ever carry primitive
                        // or String constants; a Class entry here would be an
                        // invalid class file.
                        classfile::ConstantValue::Class(_) => {
                            return Err(invalid(
                                "field ConstantValue attribute carries a class literal",
                            ));
                        }
                    });
                }
            }
            Ok(Field {
                name: parser_error(pool.utf8(field.name_index))?.to_owned(),
                ty,
                is_static: field.access_flags & 8 != 0,
                is_final: field.access_flags & 16 != 0,
                is_private: field.access_flags & 2 != 0,
                constant,
            })
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    let mut methods = Vec::new();
    let mut clinit = None;
    for member in &class.methods {
        let method_name = parser_error(pool.utf8(member.name_index))?.to_owned();
        let descriptor = parser_error(pool.utf8(member.descriptor_index))?.to_owned();
        let signature = parse_signature(&descriptor)?;
        let mut instructions = None;
        let mut handlers = Vec::new();
        for attribute in &member.attributes {
            if let Some(code) = parser_error(classfile::code(attribute, pool))? {
                handlers = code
                    .exception_handlers
                    .iter()
                    .map(|handler| {
                        Ok(ExceptionHandler {
                            start: handler.start.into(),
                            end: handler.end.into(),
                            handler: handler.handler.into(),
                            catch_type: if handler.catch_type == 0 {
                                None
                            } else {
                                Some(parser_error(pool.class_name(handler.catch_type))?)
                            },
                        })
                    })
                    .collect::<Result<Vec<_>, CompileError>>()?;
                instructions = Some(
                    parser_error(classfile::instructions(&code.code))?
                        .into_iter()
                        .map(|(offset, raw)| {
                            match parse_custom_op(pool, raw) {
                                Ok(op) => Ok(Instruction { offset, op }),
                                // Decode every method in a real library class,
                                // but defer the lowering diagnostic until the
                                // member is selected by the JAR entrypoint.
                                Err(CompileError::InvalidClass(operation)) => Ok(Instruction {
                                    offset,
                                    op: Op::Unsupported(operation),
                                }),
                                Err(other) => Err(other),
                            }
                        })
                        .collect::<Result<Vec<_>, CompileError>>()?,
                );
            }
        }
        let parsed = Method {
            name: method_name.clone(),
            descriptor,
            signature,
            is_static: member.access_flags & 8 != 0,
            is_public: member.access_flags & 1 != 0,
            instructions: match instructions {
                Some(value) => value,
                None if member.access_flags & 0x400 != 0 => Vec::new(),
                None => return Err(invalid("methods must have Code attributes")),
            },
            handlers,
        };
        if method_name == "<clinit>" {
            clinit = Some(parsed)
        } else {
            methods.push(parsed)
        }
    }
    Ok(Program {
        name,
        superclass,
        interfaces,
        is_interface: class.access_flags & 0x200 != 0,
        fields,
        methods,
        clinit,
    })
}

fn rust_ident(name: &str) -> Result<String, CompileError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    {
        return Err(invalid(format!("unsupported Rust identifier `{name}`")));
    }
    let escaped = match name {
        "as" | "async" | "await" | "box" | "break" | "const" | "continue" | "crate" | "dyn"
        | "else" | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let"
        | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "Self"
        | "self" | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe" | "use"
        | "where" | "while" => format!("r#{name}"),
        _ => name.to_owned(),
    };
    Ok(escaped)
}

fn is_binary_class_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|segment| {
            !segment.is_empty()
                && !segment
                    .bytes()
                    .any(|byte| matches!(byte, b'.' | b';' | b'['))
        })
}

/// Maps a JVM binary class name to a Rust identifier without changing the
/// familiar spelling of ordinary default-package fixture classes. Package
/// separators, inner-class markers, and non-ASCII names are encoded as UTF-8
/// bytes so the generated identifier never needs to interpret Java names at
/// runtime.
fn class_ident(name: &str) -> Result<String, CompileError> {
    if !is_binary_class_name(name) {
        return Err(invalid(format!("invalid class binary name `{name}`")));
    }
    if !name.contains('/') {
        if let Ok(identifier) = rust_ident(name) {
            return Ok(identifier);
        }
    }
    Ok(format!(
        "__jars_class_{}",
        name.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn validate_class_idents(programs: &[Program]) -> Result<(), CompileError> {
    let mut by_identifier = HashMap::new();
    for program in programs {
        let identifier = class_ident(&program.name)?;
        if let Some(previous) = by_identifier.insert(identifier.clone(), &program.name) {
            return Err(invalid(format!(
                "classes `{previous}` and `{}` both map to Rust identifier `{identifier}`",
                program.name
            )));
        }
    }
    Ok(())
}

fn unsupported(
    program: &Program,
    method: &Method,
    instruction: &Instruction,
    operation: impl Into<String>,
) -> CompileError {
    CompileError::Unsupported {
        class: program.name.clone(),
        method: method.name.clone(),
        offset: instruction.offset,
        operation: operation.into(),
    }
}

fn stack_error(
    program: &Program,
    method: &Method,
    instruction: &Instruction,
    detail: impl Into<String>,
) -> CompileError {
    CompileError::InvalidStack {
        class: program.name.clone(),
        method: method.name.clone(),
        offset: instruction.offset,
        detail: detail.into(),
    }
}

#[derive(Clone)]
enum Value {
    Int(String),
    /// A `boolean`-typed stack value. Its text is a Rust `bool` expression,
    /// distinct from [`Value::Int`] whose text is an `i32`.
    Boolean(String),
    Long(String),
    Float(String),
    Double(String),
    String(String),
    /// An instance of a registered stdlib class (`stdlib::StdClass`), such as
    /// `System.out`.  `expression` is always `Option<...>`-typed text, the
    /// same convention `Object` uses, so instance-method lowering can reuse
    /// the same `.ok_or_else(jars_runtime::null_pointer)?` unwrap template.
    Platform {
        expression: String,
        class: String,
    },
    This,
    Object {
        expression: String,
        class: String,
    },
    Array {
        expression: String,
        element: Type,
    },
    Null,
    Uninitialized(usize),
}

impl Value {
    fn expression(&self) -> Option<&str> {
        match self {
            Self::Int(value)
            | Self::Long(value)
            | Self::Float(value)
            | Self::Double(value)
            | Self::String(value) => Some(value),
            Self::Boolean(value) => Some(value),
            Self::Platform { expression, .. } => Some(expression),
            Self::Object { expression, .. } => Some(expression),
            Self::Array { expression, .. } => Some(expression),
            Self::Null => Some("None"),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum BodyKind {
    Static,
    Instance,
}

struct Body<'a> {
    program: &'a Program,
    known_classes: &'a [String],
    method: &'a Method,
    stack: Vec<Value>,
    locals: Vec<Option<Value>>,
    statements: Vec<String>,
    next_temp: usize,
    next_uninitialized: usize,
}

impl<'a> Body<'a> {
    fn new(
        program: &'a Program,
        known_classes: &'a [String],
        method: &'a Method,
        kind: BodyKind,
    ) -> Self {
        let mut locals = Vec::new();
        match kind {
            BodyKind::Static => {}
            BodyKind::Instance => locals.push(Some(Value::This)),
        }
        let mut slot = locals.len();
        for (index, ty) in method.signature.parameters.iter().enumerate() {
            let value = match ty {
                Type::Boolean | Type::Byte | Type::Char | Type::Short | Type::Int => {
                    Value::Int(format!("arg{index}"))
                }
                Type::Long => Value::Long(format!("arg{index}")),
                Type::Float => Value::Float(format!("arg{index}")),
                Type::Double => Value::Double(format!("arg{index}")),
                Type::String => Value::String(format!("arg{index}")),
                Type::Array(element) => Value::Array {
                    expression: format!("arg{index}"),
                    element: (**element).clone(),
                },
                Type::Class(class)
                    if stdlib::class(class)
                        .and_then(|entry| entry.rust_type)
                        .is_some() =>
                {
                    Value::Platform {
                        expression: format!("arg{index}"),
                        class: class.clone(),
                    }
                }
                Type::Class(class) => Value::Object {
                    expression: format!("arg{index}"),
                    class: class.clone(),
                },
                Type::Void => unreachable!(),
            };
            if locals.len() <= slot {
                locals.resize(slot + 1, None);
            }
            locals[slot] = Some(value);
            slot += matches!(ty, Type::Long | Type::Double) as usize + 1;
        }
        Self {
            program,
            known_classes,
            method,
            stack: Vec::new(),
            locals,
            statements: Vec::new(),
            next_temp: 0,
            next_uninitialized: 0,
        }
    }

    fn class_path(&self, class: &str) -> Result<String, CompileError> {
        if class == self.program.name {
            Ok(String::new())
        } else if self.known_classes.iter().any(|known| known == class) {
            Ok(format!("super::{}::", class_ident(class)?))
        } else {
            Err(invalid(format!(
                "class `{class}` is not in the compilation set"
            )))
        }
    }

    fn pop(&mut self, instruction: &Instruction) -> Result<Value, CompileError> {
        self.stack
            .pop()
            .ok_or_else(|| stack_error(self.program, self.method, instruction, "stack underflow"))
    }

    fn pop_expression(&mut self, instruction: &Instruction) -> Result<String, CompileError> {
        self.pop(instruction)?
            .expression()
            .map(str::to_owned)
            .ok_or_else(|| {
                stack_error(
                    self.program,
                    self.method,
                    instruction,
                    "expected a value expression",
                )
            })
    }

    fn local(&self, index: usize, instruction: &Instruction) -> Result<Value, CompileError> {
        self.locals
            .get(index)
            .and_then(Clone::clone)
            .ok_or_else(|| {
                stack_error(
                    self.program,
                    self.method,
                    instruction,
                    format!("uninitialized local {index}"),
                )
            })
    }

    fn set_local(&mut self, index: usize, value: Value) {
        if self.locals.len() <= index {
            self.locals.resize(index + 1, None);
        }
        self.locals[index] = Some(value);
    }

    fn temp<F>(&mut self, expression: String, value: F) -> Value
    where
        F: FnOnce(String) -> Value,
    {
        let name = format!("value{}", self.next_temp);
        self.next_temp += 1;
        self.statements.push(format!("let {name} = {expression};"));
        value(name)
    }

    fn arguments(
        &mut self,
        instruction: &Instruction,
        signature: &Signature,
    ) -> Result<Vec<String>, CompileError> {
        let mut args = Vec::with_capacity(signature.parameters.len());
        for parameter in signature.parameters.iter().rev() {
            let value = self.pop(instruction)?;
            let expression = match (parameter, value) {
                (Type::Class(class), Value::String(expression)) => {
                    stdlib::coerce("java/lang/String", class, &expression).ok_or_else(|| {
                        stack_error(
                            self.program,
                            self.method,
                            instruction,
                            format!("cannot pass String as `{class}`"),
                        )
                    })?
                }
                (Type::Class(target), Value::Platform { expression, class })
                    if class != *target =>
                {
                    stdlib::coerce(&class, target, &expression).ok_or_else(|| {
                        stack_error(
                            self.program,
                            self.method,
                            instruction,
                            format!("cannot pass platform class `{class}` as `{target}`"),
                        )
                    })?
                }
                (Type::Class(_) | Type::Array(_), Value::Null) => "None".to_owned(),
                // Narrow and boolean stack operands are carried as i32 text;
                // cast them to the parameter's Rust type at the call site.
                (Type::Boolean, Value::Int(expression)) => format!("{expression} != 0"),
                (Type::Byte, Value::Int(expression)) => format!("{expression} as i8"),
                (Type::Char, Value::Int(expression)) => format!("{expression} as u16"),
                (Type::Short, Value::Int(expression)) => format!("{expression} as i16"),
                (_, value) => value.expression().map(str::to_owned).ok_or_else(|| {
                    stack_error(
                        self.program,
                        self.method,
                        instruction,
                        "cannot pass this value as a method argument",
                    )
                })?,
            };
            args.push(expression);
        }
        args.reverse();
        Ok(args)
    }

    fn pop_array(&mut self, instruction: &Instruction) -> Result<(String, Type), CompileError> {
        match self.pop(instruction)? {
            Value::Array {
                expression,
                element,
            } => Ok((expression, element)),
            value => Err(stack_error(
                self.program,
                self.method,
                instruction,
                format!(
                    "expected array reference, found {}",
                    match value {
                        Value::Null => "null without a verifier type",
                        _ => "non-array value",
                    }
                ),
            )),
        }
    }

    fn array_default(element: &Type) -> Result<String, CompileError> {
        Ok(match element {
            Type::Boolean => "false".to_owned(),
            Type::Byte => "0_i8".to_owned(),
            Type::Char => "0_u16".to_owned(),
            Type::Short => "0_i16".to_owned(),
            Type::Int => "0_i32".to_owned(),
            Type::Long => "0_i64".to_owned(),
            Type::Float => "0.0_f32".to_owned(),
            Type::Double => "0.0_f64".to_owned(),
            Type::String => "None".to_owned(),
            Type::Class(_) | Type::Array(_) => "None".to_owned(),
            Type::Void => return Err(invalid("void cannot be an array element")),
        })
    }

    fn multi_array_code(
        &mut self,
        ty: &Type,
        active_dimensions: usize,
        lengths: &[String],
        depth: usize,
    ) -> Result<(String, String), CompileError> {
        let Type::Array(element) = ty else {
            return Err(invalid("multianewarray descriptor is not an array"));
        };
        let name = format!("array{}", self.next_temp);
        self.next_temp += 1;
        let element_rust = element.array_element_rust(&self.program.name)?;
        let default = if depth + 1 < active_dimensions {
            "None".to_owned()
        } else {
            Self::array_default(element)?
        };
        let mut code = format!(
            "let {name} = Some({}?);",
            crate::model::array_new(&element_rust, &lengths[depth], &default)
        );
        if depth + 1 < active_dimensions {
            let (child, child_code) =
                self.multi_array_code(element, active_dimensions, lengths, depth + 1)?;
            let index = format!("index{}", self.next_temp);
            self.next_temp += 1;
            code.push_str(&format!(
                "for {index} in 0..{} {{ {child_code} {name}.as_ref().expect(\"new Java array\").{}({index}, {child}){}?; }}",
                lengths[depth],
                crate::model::runtime_method_name("set"),
                crate::model::await_token()
            ));
        }
        Ok((name, code))
    }

    fn run(mut self) -> Result<String, CompileError> {
        for instruction in &self.method.instructions {
            match &instruction.op {
                Op::Unsupported(operation) => {
                    return Err(unsupported(
                        self.program,
                        self.method,
                        instruction,
                        operation.clone(),
                    ));
                }
                Op::AConstNull => self.stack.push(Value::Null),
                Op::NewArray(element) => {
                    let length = self.pop_expression(instruction)?;
                    let rust = element.array_element_rust(&self.program.name)?;
                    let default = Self::array_default(element)?;
                    let expression = format!(
                        "Some({}?)",
                        crate::model::array_new(&rust, &length, &default)
                    );
                    let value = self.temp(expression, |expression| Value::Array {
                        expression,
                        element: element.clone(),
                    });
                    self.stack.push(value);
                }
                Op::MultiNewArray(ty, dimensions) => {
                    let Type::Array(element) = ty else {
                        unreachable!()
                    };
                    let mut lengths = Vec::with_capacity((*dimensions).into());
                    for _ in 0..*dimensions {
                        lengths.push(self.pop_expression(instruction)?);
                    }
                    lengths.reverse();
                    let (expression, code) =
                        self.multi_array_code(ty, (*dimensions).into(), &lengths, 0)?;
                    self.statements.push(code);
                    self.stack.push(Value::Array {
                        expression,
                        element: (**element).clone(),
                    });
                }
                Op::ArrayLength => {
                    let (array, _) = self.pop_array(instruction)?;
                    let value = self.temp(
                        format!(
                            "{array}.ok_or_else(jars_runtime::null_pointer)?.{}(){}?",
                            crate::model::runtime_method_name("length"),
                            crate::model::await_token()
                        ),
                        Value::Int,
                    );
                    self.stack.push(value);
                }
                Op::IALoad
                | Op::LALoad
                | Op::FALoad
                | Op::DALoad
                | Op::BALoad
                | Op::CALoad
                | Op::SALoad => {
                    let index = self.pop_expression(instruction)?;
                    let (array, element) = self.pop_array(instruction)?;
                    let get = format!(
                        "{array}.ok_or_else(jars_runtime::null_pointer)?.{}({index}){}?",
                        crate::model::runtime_method_name("get"),
                        crate::model::await_token()
                    );
                    let (expression, value): (String, Value) = match &instruction.op {
                        Op::IALoad => (get, Value::Int(String::new())),
                        Op::LALoad => (get, Value::Long(String::new())),
                        Op::FALoad => (get, Value::Float(String::new())),
                        Op::DALoad => (get, Value::Double(String::new())),
                        Op::BALoad | Op::CALoad | Op::SALoad => {
                            (format!("({get}) as i32"), Value::Int(String::new()))
                        }
                        _ => unreachable!(),
                    };
                    let valid = matches!(
                        (&instruction.op, &element),
                        (Op::IALoad, Type::Int)
                            | (Op::LALoad, Type::Long)
                            | (Op::FALoad, Type::Float)
                            | (Op::DALoad, Type::Double)
                            | (Op::BALoad, Type::Boolean | Type::Byte)
                            | (Op::CALoad, Type::Char)
                            | (Op::SALoad, Type::Short)
                    );
                    if !valid {
                        return Err(stack_error(
                            self.program,
                            self.method,
                            instruction,
                            "array opcode does not match element type",
                        ));
                    }
                    let value = match value {
                        Value::Int(_) => self.temp(expression, Value::Int),
                        Value::Long(_) => self.temp(expression, Value::Long),
                        Value::Float(_) => self.temp(expression, Value::Float),
                        Value::Double(_) => self.temp(expression, Value::Double),
                        _ => unreachable!(),
                    };
                    self.stack.push(value);
                }
                Op::IAStore
                | Op::LAStore
                | Op::FAStore
                | Op::DAStore
                | Op::BAStore
                | Op::CAStore
                | Op::SAStore => {
                    let value = self.pop_expression(instruction)?;
                    let index = self.pop_expression(instruction)?;
                    let (array, element) = self.pop_array(instruction)?;
                    let stored = match (&instruction.op, &element) {
                        (Op::IAStore, Type::Int)
                        | (Op::LAStore, Type::Long)
                        | (Op::FAStore, Type::Float)
                        | (Op::DAStore, Type::Double) => value,
                        (Op::BAStore, Type::Boolean) => format!("{value} != 0"),
                        (Op::BAStore, Type::Byte) => format!("{value} as i8"),
                        (Op::CAStore, Type::Char) => format!("{value} as u16"),
                        (Op::SAStore, Type::Short) => format!("{value} as i16"),
                        _ => {
                            return Err(stack_error(
                                self.program,
                                self.method,
                                instruction,
                                "array opcode does not match element type",
                            ));
                        }
                    };
                    self.statements.push(format!(
                        "{array}.ok_or_else(jars_runtime::null_pointer)?.{}({index}, {stored}){}?;",
                        crate::model::runtime_method_name("set"),
                        crate::model::await_token()
                    ));
                }
                Op::AALoad => {
                    let index = self.pop_expression(instruction)?;
                    let (array, element) = self.pop_array(instruction)?;
                    let expression = format!(
                        "{array}.ok_or_else(jars_runtime::null_pointer)?.{}({index}){}?",
                        crate::model::runtime_method_name("get"),
                        crate::model::await_token()
                    );
                    let value = match element {
                        Type::Class(class) => {
                            self.temp(expression, |expression| Value::Object { expression, class })
                        }
                        Type::Array(element) => self.temp(expression, |expression| Value::Array {
                            expression,
                            element: *element,
                        }),
                        Type::String => self.temp(expression, Value::String),
                        _ => {
                            return Err(stack_error(
                                self.program,
                                self.method,
                                instruction,
                                "aaload on primitive array",
                            ));
                        }
                    };
                    self.stack.push(value);
                }
                Op::AAStore => {
                    let value = self.pop_expression(instruction)?;
                    let index = self.pop_expression(instruction)?;
                    let (array, element) = self.pop_array(instruction)?;
                    if !matches!(element, Type::Class(_) | Type::Array(_) | Type::String) {
                        return Err(stack_error(
                            self.program,
                            self.method,
                            instruction,
                            "aastore on primitive array",
                        ));
                    }
                    self.statements.push(format!(
                        "{array}.ok_or_else(jars_runtime::null_pointer)?.{}({index}, {value}){}?;",
                        crate::model::runtime_method_name("set"),
                        crate::model::await_token()
                    ));
                }
                Op::AThrow => {
                    return Err(unsupported(
                        self.program,
                        self.method,
                        instruction,
                        "athrow requires exception-table state-machine lowering",
                    ));
                }
                Op::IConst(value) => self.stack.push(Value::Int(value.to_string())),
                Op::LConst(value) => self.stack.push(Value::Long(format!("{value}_i64"))),
                Op::FConst(value) => self.stack.push(Value::Float(format!("{value:?}_f32"))),
                Op::DConst(value) => self.stack.push(Value::Double(format!("{value:?}_f64"))),
                Op::LdcString(value) => self.stack.push(Value::String(format!(
                    "Some(jars_runtime::JavaString::new({value:?}))"
                ))),
                Op::ILoad(index) => self.stack.push(self.local(*index, instruction)?),
                Op::LLoad(index) | Op::FLoad(index) | Op::DLoad(index) => {
                    self.stack.push(self.local(*index, instruction)?)
                }
                Op::ALoad(index) => {
                    let value = self.local(*index, instruction)?;
                    self.stack.push(match value {
                        Value::Platform { expression, class } => Value::Platform {
                            expression: format!("{expression}.clone()"),
                            class,
                        },
                        Value::Object { expression, class } => Value::Object {
                            expression: format!("{expression}.clone()"),
                            class,
                        },
                        Value::Array {
                            expression,
                            element,
                        } => Value::Array {
                            expression: format!("{expression}.clone()"),
                            element,
                        },
                        other => other,
                    });
                }
                Op::IStore(index)
                | Op::AStore(index)
                | Op::LStore(index)
                | Op::FStore(index)
                | Op::DStore(index) => {
                    let value = self.pop(instruction)?;
                    let expression = value.expression().ok_or_else(|| {
                        stack_error(
                            self.program,
                            self.method,
                            instruction,
                            "cannot store this stack value",
                        )
                    })?;
                    let local = format!("local{index}");
                    self.statements.push(format!("let {local} = {expression};"));
                    self.set_local(
                        *index,
                        match value {
                            Value::Int(_) => Value::Int(local),
                            Value::Long(_) => Value::Long(local),
                            Value::Float(_) => Value::Float(local),
                            Value::Double(_) => Value::Double(local),
                            Value::String(_) => Value::String(local),
                            Value::Platform { class, .. } => Value::Platform {
                                expression: local,
                                class,
                            },
                            Value::Object { class, .. } => Value::Object {
                                expression: local,
                                class,
                            },
                            Value::Array { element, .. } => Value::Array {
                                expression: local,
                                element,
                            },
                            Value::Null => Value::Null,
                            _ => unreachable!(),
                        },
                    );
                }
                Op::IAdd => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let value = self.temp(format!("{left}.wrapping_add({right})"), Value::Int);
                    self.stack.push(value);
                }
                Op::ISub => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let value = self.temp(format!("{left}.wrapping_sub({right})"), Value::Int);
                    self.stack.push(value);
                }
                Op::IMul => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let value = self.temp(format!("{left}.wrapping_mul({right})"), Value::Int);
                    self.stack.push(value);
                }
                Op::IDiv => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let value =
                        self.temp(format!("jars_runtime::idiv({left}, {right})?"), Value::Int);
                    self.stack.push(value);
                }
                Op::IRem => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let value =
                        self.temp(format!("jars_runtime::irem({left}, {right})?"), Value::Int);
                    self.stack.push(value);
                }
                Op::LAdd | Op::LSub | Op::LMul | Op::LDiv | Op::LRem => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let expression = match &instruction.op {
                        Op::LAdd => format!("{left}.wrapping_add({right})"),
                        Op::LSub => format!("{left}.wrapping_sub({right})"),
                        Op::LMul => format!("{left}.wrapping_mul({right})"),
                        Op::LDiv => format!("jars_runtime::ldiv({left}, {right})?"),
                        Op::LRem => format!("jars_runtime::lrem({left}, {right})?"),
                        _ => unreachable!(),
                    };
                    let value = self.temp(expression, Value::Long);
                    self.stack.push(value);
                }
                Op::LNeg => {
                    let value = self.pop_expression(instruction)?;
                    let value = self.temp(format!("{value}.wrapping_neg()"), Value::Long);
                    self.stack.push(value);
                }
                Op::FAdd | Op::FSub | Op::FMul | Op::FDiv | Op::FRem => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let operation = match &instruction.op {
                        Op::FAdd => "+",
                        Op::FSub => "-",
                        Op::FMul => "*",
                        Op::FDiv => "/",
                        Op::FRem => "%",
                        _ => unreachable!(),
                    };
                    let value = self.temp(format!("{left} {operation} {right}"), Value::Float);
                    self.stack.push(value);
                }
                Op::FNeg => {
                    let value = self.pop_expression(instruction)?;
                    let value = self.temp(format!("-{value}"), Value::Float);
                    self.stack.push(value);
                }
                Op::DAdd | Op::DSub | Op::DMul | Op::DDiv | Op::DRem => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let operation = match &instruction.op {
                        Op::DAdd => "+",
                        Op::DSub => "-",
                        Op::DMul => "*",
                        Op::DDiv => "/",
                        Op::DRem => "%",
                        _ => unreachable!(),
                    };
                    let value = self.temp(format!("{left} {operation} {right}"), Value::Double);
                    self.stack.push(value);
                }
                Op::DNeg => {
                    let value = self.pop_expression(instruction)?;
                    let value = self.temp(format!("-{value}"), Value::Double);
                    self.stack.push(value);
                }
                Op::INeg => {
                    let value = self.pop_expression(instruction)?;
                    let value = self.temp(format!("{value}.wrapping_neg()"), Value::Int);
                    self.stack.push(value);
                }
                Op::IAnd | Op::IOr | Op::IXor | Op::IShl | Op::IShr | Op::IUshr => {
                    let right = self.pop_expression(instruction)?;
                    let left = self.pop_expression(instruction)?;
                    let expression = match &instruction.op {
                        Op::IAnd => format!("{left} & {right}"),
                        Op::IOr => format!("{left} | {right}"),
                        Op::IXor => format!("{left} ^ {right}"),
                        Op::IShl => format!("{left}.wrapping_shl(({right} as u32) & 31)"),
                        Op::IShr => format!("{left}.wrapping_shr(({right} as u32) & 31)"),
                        Op::IUshr => format!("jars_runtime::iushr({left}, {right})"),
                        _ => unreachable!(),
                    };
                    let value = self.temp(expression, Value::Int);
                    self.stack.push(value);
                }
                Op::GetStatic(reference) => {
                    if let Some(stdlib::StdMember::StaticField { lower }) =
                        stdlib::member(&reference.class, &reference.name, &reference.descriptor)
                    {
                        let mut cursor = 0;
                        let Type::Class(value_class) =
                            parse_type(&reference.descriptor, &mut cursor)?
                        else {
                            return Err(invalid(format!(
                                "stdlib static field `{}.{}` has a non-class descriptor",
                                reference.class, reference.name
                            )));
                        };
                        self.stack.push(Value::Platform {
                            expression: lower(),
                            class: value_class,
                        });
                    } else {
                        if !self
                            .known_classes
                            .iter()
                            .any(|known| known == &reference.class)
                        {
                            return Err(unsupported(
                                self.program,
                                self.method,
                                instruction,
                                "getstatic",
                            ));
                        }
                        let class = class_ident(&reference.class)?;
                        let class_path = self.class_path(&reference.class)?;
                        let field = rust_ident(&reference.name)?;
                        self.statements
                            .push(format!("{class_path}__ensure(program).await?;"));
                        let mut cursor = 0;
                        let ty = parse_type(&reference.descriptor, &mut cursor)?;
                        if cursor != reference.descriptor.len() {
                            return Err(unsupported(
                                self.program,
                                self.method,
                                instruction,
                                "getstatic descriptor",
                            ));
                        }
                        let expression = format!("program.state.borrow().{class}.{field}.clone()");
                        let value = match ty {
                            Type::Boolean => self.temp(expression, Value::Boolean),
                            Type::Byte | Type::Char | Type::Short | Type::Int => {
                                self.temp(expression, Value::Int)
                            }
                            Type::Long => self.temp(expression, Value::Long),
                            Type::Float => self.temp(expression, Value::Float),
                            Type::Double => self.temp(expression, Value::Double),
                            Type::String => self.temp(expression, Value::String),
                            Type::Array(element) => {
                                self.temp(expression, |expression| Value::Array {
                                    expression,
                                    element: (*element).clone(),
                                })
                            }
                            Type::Class(class) => {
                                self.temp(expression, |expression| Value::Object {
                                    expression,
                                    class: class.clone(),
                                })
                            }
                            Type::Void => unreachable!(),
                        };
                        self.stack.push(value);
                    }
                }
                Op::GetField(reference) => {
                    if !self
                        .known_classes
                        .iter()
                        .any(|known| known == &reference.class)
                    {
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "getfield",
                        ));
                    }
                    let receiver = self.pop(instruction)?;
                    let field = rust_ident(&reference.name)?;
                    let mut cursor = 0;
                    let ty = parse_type(&reference.descriptor, &mut cursor)?;
                    if cursor != reference.descriptor.len() {
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "getfield descriptor",
                        ));
                    }
                    let expression = match receiver {
                        Value::This if reference.class == self.program.name => match ty {
                            Type::Int | Type::Long | Type::Float | Type::Double => {
                                format!("{}.{field}", crate::model::state_read("state"))
                            }
                            _ => format!("{}.{field}.clone()", crate::model::state_read("state")),
                        },
                        Value::Object { expression, class } if class == reference.class => format!(
                            "{expression}.ok_or_else(jars_runtime::null_pointer)?.__get_{field}().await?"
                        ),
                        Value::This => {
                            return Err(unsupported(
                                self.program,
                                self.method,
                                instruction,
                                "inherited getfield",
                            ));
                        }
                        _ => {
                            return Err(stack_error(
                                self.program,
                                self.method,
                                instruction,
                                "getfield receiver has incompatible class",
                            ));
                        }
                    };
                    let value = match ty {
                        Type::Boolean | Type::Byte | Type::Char | Type::Short | Type::Int => {
                            self.temp(expression, Value::Int)
                        }
                        Type::Long => self.temp(expression, Value::Long),
                        Type::Float => self.temp(expression, Value::Float),
                        Type::Double => self.temp(expression, Value::Double),
                        Type::String => self.temp(expression, Value::String),
                        Type::Array(element) => self.temp(expression, |expression| Value::Array {
                            expression,
                            element: (*element).clone(),
                        }),
                        Type::Class(class) => self.temp(expression, |expression| Value::Object {
                            expression,
                            class: class.clone(),
                        }),
                        Type::Void => unreachable!(),
                    };
                    self.stack.push(value);
                }
                Op::PutField(reference) => {
                    if !self
                        .known_classes
                        .iter()
                        .any(|known| known == &reference.class)
                    {
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "putfield",
                        ));
                    }
                    let value = self.pop_expression(instruction)?;
                    let receiver = self.pop(instruction)?;
                    let field = rust_ident(&reference.name)?;
                    match receiver {
                        Value::This if reference.class == self.program.name => {
                            self.statements.push(format!(
                                "{}.{field} = {value};",
                                crate::model::state_write("state")
                            ));
                        }
                        Value::Object { expression, class } if class == reference.class => {
                            self.statements.push(format!(
                                "{expression}.ok_or_else(jars_runtime::null_pointer)?.__set_{field}({value}).await?;"
                            ));
                        }
                        Value::This => {
                            return Err(unsupported(
                                self.program,
                                self.method,
                                instruction,
                                "inherited putfield",
                            ));
                        }
                        _ => {
                            return Err(stack_error(
                                self.program,
                                self.method,
                                instruction,
                                "putfield receiver has incompatible class",
                            ));
                        }
                    }
                }
                Op::PutStatic(reference) => {
                    if !self
                        .known_classes
                        .iter()
                        .any(|known| known == &reference.class)
                    {
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "putstatic",
                        ));
                    }
                    let value = self.pop_expression(instruction)?;
                    let class = class_ident(&reference.class)?;
                    let class_path = self.class_path(&reference.class)?;
                    let field = rust_ident(&reference.name)?;
                    self.statements
                        .push(format!("{class_path}__ensure(program).await?;"));
                    self.statements.push(format!(
                        "program.state.borrow_mut().{class}.{field} = {value};"
                    ));
                }
                Op::New(class) => {
                    if !self.known_classes.iter().any(|known| known == class)
                        && !stdlib::is_constructible(class)
                    {
                        return Err(unsupported(self.program, self.method, instruction, "new"));
                    }
                    let id = self.next_uninitialized;
                    self.next_uninitialized += 1;
                    self.stack.push(Value::Uninitialized(id));
                }
                Op::Dup => {
                    let value = self.stack.last().cloned().ok_or_else(|| {
                        stack_error(
                            self.program,
                            self.method,
                            instruction,
                            "dup on an empty stack",
                        )
                    })?;
                    // Cloning an array stack entry clones its `Option<…>`
                    // expression text, not the referenced runtime value, so
                    // each dup consumer gets an independent handle text.
                    let value = match value {
                        Value::Array {
                            expression,
                            element,
                        } => Value::Array {
                            expression: format!("{expression}.clone()"),
                            element,
                        },
                        other => other,
                    };
                    self.stack.push(value);
                }
                Op::Pop => {
                    // The discarded value's side effects, if any, were already
                    // emitted as a statement when it was produced (`Body::temp`
                    // always pushes a `let` binding immediately), so dropping
                    // the stack entry here is sufficient.
                    self.pop(instruction)?;
                }
                Op::InvokeSpecial(reference) => {
                    let signature = parse_signature(&reference.descriptor)?;
                    let args = self.arguments(instruction, &signature)?;
                    let receiver = self.pop(instruction)?;
                    let stdlib_ctor =
                        stdlib::member(&reference.class, &reference.name, &reference.descriptor)
                            .and_then(|member| match member {
                                stdlib::StdMember::Constructor { lower } => Some(lower),
                                _ => None,
                            });
                    if let Some(lower) = stdlib_ctor {
                        match lower(&args) {
                            None => {
                                if !matches!(receiver, Value::This) || !args.is_empty() {
                                    return Err(unsupported(
                                        self.program,
                                        self.method,
                                        instruction,
                                        "stdlib no-op constructor",
                                    ));
                                }
                            }
                            Some(expression) => {
                                let Value::Uninitialized(id) = receiver else {
                                    return Err(stack_error(
                                        self.program,
                                        self.method,
                                        instruction,
                                        "stdlib constructor receiver is not uninitialized",
                                    ));
                                };
                                let variable = format!("platform_object{id}");
                                self.statements
                                    .push(format!("let {variable} = {expression};"));
                                for value in &mut self.stack {
                                    if matches!(value, Value::Uninitialized(other) if *other == id)
                                    {
                                        *value = Value::Platform {
                                            expression: variable.clone(),
                                            class: reference.class.clone(),
                                        };
                                    }
                                }
                            }
                        }
                    } else if self
                        .known_classes
                        .iter()
                        .any(|known| known == &reference.class)
                        && reference.name == "<init>"
                        && matches!(receiver, Value::This)
                        && args.is_empty()
                    {
                        // A derived actor owns the dynamic object's state.  An
                        // empty superclass constructor has no independent actor
                        // to initialize, so its constructor edge is represented
                        // by the already-created derived state.
                    } else if self
                        .known_classes
                        .iter()
                        .any(|known| known == &reference.class)
                        && reference.name == "<init>"
                    {
                        let Value::Uninitialized(id) = receiver else {
                            return Err(stack_error(
                                self.program,
                                self.method,
                                instruction,
                                "constructor receiver is not uninitialized",
                            ));
                        };
                        let variable = format!("object{id}");
                        let class = format!(
                            "{}{}",
                            self.class_path(&reference.class)?,
                            class_ident(&reference.class)?
                        );
                        self.statements.push(format!(
                            "{}__ensure(program).await?;",
                            self.class_path(&reference.class)?
                        ));
                        let call_arguments = if args.is_empty() {
                            "program".to_owned()
                        } else {
                            format!("program, {}", args.join(", "))
                        };
                        self.statements.push(format!(
                            "let {variable} = {class}::new({call_arguments}).await?;",
                        ));
                        for value in &mut self.stack {
                            if matches!(value, Value::Uninitialized(other) if *other == id) {
                                *value = Value::Object {
                                    expression: format!("Some({variable})"),
                                    class: reference.class.clone(),
                                };
                            }
                        }
                    } else {
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "invokespecial",
                        ));
                    }
                }
                Op::InvokeStatic(reference) => {
                    let signature = parse_signature(&reference.descriptor)?;
                    let args = self.arguments(instruction, &signature)?;
                    let expression = if let Some(stdlib::StdMember::StaticMethod { lower }) =
                        stdlib::member(&reference.class, &reference.name, &reference.descriptor)
                    {
                        lower(&args)
                    } else {
                        if !self
                            .known_classes
                            .iter()
                            .any(|known| known == &reference.class)
                        {
                            return Err(unsupported(
                                self.program,
                                self.method,
                                instruction,
                                "invokestatic",
                            ));
                        }
                        let method = format!(
                            "{}{}",
                            self.class_path(&reference.class)?,
                            rust_ident(&reference.name)?
                        );
                        format!("{method}(program, {}).await?", args.join(", "))
                    };
                    match signature.returns {
                        Type::Boolean => {
                            let value = self.temp(expression, Value::Boolean);
                            self.stack.push(value);
                        }
                        Type::Byte | Type::Char | Type::Short | Type::Int => {
                            let value = self.temp(expression, Value::Int);
                            self.stack.push(value);
                        }
                        Type::Long => {
                            let value = self.temp(expression, Value::Long);
                            self.stack.push(value);
                        }
                        Type::Float => {
                            let value = self.temp(expression, Value::Float);
                            self.stack.push(value);
                        }
                        Type::Double => {
                            let value = self.temp(expression, Value::Double);
                            self.stack.push(value);
                        }
                        Type::String => {
                            let value = self.temp(expression, Value::String);
                            self.stack.push(value);
                        }
                        Type::Void => self.statements.push(format!("{expression};")),
                        Type::Array(element) => {
                            let value = self.temp(expression, |expression| Value::Array {
                                expression,
                                element: (*element).clone(),
                            });
                            self.stack.push(value);
                        }
                        Type::Class(class)
                            if stdlib::class(&class)
                                .and_then(|entry| entry.rust_type)
                                .is_some() =>
                        {
                            let value = self.temp(expression, |expression| Value::Platform {
                                expression,
                                class: class.clone(),
                            });
                            self.stack.push(value);
                        }
                        Type::Class(class) => {
                            let value = self.temp(expression, |expression| Value::Object {
                                expression,
                                class: class.clone(),
                            });
                            self.stack.push(value);
                        }
                    }
                }
                Op::InvokeVirtual(reference) | Op::InvokeInterface(reference) => {
                    let signature = parse_signature(&reference.descriptor)?;
                    let args = self.arguments(instruction, &signature)?;
                    let receiver = self.pop(instruction)?;
                    if let Some(stdlib::StdMember::InstanceMethod { lower }) =
                        stdlib::member(&reference.class, &reference.name, &reference.descriptor)
                    {
                        // `String` receivers arrive as dedicated string values
                        // while other platform receivers arrive as optional
                        // platform values; both unwrap through the same null
                        // check below.
                        let receiver_expr = match receiver {
                            Value::String(expression) => {
                                format!("{expression}.ok_or_else(jars_runtime::null_pointer)?")
                            }
                            Value::Platform { expression, class } if class == reference.class => {
                                format!("{expression}.ok_or_else(jars_runtime::null_pointer)?")
                            }
                            _ => {
                                return Err(stack_error(
                                    self.program,
                                    self.method,
                                    instruction,
                                    "virtual receiver is not a platform value",
                                ));
                            }
                        };
                        let expression = lower(&receiver_expr, &args);
                        match signature.returns {
                            Type::Boolean => {
                                let value = self.temp(expression, Value::Boolean);
                                self.stack.push(value);
                            }
                            Type::Byte | Type::Char | Type::Short | Type::Int => {
                                let value = self.temp(expression, Value::Int);
                                self.stack.push(value);
                            }
                            Type::Long => {
                                let value = self.temp(expression, Value::Long);
                                self.stack.push(value);
                            }
                            Type::Float => {
                                let value = self.temp(expression, Value::Float);
                                self.stack.push(value);
                            }
                            Type::Double => {
                                let value = self.temp(expression, Value::Double);
                                self.stack.push(value);
                            }
                            Type::String => {
                                let value = self.temp(format!("Some({expression})"), Value::String);
                                self.stack.push(value);
                            }
                            Type::Void => self.statements.push(format!("{expression};")),
                            Type::Array(element) => {
                                let value = self.temp(expression, |expression| Value::Array {
                                    expression,
                                    element: (*element).clone(),
                                });
                                self.stack.push(value);
                            }
                            Type::Class(class) => {
                                let value = self.temp(expression, |expression| Value::Object {
                                    expression,
                                    class: class.clone(),
                                });
                                self.stack.push(value);
                            }
                        }
                    } else if self
                        .known_classes
                        .iter()
                        .any(|known| known == &reference.class)
                    {
                        let object = match receiver {
                            Value::Object { expression, .. } => expression,
                            _ => {
                                return Err(stack_error(
                                    self.program,
                                    self.method,
                                    instruction,
                                    "virtual receiver is not an object",
                                ));
                            }
                        };
                        let method = rust_ident(&reference.name)?;
                        let expression = format!(
                            "{object}.ok_or_else(jars_runtime::null_pointer)?.{method}({}).await?",
                            args.join(", ")
                        );
                        match signature.returns {
                            Type::Boolean => {
                                let value = self.temp(expression, Value::Boolean);
                                self.stack.push(value);
                            }
                            Type::Byte | Type::Char | Type::Short | Type::Int => {
                                let value = self.temp(expression, Value::Int);
                                self.stack.push(value);
                            }
                            Type::Long => {
                                let value = self.temp(expression, Value::Long);
                                self.stack.push(value);
                            }
                            Type::Float => {
                                let value = self.temp(expression, Value::Float);
                                self.stack.push(value);
                            }
                            Type::Double => {
                                let value = self.temp(expression, Value::Double);
                                self.stack.push(value);
                            }
                            Type::String => {
                                let value = self.temp(expression, Value::String);
                                self.stack.push(value);
                            }
                            Type::Void => self.statements.push(format!("{expression};")),
                            Type::Array(element) => {
                                let value = self.temp(expression, |expression| Value::Array {
                                    expression,
                                    element: (*element).clone(),
                                });
                                self.stack.push(value);
                            }
                            Type::Class(class) => {
                                let value = self.temp(expression, |expression| Value::Object {
                                    expression,
                                    class: class.clone(),
                                });
                                self.stack.push(value);
                            }
                        }
                    } else {
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "invokevirtual",
                        ));
                    }
                }
                Op::IReturn => {
                    let value = self.pop_expression(instruction)?;
                    self.statements.push(format!("return Ok({value});"));
                }
                Op::LReturn | Op::FReturn | Op::DReturn => {
                    let value = self.pop_expression(instruction)?;
                    self.statements.push(format!("return Ok({value});"));
                }
                Op::AReturn => {
                    let value = self.pop_expression(instruction)?;
                    self.statements.push(format!("return Ok({value});"));
                }
                Op::Return => self.statements.push("return Ok(());".to_owned()),
                Op::IInc(_, _)
                | Op::Goto(_)
                | Op::If(_, _)
                | Op::LookupSwitch { .. }
                | Op::TableSwitch { .. } => {
                    return Err(unsupported(
                        self.program,
                        self.method,
                        instruction,
                        "control flow requires AOT state-machine lowering",
                    ));
                }
            }
        }
        Ok(self.statements.join("\n"))
    }
}

fn parameters(program: &Program, method: &Method) -> Result<String, CompileError> {
    method
        .signature
        .parameters
        .iter()
        .enumerate()
        .map(|(index, ty)| Ok(format!("arg{index}: {}", ty.rust(&program.name)?)))
        .collect::<Result<Vec<_>, CompileError>>()
        .map(|parameters| parameters.join(", "))
}

fn reply_fields(parameters: &str, reply: &str) -> String {
    if parameters.is_empty() {
        reply.to_owned()
    } else {
        format!("{parameters}, {reply}")
    }
}

fn default_return(ty: &Type) -> &'static str {
    match ty {
        Type::Boolean => "false",
        Type::Byte | Type::Char | Type::Short | Type::Int => "0",
        Type::Long => "0",
        Type::Float => "0.0",
        Type::Double => "0.0",
        Type::Void => "()",
        Type::String => "None",
        Type::Class(_) | Type::Array(_) => "None",
    }
}

fn next_offset(method: &Method, index: usize) -> Option<u32> {
    method.instructions.get(index + 1).map(|next| next.offset)
}

fn branch_target(
    program: &Program,
    method: &Method,
    instruction: &Instruction,
    delta: i32,
) -> Result<u32, CompileError> {
    let target = instruction.offset as i64 + i64::from(delta);
    if target < 0
        || !method
            .instructions
            .iter()
            .any(|candidate| candidate.offset == target as u32)
    {
        return Err(stack_error(
            program,
            method,
            instruction,
            format!("branch target {target} is not an instruction boundary"),
        ));
    }
    Ok(target as u32)
}

fn exception_dispatch(method: &Method) -> String {
    let checks = method
        .handlers
        .iter()
        .map(|handler| {
            let catch = handler
                .catch_type
                .as_ref()
                .map(|class| format!("jars_runtime::catches(&error, {class:?})"))
                .unwrap_or_else(|| "true".to_owned());
            format!(
                "if pc >= {} && pc < {} && {catch} {{ Some({}) }} else ",
                handler.start, handler.end, handler.handler
            )
        })
        .collect::<String>();
    format!(
        "let target = {checks}{{ None }}; if let Some(target) = target {{ pending_exception = Some(error); stack.clear(); pc = target; continue; }} return Err(error);"
    )
}

/// Java's verifier treats `Ljava/lang/String;` and `Ljava/lang/Object;`
/// receivers uniformly, and the shared declaration models `String` as the
/// dedicated compiler type.  Normalizing `Type::Class("java/lang/String")`
/// onto `Type::String` lets one frame slot carry both descriptor spellings.
fn normalize_string_class(ty: Type) -> Type {
    match ty {
        Type::Class(class) if class == "java/lang/String" => Type::String,
        other => other,
    }
}

#[derive(Default)]
struct FrameLayout {
    references: Vec<Type>,
    has_this: bool,
}

impl FrameLayout {
    fn add_type(&mut self, ty: Type) {
        let ty = normalize_string_class(ty);
        match ty {
            Type::String | Type::Class(_) | Type::Array(_) => {
                if !self.references.contains(&ty) {
                    self.references.push(ty.clone());
                }
                if let Type::Array(element) = ty {
                    self.add_type(*element);
                }
            }
            _ => {}
        }
    }

    fn reference_variant(&self, ty: &Type) -> Result<String, CompileError> {
        let ty = &normalize_string_class(ty.clone());
        self.references
            .iter()
            .position(|candidate| candidate == ty)
            .map(|index| format!("R{index}"))
            .ok_or_else(|| invalid("reference type is absent from the typed frame layout"))
    }
}

fn frame_layout(
    program: &Program,
    _known_classes: &[String],
    method: &Method,
    first_local: usize,
) -> Result<FrameLayout, CompileError> {
    let mut layout = FrameLayout {
        has_this: first_local != 0,
        ..FrameLayout::default()
    };
    for ty in method
        .signature
        .parameters
        .iter()
        .chain(std::iter::once(&method.signature.returns))
    {
        layout.add_type(ty.clone());
    }
    for instruction in &method.instructions {
        match &instruction.op {
            Op::LdcString(_) => layout.add_type(Type::String),
            Op::New(class) => layout.add_type(Type::Class(class.clone())),
            Op::NewArray(element) => layout.add_type(Type::Array(Box::new(element.clone()))),
            Op::MultiNewArray(ty, _) => layout.add_type(ty.clone()),
            Op::GetStatic(reference)
            | Op::PutStatic(reference)
            | Op::GetField(reference)
            | Op::PutField(reference) => {
                let mut cursor = 0;
                let ty = parse_type(&reference.descriptor, &mut cursor)?;
                if cursor != reference.descriptor.len() {
                    return Err(invalid("field descriptor has trailing input"));
                }
                layout.add_type(ty);
                if matches!(instruction.op, Op::GetField(_) | Op::PutField(_)) {
                    layout.add_type(Type::Class(reference.class.clone()));
                }
            }
            Op::InvokeStatic(reference)
            | Op::InvokeSpecial(reference)
            | Op::InvokeVirtual(reference)
            | Op::InvokeInterface(reference) => {
                let signature = parse_signature(&reference.descriptor)?;
                for ty in signature.parameters {
                    layout.add_type(ty);
                }
                layout.add_type(signature.returns);
                if !matches!(instruction.op, Op::InvokeStatic(_)) {
                    layout.add_type(Type::Class(reference.class.clone()));
                }
            }
            _ => {}
        }
    }
    let _ = program;
    Ok(layout)
}

fn frame_variant(
    ty: &Type,
    expression: impl AsRef<str>,
    layout: &FrameLayout,
) -> Result<String, CompileError> {
    let expression = expression.as_ref();
    let ty = &normalize_string_class(ty.clone());
    match ty {
        Type::Boolean | Type::Byte | Type::Char | Type::Short | Type::Int => {
            Ok(format!("FrameValue::I32({expression} as i32)"))
        }
        Type::Long => Ok(format!("FrameValue::I64({expression})")),
        Type::Float => Ok(format!("FrameValue::F32({expression})")),
        Type::Double => Ok(format!("FrameValue::F64({expression})")),
        Type::String | Type::Class(_) | Type::Array(_) => Ok(format!(
            "FrameValue::{}({expression})",
            layout.reference_variant(ty)?
        )),
        _ => Err(invalid(
            "the current typed frame does not model this value type",
        )),
    }
}

fn frame_pop(ty: &Type, layout: &FrameLayout) -> Result<String, CompileError> {
    let ty = &normalize_string_class(ty.clone());
    match ty {
        Type::Boolean | Type::Byte | Type::Char | Type::Short | Type::Int => {
            Ok("pop_i32".to_owned())
        }
        Type::Long => Ok("pop_i64".to_owned()),
        Type::Float => Ok("pop_f32".to_owned()),
        Type::Double => Ok("pop_f64".to_owned()),
        Type::String => Ok(format!(
            "pop_{}",
            layout.reference_variant(ty)?.to_lowercase()
        )),
        Type::Class(_) | Type::Array(_) => Ok(format!(
            "pop_{}",
            layout.reference_variant(ty)?.to_lowercase()
        )),
        _ => Err(invalid(
            "the current typed frame does not model this value type",
        )),
    }
}

fn frame_type_supported(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Boolean
            | Type::Byte
            | Type::Char
            | Type::Short
            | Type::Int
            | Type::Long
            | Type::Float
            | Type::Double
            | Type::Void
            | Type::String
            | Type::Class(_)
            | Type::Array(_)
    )
}

/// Emits the shared, typed activation-frame state machine used by supported
/// static control-flow and exception-table bodies.  Each local and operand is
/// a tagged Rust value; the compiler emits only the tag operations required by
/// the parsed instructions, never a bytecode representation or interpreter.
fn aot_typed_frame(
    program: &Program,
    known_classes: &[String],
    method: &Method,
    first_local: usize,
) -> Result<String, CompileError> {
    if !frame_type_supported(&method.signature.returns)
        || method
            .signature
            .parameters
            .iter()
            .any(|parameter| !frame_type_supported(parameter) || *parameter == Type::Void)
    {
        return Err(invalid(format!(
            "typed AOT frame for {}.{} currently supports numeric primitives, strings, closed-world references, arrays, and void",
            program.name, method.name
        )));
    }
    let layout = frame_layout(program, known_classes, method, first_local)?;
    let uninitialized = method
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.op {
            Op::New(class) => Some(class.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let parameter_slots = method
        .signature
        .parameters
        .iter()
        .scan(first_local, |slot, ty| {
            let current = *slot;
            *slot += 1 + usize::from(matches!(ty, Type::Long | Type::Double));
            Some(current)
        })
        .collect::<Vec<_>>();
    let max_local = method
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.op {
            Op::ILoad(index)
            | Op::IStore(index)
            | Op::ALoad(index)
            | Op::AStore(index)
            | Op::LLoad(index)
            | Op::LStore(index)
            | Op::FLoad(index)
            | Op::FStore(index)
            | Op::DLoad(index)
            | Op::DStore(index)
            | Op::IInc(index, _) => Some(*index),
            _ => None,
        })
        .chain(parameter_slots.iter().copied())
        .max()
        .unwrap_or(0)
        + 1;
    let mut arms = Vec::new();
    for (index, instruction) in method.instructions.iter().enumerate() {
        let next = next_offset(method, index);
        let continue_at = |next: Option<u32>| -> Result<String, CompileError> {
            next.map(|offset| format!("pc = {offset}; continue;"))
                .ok_or_else(|| {
                    stack_error(
                        program,
                        method,
                        instruction,
                        "instruction falls off the end of a method",
                    )
                })
        };
        let arm = match &instruction.op {
            Op::IConst(value) => format!(
                "stack.push(FrameValue::I32({value})); {}",
                continue_at(next)?
            ),
            Op::LConst(value) => format!(
                "stack.push(FrameValue::I64({value}_i64)); {}",
                continue_at(next)?
            ),
            Op::FConst(value) => format!(
                "stack.push(FrameValue::F32({value:?}_f32)); {}",
                continue_at(next)?
            ),
            Op::DConst(value) => format!(
                "stack.push(FrameValue::F64({value:?}_f64)); {}",
                continue_at(next)?
            ),
            Op::LdcString(value) => format!(
                "stack.push({}); {}",
                frame_variant(
                    &Type::String,
                    format!("Some(jars_runtime::JavaString::new({value:?}))"),
                    &layout
                )?,
                continue_at(next)?
            ),
            Op::ILoad(local) => format!(
                "let value = match locals[{local}].as_ref().expect(\"initialized local\") {{ FrameValue::I32(value) => *value, _ => unreachable!(\"verified local type\") }}; stack.push(FrameValue::I32(value)); {}",
                continue_at(next)?
            ),
            Op::IStore(local) => format!(
                "locals[{local}] = Some(FrameValue::I32(pop_i32(&mut stack))); {}",
                continue_at(next)?
            ),
            Op::ALoad(local) => format!(
                "stack.push(load_local(&mut locals, {local})); {}",
                continue_at(next)?
            ),
            Op::AStore(local) => format!(
                "locals[{local}] = Some(match pending_exception.take() {{ Some(error) => FrameValue::Throwable(error), None => stack.pop().expect(\"verified JVM stack\") }}); {}",
                continue_at(next)?
            ),
            Op::AConstNull => format!("stack.push(FrameValue::Null); {}", continue_at(next)?),
            Op::New(class) => {
                if !known_classes.iter().any(|known| known == class)
                    && !stdlib::is_constructible(class)
                {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "new owner outside the compilation set",
                    ));
                }
                let site = method.instructions[..index]
                    .iter()
                    .filter(|candidate| matches!(&candidate.op, Op::New(_)))
                    .count();
                format!("stack.push(FrameValue::U{site}); {}", continue_at(next)?)
            }
            Op::Dup => format!(
                "let value = duplicate(stack.last().expect(\"verified JVM stack\")); stack.push(value); {}",
                continue_at(next)?
            ),
            Op::Pop => format!(
                "let _ = stack.pop().expect(\"verified JVM stack\"); {}",
                continue_at(next)?
            ),
            Op::InvokeSpecial(reference) => {
                let signature = parse_signature(&reference.descriptor)?;
                if reference.name != "<init>"
                    || !frame_type_supported(&signature.returns)
                    || signature
                        .parameters
                        .iter()
                        .any(|ty| !frame_type_supported(ty) || *ty == Type::Void)
                {
                    return Err(unsupported(program, method, instruction, "invokespecial"));
                }
                let mut arguments = Vec::with_capacity(signature.parameters.len());
                for (argument, ty) in signature.parameters.iter().enumerate().rev() {
                    let pop = frame_pop(ty, &layout)?;
                    arguments.push(format!("let argument{argument} = {pop}(&mut stack);"));
                }
                let arguments_joined = (0..signature.parameters.len())
                    .map(|argument| format!("argument{argument}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(stdlib::StdMember::Constructor { lower }) =
                    stdlib::member(&reference.class, &reference.name, &reference.descriptor)
                {
                    match lower(
                        &(0..signature.parameters.len())
                            .map(|argument| format!("argument{argument}"))
                            .collect::<Vec<_>>(),
                    ) {
                        None if signature.parameters.is_empty() => format!(
                            "{} match stack.pop().expect(\"verified JVM stack\") {{ FrameValue::This => {{}}, _ => unreachable!(\"supported stdlib constructor receiver\"), }} {}",
                            arguments.join(" "),
                            continue_at(next)?
                        ),
                        Some(expression) => {
                            let matching_sites = uninitialized
                                .iter()
                                .enumerate()
                                .filter(|(_, class)| *class == &reference.class)
                                .map(|(site, _)| {
                                    let variant = layout.reference_variant(&Type::Class(reference.class.clone()))?;
                                    Ok(format!(
                                        "FrameValue::U{site} => {{ let value = {expression}; for local in &mut locals {{ if matches!(local, Some(FrameValue::U{site})) {{ *local = Some(FrameValue::{variant}(value.clone())); }} }} for value_slot in &mut stack {{ if matches!(value_slot, FrameValue::U{site}) {{ *value_slot = FrameValue::{variant}(value.clone()); }} }} }},"
                                    ))
                                })
                                .collect::<Result<String, CompileError>>()?;
                            if matching_sites.is_empty() {
                                return Err(unsupported(
                                    program,
                                    method,
                                    instruction,
                                    "stdlib constructor receiver",
                                ));
                            }
                            format!(
                                "{} match stack.pop().expect(\"verified JVM stack\") {{ {matching_sites} _ => unreachable!(\"verified stdlib constructor receiver\"), }} {}",
                                arguments.join(" "),
                                continue_at(next)?
                            )
                        }
                        None => {
                            return Err(unsupported(
                                program,
                                method,
                                instruction,
                                "stdlib no-op constructor with arguments",
                            ));
                        }
                    }
                } else if known_classes.iter().any(|known| known == &reference.class) {
                    let owner = class_ident(&reference.class)?;
                    let path = if reference.class == program.name {
                        String::new()
                    } else {
                        format!("super::{owner}::")
                    };
                    let matching_sites = uninitialized
                        .iter()
                        .enumerate()
                        .filter(|(_, class)| *class == &reference.class)
                        .map(|(site, _)| {
                            let variant = layout.reference_variant(&Type::Class(reference.class.clone()))?;
                            let call_arguments = if arguments_joined.is_empty() {
                                "program".to_owned()
                            } else {
                                format!("program, {arguments_joined}")
                            };
                            let dispatch = exception_dispatch(method);
                            Ok(format!(
                                "FrameValue::U{site} => match {path}__ensure(program).await {{ Ok(()) => match {path}{owner}::new({call_arguments}).await {{ Ok(value) => {{ for local in &mut locals {{ if matches!(local, Some(FrameValue::U{site})) {{ *local = Some(FrameValue::{variant}(Some(value.clone()))); }} }} for value_slot in &mut stack {{ if matches!(value_slot, FrameValue::U{site}) {{ *value_slot = FrameValue::{variant}(Some(value.clone())); }} }} }}, Err(error) => {{ {dispatch} }} }}, Err(error) => {{ {dispatch} }} }}"
                            ))
                        })
                        .collect::<Result<Vec<_>, CompileError>>()?
                        .join(", ");
                    if matching_sites.is_empty() && !signature.parameters.is_empty() {
                        return Err(unsupported(
                            program,
                            method,
                            instruction,
                            "super constructor with parameters",
                        ));
                    }
                    format!(
                        "{} match stack.pop().expect(\"verified JVM stack\") {{ FrameValue::This => {{}}, {matching_sites} _ => unreachable!(\"verified constructor receiver\"), }} {}",
                        arguments.join(" "),
                        continue_at(next)?
                    )
                } else {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "invokespecial owner outside the compilation set",
                    ));
                }
            }
            Op::NewArray(element) => {
                let ty = Type::Array(Box::new(element.clone()));
                let element_rust = element.array_element_rust(&program.name)?;
                let default = Body::array_default(element)?;
                let dispatch = exception_dispatch(method);
                let constructed = crate::model::array_new(&element_rust, "length", &default);
                format!(
                    "let length = pop_i32(&mut stack); match {constructed} {{ Ok(array) => stack.push({}), Err(error) => {{ {dispatch} }} }} {}",
                    frame_variant(&ty, "Some(array)", &layout)?,
                    continue_at(next)?
                )
            }
            Op::CALoad => {
                let dispatch = exception_dispatch(method);
                let arrays = layout
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, ty)| matches!(ty, Type::Array(element) if **element == Type::Char))
                    .map(|(index, _)| {
                        format!(
                            "FrameValue::R{index}(array) => match array {{ Some(array) => array.{}(index){}, None => Err(jars_runtime::null_pointer()), }},",
                            crate::model::runtime_method_name("get"),
                            crate::model::await_token()
                        )
                    })
                    .collect::<String>();
                if arrays.is_empty() {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "caload has no char-array type in the typed frame layout",
                    ));
                }
                format!(
                    "let index = pop_i32(&mut stack); let array = stack.pop().expect(\"verified JVM stack\"); let value = match array {{ {arrays} FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified caload receiver\"), }}; match value {{ Ok(value) => stack.push(FrameValue::I32(i32::from(value))), Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::CAStore => {
                let dispatch = exception_dispatch(method);
                let arrays = layout
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, ty)| matches!(ty, Type::Array(element) if **element == Type::Char))
                    .map(|(index, _)| {
                        format!(
                            "FrameValue::R{index}(array) => match array {{ Some(array) => array.{}(index, value as u16){}, None => Err(jars_runtime::null_pointer()), }},",
                            crate::model::runtime_method_name("set"),
                            crate::model::await_token()
                        )
                    })
                    .collect::<String>();
                if arrays.is_empty() {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "castore has no char-array type in the typed frame layout",
                    ));
                }
                format!(
                    "let value = pop_i32(&mut stack); let index = pop_i32(&mut stack); let array = stack.pop().expect(\"verified JVM stack\"); let result = match array {{ {arrays} FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified castore receiver\"), }}; match result {{ Ok(()) => {{}}, Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::IALoad => {
                let dispatch = exception_dispatch(method);
                let arrays = layout
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, ty)| matches!(ty, Type::Array(element) if **element == Type::Int))
                    .map(|(index, _)| {
                        format!(
                            "FrameValue::R{index}(array) => match array {{ Some(array) => array.{}(index){}, None => Err(jars_runtime::null_pointer()), }},",
                            crate::model::runtime_method_name("get"),
                            crate::model::await_token()
                        )
                    })
                    .collect::<String>();
                if arrays.is_empty() {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "iaload has no int-array type in the typed frame layout",
                    ));
                }
                format!(
                    "let index = pop_i32(&mut stack); let array = stack.pop().expect(\"verified JVM stack\"); let value = match array {{ {arrays} FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified iaload receiver\"), }}; match value {{ Ok(value) => stack.push(FrameValue::I32(value)), Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::IAStore => {
                let dispatch = exception_dispatch(method);
                let arrays = layout
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, ty)| matches!(ty, Type::Array(element) if **element == Type::Int))
                    .map(|(index, _)| {
                        format!(
                            "FrameValue::R{index}(array) => match array {{ Some(array) => array.{}(index, value){}, None => Err(jars_runtime::null_pointer()), }},",
                            crate::model::runtime_method_name("set"),
                            crate::model::await_token()
                        )
                    })
                    .collect::<String>();
                if arrays.is_empty() {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "iastore has no int-array type in the typed frame layout",
                    ));
                }
                format!(
                    "let value = pop_i32(&mut stack); let index = pop_i32(&mut stack); let array = stack.pop().expect(\"verified JVM stack\"); let result = match array {{ {arrays} FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified iastore receiver\"), }}; match result {{ Ok(()) => {{}}, Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::BALoad => {
                let dispatch = exception_dispatch(method);
                let arrays = layout
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, ty)| {
                        matches!(ty, Type::Array(element) if matches!(**element, Type::Byte | Type::Boolean))
                    })
                    .map(|(index, _)| {
                        format!(
                            "FrameValue::R{index}(array) => match array {{ Some(array) => array.{}(index){}, None => Err(jars_runtime::null_pointer()), }},",
                            crate::model::runtime_method_name("get"),
                            crate::model::await_token()
                        )
                    })
                    .collect::<String>();
                if arrays.is_empty() {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "baload has no byte-array type in the typed frame layout",
                    ));
                }
                format!(
                    "let index = pop_i32(&mut stack); let array = stack.pop().expect(\"verified JVM stack\"); let value = match array {{ {arrays} FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified baload receiver\"), }}; match value {{ Ok(value) => stack.push(FrameValue::I32(i32::from(value))), Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::BAStore => {
                let dispatch = exception_dispatch(method);
                let arrays = layout
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, ty)| matches!(ty, Type::Array(element) if **element == Type::Byte))
                    .map(|(index, _)| {
                        format!(
                            "FrameValue::R{index}(array) => match array {{ Some(array) => array.{}(index, value as i8){}, None => Err(jars_runtime::null_pointer()), }},",
                            crate::model::runtime_method_name("set"),
                            crate::model::await_token()
                        )
                    })
                    .collect::<String>();
                if arrays.is_empty() {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "bastore has no byte-array type in the typed frame layout",
                    ));
                }
                format!(
                    "let value = pop_i32(&mut stack); let index = pop_i32(&mut stack); let array = stack.pop().expect(\"verified JVM stack\"); let result = match array {{ {arrays} FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified bastore receiver\"), }}; match result {{ Ok(()) => {{}}, Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::ArrayLength => {
                let arrays = layout
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, ty)| matches!(ty, Type::Array(_)))
                    .map(|(index, _)| {
                        format!(
                            "FrameValue::R{index}(array) => match array {{ Some(array) => array.{}(){}, None => Err(jars_runtime::null_pointer()), }},",
                            crate::model::runtime_method_name("length"),
                            crate::model::await_token()
                        )
                    })
                    .collect::<String>();
                if arrays.is_empty() {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "arraylength has no array type in the typed frame layout",
                    ));
                }
                let dispatch = exception_dispatch(method);
                format!(
                    "let array = stack.pop().expect(\"verified JVM stack\"); let length = match array {{ {arrays} FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified arraylength receiver\"), }}; match length {{ Ok(value) => stack.push(FrameValue::I32(value)), Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::LLoad(local) => format!(
                "let value = match locals[{local}].as_ref().expect(\"initialized local\") {{ FrameValue::I64(value) => *value, _ => unreachable!(\"verified local type\") }}; stack.push(FrameValue::I64(value)); {}",
                continue_at(next)?
            ),
            Op::FLoad(local) => format!(
                "let value = match locals[{local}].as_ref().expect(\"initialized local\") {{ FrameValue::F32(value) => *value, _ => unreachable!(\"verified local type\") }}; stack.push(FrameValue::F32(value)); {}",
                continue_at(next)?
            ),
            Op::DLoad(local) => format!(
                "let value = match locals[{local}].as_ref().expect(\"initialized local\") {{ FrameValue::F64(value) => *value, _ => unreachable!(\"verified local type\") }}; stack.push(FrameValue::F64(value)); {}",
                continue_at(next)?
            ),
            Op::LStore(local) => format!(
                "locals[{local}] = Some(FrameValue::I64(pop_i64(&mut stack))); {}",
                continue_at(next)?
            ),
            Op::FStore(local) => format!(
                "locals[{local}] = Some(FrameValue::F32(pop_f32(&mut stack))); {}",
                continue_at(next)?
            ),
            Op::DStore(local) => format!(
                "locals[{local}] = Some(FrameValue::F64(pop_f64(&mut stack))); {}",
                continue_at(next)?
            ),
            Op::IInc(local, value) => format!(
                "let value = match locals[{local}].take().expect(\"initialized local\") {{ FrameValue::I32(value) => value, _ => unreachable!(\"verified local type\") }}; locals[{local}] = Some(FrameValue::I32(value.wrapping_add({value}))); {}",
                continue_at(next)?
            ),
            Op::IAdd
            | Op::ISub
            | Op::IMul
            | Op::IAnd
            | Op::IOr
            | Op::IXor
            | Op::IShl
            | Op::IShr
            | Op::IUshr => {
                let expression = match &instruction.op {
                    Op::IAdd => "left.wrapping_add(right)",
                    Op::ISub => "left.wrapping_sub(right)",
                    Op::IMul => "left.wrapping_mul(right)",
                    Op::IAnd => "left & right",
                    Op::IOr => "left | right",
                    Op::IXor => "left ^ right",
                    Op::IShl => "left.wrapping_shl((right as u32) & 31)",
                    Op::IShr => "left.wrapping_shr((right as u32) & 31)",
                    Op::IUshr => "jars_runtime::iushr(left, right)",
                    _ => unreachable!(),
                };
                format!(
                    "let right = pop_i32(&mut stack); let left = pop_i32(&mut stack); stack.push(FrameValue::I32({expression})); {}",
                    continue_at(next)?
                )
            }
            Op::IDiv | Op::IRem => {
                let operation = if matches!(&instruction.op, Op::IDiv) {
                    "idiv"
                } else {
                    "irem"
                };
                let dispatch = exception_dispatch(method);
                format!(
                    "let right = pop_i32(&mut stack); let left = pop_i32(&mut stack); match jars_runtime::{operation}(left, right) {{ Ok(value) => stack.push(FrameValue::I32(value)), Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::INeg => format!(
                "let value = pop_i32(&mut stack); stack.push(FrameValue::I32(value.wrapping_neg())); {}",
                continue_at(next)?
            ),
            Op::LAdd | Op::LSub | Op::LMul => {
                let expression = match &instruction.op {
                    Op::LAdd => "left.wrapping_add(right)",
                    Op::LSub => "left.wrapping_sub(right)",
                    Op::LMul => "left.wrapping_mul(right)",
                    _ => unreachable!(),
                };
                format!(
                    "let right = pop_i64(&mut stack); let left = pop_i64(&mut stack); stack.push(FrameValue::I64({expression})); {}",
                    continue_at(next)?
                )
            }
            Op::LDiv | Op::LRem => {
                let operation = if matches!(&instruction.op, Op::LDiv) {
                    "ldiv"
                } else {
                    "lrem"
                };
                let dispatch = exception_dispatch(method);
                format!(
                    "let right = pop_i64(&mut stack); let left = pop_i64(&mut stack); match jars_runtime::{operation}(left, right) {{ Ok(value) => stack.push(FrameValue::I64(value)), Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::LNeg => format!(
                "let value = pop_i64(&mut stack); stack.push(FrameValue::I64(value.wrapping_neg())); {}",
                continue_at(next)?
            ),
            Op::FAdd | Op::FSub | Op::FMul | Op::FDiv | Op::FRem => {
                let operation = match &instruction.op {
                    Op::FAdd => "+",
                    Op::FSub => "-",
                    Op::FMul => "*",
                    Op::FDiv => "/",
                    Op::FRem => "%",
                    _ => unreachable!(),
                };
                format!(
                    "let right = pop_f32(&mut stack); let left = pop_f32(&mut stack); stack.push(FrameValue::F32(left {operation} right)); {}",
                    continue_at(next)?
                )
            }
            Op::FNeg => format!(
                "let value = pop_f32(&mut stack); stack.push(FrameValue::F32(-value)); {}",
                continue_at(next)?
            ),
            Op::DAdd | Op::DSub | Op::DMul | Op::DDiv | Op::DRem => {
                let operation = match &instruction.op {
                    Op::DAdd => "+",
                    Op::DSub => "-",
                    Op::DMul => "*",
                    Op::DDiv => "/",
                    Op::DRem => "%",
                    _ => unreachable!(),
                };
                format!(
                    "let right = pop_f64(&mut stack); let left = pop_f64(&mut stack); stack.push(FrameValue::F64(left {operation} right)); {}",
                    continue_at(next)?
                )
            }
            Op::DNeg => format!(
                "let value = pop_f64(&mut stack); stack.push(FrameValue::F64(-value)); {}",
                continue_at(next)?
            ),
            Op::Goto(delta) => format!(
                "pc = {}; continue;",
                branch_target(program, method, instruction, *delta)?
            ),
            Op::If(kind, delta) => {
                let (values, condition) = match kind {
                    IfKind::Eq => ("let value = pop_i32(&mut stack);", "value == 0"),
                    IfKind::Ne => ("let value = pop_i32(&mut stack);", "value != 0"),
                    IfKind::Lt => ("let value = pop_i32(&mut stack);", "value < 0"),
                    IfKind::Ge => ("let value = pop_i32(&mut stack);", "value >= 0"),
                    IfKind::Gt => ("let value = pop_i32(&mut stack);", "value > 0"),
                    IfKind::Le => ("let value = pop_i32(&mut stack);", "value <= 0"),
                    IfKind::ICmpEq => (
                        "let right = pop_i32(&mut stack); let left = pop_i32(&mut stack);",
                        "left == right",
                    ),
                    IfKind::ICmpNe => (
                        "let right = pop_i32(&mut stack); let left = pop_i32(&mut stack);",
                        "left != right",
                    ),
                    IfKind::ICmpLt => (
                        "let right = pop_i32(&mut stack); let left = pop_i32(&mut stack);",
                        "left < right",
                    ),
                    IfKind::ICmpGe => (
                        "let right = pop_i32(&mut stack); let left = pop_i32(&mut stack);",
                        "left >= right",
                    ),
                    IfKind::ICmpGt => (
                        "let right = pop_i32(&mut stack); let left = pop_i32(&mut stack);",
                        "left > right",
                    ),
                    IfKind::ICmpLe => (
                        "let right = pop_i32(&mut stack); let left = pop_i32(&mut stack);",
                        "left <= right",
                    ),
                    IfKind::ACmpEq => (
                        "let right = stack.pop().expect(\"verified JVM stack\"); let left = stack.pop().expect(\"verified JVM stack\");",
                        "same_reference(&left, &right)",
                    ),
                    IfKind::ACmpNe => (
                        "let right = stack.pop().expect(\"verified JVM stack\"); let left = stack.pop().expect(\"verified JVM stack\");",
                        "!same_reference(&left, &right)",
                    ),
                    IfKind::Null => (
                        "let value = stack.pop().expect(\"verified JVM stack\");",
                        "is_null(&value)",
                    ),
                    IfKind::NonNull => (
                        "let value = stack.pop().expect(\"verified JVM stack\");",
                        "!is_null(&value)",
                    ),
                };
                format!(
                    "{values} if {condition} {{ pc = {}; }} else {{ pc = {}; }} continue;",
                    branch_target(program, method, instruction, *delta)?,
                    next.ok_or_else(|| stack_error(
                        program,
                        method,
                        instruction,
                        "conditional falls off method"
                    ))?,
                )
            }
            Op::LookupSwitch { default, cases } | Op::TableSwitch { default, cases } => {
                let default = branch_target(program, method, instruction, *default)?;
                let cases = cases
                    .iter()
                    .map(|(key, delta)| {
                        Ok(format!(
                            "{key} => {},",
                            branch_target(program, method, instruction, *delta)?
                        ))
                    })
                    .collect::<Result<String, CompileError>>()?;
                format!(
                    "let value = pop_i32(&mut stack); pc = match value {{ {cases} _ => {default}, }}; continue;"
                )
            }
            Op::InvokeStatic(reference) => {
                let signature = parse_signature(&reference.descriptor)?;
                if !frame_type_supported(&signature.returns)
                    || signature.parameters.iter().any(|parameter| {
                        !frame_type_supported(parameter) || *parameter == Type::Void
                    })
                {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "invokestatic type outside the typed frame subset",
                    ));
                }
                if let Some(stdlib::StdMember::StaticMethod { lower }) =
                    stdlib::member(&reference.class, &reference.name, &reference.descriptor)
                {
                    let mut arguments = Vec::with_capacity(signature.parameters.len());
                    for (index, ty) in signature.parameters.iter().enumerate().rev() {
                        let pop = frame_pop(ty, &layout)?;
                        arguments.push(format!("let argument{index} = {pop}(&mut stack);"));
                    }
                    let argument_names = (0..signature.parameters.len())
                        .map(|index| format!("argument{index}"))
                        .collect::<Vec<_>>();
                    let call = lower(&argument_names);
                    let push = if signature.returns == Type::Void {
                        format!("{call};")
                    } else {
                        format!(
                            "stack.push({});",
                            frame_variant(&signature.returns, &call, &layout)?
                        )
                    };
                    format!("{} {push} {}", arguments.join(" "), continue_at(next)?)
                } else if !known_classes.iter().any(|class| class == &reference.class) {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "invokestatic owner outside the compilation set",
                    ));
                } else {
                    let mut arguments = Vec::with_capacity(signature.parameters.len());
                    for (index, ty) in signature.parameters.iter().enumerate().rev() {
                        let pop = frame_pop(ty, &layout)?;
                        arguments.push(format!("let argument{index} = {pop}(&mut stack);"));
                    }
                    let method_path = if reference.class == program.name {
                        rust_ident(&reference.name)?
                    } else {
                        format!(
                            "super::{}::{}",
                            class_ident(&reference.class)?,
                            rust_ident(&reference.name)?,
                        )
                    };
                    // Narrow and boolean frame values are carried as i32; cast
                    // each argument to the callee's Rust parameter type.
                    let arguments_joined = signature
                        .parameters
                        .iter()
                        .enumerate()
                        .map(|(index, ty)| {
                            let value = format!("argument{index}");
                            match ty {
                                Type::Boolean => format!("{value} != 0"),
                                Type::Byte => format!("{value} as i8"),
                                Type::Char => format!("{value} as u16"),
                                Type::Short => format!("{value} as i16"),
                                _ => value,
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let call = if arguments_joined.is_empty() {
                        format!("{method_path}(program).await")
                    } else {
                        format!("{method_path}(program, {arguments_joined}).await")
                    };
                    let dispatch = exception_dispatch(method);
                    let success = if signature.returns == Type::Void {
                        "Ok(()) => {}".to_owned()
                    } else {
                        format!(
                            "Ok(value) => stack.push({})",
                            frame_variant(&signature.returns, "value", &layout)?
                        )
                    };
                    format!(
                        "{} match {call} {{ {success}, Err(error) => {{ {dispatch} }} }} {}",
                        arguments.join(" "),
                        continue_at(next)?,
                    )
                }
            }
            Op::GetStatic(reference) | Op::PutStatic(reference) => {
                let stdlib_field = match &instruction.op {
                    Op::GetStatic(_) => {
                        stdlib::member(&reference.class, &reference.name, &reference.descriptor)
                            .and_then(|member| match member {
                                stdlib::StdMember::StaticField { lower } => Some(lower),
                                _ => None,
                            })
                    }
                    _ => None,
                };
                if let Some(lower) = stdlib_field {
                    let mut cursor = 0;
                    let ty = parse_type(&reference.descriptor, &mut cursor)?;
                    if cursor != reference.descriptor.len()
                        || !frame_type_supported(&ty)
                        || ty == Type::Void
                    {
                        return Err(unsupported(
                            program,
                            method,
                            instruction,
                            "getstatic type outside the typed frame subset",
                        ));
                    }
                    let value = lower();
                    format!(
                        "stack.push({}); {}",
                        frame_variant(&ty, &value, &layout)?,
                        continue_at(next)?
                    )
                } else if !known_classes.iter().any(|class| class == &reference.class) {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "getstatic/putstatic owner outside the compilation set",
                    ));
                } else {
                    let mut cursor = 0;
                    let ty = parse_type(&reference.descriptor, &mut cursor)?;
                    if cursor != reference.descriptor.len()
                        || !frame_type_supported(&ty)
                        || ty == Type::Void
                    {
                        return Err(unsupported(
                            program,
                            method,
                            instruction,
                            "getstatic/putstatic type outside the typed frame subset",
                        ));
                    }
                    let owner = class_ident(&reference.class)?;
                    let field = rust_ident(&reference.name)?;
                    let dispatch = exception_dispatch(method);
                    let access = match &instruction.op {
                        Op::GetStatic(_) => format!(
                            "let value = program.state.borrow().{owner}.{field}.clone(); stack.push({});",
                            frame_variant(&ty, "value", &layout)?
                        ),
                        Op::PutStatic(_) => {
                            let pop = frame_pop(&ty, &layout)?;
                            format!(
                                "let value = {pop}(&mut stack); program.state.borrow_mut().{owner}.{field} = value;"
                            )
                        }
                        _ => unreachable!(),
                    };
                    if reference.class == program.name {
                        // The class has already been marked `initializing` before
                        // entering `<clinit>`. Re-entering its async `__ensure`
                        // here would create a recursively sized future; Java
                        // instead observes the prepared/default static state.
                        format!("{access} {}", continue_at(next)?)
                    } else {
                        let ensure = format!("super::{owner}::__ensure(program).await");
                        format!(
                            "match {ensure} {{ Ok(()) => {{ {access} }}, Err(error) => {{ {dispatch} }} }} {}",
                            continue_at(next)?
                        )
                    }
                }
            }
            Op::GetField(reference) => {
                if !known_classes.iter().any(|class| class == &reference.class) {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "getfield owner outside the compilation set",
                    ));
                }
                let mut cursor = 0;
                let ty = parse_type(&reference.descriptor, &mut cursor)?;
                if cursor != reference.descriptor.len()
                    || !frame_type_supported(&ty)
                    || ty == Type::Void
                {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "getfield type outside the typed frame subset",
                    ));
                }
                let field = rust_ident(&reference.name)?;
                let receiver = layout.reference_variant(&Type::Class(reference.class.clone()))?;
                let direct_this = if first_local != 0 && reference.class == program.name {
                    format!(
                        "FrameValue::This => Ok({}.{field}.clone()),",
                        crate::model::state_read("state")
                    )
                } else {
                    String::new()
                };
                let dispatch = exception_dispatch(method);
                let value = frame_variant(&ty, "value", &layout)?;
                format!(
                    "let receiver = stack.pop().expect(\"verified JVM stack\"); let value = match receiver {{ {direct_this} FrameValue::{receiver}(receiver) => match receiver {{ Some(receiver) => receiver.__get_{field}().await, None => Err(jars_runtime::null_pointer()), }}, FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified getfield receiver\"), }}; match value {{ Ok(value) => stack.push({value}), Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::PutField(reference) => {
                if !known_classes.iter().any(|class| class == &reference.class) {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "putfield owner outside the compilation set",
                    ));
                }
                let mut cursor = 0;
                let ty = parse_type(&reference.descriptor, &mut cursor)?;
                if cursor != reference.descriptor.len()
                    || !frame_type_supported(&ty)
                    || ty == Type::Void
                {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "putfield type outside the typed frame subset",
                    ));
                }
                let field = rust_ident(&reference.name)?;
                let receiver = layout.reference_variant(&Type::Class(reference.class.clone()))?;
                let pop = frame_pop(&ty, &layout)?;
                let direct_this = if first_local != 0 && reference.class == program.name {
                    format!(
                        "FrameValue::This => {{ {}.{field} = value; Ok(()) }},",
                        crate::model::state_write("state")
                    )
                } else {
                    String::new()
                };
                let dispatch = exception_dispatch(method);
                format!(
                    "let value = {pop}(&mut stack); let receiver = stack.pop().expect(\"verified JVM stack\"); let result = match receiver {{ {direct_this} FrameValue::{receiver}(receiver) => match receiver {{ Some(receiver) => receiver.__set_{field}(value).await, None => Err(jars_runtime::null_pointer()), }}, FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified putfield receiver\"), }}; match result {{ Ok(()) => {{}}, Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::InvokeVirtual(reference) | Op::InvokeInterface(reference) => {
                let signature = parse_signature(&reference.descriptor)?;
                if !frame_type_supported(&signature.returns)
                    || signature.parameters.iter().any(|parameter| {
                        !frame_type_supported(parameter) || *parameter == Type::Void
                    })
                {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "virtual call type outside the typed frame subset",
                    ));
                }
                if let Some(stdlib::StdMember::InstanceMethod { lower }) =
                    stdlib::member(&reference.class, &reference.name, &reference.descriptor)
                {
                    let mut arguments = Vec::with_capacity(signature.parameters.len());
                    for (index, ty) in signature.parameters.iter().enumerate().rev() {
                        let pop = frame_pop(ty, &layout)?;
                        arguments.push(format!("let argument{index} = {pop}(&mut stack);"));
                    }
                    let argument_declarations = arguments.join(" ");
                    let argument_names = (0..signature.parameters.len())
                        .map(|index| format!("argument{index}"))
                        .collect::<Vec<_>>();
                    let receiver_variant =
                        layout.reference_variant(&Type::Class(reference.class.clone()))?;
                    let call = lower("_receiver", &argument_names);
                    let dispatch = exception_dispatch(method);
                    let push = if signature.returns == Type::Void {
                        format!("{call};")
                    } else {
                        format!(
                            "stack.push({});",
                            frame_variant(&signature.returns, &call, &layout)?
                        )
                    };
                    format!(
                        "{} match stack.pop().expect(\"verified JVM stack\") {{ FrameValue::{receiver_variant}(receiver_option) => match receiver_option {{ Some(_receiver) => {{ {push} }}, None => {{ let error = jars_runtime::null_pointer(); {dispatch} }} }}, FrameValue::Null => {{ let error = jars_runtime::null_pointer(); {dispatch} }}, _ => unreachable!(\"verified invokevirtual receiver\"), }} {}",
                        argument_declarations,
                        continue_at(next)?,
                    )
                } else if !known_classes.iter().any(|class| class == &reference.class) {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "virtual call owner outside the compilation set",
                    ));
                } else {
                    let mut arguments = Vec::with_capacity(signature.parameters.len());
                    for (index, ty) in signature.parameters.iter().enumerate().rev() {
                        let pop = frame_pop(ty, &layout)?;
                        arguments.push(format!("let argument{index} = {pop}(&mut stack);"));
                    }
                    let argument_declarations = arguments.join(" ");
                    let arguments = (0..signature.parameters.len())
                        .map(|index| format!("argument{index}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let method_name = rust_ident(&reference.name)?;
                    let receiver =
                        layout.reference_variant(&Type::Class(reference.class.clone()))?;
                    let call = if arguments.is_empty() {
                        format!("receiver.{method_name}().await")
                    } else {
                        format!("receiver.{method_name}({arguments}).await")
                    };
                    let dispatch = exception_dispatch(method);
                    let success = if signature.returns == Type::Void {
                        "Ok(()) => {}".to_owned()
                    } else {
                        format!(
                            "Ok(value) => stack.push({})",
                            frame_variant(&signature.returns, "value", &layout)?
                        )
                    };
                    format!(
                        "{} let receiver = stack.pop().expect(\"verified JVM stack\"); let result = match receiver {{ FrameValue::{receiver}(receiver) => match receiver {{ Some(receiver) => {call}, None => Err(jars_runtime::null_pointer()), }}, FrameValue::Null => Err(jars_runtime::null_pointer()), _ => unreachable!(\"verified invokevirtual receiver\"), }}; match result {{ {success}, Err(error) => {{ {dispatch} }} }} {}",
                        argument_declarations,
                        continue_at(next)?,
                    )
                }
            }
            Op::AThrow => {
                let dispatch = exception_dispatch(method);
                format!(
                    "let error = match stack.pop().expect(\"athrow requires a throwable operand\") {{ FrameValue::Throwable(error) => error, FrameValue::Null => jars_runtime::null_pointer(), _ => unreachable!(\"verified throwable operand\") }}; {dispatch}"
                )
            }
            Op::IReturn => match method.signature.returns {
                Type::Boolean => "return Ok(pop_i32(&mut stack) != 0);".to_owned(),
                Type::Byte => "return Ok(pop_i32(&mut stack) as i8);".to_owned(),
                Type::Char => "return Ok(pop_i32(&mut stack) as u16);".to_owned(),
                Type::Short => "return Ok(pop_i32(&mut stack) as i16);".to_owned(),
                Type::Int => "return Ok(pop_i32(&mut stack));".to_owned(),
                _ => {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "ireturn does not match the declared return type",
                    ));
                }
            },
            Op::LReturn => "return Ok(pop_i64(&mut stack));".to_owned(),
            Op::FReturn => "return Ok(pop_f32(&mut stack));".to_owned(),
            Op::DReturn => "return Ok(pop_f64(&mut stack));".to_owned(),
            Op::AReturn => {
                if method.signature.returns == Type::Void {
                    return Err(stack_error(
                        program,
                        method,
                        instruction,
                        "areturn in a void method",
                    ));
                }
                format!(
                    "return Ok({}(&mut stack));",
                    frame_pop(&method.signature.returns, &layout)?
                )
            }
            Op::Return => "return Ok(());".to_owned(),
            _ => {
                return Err(unsupported(
                    program,
                    method,
                    instruction,
                    "AOT integer state-machine instruction",
                ));
            }
        };
        arms.push(format!("{} => {{ {arm} }}", instruction.offset));
    }
    let initial_locals = method
        .signature
        .parameters
        .iter()
        .zip(parameter_slots)
        .enumerate()
        .map(|(index, (ty, local))| {
            Ok(format!(
                "locals[{local}] = Some({});",
                frame_variant(ty, format!("arg{index}"), &layout)?
            ))
        })
        .collect::<Result<String, CompileError>>()?;
    let entry = method
        .instructions
        .first()
        .ok_or_else(|| invalid("methods must contain at least one instruction"))?
        .offset;
    let reference_variants = layout
        .references
        .iter()
        .enumerate()
        .map(|(index, ty)| Ok(format!("R{index}({})", ty.rust(&program.name)?)))
        .collect::<Result<Vec<_>, CompileError>>()?
        .join(", ");
    let reference_variants = if reference_variants.is_empty() {
        String::new()
    } else {
        format!(", {reference_variants}")
    };
    let uninitialized_variants = (0..uninitialized.len())
        .map(|site| format!("U{site}"))
        .collect::<Vec<_>>()
        .join(", ");
    let uninitialized_variants = if uninitialized_variants.is_empty() {
        String::new()
    } else {
        format!(", {uninitialized_variants}")
    };
    let reference_pops = layout
        .references
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let alternatives = match ty {
                Type::String => {
                    format!(
                        "FrameValue::R{index}(value) => value, FrameValue::Null => None,"
                    )
                }
                Type::Class(class) => {
                    let coercions = stdlib::class(class)
                        .map(|entry| {
                            entry
                                .coercions
                                .iter()
                                .filter_map(|coercion| {
                                    let source_index = if coercion.from == "java/lang/String" {
                                        layout
                                            .references
                                            .iter()
                                            .position(|candidate| *candidate == Type::String)
                                    } else {
                                        layout.references.iter().position(|candidate| {
                                            matches!(candidate, Type::Class(candidate) if candidate == coercion.from)
                                        })
                                    }?;
                                    Some(format!(
                                        "FrameValue::R{source_index}(value) => {},",
                                        (coercion.lower)("value")
                                    ))
                                })
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    format!(
                        "FrameValue::R{index}(value) => value, {coercions} FrameValue::Null => None,"
                    )
                }
                Type::Array(_) => {
                    format!("FrameValue::R{index}(value) => value, FrameValue::Null => None,")
                }
                _ => unreachable!("frame layout contains reference types only"),
            };
            Ok(format!(
                "fn pop_r{index}(stack: &mut Vec<FrameValue>) -> {} {{ match stack.pop().expect(\"verified JVM stack\") {{ {alternatives} _ => unreachable!(\"verified JVM operand type\"), }} }}",
                ty.rust(&program.name)?
            ))
        })
        .collect::<Result<String, CompileError>>()?;
    let mut load_variants = layout
        .references
        .iter()
        .enumerate()
        .map(|(index, _)| {
            format!("FrameValue::R{index}(value) => FrameValue::R{index}(value.clone())")
        })
        .collect::<Vec<_>>();
    let this_variant = if layout.has_this { ", This" } else { "" };
    if layout.has_this {
        load_variants.push("FrameValue::This => FrameValue::This".to_owned());
    }
    load_variants.extend(
        (0..uninitialized.len()).map(|site| format!("FrameValue::U{site} => FrameValue::U{site}")),
    );
    load_variants.push("FrameValue::Null => FrameValue::Null".to_owned());
    let load_variants = load_variants.join(", ");
    let this_initialize = if layout.has_this {
        "locals[0] = Some(FrameValue::This);"
    } else {
        ""
    };
    let null_checks = layout
        .references
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let _ = ty;
            format!("FrameValue::R{index}(value) => value.is_none()")
        })
        .collect::<Vec<_>>();
    let mut null_checks = null_checks;
    if layout.has_this {
        null_checks.push("FrameValue::This => false".to_owned());
    }
    null_checks.push("FrameValue::Null => true".to_owned());
    let null_checks = null_checks.join(", ");
    let reference_equalities = layout
        .references
        .iter()
        .enumerate()
        .map(|(index, ty)| match ty {
            Type::String | Type::Class(_) => format!(
                "(FrameValue::R{index}(Some(left)), FrameValue::R{index}(Some(right))) => left.__same(right), (FrameValue::R{index}(None), FrameValue::R{index}(None)) => true, (FrameValue::R{index}(None), FrameValue::Null) | (FrameValue::Null, FrameValue::R{index}(None)) => true"
            ),
            Type::Array(_) => format!(
                "(FrameValue::R{index}(Some(left)), FrameValue::R{index}(Some(right))) => left.same(right), (FrameValue::R{index}(None), FrameValue::R{index}(None)) => true, (FrameValue::R{index}(None), FrameValue::Null) | (FrameValue::Null, FrameValue::R{index}(None)) => true"
            ),
            _ => unreachable!("frame layout contains reference types only"),
        })
        .collect::<Vec<_>>();
    let mut reference_equalities = reference_equalities;
    if layout.has_this {
        reference_equalities.push("(FrameValue::This, FrameValue::This) => true".to_owned());
    }
    reference_equalities.push("(FrameValue::Null, FrameValue::Null) => true".to_owned());
    reference_equalities.push("_ => false".to_owned());
    let reference_equalities = reference_equalities.join(", ");
    Ok(format!(
        "enum FrameValue {{ I32(i32), I64(i64), F32(f32), F64(f64){reference_variants}{this_variant}{uninitialized_variants}, Throwable(jars_runtime::JavaError), Null }}\nfn pop_i32(stack: &mut Vec<FrameValue>) -> i32 {{ match stack.pop().expect(\"verified JVM stack\") {{ FrameValue::I32(value) => value, _ => unreachable!(\"verified JVM operand type\"), }} }}\nfn pop_i64(stack: &mut Vec<FrameValue>) -> i64 {{ match stack.pop().expect(\"verified JVM stack\") {{ FrameValue::I64(value) => value, _ => unreachable!(\"verified JVM operand type\"), }} }}\nfn pop_f32(stack: &mut Vec<FrameValue>) -> f32 {{ match stack.pop().expect(\"verified JVM stack\") {{ FrameValue::F32(value) => value, _ => unreachable!(\"verified JVM operand type\"), }} }}\nfn pop_f64(stack: &mut Vec<FrameValue>) -> f64 {{ match stack.pop().expect(\"verified JVM stack\") {{ FrameValue::F64(value) => value, _ => unreachable!(\"verified JVM operand type\"), }} }}\n{reference_pops}\nfn duplicate(value: &FrameValue) -> FrameValue {{ match value {{ FrameValue::I32(value) => FrameValue::I32(*value), FrameValue::I64(value) => FrameValue::I64(*value), FrameValue::F32(value) => FrameValue::F32(*value), FrameValue::F64(value) => FrameValue::F64(*value), {load_variants}, FrameValue::Throwable(_) => unreachable!(\"throwable cannot be duplicated in the supported subset\"), }} }}\nfn is_null(value: &FrameValue) -> bool {{ match value {{ {null_checks}, _ => unreachable!(\"verified reference operand\"), }} }}\nfn same_reference(left: &FrameValue, right: &FrameValue) -> bool {{ match (left, right) {{ {reference_equalities} }} }}\nfn load_local(locals: &mut [Option<FrameValue>], index: usize) -> FrameValue {{ if matches!(locals[index], Some(FrameValue::Throwable(_))) {{ return locals[index].take().expect(\"initialized throwable local\"); }} match locals[index].as_ref().expect(\"initialized local\") {{ FrameValue::I32(value) => FrameValue::I32(*value), FrameValue::I64(value) => FrameValue::I64(*value), FrameValue::F32(value) => FrameValue::F32(*value), FrameValue::F64(value) => FrameValue::F64(*value), {load_variants}, FrameValue::Throwable(_) => unreachable!(\"handled throwable local\"), }} }}\nlet mut locals: Vec<Option<FrameValue>> = (0..{max_local}).map(|_| None).collect();\n{this_initialize}\n{initial_locals}\nlet mut stack: Vec<FrameValue> = Vec::new();\nlet mut pending_exception: Option<jars_runtime::JavaError> = None;\nlet mut pc: u32 = {entry};\nloop {{ match pc {{ {} , _ => unreachable!(\"verified JVM program counter\"), }} }}",
        arms.join(",\n"),
    ))
}

/// The sole method-body entry point.  The typed frame is necessary for control
/// flow and exception dispatch.  Straight-line code retains the compact body
/// lowering until its remaining operations have been moved into that table.
fn lower_method_body(
    program: &Program,
    known_classes: &[String],
    method: &Method,
    kind: BodyKind,
) -> Result<String, CompileError> {
    let needs_typed_frame = !method.handlers.is_empty()
        || method.instructions.iter().any(|instruction| {
            matches!(
                instruction.op,
                Op::IInc(_, _)
                    | Op::Goto(_)
                    | Op::If(_, _)
                    | Op::LookupSwitch { .. }
                    | Op::TableSwitch { .. }
                    | Op::AThrow
            )
        });
    if !needs_typed_frame {
        return Body::new(program, known_classes, method, kind).run();
    }
    aot_typed_frame(
        program,
        known_classes,
        method,
        match kind {
            BodyKind::Static => 0,
            BodyKind::Instance => 1,
        },
    )
}

fn static_method(
    program: &Program,
    known_classes: &[String],
    method: &Method,
) -> Result<String, CompileError> {
    let name = rust_ident(&method.name)?;
    let parameters = parameters(program, method)?;
    let body = lower_method_body(program, known_classes, method, BodyKind::Static)?;
    let fallback = default_return(&method.signature.returns);
    let returns = method.signature.returns.rust(&program.name)?;
    if crate::model::is_sync() {
        Ok(format!(
            "pub fn {name}(program: &super::Program, {parameters}) -> jars_runtime::JavaResult<{returns}> {{\n__ensure(program)?;\n{body}\nOk({fallback})\n}}"
        ))
    } else {
        Ok(format!(
            "pub async fn {name}<S: jars_runtime::Spawner>(program: &super::Program<S>, {parameters}) -> jars_runtime::JavaResult<{returns}> {{\n__ensure(program).await?;\n{body}\nOk({fallback})\n}}"
        ))
    }
}

fn instance_method(
    program: &Program,
    known_classes: &[String],
    method: &Method,
) -> Result<(String, String, String), CompileError> {
    let name = rust_ident(&method.name)?;
    let parameters = parameters(program, method)?;
    let body = lower_method_body(program, known_classes, method, BodyKind::Instance)?;
    let fallback = default_return(&method.signature.returns);
    let implementation = format!(
        "async fn {name}_impl<S: jars_runtime::Spawner>(state: std::rc::Rc<std::sync::Mutex<{}State>>, program: &super::Program<S>, {parameters}) -> jars_runtime::JavaResult<{}> {{\n{body}\nOk({fallback})\n}}",
        class_ident(&program.name)?,
        method.signature.returns.rust(&program.name)?,
    );
    let variant = format!(
        "{name} {{ {} }}",
        reply_fields(
            &parameters,
            &format!(
                "reply: jars_runtime::Reply<jars_runtime::JavaResult<{}>>",
                method.signature.returns.rust(&program.name)?,
            ),
        ),
    );
    let arguments = (0..method.signature.parameters.len())
        .map(|index| format!("arg{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let proxy = format!(
        "pub async fn {name}(&self{}) -> jars_runtime::JavaResult<{}> {{\nlet (reply, response) = jars_runtime::reply();\nself.actor.send({}Message::{name} {{ {} }}).await?;\nresponse.recv().await?\n}}",
        if parameters.is_empty() {
            String::new()
        } else {
            format!(", {parameters}")
        },
        method.signature.returns.rust(&program.name)?,
        class_ident(&program.name)?,
        reply_fields(&arguments, "reply"),
    );
    Ok((implementation, variant, proxy))
}

fn sync_actor_code(
    program: &Program,
    known_classes: &[String],
    constructor: &Method,
    methods: &[&Method],
) -> Result<String, CompileError> {
    let class = class_ident(&program.name)?;
    let field_declarations = program
        .fields
        .iter()
        .filter(|field| !field.is_static)
        .map(|field| {
            Ok(format!(
                "{}: {}",
                rust_ident(&field.name)?,
                field.ty.rust(&program.name)?
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()
        .map(|fields| fields.join(", "))?;
    let field_initializers = program
        .fields
        .iter()
        .filter(|field| !field.is_static)
        .map(|field| {
            Ok(format!(
                "{}: {}",
                rust_ident(&field.name)?,
                default_return(&field.ty)
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()
        .map(|fields| fields.join(", "))?;
    let constructor_parameters = parameters(program, constructor)?;
    let constructor_body =
        lower_method_body(program, known_classes, constructor, BodyKind::Instance)?;
    let state_ty = format!("std::rc::Rc<std::cell::RefCell<{class}State>>");
    let constructor_args = (0..constructor.signature.parameters.len())
        .map(|index| format!("arg{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut impls = vec![format!(
        "fn init_impl(state: {state_ty}, program: &super::Program, {constructor_parameters}) -> jars_runtime::JavaResult<()> {{\n{constructor_body}\nOk(())\n}}"
    )];
    let mut proxies = vec![if crate::model::current().is_entity() {
        "pub fn __same(&self, other: &Self) -> bool { self.entity == other.entity }".to_owned()
    } else {
        "pub fn __same(&self, other: &Self) -> bool { std::rc::Rc::ptr_eq(&self.state, &other.state) }".to_owned()
    }];
    for field in program.fields.iter().filter(|field| !field.is_static) {
        let field_name = rust_ident(&field.name)?;
        let getter = rust_ident(&format!("__get_{}", field.name))?;
        let setter = rust_ident(&format!("__set_{}", field.name))?;
        let ty = field.ty.rust(&program.name)?;
        proxies.push(format!(
            "pub fn {getter}(&self) -> jars_runtime::JavaResult<{ty}> {{ Ok(self.state.borrow().{field_name}.clone()) }}\npub fn {setter}(&self, value: {ty}) -> jars_runtime::JavaResult<()> {{ self.state.borrow_mut().{field_name} = value; Ok(()) }}"
        ));
    }
    for method in methods {
        let name = rust_ident(&method.name)?;
        let method_parameters = parameters(program, method)?;
        let body = lower_method_body(program, known_classes, method, BodyKind::Instance)?;
        let fallback = default_return(&method.signature.returns);
        let returns = method.signature.returns.rust(&program.name)?;
        impls.push(format!(
            "fn {name}_impl(state: {state_ty}, program: &super::Program, {method_parameters}) -> jars_runtime::JavaResult<{returns}> {{\n{body}\nOk({fallback})\n}}"
        ));
        let args = (0..method.signature.parameters.len())
            .map(|index| format!("arg{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let call_args = if args.is_empty() {
            String::new()
        } else {
            format!(", {args}")
        };
        let param_list = if method_parameters.is_empty() {
            String::new()
        } else {
            format!(", {method_parameters}")
        };
        proxies.push(format!(
            "pub fn {name}(&self{param_list}) -> jars_runtime::JavaResult<{returns}> {{ Self::{name}_impl(self.state.clone(), &self.program{call_args}) }}"
        ));
    }
    let constructor_list = if constructor_parameters.is_empty() {
        String::new()
    } else {
        format!(", {constructor_parameters}")
    };
    let init_args = if constructor_args.is_empty() {
        String::new()
    } else {
        format!(", {constructor_args}")
    };
    let (entity_field, spawn, entity_init) = if crate::model::current().is_entity() {
        (
            "entity: jars_bevy::ObjectId,",
            "let entity = program.table.borrow_mut().insert(std::rc::Rc::clone(&state));",
            "entity,",
        )
    } else {
        ("", "", "")
    };
    Ok(format!(
        "struct {class}State {{ {field_declarations} }}\n#[derive(Clone)]\npub struct {class} {{ state: {state_ty}, program: super::Program, {entity_field} }}\nimpl {class} {{\npub fn new(program: &super::Program{constructor_list}) -> jars_runtime::JavaResult<Self> {{\nlet state = std::rc::Rc::new(std::cell::RefCell::new({class}State {{ {field_initializers} }}));\n{spawn}\nSelf::init_impl(std::rc::Rc::clone(&state), program{init_args})?;\nOk(Self {{ state, program: program.clone(), {entity_init} }})\n}}\n{}\n{}\n}}",
        proxies.join("\n"),
        impls.join("\n"),
    ))
}

fn actor_code(
    program: &Program,
    known_classes: &[String],
    constructor: &Method,
    methods: &[&Method],
) -> Result<String, CompileError> {
    if crate::model::is_sync() {
        return sync_actor_code(program, known_classes, constructor, methods);
    }
    let class = class_ident(&program.name)?;
    let field_declarations = program
        .fields
        .iter()
        .filter(|field| !field.is_static)
        .map(|field| {
            Ok(format!(
                "{}: {}",
                rust_ident(&field.name)?,
                field.ty.rust(&program.name)?
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()
        .map(|fields| fields.join(", "))?;
    let field_initializers = program
        .fields
        .iter()
        .filter(|field| !field.is_static)
        .map(|field| {
            Ok(format!(
                "{}: {}",
                rust_ident(&field.name)?,
                default_return(&field.ty)
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()
        .map(|fields| fields.join(", "))?;
    let constructor_parameters = parameters(program, constructor)?;
    let constructor_message_fields = reply_fields(
        &constructor_parameters,
        "reply: jars_runtime::Reply<jars_runtime::JavaResult<()>>",
    );
    let constructor_body =
        lower_method_body(program, known_classes, constructor, BodyKind::Instance)?;
    let constructor_args = (0..constructor.signature.parameters.len())
        .map(|index| format!("arg{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut implementations = Vec::new();
    let mut variants = vec![format!("Init {{ {constructor_message_fields} }}")];
    let mut proxies = vec![
        "pub fn __same(&self, other: &Self) -> bool { self.actor.same(&other.actor) }".to_owned(),
    ];
    let mut dispatch = vec![format!(
        "{class}Message::Init {{ {} }} => {{ let value = {class}::init_impl(handler_state, &handler_program{}).await; let _ = reply.send(value); }}",
        reply_fields(&constructor_args, "reply"),
        if constructor_args.is_empty() {
            String::new()
        } else {
            format!(", {constructor_args}")
        },
    )];
    implementations.push(format!(
        "async fn init_impl<S: jars_runtime::Spawner>(state: std::rc::Rc<std::sync::Mutex<{class}State>>, program: &super::Program<S>, {constructor_parameters}) -> jars_runtime::JavaResult<()> {{\n{constructor_body}\nOk(())\n}}"
    ));
    for field in program.fields.iter().filter(|field| !field.is_static) {
        let field_name = rust_ident(&field.name)?;
        let getter = rust_ident(&format!("__get_{}", field.name))?;
        let setter = rust_ident(&format!("__set_{}", field.name))?;
        let ty = field.ty.rust(&program.name)?;
        variants.push(format!(
            "{getter} {{ reply: jars_runtime::Reply<jars_runtime::JavaResult<{ty}>> }}"
        ));
        variants.push(format!(
            "{setter} {{ value: {ty}, reply: jars_runtime::Reply<jars_runtime::JavaResult<()>> }}"
        ));
        dispatch.push(format!(
            "{class}Message::{getter} {{ reply }} => {{ let _ = reply.send(Ok(handler_state.lock().expect(\"actor state mutex\").{field_name}.clone())); }}"
        ));
        dispatch.push(format!(
            "{class}Message::{setter} {{ value, reply }} => {{ handler_state.lock().expect(\"actor state mutex\").{field_name} = value; let _ = reply.send(Ok(())); }}"
        ));
        proxies.push(format!(
            "pub async fn {getter}(&self) -> jars_runtime::JavaResult<{ty}> {{\nlet (reply, response) = jars_runtime::reply();\nself.actor.send({class}Message::{getter} {{ reply }}).await?;\nresponse.recv().await?\n}}\npub async fn {setter}(&self, value: {ty}) -> jars_runtime::JavaResult<()> {{\nlet (reply, response) = jars_runtime::reply();\nself.actor.send({class}Message::{setter} {{ value, reply }}).await?;\nresponse.recv().await?\n}}"
        ));
    }
    for method in methods {
        let (implementation, variant, proxy) = instance_method(program, known_classes, method)?;
        let name = rust_ident(&method.name)?;
        let args = (0..method.signature.parameters.len())
            .map(|index| format!("arg{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        implementations.push(implementation);
        variants.push(variant);
        proxies.push(proxy);
        dispatch.push(format!(
            "{class}Message::{name} {{ {} }} => {{ let value = {class}::{name}_impl(handler_state, &handler_program{}).await; let _ = reply.send(value); }}",
            reply_fields(&args, "reply"),
            if args.is_empty() { String::new() } else { format!(", {args}") },
        ));
    }
    Ok(format!(
        "struct {class}State {{ {field_declarations} }}\nenum {class}Message {{ {} }}\n#[derive(Clone)]\npub struct {class} {{ actor: jars_runtime::ActorRef<{class}Message> }}\nimpl {class} {{\npub async fn new<S: jars_runtime::Spawner>(program: &super::Program<S>{}) -> jars_runtime::JavaResult<Self> {{\nlet (actor, mailbox) = jars_runtime::actor_channel();\nlet actor_program = program.clone();\nlet spawner = actor_program.spawner.clone();\nspawner.spawn(async move {{\nuse jars_runtime::{{FutureExt as _, StreamExt as _}};\nlet state = std::rc::Rc::new(std::sync::Mutex::new({class}State {{ {field_initializers} }}));\nlet mut in_flight: jars_runtime::FuturesUnordered<std::pin::Pin<std::boxed::Box<dyn std::future::Future<Output = ()>>>> = jars_runtime::FuturesUnordered::new();\nloop {{\nif in_flight.is_empty() {{\nlet message = match mailbox.recv().await {{ Ok(message) => message, Err(_) => break }};\nlet handler_state = state.clone(); let handler_program = actor_program.clone();\nin_flight.push(std::boxed::Box::pin(async move {{ match message {{ {} }} }}));\n}} else {{\njars_runtime::select_biased! {{\nmessage = mailbox.recv().fuse() => match message {{\nOk(message) => {{ let handler_state = state.clone(); let handler_program = actor_program.clone(); in_flight.push(std::boxed::Box::pin(async move {{ match message {{ {} }} }})); }},\nErr(_) => break,\n}},\n_ = in_flight.next().fuse() => {{}},\n}}\n}}\n}}\n}});\nlet (reply, response) = jars_runtime::reply();\nactor.send({class}Message::Init {{ {} }}).await?;\nresponse.recv().await??;\nOk(Self {{ actor }})\n}}\n{}\n{}\n}}",
        variants.join(", "),
        if constructor_parameters.is_empty() {
            String::new()
        } else {
            format!(", {constructor_parameters}")
        },
        dispatch.join(", "),
        dispatch.join(", "),
        reply_fields(&constructor_args, "reply"),
        implementations.join("\n"),
        proxies.join("\n"),
    ))
}

fn constant_expression(constant: &Constant) -> String {
    match constant {
        Constant::Int(value) => value.to_string(),
        Constant::Long(value) => format!("{value}_i64"),
        Constant::Float(value) => format!("{value:?}_f32"),
        Constant::Double(value) => format!("{value:?}_f64"),
        Constant::String(value) => {
            format!("Some(jars_runtime::JavaString::new({value:?}))")
        }
    }
}

fn static_state_code(program: &Program, known_classes: &[String]) -> Result<String, CompileError> {
    let class = class_ident(&program.name)?;
    let fields = program
        .fields
        .iter()
        .filter(|field| field.is_static)
        .map(|field| {
            Ok(format!(
                "pub {}: {}",
                rust_ident(&field.name)?,
                field.ty.rust(&program.name)?
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()?
        .join(", ");
    let initializers = program
        .fields
        .iter()
        .filter(|field| field.is_static)
        .map(|field| {
            Ok(format!(
                "{}: {}",
                rust_ident(&field.name)?,
                field
                    .constant
                    .as_ref()
                    .map(constant_expression)
                    .unwrap_or_else(|| default_return(&field.ty).to_owned())
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()?
        .join(", ");
    let clinit = if let Some(method) = &program.clinit {
        // A class is marked `initializing` before entering its initializer.  Its
        // own static field operations must therefore access the prepared state
        // directly; emitting another `__ensure` call would recurse.
        let body = lower_method_body(program, known_classes, method, BodyKind::Static)?
            .replace("__ensure(program).await?;\n", "")
            .replace("__ensure(program)?;\n", "");
        if crate::model::is_sync() {
            format!(
                "pub(crate) fn __clinit(program: &super::Program) -> jars_runtime::JavaResult<()> {{\n{body}\nOk(())\n}}"
            )
        } else {
            format!(
                "pub(crate) async fn __clinit<S: jars_runtime::Spawner>(program: &super::Program<S>) -> jars_runtime::JavaResult<()> {{\n{body}\nOk(())\n}}"
            )
        }
    } else if crate::model::is_sync() {
        "pub(crate) fn __clinit(_program: &super::Program) -> jars_runtime::JavaResult<()> { Ok(()) }".to_owned()
    } else {
        "pub(crate) async fn __clinit<S: jars_runtime::Spawner>(_program: &super::Program<S>) -> jars_runtime::JavaResult<()> { Ok(()) }".to_owned()
    };
    let ensure = if crate::model::is_sync() {
        format!(
            "pub(crate) fn __ensure(program: &super::Program) -> jars_runtime::JavaResult<()> {{\nlet begin = {{ let mut state = program.state.borrow_mut(); let class = &mut state.{class}; if class.failure {{ return Err(jars_runtime::class_initialization_failed(stringify!({class}))); }} if class.initialized || class.initializing {{ false }} else {{ class.initializing = true; true }} }};\nif !begin {{ return Ok(()); }}\nlet result = __clinit(program);\nlet mut state = program.state.borrow_mut(); let class = &mut state.{class}; class.initializing = false; match result {{ Ok(()) => {{ class.initialized = true; Ok(()) }}, Err(error) => {{ class.failure = true; Err(error) }} }}\n}}"
        )
    } else {
        format!(
            "pub(crate) async fn __ensure<S: jars_runtime::Spawner>(program: &super::Program<S>) -> jars_runtime::JavaResult<()> {{\nlet begin = {{ let mut state = program.state.borrow_mut(); let class = &mut state.{class}; if class.failure {{ return Err(jars_runtime::class_initialization_failed(stringify!({class}))); }} if class.initialized || class.initializing {{ false }} else {{ class.initializing = true; true }} }};\nif !begin {{ return Ok(()); }}\nlet result = __clinit(program).await;\nlet mut state = program.state.borrow_mut(); let class = &mut state.{class}; class.initializing = false; match result {{ Ok(()) => {{ class.initialized = true; Ok(()) }}, Err(error) => {{ class.failure = true; Err(error) }} }}\n}}"
        )
    };
    Ok(format!(
        "pub struct {class}Statics {{ pub initialized: bool, pub initializing: bool, pub failure: bool, {fields} }}\nimpl {class}Statics {{ pub fn new() -> Self {{ Self {{ initialized: false, initializing: false, failure: false, {initializers} }} }} }}\n\n{ensure}\n{clinit}"
    ))
}

fn program_code(programs: &[Program]) -> Result<String, CompileError> {
    let declarations = programs
        .iter()
        .map(|program| {
            let class = class_ident(&program.name)?;
            Ok(format!("{}: {class}::{class}Statics", class))
        })
        .collect::<Result<Vec<_>, CompileError>>()?
        .join(", ");
    let initializers = programs
        .iter()
        .map(|program| {
            let class = class_ident(&program.name)?;
            Ok(format!("{class}: {class}::{class}Statics::new()"))
        })
        .collect::<Result<Vec<_>, CompileError>>()?
        .join(", ");
    if crate::model::current().is_entity() {
        Ok(format!(
            "struct ProgramState {{ {declarations} }}\n#[derive(Clone)]\npub struct Program {{ pub(crate) table: std::rc::Rc<std::cell::RefCell<jars_bevy::ObjectTable>>, pub(crate) state: std::rc::Rc<std::cell::RefCell<ProgramState>> }}\nimpl Program {{ pub fn new(table: std::rc::Rc<std::cell::RefCell<jars_bevy::ObjectTable>>) -> Self {{ Self {{ table, state: std::rc::Rc::new(std::cell::RefCell::new(ProgramState {{ {initializers} }})) }} }} }}"
        ))
    } else if crate::model::is_sync() {
        Ok(format!(
            "struct ProgramState {{ {declarations} }}\n#[derive(Clone)]\npub struct Program {{ pub(crate) state: std::rc::Rc<std::cell::RefCell<ProgramState>> }}\nimpl Program {{ pub fn new() -> Self {{ Self {{ state: std::rc::Rc::new(std::cell::RefCell::new(ProgramState {{ {initializers} }})) }} }} }}"
        ))
    } else {
        Ok(format!(
            "struct ProgramState {{ {declarations} }}\n#[derive(Clone)]\npub struct Program<S: jars_runtime::Spawner> {{ pub(crate) spawner: S, pub(crate) state: std::rc::Rc<std::cell::RefCell<ProgramState>> }}\nimpl<S: jars_runtime::Spawner> Program<S> {{ pub fn new(spawner: S) -> Self {{ Self {{ spawner, state: std::rc::Rc::new(std::cell::RefCell::new(ProgramState {{ {initializers} }})) }} }} }}"
        ))
    }
}

fn render_module(program: &Program, known_classes: &[String]) -> Result<String, CompileError> {
    let class = class_ident(&program.name)?;
    let static_state = static_state_code(program, known_classes)?;
    if program.is_interface {
        if program.methods.iter().any(|method| method.is_static) {
            return Err(invalid("interface static methods are not supported"));
        }
        return Ok(format!("pub mod {class} {{\n{static_state}\n}}"));
    }
    let statics = program
        .methods
        .iter()
        // A selected private static helper is still a direct Rust call from a
        // selected method in the same generated module.  Rust visibility here
        // is an implementation detail, never a Java reflective surface.
        .filter(|method| method.is_static)
        .map(|method| static_method(program, known_classes, method))
        .collect::<Result<Vec<_>, _>>()?;
    let constructors = program
        .methods
        .iter()
        .filter(|method| !method.is_static && method.name == "<init>")
        .collect::<Vec<_>>();
    let instances = program
        .methods
        .iter()
        .filter(|method| !method.is_static && method.name != "<init>")
        .collect::<Vec<_>>();
    if constructors.len() > 1 {
        return Err(invalid("v1 supports one constructor per class"));
    }
    let actor_needed = !instances.is_empty() || program.fields.iter().any(|field| !field.is_static);
    let actor = if actor_needed {
        let constructor = constructors
            .first()
            .ok_or_else(|| invalid("instance methods and fields require a constructor"))?;
        actor_code(program, known_classes, constructor, &instances)?
    } else {
        String::new()
    };
    Ok(format!(
        "pub mod {class} {{\n{static_state}\n{actor}\n{}\n}}",
        statics.join("\n")
    ))
}

fn entry_point(programs: &[Program]) -> Result<&Program, CompileError> {
    let entries = programs
        .iter()
        .filter(|program| {
            program.methods.iter().any(|method| {
                method.is_public
                    && method.is_static
                    && method.name == "main"
                    && method.signature.parameters == [Type::Array(Box::new(Type::String))]
                    && method.signature.returns == Type::Void
            })
        })
        .collect::<Vec<_>>();
    match entries.as_slice() {
        [entry] => Ok(*entry),
        [] if programs.len() == 1 => Err(CompileError::MissingEntryPoint {
            class: programs[0].name.clone(),
        }),
        [] => Err(invalid(
            "the compilation set does not define public static main(String[])",
        )),
        _ => Err(invalid(
            "the compilation set defines more than one public static main(String[])",
        )),
    }
}

fn render(programs: &[Program]) -> Result<String, CompileError> {
    let entry = entry_point(programs)?;
    let known_classes = programs
        .iter()
        .map(|program| program.name.clone())
        .collect::<Vec<_>>();
    let modules = programs
        .iter()
        .map(|program| render_module(program, &known_classes))
        .collect::<Result<Vec<_>, _>>()?;
    let program = program_code(programs)?;
    let class = class_ident(&entry.name)?;
    let source = if crate::model::current().is_entity() {
        format!(
            "{}\n{program}\nfn main() {{\nlet table = std::rc::Rc::new(std::cell::RefCell::new(jars_bevy::ObjectTable::new()));\nlet mut app = jars_bevy::headless_app(std::rc::Rc::clone(&table));\nlet program = Program::new(table);\napp.insert_non_send(program.clone());\nlet values: Vec<String> = std::env::args().skip(1).collect();\nlet result: jars_runtime::JavaResult<()> = (|| {{\nlet args = jars_runtime::JavaArray::direct(values.len() as i32, None)?;\nfor (index, value) in values.into_iter().enumerate() {{ args.set_sync(index as i32, Some(jars_runtime::JavaString::new(value)))?; }}\n{class}::main(&program, Some(args))?;\nOk(())\n}})();\nresult.expect(\"Java call failed\");\njars_bevy::update_once(&mut app);\n}}",
            modules.join("\n"),
        )
    } else if crate::model::is_sync() {
        format!(
            "{}\n{program}\nfn main() {{\nlet program = Program::new();\nlet values: Vec<String> = std::env::args().skip(1).collect();\nlet result: jars_runtime::JavaResult<()> = (|| {{\nlet args = jars_runtime::JavaArray::direct(values.len() as i32, None)?;\nfor (index, value) in values.into_iter().enumerate() {{ args.set_sync(index as i32, Some(jars_runtime::JavaString::new(value)))?; }}\n{class}::main(&program, Some(args))?;\nOk(())\n}})();\nresult.expect(\"Java call failed\");\n}}",
            modules.join("\n"),
        )
    } else {
        format!(
            "{}\n{program}\nfn main() {{\nlet runtime = jars_runtime::Runtime::new();\nlet program = Program::new(runtime.clone());\nlet values: Vec<String> = std::env::args().skip(1).collect();\nruntime.block_on(async {{\nlet args = jars_runtime::JavaArray::new(runtime.clone(), values.len() as i32, None)?;\nfor (index, value) in values.into_iter().enumerate() {{ args.set(index as i32, Some(jars_runtime::JavaString::new(value))).await?; }}\n{class}::main(&program, Some(args)).await\n}}).expect(\"Java actor call failed\");\n}}",
            modules.join("\n"),
        )
    };
    let source = crate::model::finish_source(source);
    let file = syn::parse_file(&source)
        .map_err(|error| invalid(format!("internal generated Rust was invalid: {error}")))?;
    Ok(prettyplease::unparse(&file))
}

fn validate_reference_type(ty: &Type, known_classes: &[String]) -> Result<(), CompileError> {
    match ty {
        Type::Class(class) => {
            if !stdlib::is_known_type(class) && !known_classes.iter().any(|known| known == class) {
                return Err(invalid(format!(
                    "reference type `{class}` is not in the compilation set"
                )));
            }
        }
        Type::Array(element) => validate_reference_type(element, known_classes)?,
        _ => {}
    }
    Ok(())
}

fn validate_reference_types(programs: &[Program]) -> Result<(), CompileError> {
    let known_classes = programs
        .iter()
        .map(|program| program.name.clone())
        .collect::<Vec<_>>();
    for program in programs {
        for field in &program.fields {
            validate_reference_type(&field.ty, &known_classes)?;
        }
        for method in &program.methods {
            for parameter in &method.signature.parameters {
                validate_reference_type(parameter, &known_classes)?;
            }
            validate_reference_type(&method.signature.returns, &known_classes)?;
        }
    }
    Ok(())
}

fn validate_field_accesses(programs: &[Program]) -> Result<(), CompileError> {
    let by_name = programs
        .iter()
        .map(|program| (program.name.as_str(), program))
        .collect::<HashMap<_, _>>();
    for program in programs {
        for method in program.methods.iter().chain(program.clinit.iter()) {
            for instruction in &method.instructions {
                let (reference, write, expect_static) = match &instruction.op {
                    Op::GetField(reference) => (reference, false, false),
                    Op::PutField(reference) => (reference, true, false),
                    Op::GetStatic(reference)
                        if stdlib::member(
                            &reference.class,
                            &reference.name,
                            &reference.descriptor,
                        )
                        .is_none() =>
                    {
                        (reference, false, true)
                    }
                    Op::PutStatic(reference) => (reference, true, true),
                    _ => continue,
                };
                let owner = by_name.get(reference.class.as_str()).ok_or_else(|| {
                    invalid(format!(
                        "field owner `{}` is not in the compilation set",
                        reference.class
                    ))
                })?;
                let field = owner
                    .fields
                    .iter()
                    .find(|field| field.name == reference.name)
                    .ok_or_else(|| {
                        invalid(format!(
                            "field `{}.{}` is not defined",
                            reference.class, reference.name
                        ))
                    })?;
                if field.is_static != expect_static {
                    return Err(invalid(format!(
                        "field access kind does not match `{}.{}`",
                        reference.class, reference.name
                    )));
                }
                if field.is_private && program.name != owner.name {
                    return Err(invalid(format!(
                        "private field `{}.{}` is accessed from `{}`",
                        owner.name, field.name, program.name
                    )));
                }
                if write && field.is_final {
                    let legal = if field.is_static {
                        method.name == "<clinit>" && program.name == owner.name
                    } else {
                        method.name == "<init>" && program.name == owner.name
                    };
                    if !legal {
                        return Err(invalid(format!(
                            "final field `{}.{}` may only be assigned by its declaring initializer",
                            owner.name, field.name
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_exception_tables(programs: &[Program]) -> Result<(), CompileError> {
    for program in programs {
        for method in program.methods.iter().chain(program.clinit.iter()) {
            let offsets = method
                .instructions
                .iter()
                .map(|instruction| instruction.offset)
                .collect::<HashSet<_>>();
            for handler in &method.handlers {
                if handler.start >= handler.end {
                    return Err(invalid(format!(
                        "exception table range {}..{} in {}.{} is empty or inverted",
                        handler.start, handler.end, program.name, method.name
                    )));
                }
                if !offsets.contains(&handler.start) {
                    return Err(invalid(format!(
                        "exception table start {} in {}.{} is not an instruction boundary",
                        handler.start, program.name, method.name
                    )));
                }
                if !offsets.contains(&handler.handler) {
                    return Err(invalid(format!(
                        "exception table handler {} in {}.{} is not an instruction boundary",
                        handler.handler, program.name, method.name
                    )));
                }
                // `end` may legally be the code length, which has no
                // instruction at that offset.  It must otherwise be a
                // boundary between two emitted instruction arms.
                if !offsets.contains(&handler.end)
                    && method
                        .instructions
                        .last()
                        .is_some_and(|instruction| handler.end <= instruction.offset)
                {
                    return Err(invalid(format!(
                        "exception table end {} in {}.{} is not an instruction boundary",
                        handler.end, program.name, method.name
                    )));
                }
            }
        }
    }
    Ok(())
}

fn validate_hierarchy(programs: &[Program]) -> Result<(), CompileError> {
    let by_name = programs
        .iter()
        .map(|program| (program.name.as_str(), program))
        .collect::<HashMap<_, _>>();
    for program in programs {
        if let Some(parent) = &program.superclass {
            if parent != "java/lang/Object" {
                let parent_program = by_name.get(parent.as_str()).ok_or_else(|| {
                    invalid(format!(
                        "superclass `{parent}` of `{}` is not in the compilation set",
                        program.name
                    ))
                })?;
                if parent_program.is_interface {
                    return Err(invalid(format!(
                        "superclass `{parent}` of `{}` is an interface",
                        program.name
                    )));
                }
            }
        }
        for interface in &program.interfaces {
            let interface_program = by_name.get(interface.as_str()).ok_or_else(|| {
                invalid(format!(
                    "interface `{interface}` of `{}` is not in the compilation set",
                    program.name
                ))
            })?;
            if !interface_program.is_interface {
                return Err(invalid(format!(
                    "interface `{interface}` of `{}` is not an interface",
                    program.name
                )));
            }
        }
    }

    fn visit<'a>(
        program: &'a Program,
        by_name: &HashMap<&'a str, &'a Program>,
        visiting: &mut HashSet<&'a str>,
        complete: &mut HashSet<&'a str>,
    ) -> Result<(), CompileError> {
        if complete.contains(program.name.as_str()) {
            return Ok(());
        }
        if !visiting.insert(program.name.as_str()) {
            return Err(invalid(format!(
                "cyclic class/interface hierarchy involving `{}`",
                program.name
            )));
        }
        let mut parents = program
            .interfaces
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if let Some(parent) = &program.superclass {
            if parent != "java/lang/Object" {
                parents.push(parent);
            }
        }
        for parent in parents {
            visit(by_name[parent], by_name, visiting, complete)?;
        }
        visiting.remove(program.name.as_str());
        complete.insert(program.name.as_str());
        Ok(())
    }

    let mut visiting = HashSet::new();
    let mut complete = HashSet::new();
    for program in programs {
        visit(program, &by_name, &mut visiting, &mut complete)?;
    }
    Ok(())
}

fn compile_programs(
    programs: Vec<Program>,
    object_model: ObjectModel,
) -> Result<String, CompileError> {
    let _guard = crate::model::enter(object_model);
    if programs.is_empty() {
        return Err(invalid("the compilation set is empty"));
    }
    for (index, program) in programs.iter().enumerate() {
        if programs[..index]
            .iter()
            .any(|previous| previous.name == program.name)
        {
            return Err(invalid(format!(
                "the compilation set contains duplicate class `{}`",
                program.name
            )));
        }
    }
    validate_class_idents(&programs)?;
    validate_hierarchy(&programs)?;
    validate_reference_types(&programs)?;
    validate_field_accesses(&programs)?;
    validate_exception_tables(&programs)?;
    render(&programs)
}

#[derive(Debug)]
struct JarClass {
    bytes: Vec<u8>,
    origin: PathBuf,
    release: u16,
}

fn class_name_from_bytes(bytes: &[u8]) -> Result<String, CompileError> {
    let class = parser_error(classfile::parse(bytes))?;
    parser_error(class.constant_pool.class_name(class.this_class))
}

fn read_jar_classpath(
    paths: &[impl AsRef<Path>],
    options: JarImportOptions,
) -> Result<BTreeMap<String, JarClass>, CompileError> {
    if paths.is_empty() {
        return Err(invalid("the JAR classpath is empty"));
    }
    if options.target_release < 8 {
        return Err(invalid("the JAR target release must be at least Java 8"));
    }

    let mut classes: BTreeMap<String, JarClass> = BTreeMap::new();
    for path in paths {
        let path = path.as_ref();
        let file = File::open(path).map_err(|error| CompileError::Jar {
            jar: path.to_owned(),
            detail: error.to_string(),
        })?;
        let mut archive = zip::ZipArchive::new(file).map_err(|error| CompileError::Jar {
            jar: path.to_owned(),
            detail: error.to_string(),
        })?;
        let multi_release = match archive.by_name("META-INF/MANIFEST.MF") {
            Ok(mut manifest) => {
                let mut value = String::new();
                manifest
                    .read_to_string(&mut value)
                    .map_err(|error| CompileError::Jar {
                        jar: path.to_owned(),
                        detail: format!("could not read manifest: {error}"),
                    })?;
                value
                    .lines()
                    .any(|line| line.trim().eq_ignore_ascii_case("Multi-Release: true"))
            }
            Err(zip::result::ZipError::FileNotFound) => false,
            Err(error) => {
                return Err(CompileError::Jar {
                    jar: path.to_owned(),
                    detail: format!("could not read manifest: {error}"),
                });
            }
        };

        // Select the base entry or the highest eligible overlay for every
        // logical class name before parsing it.  `module-info` describes the
        // archive module and is not a loadable closed-world program class.
        let mut selected: BTreeMap<String, (String, u16)> = BTreeMap::new();
        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(|error| CompileError::Jar {
                jar: path.to_owned(),
                detail: error.to_string(),
            })?;
            let entry_name = entry.name().to_owned();
            if !entry_name.ends_with(".class") || entry_name.ends_with('/') {
                continue;
            }
            let (logical_name, release) =
                if let Some(rest) = entry_name.strip_prefix("META-INF/versions/") {
                    let Some((release, logical_name)) = rest.split_once('/') else {
                        return Err(CompileError::Jar {
                            jar: path.to_owned(),
                            detail: format!("invalid multi-release class entry `{entry_name}`"),
                        });
                    };
                    let release = release.parse::<u16>().map_err(|_| CompileError::Jar {
                        jar: path.to_owned(),
                        detail: format!("invalid multi-release release in `{entry_name}`"),
                    })?;
                    if release < 9 || logical_name.is_empty() {
                        return Err(CompileError::Jar {
                            jar: path.to_owned(),
                            detail: format!("invalid multi-release class entry `{entry_name}`"),
                        });
                    }
                    if !multi_release {
                        return Err(CompileError::Jar {
                            jar: path.to_owned(),
                            detail: "multi-release class entries require Multi-Release: true"
                                .to_owned(),
                        });
                    }
                    (logical_name.to_owned(), release)
                } else {
                    (entry_name.clone(), 0)
                };
            if logical_name == "module-info.class" || release > options.target_release {
                continue;
            }
            match selected.get(&logical_name) {
                Some((_, previous_release)) if *previous_release > release => {}
                Some((previous, previous_release)) if *previous_release == release => {
                    return Err(CompileError::Jar {
                        jar: path.to_owned(),
                        detail: format!(
                            "multiple entries select `{logical_name}` for Java {release}: `{previous}` and `{entry_name}`"
                        ),
                    });
                }
                _ => {
                    selected.insert(logical_name, (entry_name, release));
                }
            }
        }
        for (logical_name, (entry_name, release)) in selected {
            let mut entry = archive
                .by_name(&entry_name)
                .map_err(|error| CompileError::Jar {
                    jar: path.to_owned(),
                    detail: error.to_string(),
                })?;
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|error| CompileError::Jar {
                    jar: path.to_owned(),
                    detail: format!("could not read `{entry_name}`: {error}"),
                })?;
            let class = class_name_from_bytes(&bytes).map_err(|error| CompileError::Jar {
                jar: path.to_owned(),
                detail: format!("could not read `{entry_name}`: {error}"),
            })?;
            if logical_name != format!("{class}.class") {
                return Err(CompileError::Jar {
                    jar: path.to_owned(),
                    detail: format!(
                        "class entry `{entry_name}` declares `{class}`, expected `{logical_name}`"
                    ),
                });
            }
            if let Some(previous) = classes.get(&class) {
                return Err(CompileError::DuplicateClass {
                    class,
                    first: previous.origin.clone(),
                    second: path.to_owned(),
                });
            }
            classes.insert(
                class,
                JarClass {
                    bytes,
                    origin: path.to_owned(),
                    release,
                },
            );
        }
    }
    Ok(classes)
}

fn add_type_dependencies(ty: &Type, dependencies: &mut Vec<String>) {
    match ty {
        Type::Class(class) => dependencies.push(class.clone()),
        Type::Array(element) => add_type_dependencies(element, dependencies),
        _ => {}
    }
}

fn add_signature_dependencies(signature: &Signature, dependencies: &mut Vec<String>) {
    for parameter in &signature.parameters {
        add_type_dependencies(parameter, dependencies);
    }
    add_type_dependencies(&signature.returns, dependencies);
}

fn add_member_dependencies(
    reference: &MemberRef,
    dependencies: &mut Vec<String>,
) -> Result<(), CompileError> {
    dependencies.push(reference.class.clone());
    // Platform members are classified by their owner before inspecting their
    // descriptors.  Their descriptors routinely contain package-qualified
    // JDK types that are intentionally outside the default-package compiler
    // subset; modeled calls are lowered by the emitter, and unmodeled calls
    // receive the stable platform diagnostic below.
    if reference.class.starts_with("java/") {
        return Ok(());
    }
    let signature = if reference.descriptor.starts_with('(') {
        parse_signature(&reference.descriptor)?
    } else {
        let mut cursor = 0;
        let ty = parse_type(&reference.descriptor, &mut cursor)?;
        if cursor != reference.descriptor.len() {
            return Err(invalid(format!(
                "invalid field descriptor `{}`",
                reference.descriptor
            )));
        }
        Signature {
            parameters: Vec::new(),
            returns: ty,
        }
    };
    add_signature_dependencies(&signature, dependencies);
    Ok(())
}

fn program_dependencies(program: &Program) -> Result<Vec<String>, CompileError> {
    let mut dependencies = Vec::new();
    if let Some(superclass) = &program.superclass {
        dependencies.push(superclass.clone());
    }
    dependencies.extend(program.interfaces.iter().cloned());
    for field in &program.fields {
        add_type_dependencies(&field.ty, &mut dependencies);
    }
    for method in program.methods.iter().chain(program.clinit.iter()) {
        add_signature_dependencies(&method.signature, &mut dependencies);
        for handler in &method.handlers {
            if let Some(class) = &handler.catch_type {
                dependencies.push(class.clone());
            }
        }
        for instruction in &method.instructions {
            match &instruction.op {
                Op::New(class) => dependencies.push(class.clone()),
                Op::NewArray(ty) | Op::MultiNewArray(ty, _) => {
                    add_type_dependencies(ty, &mut dependencies)
                }
                Op::GetStatic(reference)
                | Op::GetField(reference)
                | Op::PutField(reference)
                | Op::PutStatic(reference)
                | Op::InvokeSpecial(reference)
                | Op::InvokeStatic(reference)
                | Op::InvokeVirtual(reference)
                | Op::InvokeInterface(reference) => {
                    add_member_dependencies(reference, &mut dependencies)?;
                }
                _ => {}
            }
        }
    }
    Ok(dependencies)
}

fn method_dependencies(method: &Method) -> Result<Vec<String>, CompileError> {
    let mut dependencies = Vec::new();
    add_signature_dependencies(&method.signature, &mut dependencies);
    for handler in &method.handlers {
        if let Some(class) = &handler.catch_type {
            dependencies.push(class.clone());
        }
    }
    for instruction in &method.instructions {
        match &instruction.op {
            Op::New(class) => dependencies.push(class.clone()),
            Op::NewArray(ty) | Op::MultiNewArray(ty, _) => {
                add_type_dependencies(ty, &mut dependencies)
            }
            Op::GetStatic(reference)
            | Op::GetField(reference)
            | Op::PutField(reference)
            | Op::PutStatic(reference)
            | Op::InvokeSpecial(reference)
            | Op::InvokeStatic(reference)
            | Op::InvokeVirtual(reference)
            | Op::InvokeInterface(reference) => {
                add_member_dependencies(reference, &mut dependencies)?;
            }
            _ => {}
        }
    }
    Ok(dependencies)
}

fn active_class_dependencies(program: &Program) -> Vec<String> {
    let mut dependencies = Vec::new();
    if let Some(superclass) = &program.superclass {
        dependencies.push(superclass.clone());
    }
    dependencies.extend(program.interfaces.iter().cloned());
    // Fields are emitted as generated static/actor state even when no method
    // reads them. Their types therefore remain part of the closed set.
    for field in &program.fields {
        add_type_dependencies(&field.ty, &mut dependencies);
    }
    dependencies
}

fn enqueue_dependency(
    dependency: String,
    referenced_by: &str,
    classes: &BTreeMap<String, JarClass>,
    active: &mut VecDeque<(String, String)>,
    active_set: &mut HashSet<String>,
) -> Result<(), CompileError> {
    if stdlib::is_known_type(&dependency) {
        return Ok(());
    }
    if dependency.starts_with("java/") {
        return Err(CompileError::UnsupportedPlatformClass {
            class: dependency,
            referenced_by: referenced_by.to_owned(),
        });
    }
    if !classes.contains_key(&dependency) {
        return Err(CompileError::MissingClass {
            class: dependency,
            referenced_by: referenced_by.to_owned(),
        });
    }
    if active_set.insert(dependency.clone()) {
        active.push_back((dependency, referenced_by.to_owned()));
    }
    Ok(())
}

/// Parses only the classes needed by `entry`, then retains only methods that
/// are directly reachable from it (plus required `<clinit>` bodies).  Parsing
/// remains structural for every member, so malformed class files still fail
/// early; unlowered bytecodes in unrelated methods are never emitted.
fn select_jar_programs(
    classes: &BTreeMap<String, JarClass>,
    entry: &JarEntrypoint,
) -> Result<Vec<Program>, CompileError> {
    let mut active = VecDeque::from([(entry.class.clone(), "the declared entrypoint".to_owned())]);
    let mut active_set = HashSet::from([entry.class.clone()]);
    let mut method_queue = VecDeque::from([(
        entry.class.clone(),
        MethodKey::new(&entry.method, &entry.descriptor),
        "the declared entrypoint".to_owned(),
    )]);
    let mut selected: BTreeMap<String, HashSet<MethodKey>> = BTreeMap::new();
    let mut programs = BTreeMap::<String, Program>::new();

    while !active.is_empty() || !method_queue.is_empty() {
        while let Some((class, referenced_by)) = active.pop_front() {
            let class_file = classes
                .get(&class)
                .ok_or_else(|| CompileError::MissingClass {
                    class: class.clone(),
                    referenced_by,
                })?;
            let program = if let Some(program) = programs.get(&class) {
                program.clone()
            } else {
                parse_program_custom(&class_file.bytes)?
            };
            for dependency in active_class_dependencies(&program) {
                enqueue_dependency(
                    dependency,
                    &program.name,
                    classes,
                    &mut active,
                    &mut active_set,
                )?;
            }
            if let Some(clinit) = &program.clinit {
                method_queue.push_back((
                    program.name.clone(),
                    MethodKey::new("<clinit>", &clinit.descriptor),
                    program.name.clone(),
                ));
            }
            programs.entry(class).or_insert(program);
        }

        let Some((class, key, referenced_by)) = method_queue.pop_front() else {
            continue;
        };
        if !active_set.contains(&class) {
            active_set.insert(class.clone());
            active.push_back((class.clone(), referenced_by));
            method_queue.push_front((class, key, "selected method owner".to_owned()));
            continue;
        }
        let Some(program) = programs.get(&class) else {
            // The active queue will parse it on the next outer iteration.
            method_queue.push_front((class, key, referenced_by));
            continue;
        };
        if !selected
            .entry(class.clone())
            .or_default()
            .insert(key.clone())
        {
            continue;
        }
        let method = if key.name == "<clinit>" {
            program
                .clinit
                .as_ref()
                .filter(|method| method.descriptor == key.descriptor)
        } else {
            program
                .methods
                .iter()
                .find(|method| method.name == key.name && method.descriptor == key.descriptor)
        }
        .ok_or_else(|| {
            if class == entry.class
                && key.name == entry.method
                && key.descriptor == entry.descriptor
            {
                CompileError::MissingEntryMethod {
                    class: class.clone(),
                    method: key.name.clone(),
                    descriptor: key.descriptor.clone(),
                }
            } else {
                invalid(format!("method `{}.{}` is not defined", class, key.name))
            }
        })?;
        for dependency in method_dependencies(method)? {
            enqueue_dependency(
                dependency,
                &program.name,
                classes,
                &mut active,
                &mut active_set,
            )?;
        }
        for instruction in &method.instructions {
            let reference = match &instruction.op {
                Op::InvokeSpecial(reference)
                | Op::InvokeStatic(reference)
                | Op::InvokeVirtual(reference)
                | Op::InvokeInterface(reference)
                    if !reference.class.starts_with("java/")
                        && !stdlib::is_known_type(&reference.class) =>
                {
                    Some(reference)
                }
                _ => None,
            };
            if let Some(reference) = reference {
                method_queue.push_back((
                    reference.class.clone(),
                    MethodKey::new(&reference.name, &reference.descriptor),
                    program.name.clone(),
                ));
            }
        }
    }

    let mut selected_programs = programs
        .into_iter()
        .map(|(class, mut program)| {
            let selected_methods = selected.remove(&class).unwrap_or_default();
            program.methods.retain(|method| {
                selected_methods.contains(&MethodKey::new(&method.name, &method.descriptor))
            });
            if let Some(clinit) = &program.clinit
                && !selected_methods.contains(&MethodKey::new("<clinit>", &clinit.descriptor))
            {
                program.clinit = None;
            }
            program
        })
        .collect::<Vec<_>>();
    selected_programs.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(selected_programs)
}

/// Produces a deterministic Markdown report for a real JAR classpath using
/// the same owned parser and import-selection rules as [`compile_jars`].
///
/// It intentionally reports both the selected member closure and class-wide
/// references: the latter makes future expansion blockers visible without
/// allowing unrelated methods to affect the AOT output.
pub fn triage_jars(
    paths: &[impl AsRef<Path>],
    entry: &JarEntrypoint,
    options: JarImportOptions,
) -> Result<String, CompileError> {
    let classes = read_jar_classpath(paths, options)?;
    let programs = select_jar_programs(&classes, entry)?;
    let mut report = format!(
        "# JAR triage\n\nTarget Java release: {}\n\nEntrypoint: `{}.{}`{}\n\n## Selected closure\n\n",
        options.target_release, entry.class, entry.method, entry.descriptor
    );
    for program in &programs {
        let class_file = classes.get(&program.name).expect("selected JAR class");
        let parsed = parser_error(classfile::parse(&class_file.bytes))?;
        let methods = program
            .methods
            .iter()
            .map(|method| format!("`{}{}`", method.name, method.descriptor))
            .chain(
                program
                    .clinit
                    .iter()
                    .map(|method| format!("`{}{}`", method.name, method.descriptor)),
            )
            .collect::<Vec<_>>()
            .join(", ");
        report.push_str(&format!(
            "- `{}` — Java class-file {}.{}, selected JAR release {}, source `{}`; members: {}\n",
            program.name,
            parsed.version.major,
            parsed.version.minor,
            class_file.release,
            class_file.origin.display(),
            if methods.is_empty() {
                "(none)"
            } else {
                &methods
            },
        ));
    }
    report.push_str("\n## Class-wide references\n\n");
    for program in &programs {
        let original = parse_program_custom(&classes[&program.name].bytes)?;
        let mut dependencies = program_dependencies(&original)?;
        dependencies.sort();
        dependencies.dedup();
        let jdk = dependencies
            .iter()
            .filter(|dependency| dependency.starts_with("java/"))
            .cloned()
            .collect::<Vec<_>>();
        let third_party = dependencies
            .iter()
            .filter(|dependency| !dependency.starts_with("java/"))
            .cloned()
            .collect::<Vec<_>>();
        let jdk = if jdk.is_empty() {
            "(none)".to_owned()
        } else {
            jdk.join(", ")
        };
        let third_party = if third_party.is_empty() {
            "(none)".to_owned()
        } else {
            third_party.join(", ")
        };
        report.push_str(&format!(
            "- `{}`\n  - JDK: {}\n  - non-JDK: {}\n",
            program.name, jdk, third_party,
        ));
    }
    Ok(report)
}

fn render_declared_entry(
    programs: &[Program],
    entry: &JarEntrypoint,
) -> Result<String, CompileError> {
    let entry_program = programs
        .iter()
        .find(|program| program.name == entry.class)
        .ok_or_else(|| CompileError::MissingClass {
            class: entry.class.clone(),
            referenced_by: "the declared entrypoint".to_owned(),
        })?;
    let entry_method = entry_program
        .methods
        .iter()
        .find(|method| {
            method.is_public
                && method.is_static
                && method.name == entry.method
                && method.descriptor == entry.descriptor
        })
        .ok_or_else(|| CompileError::MissingEntryMethod {
            class: entry.class.clone(),
            method: entry.method.clone(),
            descriptor: entry.descriptor.clone(),
        })?;
    let class = class_ident(&entry_program.name)?;
    let method = rust_ident(&entry_method.name)?;
    let invocation_arguments = entry_method
        .signature
        .parameters
        .iter()
        .enumerate()
        .map(|(index, ty)| default_argument(ty).map(|value| (index, value)))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            invalid(format!(
                "JAR entrypoint {}.{}{} has a parameter type with no default-argument lowering",
                entry.class, entry.method, entry.descriptor
            ))
        })?;
    let call = format!(
        "{class}::{method}(&program{})",
        invocation_arguments
            .iter()
            .map(|(index, value)| format!(", arg{index} = {value}"))
            .collect::<String>()
    );
    // The entry result is discarded uniformly: `JavaUnit` wraps both `()`
    // and meaningful values so the generated `block_on` can always `.expect`.
    let invocation = if crate::model::is_sync() {
        format!("let _entry = jars_runtime::JavaUnit({call});")
    } else {
        format!("let _entry = jars_runtime::JavaUnit({call}.await);")
    };
    let known_classes = programs
        .iter()
        .map(|program| program.name.clone())
        .collect::<Vec<_>>();
    let modules = programs
        .iter()
        .map(|program| render_module(program, &known_classes))
        .collect::<Result<Vec<_>, _>>()?;
    let program = program_code(programs)?;
    let source = if crate::model::current().is_entity() {
        format!(
            "{}\n{program}\nfn main() {{\nlet table = std::rc::Rc::new(std::cell::RefCell::new(jars_bevy::ObjectTable::new()));\nlet mut app = jars_bevy::headless_app(std::rc::Rc::clone(&table));\nlet program = Program::new(table);\napp.insert_non_send(program.clone());\nlet result: jars_runtime::JavaResult<()> = (|| {{\n{invocation}\nOk(())\n}})();\nresult.expect(\"Java call failed\");\njars_bevy::update_once(&mut app);\n}}",
            modules.join("\n"),
        )
    } else if crate::model::is_sync() {
        format!(
            "{}\n{program}\nfn main() {{\nlet program = Program::new();\nlet result: jars_runtime::JavaResult<()> = (|| {{\n{invocation}\nOk(())\n}})();\nresult.expect(\"Java call failed\");\n}}",
            modules.join("\n"),
        )
    } else {
        format!(
            "{}\n{program}\nfn main() {{\nlet runtime = jars_runtime::Runtime::new();\nlet program = Program::new(runtime.clone());\nlet _entry = jars_runtime::JavaUnit(runtime.block_on(async {{\n{invocation}\n}}));\n}}",
            modules.join("\n"),
        )
    };
    let source = crate::model::finish_source(source);
    let file = syn::parse_file(&source)
        .map_err(|error| invalid(format!("internal generated Rust was invalid: {error}")))?;
    Ok(prettyplease::unparse(&file))
}

/// Renders the generated Rust for an already-parsed program set rooted at
/// `entry`. This is the seam the coverage report uses to compile one class in
/// isolation without re-reading a JAR.
pub(crate) fn render_jar_program(
    programs: &[Program],
    entry: &JarEntrypoint,
) -> Result<String, CompileError> {
    render_declared_entry(programs, entry)
}

/// Imports the reachable closed class set from explicit JARs and emits an AOT
/// Rust executable rooted at `entry`.
///
/// The archive is only an input format: class bytes are parsed while compiling
/// and are not retained by the generated program. Package-qualified binary
/// names remain the compiler identity and are deterministically encoded only
/// where Rust identifiers are needed.
pub fn compile_jars(
    paths: &[impl AsRef<Path>],
    entry: &JarEntrypoint,
) -> Result<String, CompileError> {
    compile_jars_with_options(paths, entry, JarImportOptions::default())
}

/// Like [`compile_jars`], with an explicit target used to select
/// multi-release JAR entries.
pub fn compile_jars_with_options(
    paths: &[impl AsRef<Path>],
    entry: &JarEntrypoint,
    options: JarImportOptions,
) -> Result<String, CompileError> {
    compile_jars_with_model(paths, entry, options, ObjectModel::Task)
}

/// Like [`compile_jars_with_options`], emitting the selected object model.
pub fn compile_jars_with_model(
    paths: &[impl AsRef<Path>],
    entry: &JarEntrypoint,
    options: JarImportOptions,
    object_model: ObjectModel,
) -> Result<String, CompileError> {
    let _guard = crate::model::enter(object_model);
    let classes = read_jar_classpath(paths, options)?;
    let programs = select_jar_programs(&classes, entry)?;
    if programs.is_empty() {
        return Err(invalid("the compilation set is empty"));
    }
    for (index, program) in programs.iter().enumerate() {
        if programs[..index]
            .iter()
            .any(|previous| previous.name == program.name)
        {
            return Err(invalid(format!(
                "the compilation set contains duplicate class `{}`",
                program.name
            )));
        }
    }
    validate_class_idents(&programs)?;
    validate_hierarchy(&programs)?;
    validate_reference_types(&programs)?;
    validate_field_accesses(&programs)?;
    validate_exception_tables(&programs)?;
    render_declared_entry(&programs, entry)
}

/// Compiles one default-package Java class file into a complete Rust source file.
pub fn compile_class(bytes: &[u8]) -> Result<String, CompileError> {
    compile_classes(&[bytes])
}

/// Compiles one class for an explicit object model.
pub fn compile_class_with_model(
    bytes: &[u8],
    object_model: ObjectModel,
) -> Result<String, CompileError> {
    compile_classes_with_model(&[bytes], object_model)
}

/// Compiles a closed, default-package class set into one AOT Rust source file.
///
/// References to classes outside `classes` are rejected during code generation;
/// the generated modules call each other directly and retain no class-file data.
pub fn compile_classes(classes: &[&[u8]]) -> Result<String, CompileError> {
    compile_classes_with_model(classes, ObjectModel::Task)
}

/// Compiles a closed class set for an explicit object model.
pub fn compile_classes_with_model(
    classes: &[&[u8]],
    object_model: ObjectModel,
) -> Result<String, CompileError> {
    let programs = classes
        .iter()
        .map(|bytes| parse_program_custom(bytes))
        .collect::<Result<Vec<_>, _>>()?;
    compile_programs(programs, object_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn java_26_main_class() -> Vec<u8> {
        fn u16(bytes: &mut Vec<u8>, value: u16) {
            bytes.extend(value.to_be_bytes());
        }
        fn u32(bytes: &mut Vec<u8>, value: u32) {
            bytes.extend(value.to_be_bytes());
        }
        fn utf8(bytes: &mut Vec<u8>, value: &str) {
            bytes.push(1);
            u16(bytes, value.len().try_into().unwrap());
            bytes.extend(value.as_bytes());
        }

        let mut bytes = Vec::new();
        bytes.extend(0xcafe_babe_u32.to_be_bytes());
        u16(&mut bytes, 0);
        u16(&mut bytes, 70);
        u16(&mut bytes, 8);
        utf8(&mut bytes, "Java26Main");
        bytes.extend([7, 0, 1]);
        utf8(&mut bytes, "java/lang/Object");
        bytes.extend([7, 0, 3]);
        utf8(&mut bytes, "main");
        utf8(&mut bytes, "([Ljava/lang/String;)V");
        utf8(&mut bytes, "Code");
        u16(&mut bytes, 0x0021);
        u16(&mut bytes, 2);
        u16(&mut bytes, 4);
        u16(&mut bytes, 0);
        u16(&mut bytes, 0);
        u16(&mut bytes, 1);
        u16(&mut bytes, 0x0009);
        u16(&mut bytes, 5);
        u16(&mut bytes, 6);
        u16(&mut bytes, 1);
        u16(&mut bytes, 7);
        u32(&mut bytes, 13);
        u16(&mut bytes, 0);
        u16(&mut bytes, 1);
        u32(&mut bytes, 1);
        bytes.push(0xb1);
        u16(&mut bytes, 0);
        u16(&mut bytes, 0);
        u16(&mut bytes, 0);
        bytes
    }

    #[test]
    fn descriptors_accept_numeric_and_recursive_array_types() {
        assert!(parse_signature("(JFD)V").is_ok());
        assert!(parse_signature("(Z[[I[[[Ljava/lang/String;)V").is_ok());
        assert!(parse_signature("(Lapp/Entry;)[Llibrary/Value;").is_ok());
        assert!(parse_signature("(I").is_err());
    }

    #[test]
    fn public_compiler_accepts_a_java_26_class_file() {
        let generated = compile_class(&java_26_main_class()).unwrap();
        assert!(generated.contains("pub mod Java26Main"));
    }

    #[test]
    fn rust_keywords_are_escaped() {
        assert_eq!(rust_ident("type").unwrap(), "r#type");
    }

    #[test]
    fn package_and_inner_class_binary_names_are_encoded_for_rust() {
        assert_eq!(
            class_ident("app/Entry").unwrap(),
            "__jars_class_6170702f456e747279"
        );
        assert_eq!(
            class_ident("app/Entry$Inner").unwrap(),
            "__jars_class_6170702f456e74727924496e6e6572"
        );
    }
}
