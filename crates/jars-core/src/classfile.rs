//! A deliberately small, owned reader for the Java SE 26 class-file format.
//!
//! The compiler only lowers a subset of the JVM, but its import reader must be
//! able to walk current class files without relying on the bytecode support of
//! a third-party crate.  Attributes that do not affect lowering are retained
//! as opaque, validated byte ranges.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Version {
    pub(crate) major: u16,
    pub(crate) minor: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct ClassFile {
    #[allow(dead_code)]
    pub(crate) version: Version,
    pub(crate) constant_pool: ConstantPool,
    pub(crate) access_flags: u16,
    pub(crate) this_class: u16,
    pub(crate) super_class: u16,
    pub(crate) interfaces: Vec<u16>,
    pub(crate) fields: Vec<MemberInfo>,
    pub(crate) methods: Vec<MemberInfo>,
    #[allow(dead_code)]
    pub(crate) attributes: Vec<Attribute>,
}

#[derive(Clone, Debug)]
pub(crate) struct MemberInfo {
    pub(crate) access_flags: u16,
    pub(crate) name_index: u16,
    pub(crate) descriptor_index: u16,
    pub(crate) attributes: Vec<Attribute>,
}

#[derive(Clone, Debug)]
pub(crate) struct Attribute {
    pub(crate) name: String,
    pub(crate) info: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct Code {
    pub(crate) code: Vec<u8>,
    pub(crate) exception_handlers: Vec<ExceptionHandler>,
}

#[derive(Clone, Debug)]
pub(crate) struct ExceptionHandler {
    pub(crate) start: u16,
    pub(crate) end: u16,
    pub(crate) handler: u16,
    pub(crate) catch_type: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct ConstantPool {
    entries: Vec<Option<CpEntry>>,
}

#[derive(Clone, Debug)]
enum CpEntry {
    Utf8(String),
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    Class(u16),
    String(u16),
    FieldRef { class: u16, name_and_type: u16 },
    MethodRef { class: u16, name_and_type: u16 },
    InterfaceMethodRef { class: u16, name_and_type: u16 },
    NameAndType { name: u16, descriptor: u16 },
    MethodHandle { kind: u8, reference: u16 },
    MethodType(u16),
    Dynamic { bootstrap: u16, name_and_type: u16 },
    InvokeDynamic { bootstrap: u16, name_and_type: u16 },
    Module(u16),
    Package(u16),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ConstantValue<'a> {
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    String(&'a str),
}

#[derive(Clone, Debug)]
pub(crate) struct MemberRef {
    pub(crate) class: String,
    pub(crate) name: String,
    pub(crate) descriptor: String,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ArrayType {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum IfKind {
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

/// Canonical bytecode forms used by the compiler.  Short local forms and
/// `wide` variants are normalised while decoding.
#[derive(Clone, Debug)]
pub(crate) enum RawInstruction {
    AConstNull,
    ANewArray {
        index: u16,
    },
    NewArray {
        atype: ArrayType,
    },
    MultiANewArray {
        index: u16,
        dimensions: u8,
    },
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
    IConst(i32),
    LConst(i64),
    FConst(f32),
    DConst(f64),
    ILoad {
        index: u16,
    },
    IStore {
        index: u16,
    },
    ALoad {
        index: u16,
    },
    AStore {
        index: u16,
    },
    LLoad {
        index: u16,
    },
    LStore {
        index: u16,
    },
    FLoad {
        index: u16,
    },
    FStore {
        index: u16,
    },
    DLoad {
        index: u16,
    },
    DStore {
        index: u16,
    },
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
    IInc {
        index: u16,
        value: i16,
    },
    Goto {
        offset: i32,
    },
    If {
        kind: IfKind,
        offset: i32,
    },
    LookupSwitch {
        default: i32,
        cases: Vec<(i32, i32)>,
    },
    TableSwitch {
        default: i32,
        cases: Vec<(i32, i32)>,
    },
    IReturn,
    LReturn,
    FReturn,
    DReturn,
    AReturn,
    Return,
    Ldc {
        index: u16,
    },
    Ldc2 {
        index: u16,
    },
    GetStatic {
        index: u16,
    },
    GetField {
        index: u16,
    },
    PutField {
        index: u16,
    },
    PutStatic {
        index: u16,
    },
    New {
        index: u16,
    },
    Dup,
    Pop,
    InvokeSpecial {
        index: u16,
    },
    InvokeStatic {
        index: u16,
    },
    InvokeVirtual {
        index: u16,
    },
    InvokeInterface {
        index: u16,
    },
    Unsupported {
        opcode: u8,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| Error::new("class file length overflow"))?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| Error::new("truncated class file"))?;
        self.position = end;
        Ok(result)
    }

    fn u1(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn u2(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("two bytes"),
        ))
    }

