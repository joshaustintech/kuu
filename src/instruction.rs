use crate::bytecode::{ByteReader, ByteWriter};
use crate::error::{KError, KResult};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Register(u16);

impl Register {
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u16 {
        self.0
    }
}

impl fmt::Display for Register {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConstantIndex(u32);

impl ConstantIndex {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ConstantIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "k{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrototypeIndex(u32);

impl PrototypeIndex {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Display for PrototypeIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JumpOffset(i32);

impl JumpOffset {
    pub fn new(offset: i64) -> KResult<Self> {
        if offset < i64::from(i32::MIN) || offset > i64::from(i32::MAX) {
            return Err(KError::bytecode("jump offset out of range"));
        }
        Ok(Self(offset as i32))
    }

    pub const fn from_i32(offset: i32) -> Self {
        Self(offset)
    }

    pub const fn value(self) -> i32 {
        self.0
    }
}

impl fmt::Display for JumpOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:+}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
}

impl ArithmeticOp {
    fn to_tag(self) -> u8 {
        match self {
            Self::Add => 0,
            Self::Sub => 1,
            Self::Mul => 2,
            Self::Div => 3,
            Self::FloorDiv => 4,
            Self::Mod => 5,
            Self::Pow => 6,
        }
    }

    fn from_tag(tag: u8) -> KResult<Self> {
        match tag {
            0 => Ok(Self::Add),
            1 => Ok(Self::Sub),
            2 => Ok(Self::Mul),
            3 => Ok(Self::Div),
            4 => Ok(Self::FloorDiv),
            5 => Ok(Self::Mod),
            6 => Ok(Self::Pow),
            _ => Err(KError::bytecode(format!("unknown arithmetic opcode {tag}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
}

impl CompareOp {
    fn to_tag(self) -> u8 {
        match self {
            Self::Eq => 0,
            Self::NotEq => 1,
            Self::Less => 2,
            Self::LessEq => 3,
            Self::Greater => 4,
            Self::GreaterEq => 5,
        }
    }

    fn from_tag(tag: u8) -> KResult<Self> {
        match tag {
            0 => Ok(Self::Eq),
            1 => Ok(Self::NotEq),
            2 => Ok(Self::Less),
            3 => Ok(Self::LessEq),
            4 => Ok(Self::Greater),
            5 => Ok(Self::GreaterEq),
            _ => Err(KError::bytecode(format!("unknown compare opcode {tag}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    LoadNil {
        dst: Register,
    },
    LoadBool {
        dst: Register,
        value: bool,
    },
    LoadInteger {
        dst: Register,
        value: i64,
    },
    LoadNumber {
        dst: Register,
        value: f64,
    },
    LoadConstant {
        dst: Register,
        constant: ConstantIndex,
    },
    Move {
        dst: Register,
        src: Register,
    },
    GetGlobal {
        dst: Register,
        name: ConstantIndex,
    },
    SetGlobal {
        src: Register,
        name: ConstantIndex,
    },
    GetTable {
        dst: Register,
        table: Register,
        key: Register,
    },
    SetTable {
        table: Register,
        key: Register,
        value: Register,
    },
    Arithmetic {
        op: ArithmeticOp,
        dst: Register,
        left: Register,
        right: Register,
    },
    Compare {
        op: CompareOp,
        dst: Register,
        left: Register,
        right: Register,
    },
    Jump {
        offset: JumpOffset,
    },
    Call {
        function: Register,
        args: u16,
        results: u16,
    },
    Return {
        first: Register,
        count: u16,
    },
    Closure {
        dst: Register,
        proto: PrototypeIndex,
    },
    Vararg {
        dst: Register,
        count: Option<u16>,
    },
    ForPrep {
        base: Register,
        offset: JumpOffset,
    },
    ForLoop {
        base: Register,
        offset: JumpOffset,
    },
    Concat {
        dst: Register,
        first: Register,
        last: Register,
    },
    Close {
        from: Register,
    },
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opcode {
    LoadNil = 0,
    LoadBool = 1,
    LoadInteger = 2,
    LoadNumber = 3,
    LoadConstant = 4,
    Move = 5,
    GetGlobal = 6,
    SetGlobal = 7,
    GetTable = 8,
    SetTable = 9,
    Arithmetic = 10,
    Compare = 11,
    Jump = 12,
    Call = 13,
    Return = 14,
    Closure = 15,
    Vararg = 16,
    ForPrep = 17,
    ForLoop = 18,
    Concat = 19,
    Close = 20,
}

impl Opcode {
    fn from_u8(value: u8) -> KResult<Self> {
        match value {
            0 => Ok(Self::LoadNil),
            1 => Ok(Self::LoadBool),
            2 => Ok(Self::LoadInteger),
            3 => Ok(Self::LoadNumber),
            4 => Ok(Self::LoadConstant),
            5 => Ok(Self::Move),
            6 => Ok(Self::GetGlobal),
            7 => Ok(Self::SetGlobal),
            8 => Ok(Self::GetTable),
            9 => Ok(Self::SetTable),
            10 => Ok(Self::Arithmetic),
            11 => Ok(Self::Compare),
            12 => Ok(Self::Jump),
            13 => Ok(Self::Call),
            14 => Ok(Self::Return),
            15 => Ok(Self::Closure),
            16 => Ok(Self::Vararg),
            17 => Ok(Self::ForPrep),
            18 => Ok(Self::ForLoop),
            19 => Ok(Self::Concat),
            20 => Ok(Self::Close),
            _ => Err(KError::bytecode(format!("unknown opcode {value}"))),
        }
    }
}

impl Instruction {
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = ByteWriter::new();
        self.encode_into(&mut writer);
        writer.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> KResult<Self> {
        let mut reader = ByteReader::new(bytes);
        let instruction = Self::decode_from(&mut reader)?;
        if !reader.is_empty() {
            return Err(KError::bytecode("trailing bytes after instruction"));
        }
        Ok(instruction)
    }

    pub(crate) fn encode_into(&self, writer: &mut ByteWriter) {
        match self {
            Self::LoadNil { dst } => {
                writer.write_u8(Opcode::LoadNil as u8);
                write_register(writer, *dst);
            }
            Self::LoadBool { dst, value } => {
                writer.write_u8(Opcode::LoadBool as u8);
                write_register(writer, *dst);
                writer.write_bool(*value);
            }
            Self::LoadInteger { dst, value } => {
                writer.write_u8(Opcode::LoadInteger as u8);
                write_register(writer, *dst);
                writer.write_i64(*value);
            }
            Self::LoadNumber { dst, value } => {
                writer.write_u8(Opcode::LoadNumber as u8);
                write_register(writer, *dst);
                writer.write_f64(*value);
            }
            Self::LoadConstant { dst, constant } => {
                writer.write_u8(Opcode::LoadConstant as u8);
                write_register(writer, *dst);
                write_constant_index(writer, *constant);
            }
            Self::Move { dst, src } => {
                writer.write_u8(Opcode::Move as u8);
                write_register(writer, *dst);
                write_register(writer, *src);
            }
            Self::GetGlobal { dst, name } => {
                writer.write_u8(Opcode::GetGlobal as u8);
                write_register(writer, *dst);
                write_constant_index(writer, *name);
            }
            Self::SetGlobal { src, name } => {
                writer.write_u8(Opcode::SetGlobal as u8);
                write_register(writer, *src);
                write_constant_index(writer, *name);
            }
            Self::GetTable { dst, table, key } => {
                writer.write_u8(Opcode::GetTable as u8);
                write_register(writer, *dst);
                write_register(writer, *table);
                write_register(writer, *key);
            }
            Self::SetTable { table, key, value } => {
                writer.write_u8(Opcode::SetTable as u8);
                write_register(writer, *table);
                write_register(writer, *key);
                write_register(writer, *value);
            }
            Self::Arithmetic {
                op,
                dst,
                left,
                right,
            } => {
                writer.write_u8(Opcode::Arithmetic as u8);
                writer.write_u8(op.to_tag());
                write_register(writer, *dst);
                write_register(writer, *left);
                write_register(writer, *right);
            }
            Self::Compare {
                op,
                dst,
                left,
                right,
            } => {
                writer.write_u8(Opcode::Compare as u8);
                writer.write_u8(op.to_tag());
                write_register(writer, *dst);
                write_register(writer, *left);
                write_register(writer, *right);
            }
            Self::Jump { offset } => {
                writer.write_u8(Opcode::Jump as u8);
                writer.write_i32(offset.value());
            }
            Self::Call {
                function,
                args,
                results,
            } => {
                writer.write_u8(Opcode::Call as u8);
                write_register(writer, *function);
                writer.write_u16(*args);
                writer.write_u16(*results);
            }
            Self::Return { first, count } => {
                writer.write_u8(Opcode::Return as u8);
                write_register(writer, *first);
                writer.write_u16(*count);
            }
            Self::Closure { dst, proto } => {
                writer.write_u8(Opcode::Closure as u8);
                write_register(writer, *dst);
                write_prototype_index(writer, *proto);
            }
            Self::Vararg { dst, count } => {
                writer.write_u8(Opcode::Vararg as u8);
                write_register(writer, *dst);
                match count {
                    Some(count) => {
                        writer.write_bool(true);
                        writer.write_u16(*count);
                    }
                    None => writer.write_bool(false),
                }
            }
            Self::ForPrep { base, offset } => {
                writer.write_u8(Opcode::ForPrep as u8);
                write_register(writer, *base);
                writer.write_i32(offset.value());
            }
            Self::ForLoop { base, offset } => {
                writer.write_u8(Opcode::ForLoop as u8);
                write_register(writer, *base);
                writer.write_i32(offset.value());
            }
            Self::Concat { dst, first, last } => {
                writer.write_u8(Opcode::Concat as u8);
                write_register(writer, *dst);
                write_register(writer, *first);
                write_register(writer, *last);
            }
            Self::Close { from } => {
                writer.write_u8(Opcode::Close as u8);
                write_register(writer, *from);
            }
        }
    }

    pub(crate) fn decode_from(reader: &mut ByteReader<'_>) -> KResult<Self> {
        let opcode = Opcode::from_u8(reader.read_u8()?)?;
        match opcode {
            Opcode::LoadNil => Ok(Self::LoadNil {
                dst: read_register(reader)?,
            }),
            Opcode::LoadBool => Ok(Self::LoadBool {
                dst: read_register(reader)?,
                value: reader.read_bool()?,
            }),
            Opcode::LoadInteger => Ok(Self::LoadInteger {
                dst: read_register(reader)?,
                value: reader.read_i64()?,
            }),
            Opcode::LoadNumber => Ok(Self::LoadNumber {
                dst: read_register(reader)?,
                value: reader.read_f64()?,
            }),
            Opcode::LoadConstant => Ok(Self::LoadConstant {
                dst: read_register(reader)?,
                constant: read_constant_index(reader)?,
            }),
            Opcode::Move => Ok(Self::Move {
                dst: read_register(reader)?,
                src: read_register(reader)?,
            }),
            Opcode::GetGlobal => Ok(Self::GetGlobal {
                dst: read_register(reader)?,
                name: read_constant_index(reader)?,
            }),
            Opcode::SetGlobal => Ok(Self::SetGlobal {
                src: read_register(reader)?,
                name: read_constant_index(reader)?,
            }),
            Opcode::GetTable => Ok(Self::GetTable {
                dst: read_register(reader)?,
                table: read_register(reader)?,
                key: read_register(reader)?,
            }),
            Opcode::SetTable => Ok(Self::SetTable {
                table: read_register(reader)?,
                key: read_register(reader)?,
                value: read_register(reader)?,
            }),
            Opcode::Arithmetic => Ok(Self::Arithmetic {
                op: ArithmeticOp::from_tag(reader.read_u8()?)?,
                dst: read_register(reader)?,
                left: read_register(reader)?,
                right: read_register(reader)?,
            }),
            Opcode::Compare => Ok(Self::Compare {
                op: CompareOp::from_tag(reader.read_u8()?)?,
                dst: read_register(reader)?,
                left: read_register(reader)?,
                right: read_register(reader)?,
            }),
            Opcode::Jump => Ok(Self::Jump {
                offset: JumpOffset::from_i32(reader.read_i32()?),
            }),
            Opcode::Call => Ok(Self::Call {
                function: read_register(reader)?,
                args: reader.read_u16()?,
                results: reader.read_u16()?,
            }),
            Opcode::Return => Ok(Self::Return {
                first: read_register(reader)?,
                count: reader.read_u16()?,
            }),
            Opcode::Closure => Ok(Self::Closure {
                dst: read_register(reader)?,
                proto: read_prototype_index(reader)?,
            }),
            Opcode::Vararg => Ok(Self::Vararg {
                dst: read_register(reader)?,
                count: if reader.read_bool()? {
                    Some(reader.read_u16()?)
                } else {
                    None
                },
            }),
            Opcode::ForPrep => Ok(Self::ForPrep {
                base: read_register(reader)?,
                offset: JumpOffset::from_i32(reader.read_i32()?),
            }),
            Opcode::ForLoop => Ok(Self::ForLoop {
                base: read_register(reader)?,
                offset: JumpOffset::from_i32(reader.read_i32()?),
            }),
            Opcode::Concat => Ok(Self::Concat {
                dst: read_register(reader)?,
                first: read_register(reader)?,
                last: read_register(reader)?,
            }),
            Opcode::Close => Ok(Self::Close {
                from: read_register(reader)?,
            }),
        }
    }

    pub fn decode_all(bytes: &[u8]) -> KResult<Vec<Self>> {
        let mut reader = ByteReader::new(bytes);
        let mut instructions = Vec::new();
        while !reader.is_empty() {
            instructions.push(Self::decode_from(&mut reader)?);
        }
        Ok(instructions)
    }
}

fn write_register(writer: &mut ByteWriter, register: Register) {
    writer.write_u16(register.index());
}

fn read_register(reader: &mut ByteReader<'_>) -> KResult<Register> {
    Ok(Register::new(reader.read_u16()?))
}

fn write_constant_index(writer: &mut ByteWriter, index: ConstantIndex) {
    writer.write_u32(index.index());
}

fn read_constant_index(reader: &mut ByteReader<'_>) -> KResult<ConstantIndex> {
    Ok(ConstantIndex::new(reader.read_u32()?))
}

fn write_prototype_index(writer: &mut ByteWriter, index: PrototypeIndex) {
    writer.write_u32(index.index());
}

fn read_prototype_index(reader: &mut ByteReader<'_>) -> KResult<PrototypeIndex> {
    Ok(PrototypeIndex::new(reader.read_u32()?))
}
