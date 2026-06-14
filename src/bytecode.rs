use crate::error::{KError, KResult};
use std::convert::TryFrom;

pub(crate) struct ByteWriter {
    bytes: Vec<u8>,
}

impl ByteWriter {
    pub(crate) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    pub(crate) fn write_u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn write_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn write_u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn write_i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn write_i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn write_f64(&mut self, value: f64) {
        self.write_u64(value.to_bits());
    }

    pub(crate) fn write_bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }
}

pub(crate) struct ByteReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> ByteReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.cursor >= self.bytes.len()
    }

    pub(crate) fn read_u8(&mut self) -> KResult<u8> {
        let byte = self
            .bytes
            .get(self.cursor)
            .copied()
            .ok_or_else(|| KError::bytecode("unexpected end of bytecode"))?;
        self.cursor = self
            .cursor
            .checked_add(1)
            .ok_or_else(|| KError::bytecode("bytecode cursor overflow"))?;
        Ok(byte)
    }

    pub(crate) fn read_bool(&mut self) -> KResult<bool> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(KError::bytecode("invalid boolean encoding")),
        }
    }

    pub(crate) fn read_u16(&mut self) -> KResult<u16> {
        let bytes = self.read_exact(2)?;
        let mut array = [0u8; 2];
        array.copy_from_slice(bytes);
        Ok(u16::from_le_bytes(array))
    }

    pub(crate) fn read_u32(&mut self) -> KResult<u32> {
        let bytes = self.read_exact(4)?;
        let mut array = [0u8; 4];
        array.copy_from_slice(bytes);
        Ok(u32::from_le_bytes(array))
    }

    pub(crate) fn read_u64(&mut self) -> KResult<u64> {
        let bytes = self.read_exact(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }

    pub(crate) fn read_i32(&mut self) -> KResult<i32> {
        let bytes = self.read_exact(4)?;
        let mut array = [0u8; 4];
        array.copy_from_slice(bytes);
        Ok(i32::from_le_bytes(array))
    }

    pub(crate) fn read_i64(&mut self) -> KResult<i64> {
        let bytes = self.read_exact(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(i64::from_le_bytes(array))
    }

    pub(crate) fn read_f64(&mut self) -> KResult<f64> {
        Ok(f64::from_bits(self.read_u64()?))
    }

    pub(crate) fn read_bytes(&mut self, len: usize) -> KResult<Vec<u8>> {
        Ok(self.read_exact(len)?.to_vec())
    }

    pub(crate) fn read_len(&mut self) -> KResult<usize> {
        let len = self.read_u32()?;
        usize::try_from(len).map_err(|_| KError::bytecode("length does not fit in usize"))
    }

    fn read_exact(&mut self, len: usize) -> KResult<&'a [u8]> {
        let end = self
            .cursor
            .checked_add(len)
            .ok_or_else(|| KError::bytecode("bytecode cursor overflow"))?;
        let slice = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(|| KError::bytecode("unexpected end of bytecode"))?;
        self.cursor = end;
        Ok(slice)
    }
}

pub(crate) fn write_len(writer: &mut ByteWriter, len: usize) -> KResult<()> {
    let value = u32::try_from(len).map_err(|_| KError::bytecode("length exceeds u32"))?;
    writer.write_u32(value);
    Ok(())
}