    fn u4(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }

    fn i1(&mut self) -> Result<i8, Error> {
        Ok(self.u1()? as i8)
    }

    fn i2(&mut self) -> Result<i16, Error> {
        Ok(self.u2()? as i16)
    }

    fn i4(&mut self) -> Result<i32, Error> {
        Ok(self.u4()? as i32)
    }

    fn finish(&self, context: &str) -> Result<(), Error> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::new(format!("trailing bytes in {context}")))
        }
    }
}

fn modified_utf8(bytes: &[u8]) -> Result<String, Error> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        let first = bytes[position];
        position += 1;
        let unit = match first {
            0x01..=0x7f => u16::from(first),
            0xc0..=0xdf => {
                let second = *bytes
                    .get(position)
                    .ok_or_else(|| Error::new("truncated modified UTF-8 sequence"))?;
                position += 1;
                if second & 0xc0 != 0x80 {
                    return Err(Error::new("invalid modified UTF-8 continuation byte"));
                }
                let value = (u16::from(first & 0x1f) << 6) | u16::from(second & 0x3f);
                if value == 0 || value >= 0x80 {
                    value
                } else {
                    return Err(Error::new("overlong modified UTF-8 sequence"));
                }
            }
            0xe0..=0xef => {
                let second = *bytes
                    .get(position)
                    .ok_or_else(|| Error::new("truncated modified UTF-8 sequence"))?;
                let third = *bytes
                    .get(position + 1)
                    .ok_or_else(|| Error::new("truncated modified UTF-8 sequence"))?;
                position += 2;
                if second & 0xc0 != 0x80 || third & 0xc0 != 0x80 {
                    return Err(Error::new("invalid modified UTF-8 continuation byte"));
                }
                let value = (u16::from(first & 0x0f) << 12)
                    | (u16::from(second & 0x3f) << 6)
                    | u16::from(third & 0x3f);
                if value >= 0x800 {
                    value
                } else {
                    return Err(Error::new("overlong modified UTF-8 sequence"));
                }
            }
            _ => return Err(Error::new("invalid modified UTF-8 leading byte")),
        };
        units.push(unit);
    }
    String::from_utf16(&units).map_err(|_| Error::new("invalid modified UTF-8 UTF-16 sequence"))
}

impl ConstantPool {
    fn entry(&self, index: u16) -> Result<&CpEntry, Error> {
        self.entries
            .get(usize::from(index))
            .and_then(Option::as_ref)
            .ok_or_else(|| Error::new(format!("invalid constant-pool index {index}")))
    }

    pub(crate) fn utf8(&self, index: u16) -> Result<&str, Error> {
        match self.entry(index)? {
            CpEntry::Utf8(value) => Ok(value),
            _ => Err(Error::new(format!(
                "constant-pool index {index} is not UTF-8"
            ))),
        }
    }

    pub(crate) fn class_name(&self, index: u16) -> Result<String, Error> {
        match self.entry(index)? {
            CpEntry::Class(name) => Ok(self.utf8(*name)?.to_owned()),
            _ => Err(Error::new(format!(
                "constant-pool index {index} is not a class"
            ))),
        }
    }

    fn name_and_type(&self, index: u16) -> Result<(&str, &str), Error> {
        match self.entry(index)? {
            CpEntry::NameAndType { name, descriptor } => {
                Ok((self.utf8(*name)?, self.utf8(*descriptor)?))
            }
            _ => Err(Error::new(format!(
                "constant-pool index {index} is not a name-and-type"
            ))),
        }
    }

