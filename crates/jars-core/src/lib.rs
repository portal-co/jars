//! A deliberately small Java class-file to actor-oriented Rust compiler.
//!
//! The public surface is intentionally narrow: `compile_class` accepts one
//! default-package class and emits a complete Rust binary which depends on
//! `jars-runtime`.

use std::fmt;

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
    Int,
    Void,
    String,
    StringArray,
}

impl Type {
    fn rust(&self) -> &'static str {
        match self {
            Self::Int => "i32",
            Self::Void => "()",
            Self::String => "&'static str",
            Self::StringArray => "Vec<String>",
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
    ILoad(usize),
    IStore(usize),
    ALoad(usize),
    AStore(usize),
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
    IInc(usize, i32),
    Goto(i32),
    If(IfKind, i32),
    IReturn,
    Return,
    LdcString(String),
    GetStatic(MemberRef),
    GetField(MemberRef),
    PutField(MemberRef),
    New(String),
    Dup,
    InvokeSpecial(MemberRef),
    InvokeStatic(MemberRef),
    InvokeVirtual(MemberRef),
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
}

#[derive(Clone, Debug)]
struct Instruction {
    offset: u32,
    op: Op,
}

#[derive(Clone, Debug)]
struct Method {
    name: String,
    signature: Signature,
    is_static: bool,
    is_public: bool,
    instructions: Vec<Instruction>,
}

#[derive(Clone, Debug)]
struct Field {
    name: String,
    ty: Type,
}

#[derive(Clone, Debug)]
struct Program {
    name: String,
    fields: Vec<Field>,
    methods: Vec<Method>,
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
    if remainder.starts_with('I') {
        *cursor += 1;
        Ok(Type::Int)
    } else if remainder.starts_with('V') {
        *cursor += 1;
        Ok(Type::Void)
    } else if remainder.starts_with("Ljava/lang/String;") {
        *cursor += "Ljava/lang/String;".len();
        Ok(Type::String)
    } else if remainder.starts_with("[Ljava/lang/String;") {
        *cursor += "[Ljava/lang/String;".len();
        Ok(Type::StringArray)
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

fn parse_op(pool: &cpool::ConstantPool<'_>, raw: RawInstruction<'_>) -> Result<Op, CompileError> {
    use RawInstruction::*;
    let op = match raw {
        IConstM1 => Op::IConst(-1),
        IConst0 => Op::IConst(0),
        IConst1 => Op::IConst(1),
        IConst2 => Op::IConst(2),
        IConst3 => Op::IConst(3),
        IConst4 => Op::IConst(4),
        IConst5 => Op::IConst(5),
        BIPush { value } => Op::IConst(value.into()),
        SIPush { value } => Op::IConst(value.into()),
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
        AStore { index } => Op::AStore(index.into()),
        AStoreW { index } => Op::AStore(index.into()),
        AStore0 => Op::AStore(0),
        AStore1 => Op::AStore(1),
        AStore2 => Op::AStore(2),
        AStore3 => Op::AStore(3),
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
        IReturn => Op::IReturn,
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
            other => return Err(invalid(format!("unsupported ldc constant {other:?}"))),
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

    let mut fields = Vec::new();
    for field in class.fields() {
        let field = field.map_err(|error| CompileError::Parse(error.to_string()))?;
        if field.access_flags().contains(AccessFlags::STATIC) {
            return Err(invalid("static fields are not supported in v1"));
        }
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
        if ty != Type::Int || cursor != descriptor.len() {
            return Err(invalid("v1 instance fields must have type int"));
        }
        fields.push(Field { name, ty });
    }

    let mut methods = Vec::new();
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
        for attribute in method.attributes() {
            let attribute = attribute.map_err(|error| CompileError::Parse(error.to_string()))?;
            if let AttributeContent::Code(code) = attribute
                .read_content(pool)
                .map_err(|error| CompileError::Parse(error.to_string()))?
            {
                let code = code;
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
        if name != "<clinit>" {
            methods.push(Method {
                name,
                signature,
                is_static: method.access_flags().contains(AccessFlags::STATIC),
                is_public: method.access_flags().contains(AccessFlags::PUBLIC),
                instructions: instructions
                    .ok_or_else(|| invalid("methods must have Code attributes"))?,
            });
        }
    }
    Ok(Program {
        name: program_name,
        fields,
        methods,
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
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
        | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop"
        | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "Self" | "self"
        | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe" | "use"
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
    String(String),
    PrintStream,
    This,
    Object(String),
    Uninitialized(usize),
}

impl Value {
    fn expression(&self) -> Option<&str> {
        match self {
            Self::Int(value) | Self::String(value) | Self::Object(value) => Some(value),
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
    kind: BodyKind,
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
        for (index, ty) in method.signature.parameters.iter().enumerate() {
            let value = match ty {
                Type::Int => Value::Int(format!("arg{index}")),
                Type::String => Value::String(format!("arg{index}")),
                Type::StringArray => Value::Object(format!("arg{index}")),
                Type::Void => unreachable!(),
            };
            locals.push(Some(value));
        }
        Self {
            program,
            known_classes,
            method,
            kind,
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

    fn temp(&mut self, expression: String, value: fn(String) -> Value) -> Value {
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

    fn run(mut self) -> Result<String, CompileError> {
        for instruction in &self.method.instructions {
            match &instruction.op {
                Op::IConst(value) => self.stack.push(Value::Int(value.to_string())),
                Op::LdcString(value) => self.stack.push(Value::String(format!("{value:?}"))),
                Op::ILoad(index) | Op::ALoad(index) => {
                    self.stack.push(self.local(*index, instruction)?)
                }
                Op::IStore(index) | Op::AStore(index) => {
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
                            Value::String(_) => Value::String(local),
                            Value::Object(_) => Value::Object(local),
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
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "getstatic",
                        ));
                    }
                }
                Op::GetField(reference) => {
                    if !matches!(self.kind, BodyKind::Instance)
                        || reference.class != self.program.name
                        || reference.descriptor != "I"
                    {
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "getfield",
                        ));
                    }
                    if !matches!(self.pop(instruction)?, Value::This) {
                        return Err(stack_error(
                            self.program,
                            self.method,
                            instruction,
                            "getfield receiver must be this",
                        ));
                    }
                    let field = rust_ident(&reference.name)?;
                    let value = self.temp(format!("state.{field}"), Value::Int);
                    self.stack.push(value);
                }
                Op::PutField(reference) => {
                    if !matches!(self.kind, BodyKind::Instance)
                        || reference.class != self.program.name
                        || reference.descriptor != "I"
                    {
                        return Err(unsupported(
                            self.program,
                            self.method,
                            instruction,
                            "putfield",
                        ));
                    }
                    let value = self.pop_expression(instruction)?;
                    if !matches!(self.pop(instruction)?, Value::This) {
                        return Err(stack_error(
                            self.program,
                            self.method,
                            instruction,
                            "putfield receiver must be this",
                        ));
                    }
                    let field = rust_ident(&reference.name)?;
                    self.statements.push(format!("state.{field} = {value};"));
                }
                Op::New(class) => {
                    if !matches!(self.kind, BodyKind::Static)
                        || !self.known_classes.iter().any(|known| known == class)
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
                        let call_arguments = if args.is_empty() {
                            "spawner.clone()".to_owned()
                        } else {
                            format!("spawner.clone(), {}", args.join(", "))
                        };
                        self.statements.push(format!(
                            "let {variable} = {class}::new({call_arguments}).await?;",
                        ));
                        for value in &mut self.stack {
                            if matches!(value, Value::Uninitialized(other) if *other == id) {
                                *value = Value::Object(variable.clone());
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
                    if !matches!(self.kind, BodyKind::Static)
                        || !self
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
                    let expression =
                        format!("{method}(spawner.clone(), {}).await?", args.join(", "));
                    match signature.returns {
                        Type::Int => {
                            let value = self.temp(expression, Value::Int);
                            self.stack.push(value);
                        }
                        Type::String => {
                            let value = self.temp(expression, Value::String);
                            self.stack.push(value);
                        }
                        Type::Void => self.statements.push(format!("{expression};")),
                        Type::StringArray => {
                            return Err(unsupported(
                                self.program,
                                self.method,
                                instruction,
                                "array return",
                            ));
                        }
                    }
                }
                Op::InvokeVirtual(reference) => {
                    let signature = parse_signature(&reference.descriptor)?;
                    let args = self.arguments(instruction, &signature)?;
                    let receiver = self.pop(instruction)?;
                    if reference.class == "java/io/PrintStream" && reference.name == "println" {
                        if !matches!(receiver, Value::PrintStream)
                            || args.len() != 1
                            || !matches!(signature.returns, Type::Void)
                            || !matches!(
                                signature.parameters.as_slice(),
                                [Type::Int] | [Type::String]
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
                    } else if matches!(self.kind, BodyKind::Static)
                        && self
                            .known_classes
                            .iter()
                            .any(|known| known == &reference.class)
                    {
                        let object = match receiver {
                            Value::Object(value) => value,
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
                        let expression = format!("{object}.{method}({}).await?", args.join(", "));
                        match signature.returns {
                            Type::Int => {
                                let value = self.temp(expression, Value::Int);
                                self.stack.push(value);
                            }
                            Type::String => {
                                let value = self.temp(expression, Value::String);
                                self.stack.push(value);
                            }
                            Type::Void => self.statements.push(format!("{expression};")),
                            Type::StringArray => {
                                return Err(unsupported(
                                    self.program,
                                    self.method,
                                    instruction,
                                    "array return",
                                ));
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
                    match self.kind {
                        BodyKind::Static => self.statements.push(format!("return Ok({value});")),
                        BodyKind::Instance => self.statements.push(format!("return {value};")),
                    }
                }
                Op::Return => match self.kind {
                    BodyKind::Static => self.statements.push("return Ok(());".to_owned()),
                    BodyKind::Instance => self.statements.push("return ();".to_owned()),
                },
                Op::IInc(_, _) | Op::Goto(_) | Op::If(_, _) => {
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

fn parameters(method: &Method) -> String {
    method
        .signature
        .parameters
        .iter()
        .enumerate()
        .map(|(index, ty)| format!("arg{index}: {}", ty.rust()))
        .collect::<Vec<_>>()
        .join(", ")
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
        Type::Int => "0",
        Type::Void => "()",
        Type::String => "\"\"",
        Type::StringArray => "Vec::new()",
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

/// Emits an AOT state machine for integer-only static methods with branches.
///
/// There is no bytecode representation at run time: every bytecode instruction
/// becomes one Rust `match` arm.  `pc`, locals and the operand stack are merely
/// the activation record required to preserve JVM ordering across back-edges.
fn aot_int_static_method(program: &Program, method: &Method) -> Result<String, CompileError> {
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
            Op::ILoad(index) | Op::IStore(index) | Op::IInc(index, _) => Some(*index),
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
                format!(
                    "let right = stack.pop().expect(\"verified JVM stack\"); let left = stack.pop().expect(\"verified JVM stack\"); stack.push(jars_runtime::{operation}(left, right)?); {}",
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
    let parameters = parameters(method);
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
        "pub async fn {name}<S: jars_runtime::Spawner>(_spawner: S, {parameters}) -> Result<i32, jars_runtime::JavaError> {{\nlet mut locals = vec![0_i32; {max_local}];\n{initial_locals}\nlet mut stack: Vec<i32> = Vec::new();\nlet mut pc: u32 = {entry};\nloop {{ match pc {{ {} , _ => unreachable!(\"verified JVM program counter\"), }} }}\n}}",
        arms.join(",\n"),
    ))
}

fn static_method(
    program: &Program,
    known_classes: &[String],
    method: &Method,
) -> Result<String, CompileError> {
    if method
        .instructions
        .iter()
        .any(|instruction| matches!(&instruction.op, Op::IInc(_, _) | Op::Goto(_) | Op::If(_, _)))
    {
        return aot_int_static_method(program, method);
    }
    let name = rust_ident(&method.name)?;
    let parameters = parameters(method);
    let body = Body::new(program, known_classes, method, BodyKind::Static).run()?;
    let fallback = default_return(&method.signature.returns);
    Ok(format!(
        "pub async fn {name}<S: jars_runtime::Spawner>(spawner: S, {parameters}) -> Result<{}, jars_runtime::JavaError> {{\n{body}\nOk({fallback})\n}}",
        method.signature.returns.rust(),
    ))
}

fn instance_method(
    program: &Program,
    known_classes: &[String],
    method: &Method,
) -> Result<(String, String, String), CompileError> {
    let name = rust_ident(&method.name)?;
    let parameters = parameters(method);
    let body = Body::new(program, known_classes, method, BodyKind::Instance).run()?;
    let fallback = default_return(&method.signature.returns);
    let implementation = format!(
        "fn {name}_impl(state: &mut {}State, {parameters}) -> {} {{\n{body}\n{fallback}\n}}",
        rust_ident(&program.name)?,
        method.signature.returns.rust(),
    );
    let variant = format!(
        "{name} {{ {} }}",
        reply_fields(
            &parameters,
            &format!(
                "reply: jars_runtime::Reply<Result<{}, jars_runtime::JavaError>>",
                method.signature.returns.rust(),
            ),
        ),
    );
    let arguments = (0..method.signature.parameters.len())
        .map(|index| format!("arg{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let proxy = format!(
        "pub async fn {name}(&self{}) -> Result<{}, jars_runtime::JavaError> {{\nlet (reply, response) = jars_runtime::reply();\nself.actor.send({}Message::{name} {{ {} }}).await?;\nresponse.recv().await?\n}}",
        if parameters.is_empty() {
            String::new()
        } else {
            format!(", {parameters}")
        },
        method.signature.returns.rust(),
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
        .map(|field| Ok(format!("{}: {}", rust_ident(&field.name)?, field.ty.rust())))
        .collect::<Result<Vec<_>, CompileError>>()
        .map(|fields| fields.join(", "))?;
    let field_initializers = program
        .fields
        .iter()
        .map(|field| Ok(format!("{}: 0", rust_ident(&field.name)?)))
        .collect::<Result<Vec<_>, CompileError>>()
        .map(|fields| fields.join(", "))?;
    let constructor_parameters = parameters(constructor);
    let constructor_message_fields = reply_fields(
        &constructor_parameters,
        "reply: jars_runtime::Reply<Result<(), jars_runtime::JavaError>>",
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
        "{class}Message::Init {{ {} }} => {{ {class}::init_impl(&mut state{}); let _ = reply.send(Ok(())); }}",
        reply_fields(&constructor_args, "reply"),
        if constructor_args.is_empty() {
            String::new()
        } else {
            format!(", {constructor_args}")
        },
    )];
    implementations.push(format!(
        "fn init_impl(state: &mut {class}State, {constructor_parameters}) {{\n{constructor_body}\n}}"
    ));
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
            "{class}Message::{name} {{ {} }} => {{ let value = {class}::{name}_impl(&mut state{}); let _ = reply.send(Ok(value)); }}",
            reply_fields(&args, "reply"),
            if args.is_empty() { String::new() } else { format!(", {args}") },
        ));
    }
    Ok(format!(
        "struct {class}State {{ {field_declarations} }}\nenum {class}Message {{ {} }}\n#[derive(Clone)]\npub struct {class} {{ actor: jars_runtime::ActorRef<{class}Message> }}\nimpl {class} {{\npub async fn new<S: jars_runtime::Spawner>(spawner: S{}) -> Result<Self, jars_runtime::JavaError> {{\nlet (actor, mailbox) = jars_runtime::actor_channel();\nspawner.spawn(async move {{\nlet mut state = {class}State {{ {field_initializers} }};\nwhile let Ok(message) = mailbox.recv().await {{ match message {{ {} }} }}\n}});\nlet (reply, response) = jars_runtime::reply();\nactor.send({class}Message::Init {{ {} }}).await?;\nresponse.recv().await??;\nOk(Self {{ actor }})\n}}\n{}\n{}\n}}",
        variants.join(", "),
        if constructor_parameters.is_empty() {
            String::new()
        } else {
            format!(", {constructor_parameters}")
        },
        dispatch.join(", "),
        reply_fields(&constructor_args, "reply"),
        implementations.join("\n"),
        proxies.join("\n"),
    ))
}

fn render_module(program: &Program, known_classes: &[String]) -> Result<String, CompileError> {
    let class = rust_ident(&program.name)?;
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
    let actor_needed = !instances.is_empty() || !program.fields.is_empty();
    let actor = if actor_needed {
        let constructor = constructors
            .first()
            .ok_or_else(|| invalid("instance methods and fields require a constructor"))?;
        actor_code(program, known_classes, constructor, &instances)?
    } else {
        String::new()
    };
    Ok(format!(
        "pub mod {class} {{\n{actor}\n{}\n}}",
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
                    && method.signature.parameters == [Type::StringArray]
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
    let class = rust_ident(&entry.name)?;
    let source = format!(
        "{}\nfn main() {{\nlet runtime = jars_runtime::Runtime::new();\nlet args = std::env::args().skip(1).collect();\nruntime.block_on({class}::main(runtime.clone(), args)).expect(\"Java actor call failed\");\n}}",
        modules.join("\n"),
    );
    let file = syn::parse_file(&source)
        .map_err(|error| invalid(format!("internal generated Rust was invalid: {error}")))?;
    Ok(prettyplease::unparse(&file))
}

/// Compiles one default-package Java class file into a complete Rust source file.
pub fn compile_class(bytes: &[u8]) -> Result<String, CompileError> {
    render(&[parse_program(bytes)?])
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
    render(&programs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_reject_unimplemented_types() {
        assert!(parse_signature("(J)V").is_err());
        assert!(parse_signature("(I").is_err());
    }

    #[test]
    fn rust_keywords_are_escaped() {
        assert_eq!(rust_ident("type").unwrap(), "r#type");
    }
}
