//! A deliberately small Java class-file to actor-oriented Rust compiler.
//!
//! The public surface is intentionally narrow: `compile_class` accepts one
//! default-package class and emits a complete Rust binary which depends on
//! `jars-runtime`.

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use noak::{
    AccessFlags,
    reader::{
        AttributeContent, Class,
        attributes::RawInstruction,
        cpool::{self, Item},
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    Parse(String),
    InvalidClass(String),
    MissingEntryPoint {
        class: String,
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
            Self::String => Ok("&'static str".to_owned()),
            Self::Class(name) if name == current_class => {
                Ok(format!("Option<{}>", rust_ident(name)?))
            }
            Self::Class(name) => Ok(format!(
                "Option<super::{}::{}>",
                rust_ident(name)?,
                rust_ident(name)?
            )),
            Self::Array(element) => Ok(format!(
                "Option<jars_runtime::JavaArray<{}>>",
                element.array_element_rust(current_class)?
            )),
        }
    }

    fn array_element_rust(&self, current_class: &str) -> Result<String, CompileError> {
        match self {
            Self::String => Ok("String".to_owned()),
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
    InvokeSpecial(MemberRef),
    InvokeStatic(MemberRef),
    InvokeVirtual(MemberRef),
    InvokeInterface(MemberRef),
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
    signature: Signature,
    is_static: bool,
    is_public: bool,
    instructions: Vec<Instruction>,
    handlers: Vec<ExceptionHandler>,
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

fn utf8(value: &noak::MStr) -> Result<String, CompileError> {
    value
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("class-file modified UTF-8 must be valid UTF-8"))
}

fn class_name(
    pool: &cpool::ConstantPool<'_>,
    index: cpool::Index<cpool::Class<'_>>,
) -> Result<String, CompileError> {
    let value = pool
        .retrieve(index)
        .map_err(|error| CompileError::Parse(error.to_string()))?;
    utf8(value.name)
}

fn member_ref(
    pool: &cpool::ConstantPool<'_>,
    class: cpool::Index<cpool::Class<'_>>,
    name_and_type: cpool::Index<cpool::NameAndType<'_>>,
) -> Result<MemberRef, CompileError> {
    let name_and_type = pool
        .retrieve(name_and_type)
        .map_err(|error| CompileError::Parse(error.to_string()))?;
    Ok(MemberRef {
        class: class_name(pool, class)?,
        name: utf8(name_and_type.name)?,
        descriptor: utf8(name_and_type.descriptor)?,
    })
}

fn item_member_ref(
    pool: &cpool::ConstantPool<'_>,
    item: &Item<'_>,
) -> Result<MemberRef, CompileError> {
    match item {
        Item::MethodRef(reference) => member_ref(pool, reference.class, reference.name_and_type),
        _ => Err(invalid(
            "invokespecial/invokestatic must reference a method",
        )),
    }
}

fn parse_type(input: &str, cursor: &mut usize) -> Result<Type, CompileError> {
    let remainder = &input[*cursor..];
    if remainder.starts_with('Z') {
        *cursor += 1;
        Ok(Type::Boolean)
    } else if remainder.starts_with('B') {
        *cursor += 1;
        Ok(Type::Byte)
    } else if remainder.starts_with('C') {
        *cursor += 1;
        Ok(Type::Char)
    } else if remainder.starts_with('S') {
        *cursor += 1;
        Ok(Type::Short)
    } else if remainder.starts_with('I') {
        *cursor += 1;
        Ok(Type::Int)
    } else if remainder.starts_with('J') {
        *cursor += 1;
        Ok(Type::Long)
    } else if remainder.starts_with('F') {
        *cursor += 1;
        Ok(Type::Float)
    } else if remainder.starts_with('D') {
        *cursor += 1;
        Ok(Type::Double)
    } else if remainder.starts_with('V') {
        *cursor += 1;
        Ok(Type::Void)
    } else if remainder.starts_with("Ljava/lang/String;") {
        *cursor += "Ljava/lang/String;".len();
        Ok(Type::String)
    } else if remainder.starts_with('[') {
        *cursor += 1;
        Ok(Type::Array(Box::new(parse_type(input, cursor)?)))
    } else if remainder.starts_with('L') {
        let end = remainder.find(';').ok_or_else(|| {
            invalid(format!(
                "unterminated reference descriptor fragment `{remainder}`"
            ))
        })?;
        let name = &remainder[1..end];
        if name.is_empty() || name.contains('/') {
            return Err(invalid(format!(
                "unsupported descriptor fragment `{remainder}`"
            )));
        }
        *cursor += end + 1;
        Ok(Type::Class(name.to_owned()))
    } else {
        Err(invalid(format!(
            "unsupported descriptor fragment `{remainder}`"
        )))
    }
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
    if !name.is_empty() && !name.contains('/') {
        return Ok(Type::Class(name.to_owned()));
    }
    Err(invalid(format!("unsupported array component `{name}`")))
}

fn parse_op(pool: &cpool::ConstantPool<'_>, raw: RawInstruction<'_>) -> Result<Op, CompileError> {
    use RawInstruction::*;
    let op = match raw {
        AConstNull => Op::AConstNull,
        ANewArray { index } => Op::NewArray(array_component_type(&class_name(pool, index)?)?),
        NewArray { atype } => Op::NewArray(match atype {
            noak::reader::attributes::ArrayType::Boolean => Type::Boolean,
            noak::reader::attributes::ArrayType::Byte => Type::Byte,
            noak::reader::attributes::ArrayType::Char => Type::Char,
            noak::reader::attributes::ArrayType::Short => Type::Short,
            noak::reader::attributes::ArrayType::Int => Type::Int,
            noak::reader::attributes::ArrayType::Long => Type::Long,
            noak::reader::attributes::ArrayType::Float => Type::Float,
            noak::reader::attributes::ArrayType::Double => Type::Double,
        }),
        MultiANewArray { index, dimensions } => {
            let descriptor = class_name(pool, index)?;
            let mut cursor = 0;
            let ty = parse_type(&descriptor, &mut cursor)?;
            if cursor != descriptor.len() || !matches!(ty, Type::Array(_)) {
                return Err(invalid(format!(
                    "invalid multianewarray descriptor `{descriptor}`"
                )));
            }
            Op::MultiNewArray(ty, dimensions)
        }
        ArrayLength => Op::ArrayLength,
        IALoad => Op::IALoad,
        IAStore => Op::IAStore,
        LALoad => Op::LALoad,
        LAStore => Op::LAStore,
        FALoad => Op::FALoad,
        FAStore => Op::FAStore,
        DALoad => Op::DALoad,
        DAStore => Op::DAStore,
        BALoad => Op::BALoad,
        BAStore => Op::BAStore,
        CALoad => Op::CALoad,
        CAStore => Op::CAStore,
        SALoad => Op::SALoad,
        SAStore => Op::SAStore,
        AALoad => Op::AALoad,
        AAStore => Op::AAStore,
        AThrow => Op::AThrow,
        IConstM1 => Op::IConst(-1),
        IConst0 => Op::IConst(0),
        IConst1 => Op::IConst(1),
        IConst2 => Op::IConst(2),
        IConst3 => Op::IConst(3),
        IConst4 => Op::IConst(4),
        IConst5 => Op::IConst(5),
        BIPush { value } => Op::IConst(value.into()),
        SIPush { value } => Op::IConst(value.into()),
        LConst0 => Op::LConst(0),
        LConst1 => Op::LConst(1),
        FConst0 => Op::FConst(0.0),
        FConst1 => Op::FConst(1.0),
        FConst2 => Op::FConst(2.0),
        DConst0 => Op::DConst(0.0),
        DConst1 => Op::DConst(1.0),
        ILoad { index } => Op::ILoad(index.into()),
        ILoadW { index } => Op::ILoad(index.into()),
        ILoad0 => Op::ILoad(0),
        ILoad1 => Op::ILoad(1),
        ILoad2 => Op::ILoad(2),
        ILoad3 => Op::ILoad(3),
        IStore { index } => Op::IStore(index.into()),
        IStoreW { index } => Op::IStore(index.into()),
        IStore0 => Op::IStore(0),
        IStore1 => Op::IStore(1),
        IStore2 => Op::IStore(2),
        IStore3 => Op::IStore(3),
        ALoad { index } => Op::ALoad(index.into()),
        ALoadW { index } => Op::ALoad(index.into()),
        ALoad0 => Op::ALoad(0),
        ALoad1 => Op::ALoad(1),
        ALoad2 => Op::ALoad(2),
        ALoad3 => Op::ALoad(3),
        LLoad { index } => Op::LLoad(index.into()),
        LLoadW { index } => Op::LLoad(index.into()),
        LLoad0 => Op::LLoad(0),
        LLoad1 => Op::LLoad(1),
        LLoad2 => Op::LLoad(2),
        LLoad3 => Op::LLoad(3),
        FLoad { index } => Op::FLoad(index.into()),
        FLoadW { index } => Op::FLoad(index.into()),
        FLoad0 => Op::FLoad(0),
        FLoad1 => Op::FLoad(1),
        FLoad2 => Op::FLoad(2),
        FLoad3 => Op::FLoad(3),
        DLoad { index } => Op::DLoad(index.into()),
        DLoadW { index } => Op::DLoad(index.into()),
        DLoad0 => Op::DLoad(0),
        DLoad1 => Op::DLoad(1),
        DLoad2 => Op::DLoad(2),
        DLoad3 => Op::DLoad(3),
        AStore { index } => Op::AStore(index.into()),
        AStoreW { index } => Op::AStore(index.into()),
        AStore0 => Op::AStore(0),
        AStore1 => Op::AStore(1),
        AStore2 => Op::AStore(2),
        AStore3 => Op::AStore(3),
        LStore { index } => Op::LStore(index.into()),
        LStoreW { index } => Op::LStore(index.into()),
        LStore0 => Op::LStore(0),
        LStore1 => Op::LStore(1),
        LStore2 => Op::LStore(2),
        LStore3 => Op::LStore(3),
        FStore { index } => Op::FStore(index.into()),
        FStoreW { index } => Op::FStore(index.into()),
        FStore0 => Op::FStore(0),
        FStore1 => Op::FStore(1),
        FStore2 => Op::FStore(2),
        FStore3 => Op::FStore(3),
        DStore { index } => Op::DStore(index.into()),
        DStoreW { index } => Op::DStore(index.into()),
        DStore0 => Op::DStore(0),
        DStore1 => Op::DStore(1),
        DStore2 => Op::DStore(2),
        DStore3 => Op::DStore(3),
        IAdd => Op::IAdd,
        ISub => Op::ISub,
        IMul => Op::IMul,
        IDiv => Op::IDiv,
        IRem => Op::IRem,
        INeg => Op::INeg,
        IAnd => Op::IAnd,
        IOr => Op::IOr,
        IXor => Op::IXor,
        IShL => Op::IShl,
        IShR => Op::IShr,
        IUShR => Op::IUshr,
        LAdd => Op::LAdd,
        LSub => Op::LSub,
        LMul => Op::LMul,
        LDiv => Op::LDiv,
        LRem => Op::LRem,
        LNeg => Op::LNeg,
        FAdd => Op::FAdd,
        FSub => Op::FSub,
        FMul => Op::FMul,
        FDiv => Op::FDiv,
        FRem => Op::FRem,
        FNeg => Op::FNeg,
        DAdd => Op::DAdd,
        DSub => Op::DSub,
        DMul => Op::DMul,
        DDiv => Op::DDiv,
        DRem => Op::DRem,
        DNeg => Op::DNeg,
        IInc { index, value } => Op::IInc(index.into(), value.into()),
        IIncW { index, value } => Op::IInc(index.into(), value.into()),
        Goto { offset } => Op::Goto(offset.into()),
        GotoW { offset } => Op::Goto(offset),
        IfEq { offset } => Op::If(IfKind::Eq, offset.into()),
        IfNe { offset } => Op::If(IfKind::Ne, offset.into()),
        IfLt { offset } => Op::If(IfKind::Lt, offset.into()),
        IfGe { offset } => Op::If(IfKind::Ge, offset.into()),
        IfGt { offset } => Op::If(IfKind::Gt, offset.into()),
        IfLe { offset } => Op::If(IfKind::Le, offset.into()),
        IfICmpEq { offset } => Op::If(IfKind::ICmpEq, offset.into()),
        IfICmpNe { offset } => Op::If(IfKind::ICmpNe, offset.into()),
        IfICmpLt { offset } => Op::If(IfKind::ICmpLt, offset.into()),
        IfICmpGe { offset } => Op::If(IfKind::ICmpGe, offset.into()),
        IfICmpGt { offset } => Op::If(IfKind::ICmpGt, offset.into()),
        IfICmpLe { offset } => Op::If(IfKind::ICmpLe, offset.into()),
        IfACmpEq { offset } => Op::If(IfKind::ACmpEq, offset.into()),
        IfACmpNe { offset } => Op::If(IfKind::ACmpNe, offset.into()),
        IfNull { offset } => Op::If(IfKind::Null, offset.into()),
        IfNonNull { offset } => Op::If(IfKind::NonNull, offset.into()),
        LookupSwitch(switch) => Op::LookupSwitch {
            default: switch.default_offset(),
            cases: switch
                .pairs()
                .map(|pair| (pair.key(), pair.offset()))
                .collect(),
        },
        TableSwitch(switch) => Op::TableSwitch {
            default: switch.default_offset(),
            cases: switch
                .pairs()
                .map(|pair| (pair.key(), pair.offset()))
                .collect(),
        },
        IReturn => Op::IReturn,
        LReturn => Op::LReturn,
        FReturn => Op::FReturn,
        DReturn => Op::DReturn,
        AReturn => Op::AReturn,
        Return => Op::Return,
        Dup => Op::Dup,
        New { index } => Op::New(class_name(pool, index)?),
        GetStatic { index } => {
            let reference = pool
                .retrieve(index)
                .map_err(|error| CompileError::Parse(error.to_string()))?;
            Op::GetStatic(MemberRef {
                class: utf8(reference.class.name)?,
                name: utf8(reference.name_and_type.name)?,
                descriptor: utf8(reference.name_and_type.descriptor)?,
            })
        }
        GetField { index } => {
            let reference = pool
                .retrieve(index)
                .map_err(|error| CompileError::Parse(error.to_string()))?;
            Op::GetField(MemberRef {
                class: utf8(reference.class.name)?,
                name: utf8(reference.name_and_type.name)?,
                descriptor: utf8(reference.name_and_type.descriptor)?,
            })
        }
        PutField { index } => {
            let reference = pool
                .retrieve(index)
                .map_err(|error| CompileError::Parse(error.to_string()))?;
            Op::PutField(MemberRef {
                class: utf8(reference.class.name)?,
                name: utf8(reference.name_and_type.name)?,
                descriptor: utf8(reference.name_and_type.descriptor)?,
            })
        }
        PutStatic { index } => {
            let reference = pool
                .retrieve(index)
                .map_err(|error| CompileError::Parse(error.to_string()))?;
            Op::PutStatic(MemberRef {
                class: utf8(reference.class.name)?,
                name: utf8(reference.name_and_type.name)?,
                descriptor: utf8(reference.name_and_type.descriptor)?,
            })
        }
        InvokeVirtual { index } => {
            let reference = pool
                .retrieve(index)
                .map_err(|error| CompileError::Parse(error.to_string()))?;
            Op::InvokeVirtual(MemberRef {
                class: utf8(reference.class.name)?,
                name: utf8(reference.name_and_type.name)?,
                descriptor: utf8(reference.name_and_type.descriptor)?,
            })
        }
        InvokeInterface { index, .. } => {
            let reference = pool
                .retrieve(index)
                .map_err(|error| CompileError::Parse(error.to_string()))?;
            Op::InvokeInterface(MemberRef {
                class: utf8(reference.class.name)?,
                name: utf8(reference.name_and_type.name)?,
                descriptor: utf8(reference.name_and_type.descriptor)?,
            })
        }
        InvokeSpecial { index } => Op::InvokeSpecial(item_member_ref(
            pool,
            pool.get(index)
                .map_err(|error| CompileError::Parse(error.to_string()))?,
        )?),
        InvokeStatic { index } => Op::InvokeStatic(item_member_ref(
            pool,
            pool.get(index)
                .map_err(|error| CompileError::Parse(error.to_string()))?,
        )?),
        LdC { index } | LdCW { index } => match pool
            .get(index)
            .map_err(|error| CompileError::Parse(error.to_string()))?
        {
            Item::String(value) => Op::LdcString(utf8(
                pool.get(value.string)
                    .map_err(|error| CompileError::Parse(error.to_string()))?
                    .content,
            )?),
            Item::Integer(value) => Op::IConst(value.value),
            Item::Float(value) => Op::FConst(value.value),
            other => return Err(invalid(format!("unsupported ldc constant {other:?}"))),
        },
        LdC2W { index } => match pool
            .get(index)
            .map_err(|error| CompileError::Parse(error.to_string()))?
        {
            Item::Long(value) => Op::LConst(value.value),
            Item::Double(value) => Op::DConst(value.value),
            other => return Err(invalid(format!("unsupported ldc2 constant {other:?}"))),
        },
        other => return Err(invalid(format!("unsupported bytecode {other:?}"))),
    };
    Ok(op)
}

fn parse_program(bytes: &[u8]) -> Result<Program, CompileError> {
    let class = Class::new(bytes).map_err(|error| CompileError::Parse(error.to_string()))?;
    let pool = class.pool();
    let program_name = class_name(pool, class.this_class())?;
    if program_name.contains('/') {
        return Err(invalid("packages are not supported in v1"));
    }
    let superclass = class
        .super_class()
        .map(|index| class_name(pool, index))
        .transpose()?;
    let interfaces = class
        .interfaces()
        .into_iter()
        .map(|interface| {
            interface
                .map_err(|error| CompileError::Parse(error.to_string()))
                .and_then(|index| class_name(pool, index))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut fields = Vec::new();
    for field in class.fields() {
        let field = field.map_err(|error| CompileError::Parse(error.to_string()))?;
        let name = utf8(
            pool.get(field.name())
                .map_err(|error| CompileError::Parse(error.to_string()))?
                .content,
        )?;
        let descriptor = utf8(
            pool.get(field.descriptor())
                .map_err(|error| CompileError::Parse(error.to_string()))?
                .content,
        )?;
        let mut cursor = 0;
        let ty = parse_type(&descriptor, &mut cursor)?;
        if matches!(ty, Type::Void) || cursor != descriptor.len() {
            return Err(invalid("fields must use a supported non-void descriptor"));
        }
        let mut constant = None;
        for attribute in field.attributes() {
            let attribute = attribute.map_err(|error| CompileError::Parse(error.to_string()))?;
            if let AttributeContent::ConstantValue(value) = attribute
                .read_content(pool)
                .map_err(|error| CompileError::Parse(error.to_string()))?
            {
                constant = Some(
                    match pool
                        .get(value.value())
                        .map_err(|error| CompileError::Parse(error.to_string()))?
                    {
                        Item::Integer(value) => Constant::Int(value.value),
                        Item::Long(value) => Constant::Long(value.value),
                        Item::Float(value) => Constant::Float(value.value),
                        Item::Double(value) => Constant::Double(value.value),
                        Item::String(value) => Constant::String(utf8(
                            pool.get(value.string)
                                .map_err(|error| CompileError::Parse(error.to_string()))?
                                .content,
                        )?),
                        other => {
                            return Err(invalid(format!("unsupported ConstantValue {other:?}")));
                        }
                    },
                );
            }
        }
        fields.push(Field {
            name,
            ty,
            is_static: field.access_flags().contains(AccessFlags::STATIC),
            is_final: field.access_flags().contains(AccessFlags::FINAL),
            is_private: field.access_flags().contains(AccessFlags::PRIVATE),
            constant,
        });
    }

    let mut methods = Vec::new();
    let mut clinit = None;
    for method in class.methods() {
        let method = method.map_err(|error| CompileError::Parse(error.to_string()))?;
        let name = utf8(
            pool.get(method.name())
                .map_err(|error| CompileError::Parse(error.to_string()))?
                .content,
        )?;
        let descriptor = utf8(
            pool.get(method.descriptor())
                .map_err(|error| CompileError::Parse(error.to_string()))?
                .content,
        )?;
        let signature = parse_signature(&descriptor)?;
        let mut instructions = None;
        let mut handlers = Vec::new();
        for attribute in method.attributes() {
            let attribute = attribute.map_err(|error| CompileError::Parse(error.to_string()))?;
            if let AttributeContent::Code(code) = attribute
                .read_content(pool)
                .map_err(|error| CompileError::Parse(error.to_string()))?
            {
                let code = code;
                handlers = code
                    .exception_handlers()
                    .map(|handler| {
                        let catch_type = handler
                            .catch_type()
                            .map(|index| class_name(pool, index))
                            .transpose()?;
                        Ok(ExceptionHandler {
                            start: handler.start().as_u32(),
                            end: handler.end().as_u32(),
                            handler: handler.handler().as_u32(),
                            catch_type,
                        })
                    })
                    .collect::<Result<Vec<_>, CompileError>>()?;
                let mut parsed = Vec::new();
                for instruction in code.raw_instructions() {
                    let (offset, raw) =
                        instruction.map_err(|error| CompileError::Parse(error.to_string()))?;
                    let offset = offset.as_u32();
                    let op = parse_op(pool, raw).map_err(|error| match error {
                        CompileError::InvalidClass(operation) => CompileError::Unsupported {
                            class: program_name.clone(),
                            method: name.clone(),
                            offset,
                            operation,
                        },
                        other => other,
                    })?;
                    parsed.push(Instruction { offset, op });
                }
                instructions = Some(parsed);
            }
        }
        let parsed = Method {
            name: name.clone(),
            signature,
            is_static: method.access_flags().contains(AccessFlags::STATIC),
            is_public: method.access_flags().contains(AccessFlags::PUBLIC),
            instructions: match instructions {
                Some(instructions) => instructions,
                None if method.access_flags().contains(AccessFlags::ABSTRACT) => Vec::new(),
                None => return Err(invalid("methods must have Code attributes")),
            },
            handlers,
        };
        if name == "<clinit>" {
            clinit = Some(parsed);
        } else {
            methods.push(parsed);
        }
    }
    Ok(Program {
        name: program_name,
        superclass,
        interfaces,
        is_interface: class.access_flags().contains(AccessFlags::INTERFACE),
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
    Long(String),
    Float(String),
    Double(String),
    String(String),
    PrintStream,
    This,
    Object { expression: String, class: String },
    Array { expression: String, element: Type },
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
            Ok(format!("super::{}::", rust_ident(class)?))
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
        for _ in &signature.parameters {
            args.push(self.pop_expression(instruction)?);
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
            Type::String => "String::new()".to_owned(),
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
            "let {name} = Some(jars_runtime::JavaArray::<{element_rust}>::new(program.spawner.clone(), {}, {default})?);",
            lengths[depth]
        );
        if depth + 1 < active_dimensions {
            let (child, child_code) =
                self.multi_array_code(element, active_dimensions, lengths, depth + 1)?;
            let index = format!("index{}", self.next_temp);
            self.next_temp += 1;
            code.push_str(&format!(
                "for {index} in 0..{} {{ {child_code} {name}.as_ref().expect(\"new Java array\").set({index}, {child}).await?; }}",
                lengths[depth]
            ));
        }
        Ok((name, code))
    }

    fn run(mut self) -> Result<String, CompileError> {
        for instruction in &self.method.instructions {
            match &instruction.op {
                Op::AConstNull => self.stack.push(Value::Null),
                Op::NewArray(element) => {
                    let length = self.pop_expression(instruction)?;
                    let rust = element.array_element_rust(&self.program.name)?;
                    let default = Self::array_default(element)?;
                    let expression = format!(
                        "Some(jars_runtime::JavaArray::<{rust}>::new(program.spawner.clone(), {length}, {default})?)"
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
                        format!("{array}.ok_or_else(jars_runtime::null_pointer)?.length().await?"),
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
                        "{array}.ok_or_else(jars_runtime::null_pointer)?.get({index}).await?"
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
                    self.statements.push(format!("{array}.ok_or_else(jars_runtime::null_pointer)?.set({index}, {stored}).await?;"));
                }
                Op::AALoad => {
                    let index = self.pop_expression(instruction)?;
                    let (array, element) = self.pop_array(instruction)?;
                    let expression = format!(
                        "{array}.ok_or_else(jars_runtime::null_pointer)?.get({index}).await?"
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
                    self.statements.push(format!("{array}.ok_or_else(jars_runtime::null_pointer)?.set({index}, {value}).await?;"));
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
                Op::LdcString(value) => self.stack.push(Value::String(format!("{value:?}"))),
                Op::ILoad(index) => self.stack.push(self.local(*index, instruction)?),
                Op::LLoad(index) | Op::FLoad(index) | Op::DLoad(index) => {
                    self.stack.push(self.local(*index, instruction)?)
                }
                Op::ALoad(index) => {
                    let value = self.local(*index, instruction)?;
                    self.stack.push(match value {
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
                    if reference.class == "java/lang/System"
                        && reference.name == "out"
                        && reference.descriptor == "Ljava/io/PrintStream;"
                    {
                        self.stack.push(Value::PrintStream);
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
                        let class = rust_ident(&reference.class)?;
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
                            Type::Boolean | Type::Byte | Type::Char | Type::Short | Type::Int => {
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
                                format!("state.lock().expect(\"actor state mutex\").{field}")
                            }
                            _ => format!(
                                "state.lock().expect(\"actor state mutex\").{field}.clone()"
                            ),
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
                                "state.lock().expect(\"actor state mutex\").{field} = {value};"
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
                    let class = rust_ident(&reference.class)?;
                    let class_path = self.class_path(&reference.class)?;
                    let field = rust_ident(&reference.name)?;
                    self.statements
                        .push(format!("{class_path}__ensure(program).await?;"));
                    self.statements.push(format!(
                        "program.state.borrow_mut().{class}.{field} = {value};"
                    ));
                }
                Op::New(class) => {
                    if !self.known_classes.iter().any(|known| known == class) {
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
                    self.stack.push(value);
                }
                Op::InvokeSpecial(reference) => {
                    let signature = parse_signature(&reference.descriptor)?;
                    let args = self.arguments(instruction, &signature)?;
                    let receiver = self.pop(instruction)?;
                    if reference.class == "java/lang/Object" && reference.name == "<init>" {
                        if !matches!(receiver, Value::This) || !args.is_empty() {
                            return Err(unsupported(
                                self.program,
                                self.method,
                                instruction,
                                "Object.<init>",
                            ));
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
                            rust_ident(&reference.class)?
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
                    let signature = parse_signature(&reference.descriptor)?;
                    let args = self.arguments(instruction, &signature)?;
                    let method = format!(
                        "{}{}",
                        self.class_path(&reference.class)?,
                        rust_ident(&reference.name)?
                    );
                    let expression = format!("{method}(program, {}).await?", args.join(", "));
                    match signature.returns {
                        Type::Boolean | Type::Byte | Type::Char | Type::Short | Type::Int => {
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
                }
                Op::InvokeVirtual(reference) | Op::InvokeInterface(reference) => {
                    let signature = parse_signature(&reference.descriptor)?;
                    let args = self.arguments(instruction, &signature)?;
                    let receiver = self.pop(instruction)?;
                    if reference.class == "java/io/PrintStream" && reference.name == "println" {
                        if !matches!(receiver, Value::PrintStream)
                            || args.len() != 1
                            || !matches!(signature.returns, Type::Void)
                            || !matches!(
                                signature.parameters.as_slice(),
                                [Type::Int]
                                    | [Type::Long]
                                    | [Type::Float]
                                    | [Type::Double]
                                    | [Type::String]
                            )
                        {
                            return Err(unsupported(
                                self.program,
                                self.method,
                                instruction,
                                "PrintStream.println",
                            ));
                        }
                        self.statements
                            .push(format!("jars_runtime::println({});", args[0]));
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
                            Type::Boolean | Type::Byte | Type::Char | Type::Short | Type::Int => {
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
        Type::String => "\"\"",
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

/// Emits an AOT state machine for integer-only static methods with branches.
///
/// There is no bytecode representation at run time: every bytecode instruction
/// becomes one Rust `match` arm.  `pc`, locals and the operand stack are merely
/// the activation record required to preserve JVM ordering across back-edges.
fn aot_int_static_method(
    program: &Program,
    known_classes: &[String],
    method: &Method,
) -> Result<String, CompileError> {
    if method.signature.returns != Type::Int
        || method
            .signature
            .parameters
            .iter()
            .any(|parameter| *parameter != Type::Int)
    {
        return Err(invalid(format!(
            "AOT control-flow method {}.{} must use only int parameters and an int return",
            program.name, method.name
        )));
    }
    let max_local = method
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.op {
            Op::ILoad(index)
            | Op::IStore(index)
            | Op::ALoad(index)
            | Op::AStore(index)
            | Op::IInc(index, _) => Some(*index),
            _ => None,
        })
        .chain(std::iter::once(
            method.signature.parameters.len().saturating_sub(1),
        ))
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
            Op::IConst(value) => format!("stack.push({value}); {}", continue_at(next)?),
            Op::ILoad(local) => format!("stack.push(locals[{local}]); {}", continue_at(next)?),
            Op::IStore(local) => format!(
                "locals[{local}] = stack.pop().expect(\"verified JVM stack\"); {}",
                continue_at(next)?
            ),
            Op::ALoad(local) => format!(
                "thrown = Some(exception_locals[{local}].take().expect(\"initialized throwable local\")); {}",
                continue_at(next)?
            ),
            Op::AStore(local) => format!(
                "exception_locals[{local}] = Some(pending_exception.take().expect(\"exception handler entry\")); {}",
                continue_at(next)?
            ),
            Op::AConstNull => format!(
                "thrown = Some(jars_runtime::null_pointer()); {}",
                continue_at(next)?
            ),
            Op::IInc(local, value) => format!(
                "locals[{local}] = locals[{local}].wrapping_add({value}); {}",
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
                    "let right = stack.pop().expect(\"verified JVM stack\"); let left = stack.pop().expect(\"verified JVM stack\"); stack.push({expression}); {}",
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
                    "let right = stack.pop().expect(\"verified JVM stack\"); let left = stack.pop().expect(\"verified JVM stack\"); match jars_runtime::{operation}(left, right) {{ Ok(value) => stack.push(value), Err(error) => {{ {dispatch} }} }} {}",
                    continue_at(next)?
                )
            }
            Op::INeg => format!(
                "let value = stack.pop().expect(\"verified JVM stack\"); stack.push(value.wrapping_neg()); {}",
                continue_at(next)?
            ),
            Op::Goto(delta) => format!(
                "pc = {}; continue;",
                branch_target(program, method, instruction, *delta)?
            ),
            Op::If(kind, delta) => {
                let (condition, pops) = match kind {
                    IfKind::Eq => ("value == 0", 1),
                    IfKind::Ne => ("value != 0", 1),
                    IfKind::Lt => ("value < 0", 1),
                    IfKind::Ge => ("value >= 0", 1),
                    IfKind::Gt => ("value > 0", 1),
                    IfKind::Le => ("value <= 0", 1),
                    IfKind::ICmpEq => ("left == right", 2),
                    IfKind::ICmpNe => ("left != right", 2),
                    IfKind::ICmpLt => ("left < right", 2),
                    IfKind::ICmpGe => ("left >= right", 2),
                    IfKind::ICmpGt => ("left > right", 2),
                    IfKind::ICmpLe => ("left <= right", 2),
                    IfKind::ACmpEq => ("left == right", 2),
                    IfKind::ACmpNe => ("left != right", 2),
                    IfKind::Null => ("value == 0", 1),
                    IfKind::NonNull => ("value != 0", 1),
                };
                let values = if pops == 1 {
                    "let value = stack.pop().expect(\"verified JVM stack\");"
                } else {
                    "let right = stack.pop().expect(\"verified JVM stack\"); let left = stack.pop().expect(\"verified JVM stack\");"
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
                    "let value = stack.pop().expect(\"verified JVM stack\"); pc = match value {{ {cases} _ => {default}, }}; continue;"
                )
            }
            Op::InvokeStatic(reference) => {
                let signature = parse_signature(&reference.descriptor)?;
                if signature.returns != Type::Int
                    || signature
                        .parameters
                        .iter()
                        .any(|parameter| *parameter != Type::Int)
                {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "non-int invokestatic in integer state machine",
                    ));
                }
                if !known_classes.iter().any(|class| class == &reference.class) {
                    return Err(unsupported(
                        program,
                        method,
                        instruction,
                        "invokestatic owner outside the compilation set",
                    ));
                }
                let mut arguments = Vec::with_capacity(signature.parameters.len());
                for index in (0..signature.parameters.len()).rev() {
                    arguments.push(format!(
                        "let argument{index} = stack.pop().expect(\"verified JVM stack\");"
                    ));
                }
                arguments.reverse();
                let method_path = if reference.class == program.name {
                    rust_ident(&reference.name)?
                } else {
                    format!(
                        "super::{}::{}",
                        rust_ident(&reference.class)?,
                        rust_ident(&reference.name)?,
                    )
                };
                let arguments = (0..signature.parameters.len())
                    .map(|index| format!("argument{index}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let call = if arguments.is_empty() {
                    format!("{method_path}(program).await")
                } else {
                    format!("{method_path}(program, {arguments}).await")
                };
                let dispatch = exception_dispatch(method);
                format!(
                    "{} match {call} {{ Ok(value) => stack.push(value), Err(error) => {{ {dispatch} }} }} {}",
                    arguments,
                    continue_at(next)?,
                )
            }
            Op::AThrow => {
                let dispatch = exception_dispatch(method);
                format!(
                    "let error = thrown.take().expect(\"athrow requires a throwable operand\"); {dispatch}"
                )
            }
            Op::IReturn => "return Ok(stack.pop().expect(\"verified JVM stack\"));".to_owned(),
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
    let parameters = parameters(program, method)?;
    let initial_locals = method
        .signature
        .parameters
        .iter()
        .enumerate()
        .map(|(index, _)| format!("locals[{index}] = arg{index};"))
        .collect::<String>();
    let name = rust_ident(&method.name)?;
    let entry = method
        .instructions
        .first()
        .ok_or_else(|| invalid("methods must contain at least one instruction"))?
        .offset;
    Ok(format!(
        "pub async fn {name}<S: jars_runtime::Spawner>(program: &super::Program<S>, {parameters}) -> jars_runtime::JavaResult<i32> {{\n__ensure(program).await?;\nlet mut locals = vec![0_i32; {max_local}];\n{initial_locals}\nlet mut stack: Vec<i32> = Vec::new();\nlet mut pending_exception: Option<jars_runtime::JavaError> = None;\nlet mut exception_locals: Vec<Option<jars_runtime::JavaError>> = (0..{max_local}).map(|_| None).collect();\nlet mut thrown: Option<jars_runtime::JavaError> = None;\nlet mut pc: u32 = {entry};\nloop {{ match pc {{ {} , _ => unreachable!(\"verified JVM program counter\"), }} }}\n}}",
        arms.join(",\n"),
    ))
}

fn static_method(
    program: &Program,
    known_classes: &[String],
    method: &Method,
) -> Result<String, CompileError> {
    if !method.handlers.is_empty()
        || method.instructions.iter().any(|instruction| {
            matches!(
                &instruction.op,
                Op::IInc(_, _)
                    | Op::Goto(_)
                    | Op::If(_, _)
                    | Op::LookupSwitch { .. }
                    | Op::TableSwitch { .. }
            )
        })
    {
        return aot_int_static_method(program, known_classes, method);
    }
    let name = rust_ident(&method.name)?;
    let parameters = parameters(program, method)?;
    let body = Body::new(program, known_classes, method, BodyKind::Static).run()?;
    let fallback = default_return(&method.signature.returns);
    Ok(format!(
        "pub async fn {name}<S: jars_runtime::Spawner>(program: &super::Program<S>, {parameters}) -> jars_runtime::JavaResult<{}> {{\n__ensure(program).await?;\n{body}\nOk({fallback})\n}}",
        method.signature.returns.rust(&program.name)?,
    ))
}

fn instance_method(
    program: &Program,
    known_classes: &[String],
    method: &Method,
) -> Result<(String, String, String), CompileError> {
    let name = rust_ident(&method.name)?;
    let parameters = parameters(program, method)?;
    let body = Body::new(program, known_classes, method, BodyKind::Instance).run()?;
    let fallback = default_return(&method.signature.returns);
    let implementation = format!(
        "async fn {name}_impl<S: jars_runtime::Spawner>(state: std::rc::Rc<std::sync::Mutex<{}State>>, program: &super::Program<S>, {parameters}) -> jars_runtime::JavaResult<{}> {{\n{body}\nOk({fallback})\n}}",
        rust_ident(&program.name)?,
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
        rust_ident(&program.name)?,
        reply_fields(&arguments, "reply"),
    );
    Ok((implementation, variant, proxy))
}

fn actor_code(
    program: &Program,
    known_classes: &[String],
    constructor: &Method,
    methods: &[&Method],
) -> Result<String, CompileError> {
    let class = rust_ident(&program.name)?;
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
        Body::new(program, known_classes, constructor, BodyKind::Instance).run()?;
    let constructor_args = (0..constructor.signature.parameters.len())
        .map(|index| format!("arg{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut implementations = Vec::new();
    let mut variants = vec![format!("Init {{ {constructor_message_fields} }}")];
    let mut proxies = Vec::new();
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
        Constant::String(value) => format!("{value:?}"),
    }
}

fn static_state_code(program: &Program, known_classes: &[String]) -> Result<String, CompileError> {
    let class = rust_ident(&program.name)?;
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
        // directly; emitting another async `__ensure` call would make the
        // generated future recursively sized.
        let body = Body::new(program, known_classes, method, BodyKind::Static)
            .run()?
            .replace("__ensure(program).await?;\n", "");
        format!(
            "pub(crate) async fn __clinit<S: jars_runtime::Spawner>(program: &super::Program<S>) -> jars_runtime::JavaResult<()> {{\n{body}\nOk(())\n}}"
        )
    } else {
        "pub(crate) async fn __clinit<S: jars_runtime::Spawner>(_program: &super::Program<S>) -> jars_runtime::JavaResult<()> { Ok(()) }".to_owned()
    };
    Ok(format!(
        "pub struct {class}Statics {{ pub initialized: bool, pub initializing: bool, pub failure: bool, {fields} }}\nimpl {class}Statics {{ pub fn new() -> Self {{ Self {{ initialized: false, initializing: false, failure: false, {initializers} }} }} }}\n\npub(crate) async fn __ensure<S: jars_runtime::Spawner>(program: &super::Program<S>) -> jars_runtime::JavaResult<()> {{\nlet begin = {{ let mut state = program.state.borrow_mut(); let class = &mut state.{class}; if class.failure {{ return Err(jars_runtime::class_initialization_failed(stringify!({class}))); }} if class.initialized || class.initializing {{ false }} else {{ class.initializing = true; true }} }};\nif !begin {{ return Ok(()); }}\nlet result = __clinit(program).await;\nlet mut state = program.state.borrow_mut(); let class = &mut state.{class}; class.initializing = false; match result {{ Ok(()) => {{ class.initialized = true; Ok(()) }}, Err(error) => {{ class.failure = true; Err(error) }} }}\n}}\n{clinit}"
    ))
}

fn program_code(programs: &[Program]) -> Result<String, CompileError> {
    let declarations = programs
        .iter()
        .map(|program| {
            let class = rust_ident(&program.name)?;
            Ok(format!("{}: {class}::{class}Statics", class))
        })
        .collect::<Result<Vec<_>, CompileError>>()?
        .join(", ");
    let initializers = programs
        .iter()
        .map(|program| {
            let class = rust_ident(&program.name)?;
            Ok(format!("{class}: {class}::{class}Statics::new()"))
        })
        .collect::<Result<Vec<_>, CompileError>>()?
        .join(", ");
    Ok(format!(
        "struct ProgramState {{ {declarations} }}\n#[derive(Clone)]\npub struct Program<S: jars_runtime::Spawner> {{ pub(crate) spawner: S, pub(crate) state: std::rc::Rc<std::cell::RefCell<ProgramState>> }}\nimpl<S: jars_runtime::Spawner> Program<S> {{ pub fn new(spawner: S) -> Self {{ Self {{ spawner, state: std::rc::Rc::new(std::cell::RefCell::new(ProgramState {{ {initializers} }})) }} }} }}"
    ))
}

fn render_module(program: &Program, known_classes: &[String]) -> Result<String, CompileError> {
    let class = rust_ident(&program.name)?;
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
        .filter(|method| method.is_static && method.is_public)
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
    let class = rust_ident(&entry.name)?;
    let source = format!(
        "{}\n{program}\nfn main() {{\nlet runtime = jars_runtime::Runtime::new();\nlet program = Program::new(runtime.clone());\nlet values: Vec<String> = std::env::args().skip(1).collect();\nruntime.block_on(async {{\nlet args = jars_runtime::JavaArray::new(runtime.clone(), values.len() as i32, String::new())?;\nfor (index, value) in values.into_iter().enumerate() {{ args.set(index as i32, value).await?; }}\n{class}::main(&program, Some(args)).await\n}}).expect(\"Java actor call failed\");\n}}",
        modules.join("\n"),
    );
    let file = syn::parse_file(&source)
        .map_err(|error| invalid(format!("internal generated Rust was invalid: {error}")))?;
    Ok(prettyplease::unparse(&file))
}

fn validate_reference_type(ty: &Type, known_classes: &[String]) -> Result<(), CompileError> {
    if let Type::Class(class) = ty {
        if !known_classes.iter().any(|known| known == class) {
            return Err(invalid(format!(
                "reference type `{class}` is not in the compilation set"
            )));
        }
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
                    Op::GetStatic(reference) if reference.class != "java/lang/System" => {
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

/// Compiles one default-package Java class file into a complete Rust source file.
pub fn compile_class(bytes: &[u8]) -> Result<String, CompileError> {
    compile_classes(&[bytes])
}

/// Compiles a closed, default-package class set into one AOT Rust source file.
///
/// References to classes outside `classes` are rejected during code generation;
/// the generated modules call each other directly and retain no class-file data.
pub fn compile_classes(classes: &[&[u8]]) -> Result<String, CompileError> {
    if classes.is_empty() {
        return Err(invalid("the compilation set is empty"));
    }
    let programs = classes
        .iter()
        .map(|bytes| parse_program(bytes))
        .collect::<Result<Vec<_>, _>>()?;
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
    validate_hierarchy(&programs)?;
    validate_reference_types(&programs)?;
    validate_field_accesses(&programs)?;
    validate_exception_tables(&programs)?;
    render(&programs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_accept_numeric_and_recursive_array_types() {
        assert!(parse_signature("(JFD)V").is_ok());
        assert!(parse_signature("(Z[[I[[[Ljava/lang/String;)V").is_ok());
        assert!(parse_signature("(I").is_err());
    }

    #[test]
    fn rust_keywords_are_escaped() {
        assert_eq!(rust_ident("type").unwrap(), "r#type");
    }
}