    /// Resolves either constant-pool member-reference kind. `invokestatic`
    /// may legally target an `InterfaceMethodref` when the owner is an
    /// interface, so both must resolve through this shared path.
    pub(crate) fn method_ref(&self, index: u16) -> Result<MemberRef, Error> {
        let (class, name_and_type) = match self.entry(index)? {
            CpEntry::MethodRef {
                class,
                name_and_type,
            }
            | CpEntry::InterfaceMethodRef {
                class,
                name_and_type,
            } => (*class, *name_and_type),
            _ => {
                return Err(Error::new(format!(
                    "constant-pool index {index} is not a method reference"
                )));
            }
        };
        let (name, descriptor) = self.name_and_type(name_and_type)?;
        Ok(MemberRef {
            class: self.class_name(class)?,
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
        })
    }

    pub(crate) fn field_ref(&self, index: u16) -> Result<MemberRef, Error> {
        let (class, name_and_type) = match self.entry(index)? {
            CpEntry::FieldRef {
                class,
                name_and_type,
            } => (*class, *name_and_type),
            _ => {
                return Err(Error::new(format!(
                    "constant-pool index {index} is not a field reference"
                )));
            }
        };
        let (name, descriptor) = self.name_and_type(name_and_type)?;
        Ok(MemberRef {
            class: self.class_name(class)?,
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
        })
    }

    pub(crate) fn interface_method_ref(&self, index: u16) -> Result<MemberRef, Error> {
        let (class, name_and_type) = match self.entry(index)? {
            CpEntry::InterfaceMethodRef {
                class,
                name_and_type,
            } => (*class, *name_and_type),
            _ => {
                return Err(Error::new(format!(
                    "constant-pool index {index} is not an interface method reference"
                )));
            }
        };
        let (name, descriptor) = self.name_and_type(name_and_type)?;
        Ok(MemberRef {
            class: self.class_name(class)?,
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
        })
    }

    pub(crate) fn constant(&self, index: u16) -> Result<ConstantValue<'_>, Error> {
        match self.entry(index)? {
            CpEntry::Integer(value) => Ok(ConstantValue::Integer(*value)),
            CpEntry::Float(value) => Ok(ConstantValue::Float(*value)),
            CpEntry::Long(value) => Ok(ConstantValue::Long(*value)),
            CpEntry::Double(value) => Ok(ConstantValue::Double(*value)),
            CpEntry::String(value) => Ok(ConstantValue::String(self.utf8(*value)?)),
            _ => Err(Error::new(format!(
                "constant-pool index {index} is not a supported literal"
            ))),
        }
    }

    fn validate(&self) -> Result<(), Error> {
        for (index, entry) in self.entries.iter().enumerate().skip(1) {
            let index = u16::try_from(index).expect("constant pool is u16 sized");
            let Some(entry) = entry else {
                continue;
            };
            match entry {
                CpEntry::Utf8(_)
                | CpEntry::Integer(_)
                | CpEntry::Float(_)
                | CpEntry::Long(_)
                | CpEntry::Double(_) => {}
                CpEntry::Class(name)
                | CpEntry::String(name)
                | CpEntry::MethodType(name)
                | CpEntry::Module(name)
                | CpEntry::Package(name) => {
                    self.utf8(*name)?;
                }
                CpEntry::FieldRef {
                    class,
                    name_and_type,
                }
                | CpEntry::MethodRef {
                    class,
                    name_and_type,
                }
                | CpEntry::InterfaceMethodRef {
                    class,
                    name_and_type,
                } => {
                    self.class_name(*class)?;
                    self.name_and_type(*name_and_type)?;
                }
                CpEntry::NameAndType { name, descriptor } => {
                    self.utf8(*name)?;
                    self.utf8(*descriptor)?;
                }
                CpEntry::MethodHandle { kind, reference } => {
                    if !(1..=9).contains(kind) {
                        return Err(Error::new(format!("invalid method-handle kind {kind}")));
                    }
                    self.entry(*reference)?;
                }
                CpEntry::Dynamic {
                    bootstrap,
                    name_and_type,
                }
                | CpEntry::InvokeDynamic {
                    bootstrap,
                    name_and_type,
                } => {
                    let _ = bootstrap;
                    self.name_and_type(*name_and_type)?;
                }
            }
            let _ = index;
        }
        Ok(())
    }
}

