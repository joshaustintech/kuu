use crate::bytecode::{ByteReader, ByteWriter, write_len};
use crate::error::{KError, KResult};
use crate::instruction::Instruction;

#[derive(Debug, Clone, PartialEq)]
pub struct UpvalueDescriptor {
    pub instack: bool,
    pub index: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Proto {
    pub name: Option<Vec<u8>>,
    pub parameters: u16,
    pub is_vararg: bool,
    pub stack_size: u16,
    pub upvalues: Vec<UpvalueDescriptor>,
    pub constants: Vec<Constant>,
    pub instructions: Vec<Instruction>,
    pub nested: Vec<Proto>,
}

impl Proto {
    pub fn encode(&self) -> KResult<Vec<u8>> {
        let mut writer = ByteWriter::new();
        self.encode_into(&mut writer)?;
        Ok(writer.into_bytes())
    }

    pub fn decode(bytes: &[u8]) -> KResult<Self> {
        let mut reader = ByteReader::new(bytes);
        let proto = Self::decode_from(&mut reader)?;
        if !reader.is_empty() {
            return Err(KError::bytecode("trailing bytes after proto"));
        }
        Ok(proto)
    }

    pub(crate) fn encode_into(&self, writer: &mut ByteWriter) -> KResult<()> {
        match &self.name {
            Some(name) => {
                writer.write_bool(true);
                write_bytes(writer, name)?;
            }
            None => writer.write_bool(false),
        }

        writer.write_u16(self.parameters);
        writer.write_bool(self.is_vararg);
        writer.write_u16(self.stack_size);

        write_len(writer, self.upvalues.len())?;
        for upvalue in &self.upvalues {
            writer.write_bool(upvalue.instack);
            writer.write_u16(upvalue.index);
        }

        write_len(writer, self.constants.len())?;
        for constant in &self.constants {
            encode_constant(writer, constant)?;
        }

        write_len(writer, self.instructions.len())?;
        for instruction in &self.instructions {
            instruction.encode_into(writer);
        }

        write_len(writer, self.nested.len())?;
        for nested in &self.nested {
            nested.encode_into(writer)?;
        }

        Ok(())
    }

    pub(crate) fn decode_from(reader: &mut ByteReader<'_>) -> KResult<Self> {
        let name = if reader.read_bool()? {
            Some(read_bytes(reader)?)
        } else {
            None
        };

        let parameters = reader.read_u16()?;
        let is_vararg = reader.read_bool()?;
        let stack_size = reader.read_u16()?;

        let upvalue_len = reader.read_len()?;
        let mut upvalues = Vec::with_capacity(upvalue_len);
        for _ in 0..upvalue_len {
            upvalues.push(UpvalueDescriptor {
                instack: reader.read_bool()?,
                index: reader.read_u16()?,
            });
        }

        let constant_len = reader.read_len()?;
        let mut constants = Vec::with_capacity(constant_len);
        for _ in 0..constant_len {
            constants.push(decode_constant(reader)?);
        }

        let instruction_len = reader.read_len()?;
        let mut instructions = Vec::with_capacity(instruction_len);
        for _ in 0..instruction_len {
            instructions.push(Instruction::decode_from(reader)?);
        }

        let nested_len = reader.read_len()?;
        let mut nested = Vec::with_capacity(nested_len);
        for _ in 0..nested_len {
            nested.push(Self::decode_from(reader)?);
        }

        Ok(Self {
            name,
            parameters,
            is_vararg,
            stack_size,
            upvalues,
            constants,
            instructions,
            nested,
        })
    }
}

fn encode_constant(writer: &mut ByteWriter, constant: &Constant) -> KResult<()> {
    match constant {
        Constant::Nil => writer.write_u8(0),
        Constant::Boolean(value) => {
            writer.write_u8(1);
            writer.write_bool(*value);
        }
        Constant::Integer(value) => {
            writer.write_u8(2);
            writer.write_i64(*value);
        }
        Constant::Number(value) => {
            writer.write_u8(3);
            writer.write_f64(*value);
        }
        Constant::String(bytes) => {
            writer.write_u8(4);
            write_bytes(writer, bytes)?;
        }
    }

    Ok(())
}

fn decode_constant(reader: &mut ByteReader<'_>) -> KResult<Constant> {
    match reader.read_u8()? {
        0 => Ok(Constant::Nil),
        1 => Ok(Constant::Boolean(reader.read_bool()?)),
        2 => Ok(Constant::Integer(reader.read_i64()?)),
        3 => Ok(Constant::Number(reader.read_f64()?)),
        4 => Ok(Constant::String(read_bytes(reader)?)),
        tag => Err(KError::bytecode(format!("unknown constant tag {tag}"))),
    }
}

fn write_bytes(writer: &mut ByteWriter, bytes: &[u8]) -> KResult<()> {
    write_len(writer, bytes.len())?;
    writer.write_bytes(bytes);
    Ok(())
}

fn read_bytes(reader: &mut ByteReader<'_>) -> KResult<Vec<u8>> {
    let len = reader.read_len()?;
    reader.read_bytes(len)
}