fn read_constant_pool(reader: &mut Reader<'_>) -> Result<ConstantPool, Error> {
    let count = reader.u2()?;
    if count == 0 {
        return Err(Error::new("constant_pool_count must be non-zero"));
    }
    let mut entries = Vec::with_capacity(usize::from(count));
    entries.push(None);
    let mut index = 1;
    while index < count {
        let entry = match reader.u1()? {
            1 => {
                let length = usize::from(reader.u2()?);
                CpEntry::Utf8(modified_utf8(reader.take(length)?)?)
            }
            3 => CpEntry::Integer(reader.i4()?),
            4 => CpEntry::Float(f32::from_bits(reader.u4()?)),
            5 => {
                let high = u64::from(reader.u4()?);
                let low = u64::from(reader.u4()?);
                if index + 1 >= count {
                    return Err(Error::new(
                        "long constant is missing its reserved pool slot",
                    ));
                }
                entries.push(Some(CpEntry::Long(((high << 32) | low) as i64)));
                entries.push(None);
                index += 2;
                continue;
            }
            6 => {
                let high = u64::from(reader.u4()?);
                let low = u64::from(reader.u4()?);
                if index + 1 >= count {
                    return Err(Error::new(
                        "double constant is missing its reserved pool slot",
                    ));
                }
                entries.push(Some(CpEntry::Double(f64::from_bits((high << 32) | low))));
                entries.push(None);
                index += 2;
                continue;
            }
            7 => CpEntry::Class(reader.u2()?),
            8 => CpEntry::String(reader.u2()?),
            9 => CpEntry::FieldRef {
                class: reader.u2()?,
                name_and_type: reader.u2()?,
            },
            10 => CpEntry::MethodRef {
                class: reader.u2()?,
                name_and_type: reader.u2()?,
            },
            11 => CpEntry::InterfaceMethodRef {
                class: reader.u2()?,
                name_and_type: reader.u2()?,
            },
            12 => CpEntry::NameAndType {
                name: reader.u2()?,
                descriptor: reader.u2()?,
            },
            15 => CpEntry::MethodHandle {
                kind: reader.u1()?,
                reference: reader.u2()?,
            },
            16 => CpEntry::MethodType(reader.u2()?),
            17 => CpEntry::Dynamic {
                bootstrap: reader.u2()?,
                name_and_type: reader.u2()?,
            },
            18 => CpEntry::InvokeDynamic {
                bootstrap: reader.u2()?,
                name_and_type: reader.u2()?,
            },
            19 => CpEntry::Module(reader.u2()?),
            20 => CpEntry::Package(reader.u2()?),
            tag => return Err(Error::new(format!("unknown constant-pool tag {tag}"))),
        };
        entries.push(Some(entry));
        index += 1;
    }
    let pool = ConstantPool { entries };
    pool.validate()?;
    Ok(pool)
}

fn read_attributes(reader: &mut Reader<'_>, pool: &ConstantPool) -> Result<Vec<Attribute>, Error> {
    let count = reader.u2()?;
    let mut attributes = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let name = pool.utf8(reader.u2()?)?.to_owned();
        let length =
            usize::try_from(reader.u4()?).map_err(|_| Error::new("attribute is too large"))?;
        attributes.push(Attribute {
            name,
            info: reader.take(length)?.to_vec(),
        });
    }
    Ok(attributes)
}

fn read_members(reader: &mut Reader<'_>, pool: &ConstantPool) -> Result<Vec<MemberInfo>, Error> {
    let count = reader.u2()?;
    let mut members = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let access_flags = reader.u2()?;
        let name_index = reader.u2()?;
        let descriptor_index = reader.u2()?;
        pool.utf8(name_index)?;
        pool.utf8(descriptor_index)?;
        let attributes = read_attributes(reader, pool)?;
        members.push(MemberInfo {
            access_flags,
            name_index,
            descriptor_index,
            attributes,
        });
    }
    Ok(members)
}

/// Parses and validates a class file according to the Java SE 26 structural
/// rules used by this compiler. Preview class files are intentionally outside
/// the compiler contract.
pub(crate) fn parse(bytes: &[u8]) -> Result<ClassFile, Error> {
    let mut reader = Reader::new(bytes);
    if reader.u4()? != 0xcafe_babe {
        return Err(Error::new("invalid class-file magic"));
    }
    let version = Version {
        minor: reader.u2()?,
        major: reader.u2()?,
    };
    if !(45..=70).contains(&version.major) {
        return Err(Error::new(format!(
            "unsupported class-file version {}.{}",
            version.major, version.minor
        )));
    }
    if version.major >= 56 && version.minor != 0 {
        return Err(Error::new(format!(
            "unsupported class-file version {}.{}",
            version.major, version.minor
        )));
    }
    let constant_pool = read_constant_pool(&mut reader)?;
    let access_flags = reader.u2()?;
    let this_class = reader.u2()?;
    let super_class = reader.u2()?;
    constant_pool.class_name(this_class)?;
    if super_class != 0 {
        constant_pool.class_name(super_class)?;
    }
    let interface_count = reader.u2()?;
    let mut interfaces = Vec::with_capacity(usize::from(interface_count));
    for _ in 0..interface_count {
        let interface = reader.u2()?;
        constant_pool.class_name(interface)?;
        interfaces.push(interface);
    }
    let fields = read_members(&mut reader, &constant_pool)?;
    let methods = read_members(&mut reader, &constant_pool)?;
    let attributes = read_attributes(&mut reader, &constant_pool)?;
    reader.finish("class file")?;
    Ok(ClassFile {
        version,
        constant_pool,
        access_flags,
        this_class,
        super_class,
        interfaces,
        fields,
        methods,
        attributes,
    })
}

pub(crate) fn constant_value(attribute: &Attribute) -> Result<Option<u16>, Error> {
    if attribute.name != "ConstantValue" {
        return Ok(None);
    }
    let mut reader = Reader::new(&attribute.info);
    let index = reader.u2()?;
    reader.finish("ConstantValue attribute")?;
    Ok(Some(index))
}

pub(crate) fn code(attribute: &Attribute, pool: &ConstantPool) -> Result<Option<Code>, Error> {
    if attribute.name != "Code" {
        return Ok(None);
    }
    let mut reader = Reader::new(&attribute.info);
    let _max_stack = reader.u2()?;
    let _max_locals = reader.u2()?;
    let length =
        usize::try_from(reader.u4()?).map_err(|_| Error::new("Code attribute is too large"))?;
    let code = reader.take(length)?.to_vec();
    let exception_count = reader.u2()?;
    let mut exception_handlers = Vec::with_capacity(usize::from(exception_count));
    for _ in 0..exception_count {
        let handler = ExceptionHandler {
            start: reader.u2()?,
            end: reader.u2()?,
            handler: reader.u2()?,
            catch_type: reader.u2()?,
        };
        if handler.start >= handler.end
            || usize::from(handler.end) > code.len()
            || usize::from(handler.handler) >= code.len()
        {
            return Err(Error::new("invalid Code exception-table range"));
        }
        if handler.catch_type != 0 {
            pool.class_name(handler.catch_type)?;
        }
        exception_handlers.push(handler);
    }
    let nested = read_attributes(&mut reader, pool)?;
    let _ = nested;
    reader.finish("Code attribute")?;
    Ok(Some(Code {
        code,
        exception_handlers,
    }))
}

fn switch_padding(reader: &mut Reader<'_>, offset: u32) -> Result<(), Error> {
    let consumed = offset
        .checked_add(1)
        .ok_or_else(|| Error::new("bytecode offset overflow"))?;
    let padding = (4 - (consumed % 4)) % 4;
    reader.take(usize::try_from(padding).expect("padding is small"))?;
    Ok(())
}

fn wide(reader: &mut Reader<'_>, offset: u32) -> Result<RawInstruction, Error> {
    let opcode = reader.u1()?;
    let index = reader.u2()?;
    let instruction = match opcode {
        0x15 => RawInstruction::ILoad { index },
        0x16 => RawInstruction::LLoad { index },
        0x17 => RawInstruction::FLoad { index },
        0x18 => RawInstruction::DLoad { index },
        0x19 => RawInstruction::ALoad { index },
        0x36 => RawInstruction::IStore { index },
        0x37 => RawInstruction::LStore { index },
        0x38 => RawInstruction::FStore { index },
        0x39 => RawInstruction::DStore { index },
        0x3a => RawInstruction::AStore { index },
        0x84 => RawInstruction::IInc {
            index,
            value: reader.i2()?,
        },
        _ => RawInstruction::Unsupported { opcode: 0xc4 },
    };
    let _ = offset;
    Ok(instruction)
}

fn array_type(value: u8) -> Result<ArrayType, Error> {
    match value {
        4 => Ok(ArrayType::Boolean),
        8 => Ok(ArrayType::Byte),
        5 => Ok(ArrayType::Char),
        9 => Ok(ArrayType::Short),
        10 => Ok(ArrayType::Int),
        11 => Ok(ArrayType::Long),
        6 => Ok(ArrayType::Float),
        7 => Ok(ArrayType::Double),
        _ => Err(Error::new(format!("invalid newarray type {value}"))),
    }
}

fn decode_one(reader: &mut Reader<'_>, offset: u32) -> Result<RawInstruction, Error> {
    let opcode = reader.u1()?;
    use RawInstruction::*;
    Ok(match opcode {
        0x01 => AConstNull,
        0x02 => IConst(-1),
        0x03..=0x08 => IConst(i32::from(opcode) - 3),
        0x09 => LConst(0),
        0x0a => LConst(1),
        0x0b => FConst(0.0),
        0x0c => FConst(1.0),
        0x0d => FConst(2.0),
        0x0e => DConst(0.0),
        0x0f => DConst(1.0),
        0x10 => IConst(i32::from(reader.i1()?)),
        0x11 => IConst(i32::from(reader.i2()?)),
        0x12 => Ldc {
            index: u16::from(reader.u1()?),
        },
        0x13 => Ldc {
            index: reader.u2()?,
        },
        0x14 => Ldc2 {
            index: reader.u2()?,
        },
        0x15 => ILoad {
            index: u16::from(reader.u1()?),
        },
        0x16 => LLoad {
            index: u16::from(reader.u1()?),
        },
        0x17 => FLoad {
            index: u16::from(reader.u1()?),
        },
        0x18 => DLoad {
            index: u16::from(reader.u1()?),
        },
        0x19 => ALoad {
            index: u16::from(reader.u1()?),
        },
        0x1a..=0x1d => ILoad {
            index: u16::from(opcode - 0x1a),
        },
        0x1e..=0x21 => LLoad {
            index: u16::from(opcode - 0x1e),
        },
        0x22..=0x25 => FLoad {
            index: u16::from(opcode - 0x22),
        },
        0x26..=0x29 => DLoad {
            index: u16::from(opcode - 0x26),
        },
        0x2a..=0x2d => ALoad {
            index: u16::from(opcode - 0x2a),
        },
        0x2e => IALoad,
        0x2f => LALoad,
        0x30 => FALoad,
        0x31 => DALoad,
        0x32 => AALoad,
        0x33 => BALoad,
        0x34 => CALoad,
        0x35 => SALoad,
        0x36 => IStore {
            index: u16::from(reader.u1()?),
        },
        0x37 => LStore {
            index: u16::from(reader.u1()?),
        },
        0x38 => FStore {
            index: u16::from(reader.u1()?),
        },
        0x39 => DStore {
            index: u16::from(reader.u1()?),
        },
        0x3a => AStore {
            index: u16::from(reader.u1()?),
        },
        0x3b..=0x3e => IStore {
            index: u16::from(opcode - 0x3b),
        },
        0x3f..=0x42 => LStore {
            index: u16::from(opcode - 0x3f),
        },
        0x43..=0x46 => FStore {
            index: u16::from(opcode - 0x43),
        },
        0x47..=0x4a => DStore {
            index: u16::from(opcode - 0x47),
        },
        0x4b..=0x4e => AStore {
            index: u16::from(opcode - 0x4b),
        },
        0x4f => IAStore,
        0x50 => LAStore,
        0x51 => FAStore,
        0x52 => DAStore,
        0x53 => AAStore,
        0x54 => BAStore,
        0x55 => CAStore,
        0x56 => SAStore,
        0x57 => Pop,
        0x59 => Dup,
        0x60 => IAdd,
        0x61 => LAdd,
        0x62 => FAdd,
        0x63 => DAdd,
        0x64 => ISub,
        0x65 => LSub,
        0x66 => FSub,
        0x67 => DSub,
        0x68 => IMul,
        0x69 => LMul,
        0x6a => FMul,
        0x6b => DMul,
        0x6c => IDiv,
        0x6d => LDiv,
        0x6e => FDiv,
        0x6f => DDiv,
        0x70 => IRem,
        0x71 => LRem,
        0x72 => FRem,
        0x73 => DRem,
        0x74 => INeg,
        0x75 => LNeg,
        0x76 => FNeg,
        0x77 => DNeg,
        0x78 => IShl,
        0x7a => IShr,
        0x7c => IUshr,
        0x7e => IAnd,
        0x80 => IOr,
        0x82 => IXor,
        0x84 => IInc {
            index: u16::from(reader.u1()?),
            value: i16::from(reader.i1()?),
        },
        0x99 => If {
            kind: IfKind::Eq,
            offset: i32::from(reader.i2()?),
        },
        0x9a => If {
            kind: IfKind::Ne,
            offset: i32::from(reader.i2()?),
        },
        0x9b => If {
            kind: IfKind::Lt,
            offset: i32::from(reader.i2()?),
        },
        0x9c => If {
            kind: IfKind::Ge,
            offset: i32::from(reader.i2()?),
        },
        0x9d => If {
            kind: IfKind::Gt,
            offset: i32::from(reader.i2()?),
        },
        0x9e => If {
            kind: IfKind::Le,
            offset: i32::from(reader.i2()?),
        },
        0x9f => If {
            kind: IfKind::ICmpEq,
            offset: i32::from(reader.i2()?),
        },
        0xa0 => If {
            kind: IfKind::ICmpNe,
            offset: i32::from(reader.i2()?),
        },
        0xa1 => If {
            kind: IfKind::ICmpLt,
            offset: i32::from(reader.i2()?),
        },
        0xa2 => If {
            kind: IfKind::ICmpGe,
            offset: i32::from(reader.i2()?),
        },
        0xa3 => If {
            kind: IfKind::ICmpGt,
            offset: i32::from(reader.i2()?),
        },
        0xa4 => If {
            kind: IfKind::ICmpLe,
            offset: i32::from(reader.i2()?),
        },
        0xa5 => If {
            kind: IfKind::ACmpEq,
            offset: i32::from(reader.i2()?),
        },
        0xa6 => If {
            kind: IfKind::ACmpNe,
            offset: i32::from(reader.i2()?),
        },
        0xa7 => Goto {
            offset: i32::from(reader.i2()?),
        },
        0xaa => {
            switch_padding(reader, offset)?;
            let default = reader.i4()?;
            let low = reader.i4()?;
            let high = reader.i4()?;
            if high < low {
                return Err(Error::new("tableswitch high is less than low"));
            }
            let count = usize::try_from(i64::from(high) - i64::from(low) + 1)
                .map_err(|_| Error::new("tableswitch case count overflow"))?;
            let mut cases = Vec::with_capacity(count);
            for key in low..=high {
                cases.push((key, reader.i4()?));
            }
            TableSwitch { default, cases }
        }
        0xab => {
            switch_padding(reader, offset)?;
            let default = reader.i4()?;
            let count = reader.i4()?;
            if count < 0 {
                return Err(Error::new("lookupswitch has a negative pair count"));
            }
            let mut cases = Vec::with_capacity(usize::try_from(count).expect("non-negative i32"));
            let mut previous = None;
            for _ in 0..count {
                let key = reader.i4()?;
                if previous.is_some_and(|value| key <= value) {
                    return Err(Error::new("lookupswitch keys are not strictly increasing"));
                }
                previous = Some(key);
                cases.push((key, reader.i4()?));
            }
            LookupSwitch { default, cases }
        }
        0xac => IReturn,
        0xad => LReturn,
        0xae => FReturn,
        0xaf => DReturn,
        0xb0 => AReturn,
        0xb1 => Return,
        0xb2 => GetStatic {
            index: reader.u2()?,
        },
        0xb3 => PutStatic {
            index: reader.u2()?,
        },
        0xb4 => GetField {
            index: reader.u2()?,
        },
        0xb5 => PutField {
            index: reader.u2()?,
        },
        0xb6 => InvokeVirtual {
            index: reader.u2()?,
        },
        0xb7 => InvokeSpecial {
            index: reader.u2()?,
        },
        0xb8 => InvokeStatic {
            index: reader.u2()?,
        },
        0xb9 => {
            let index = reader.u2()?;
            let count = reader.u1()?;
            let zero = reader.u1()?;
            if count == 0 || zero != 0 {
                return Err(Error::new("invalid invokeinterface operands"));
            }
            InvokeInterface { index }
        }
        0xbb => New {
            index: reader.u2()?,
        },
        0xbc => NewArray {
            atype: array_type(reader.u1()?)?,
        },
        0xbd => ANewArray {
            index: reader.u2()?,
        },
        0xbe => ArrayLength,
        0xbf => AThrow,
        0xc4 => wide(reader, offset)?,
        0xc5 => MultiANewArray {
            index: reader.u2()?,
            dimensions: reader.u1()?,
        },
        0xc6 => If {
            kind: IfKind::Null,
            offset: i32::from(reader.i2()?),
        },
        0xc7 => If {
            kind: IfKind::NonNull,
            offset: i32::from(reader.i2()?),
        },
        0xc8 => Goto {
            offset: reader.i4()?,
        },
        _ => Unsupported { opcode },
    })
}

pub(crate) fn instructions(code: &[u8]) -> Result<Vec<(u32, RawInstruction)>, Error> {
    let mut reader = Reader::new(code);
    let mut instructions = Vec::new();
    while reader.remaining() != 0 {
        let offset = u32::try_from(reader.position).map_err(|_| Error::new("code is too large"))?;
        let instruction = decode_one(&mut reader, offset)?;
        let unsupported = matches!(instruction, RawInstruction::Unsupported { .. });
        instructions.push((offset, instruction));
        if unsupported {
            break;
        }
    }
    Ok(instructions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal(version: Version) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(0xcafe_babe_u32.to_be_bytes());
        bytes.extend(version.minor.to_be_bytes());
        bytes.extend(version.major.to_be_bytes());
        bytes.extend(5_u16.to_be_bytes());
        bytes.extend([1, 0, 1, b'C', 7, 0, 1]);
        bytes.extend([1, 0, 16]);
        bytes.extend(b"java/lang/Object");
        bytes.extend([7, 0, 3]);
        bytes.extend(0x0021_u16.to_be_bytes());
        bytes.extend(2_u16.to_be_bytes());
        bytes.extend(4_u16.to_be_bytes());
        bytes.extend([0; 8]);
        bytes
    }

    #[test]
    fn accepts_java_26_non_preview_format() {
        assert_eq!(
            parse(&minimal(Version {
                major: 70,
                minor: 0
            }))
            .unwrap()
            .version
            .major,
            70
        );
    }

    #[test]
    fn rejects_preview_and_future_formats() {
        assert!(
            parse(&minimal(Version {
                major: 70,
                minor: u16::MAX
            }))
            .is_err()
        );
        assert!(
            parse(&minimal(Version {
                major: 71,
                minor: 0
            }))
            .is_err()
        );
    }

    #[test]
    fn modified_utf8_handles_null_and_surrogate_pairs() {
        assert_eq!(modified_utf8(&[0xc0, 0x80]).unwrap(), "\0");
        assert_eq!(
            modified_utf8(&[0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80]).unwrap(),
            "😀"
        );
    }

    #[test]
    fn decodes_switch_padding_relative_to_code_start() {
        let code = [0x03, 0xab, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(matches!(
            instructions(&code).unwrap()[1].1,
            RawInstruction::LookupSwitch { .. }
        ));
    }
}
