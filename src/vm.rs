use crate::compiler::Compiler;
use crate::error::{KError, KErrorKind, KResult};
use crate::instruction::{
    ArithmeticOp, CompareOp, ConstantIndex, Instruction, JumpOffset, PrototypeIndex, Register,
    UnaryOpKind,
};
use crate::parser::Parser;
use crate::proto::{Constant, Proto};
use crate::value::{
    ClosureHandle, LuaKey, NativeFunction, StringHandle, TableHandle, ThreadHandle, UserdataHandle,
    Value,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::convert::TryFrom;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct UpvalueHandle(u64);

impl UpvalueHandle {
    const fn new(raw: u64) -> Self {
        Self(raw)
    }

    const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone)]
struct ReturnTarget {
    base: usize,
    results: usize,
}

#[derive(Debug, Clone, Copy)]
struct CallSite {
    frame_index: usize,
    call_slot: usize,
    results: usize,
    tail: bool,
}

#[derive(Debug, Clone)]
struct Frame {
    closure: ClosureHandle,
    base: usize,
    top: usize,
    pc: usize,
    return_target: Option<ReturnTarget>,
    varargs: Vec<Value>,
    last_call_results: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct ExecutionState {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    open_upvalues: Vec<UpvalueHandle>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct ThreadState {
    execution: ExecutionState,
}

#[derive(Debug, Clone)]
enum UpvalueState {
    Open { stack_index: usize },
    Closed(Value),
}

#[derive(Debug, Clone)]
struct Upvalue {
    state: UpvalueState,
}

#[derive(Debug, Clone)]
struct ClosureObject {
    proto: Proto,
    upvalues: Vec<UpvalueHandle>,
}

#[allow(dead_code)]
#[derive(Debug)]
enum UserdataObject {
    File(FileObject),
    Lines(LineIteratorObject),
}

#[allow(dead_code)]
#[derive(Debug)]
struct FileObject {
    kind: FileKind,
}

#[allow(dead_code)]
#[derive(Debug)]
enum FileKind {
    File(File),
    Stdin,
    Stdout,
    Stderr,
    Closed,
}

#[allow(dead_code)]
#[derive(Debug)]
struct LineIteratorObject {
    lines: Vec<Vec<u8>>,
    index: usize,
}

#[derive(Debug, Clone)]
struct TableObject {
    array: Vec<Value>,
    hash: BTreeMap<LuaKey, Value>,
    metatable: Option<TableHandle>,
    marked: bool,
    finalizer_ran: bool,
}

impl TableObject {
    fn new() -> Self {
        Self {
            array: Vec::new(),
            hash: BTreeMap::new(),
            metatable: None,
            marked: false,
            finalizer_ran: false,
        }
    }

    fn raw_get(&self, key: Value) -> KResult<Value> {
        match key.hash_key() {
            Some(LuaKey::Integer(integer)) if integer > 0 => {
                let Some(index) = usize::try_from(integer - 1).ok() else {
                    return Ok(self
                        .hash
                        .get(&LuaKey::Integer(integer))
                        .copied()
                        .unwrap_or(Value::nil()));
                };
                if let Some(value) = self.array.get(index).copied() {
                    return Ok(value);
                }
                Ok(self
                    .hash
                    .get(&LuaKey::Integer(integer))
                    .copied()
                    .unwrap_or(Value::nil()))
            }
            Some(hash_key) => Ok(self.hash.get(&hash_key).copied().unwrap_or(Value::nil())),
            None => Ok(Value::nil()),
        }
    }

    fn raw_set(&mut self, key: Value, value: Value) -> KResult<()> {
        let Some(hash_key) = key.hash_key() else {
            return Err(KError::new(
                KErrorKind::Runtime("table index is not hashable".to_owned()),
                None,
            ));
        };

        match hash_key {
            LuaKey::Integer(value_index) if value_index > 0 => {
                let Some(index) = usize::try_from(value_index - 1).ok() else {
                    self.hash.insert(LuaKey::Integer(value_index), value);
                    return Ok(());
                };

                if index < self.array.len() {
                    if let Some(slot) = self.array.get_mut(index) {
                        *slot = value;
                    }
                    self.trim_array();
                    return Ok(());
                }

                if index == self.array.len() {
                    if value == Value::nil() {
                        self.hash.remove(&LuaKey::Integer(value_index));
                    } else {
                        self.array.push(value);
                    }
                    self.trim_array();
                    return Ok(());
                }

                if value == Value::nil() {
                    self.hash.remove(&LuaKey::Integer(value_index));
                } else {
                    self.hash.insert(LuaKey::Integer(value_index), value);
                }
                Ok(())
            }
            other => {
                if value == Value::nil() {
                    self.hash.remove(&other);
                } else {
                    self.hash.insert(other, value);
                }
                Ok(())
            }
        }
    }

    fn trim_array(&mut self) {
        while self.array.last().copied() == Some(Value::nil()) {
            let _ = self.array.pop();
        }
    }

    fn entries(&self) -> impl Iterator<Item = (Value, Value)> + '_ {
        self.array
            .iter()
            .copied()
            .enumerate()
            .map(|(index, value)| {
                (
                    Value::integer(i64::try_from(index + 1).unwrap_or(i64::MAX)),
                    value,
                )
            })
            .chain(
                self.hash
                    .iter()
                    .map(|(key, value)| (key_from_luakey(*key), *value)),
            )
    }

    fn clear_weak_entries(&mut self, weak_keys: bool, weak_values: bool, dead: &BTreeSet<u64>) {
        if weak_values {
            for value in &mut self.array {
                if matches!(value, Value::Table(handle) if dead.contains(&handle.raw())) {
                    *value = Value::nil();
                }
            }
            self.trim_array();
        }
        self.hash.retain(|key, value| {
            let dead_key =
                weak_keys && matches!(key, LuaKey::Table(handle) if dead.contains(&handle.raw()));
            let dead_value = weak_values
                && matches!(value, Value::Table(handle) if dead.contains(&handle.raw()));
            !dead_key && !dead_value
        });
    }

    fn raw_len(&self) -> usize {
        self.array.len()
    }

    fn next_key(&self, key: Option<Value>) -> Option<(Value, Value)> {
        match key {
            None => {
                if let Some(value) = self.array.first().copied() {
                    return Some((Value::integer(1), value));
                }
                self.hash
                    .iter()
                    .next()
                    .map(|(k, v)| (key_from_luakey(*k), *v))
            }
            Some(Value::Integer(integer)) if integer >= 0 => {
                let mut index = integer as usize;
                if index < self.array.len() {
                    index += 1;
                    if let Some(value) = self.array.get(index - 1).copied() {
                        return Some((Value::integer(index as i64), value));
                    }
                }
                for (hash_key, value) in self.hash.iter() {
                    if let LuaKey::Integer(current) = hash_key
                        && *current > integer
                    {
                        return Some((Value::integer(*current), *value));
                    }
                }
                None
            }
            Some(other) => {
                let current_key = other.hash_key()?;
                let mut seen = false;
                for (hash_key, value) in &self.hash {
                    if seen {
                        return Some((key_from_luakey(*hash_key), *value));
                    }
                    if *hash_key == current_key {
                        seen = true;
                    }
                }
                None
            }
        }
    }
}

fn key_from_luakey(key: LuaKey) -> Value {
    match key {
        LuaKey::Nil => Value::nil(),
        LuaKey::Boolean(value) => Value::boolean(value),
        LuaKey::Integer(value) => Value::integer(value),
        LuaKey::Number(bits) => Value::number(f64::from_bits(bits)),
        LuaKey::String(handle) => Value::string(handle),
        LuaKey::Table(handle) => Value::table(handle),
        LuaKey::Closure(handle) => Value::closure(handle),
        LuaKey::NativeFunction(function) => {
            let ptr = function as *const ();
            let _ = ptr;
            Value::nil()
        }
        LuaKey::Thread(handle) => Value::thread(handle),
        LuaKey::Userdata(handle) => Value::userdata(handle),
        LuaKey::LightUserdata(value) => Value::light_userdata(value),
    }
}

#[derive(Debug, Clone)]
struct Heap {
    strings: Vec<Vec<u8>>,
    string_lookup: BTreeMap<Vec<u8>, StringHandle>,
    tables: Vec<Option<TableObject>>,
    closures: Vec<ClosureObject>,
    upvalues: Vec<Upvalue>,
}

impl Heap {
    fn new() -> Self {
        Self {
            strings: Vec::new(),
            string_lookup: BTreeMap::new(),
            tables: Vec::new(),
            closures: Vec::new(),
            upvalues: Vec::new(),
        }
    }

    fn intern_string(&mut self, bytes: Vec<u8>) -> KResult<StringHandle> {
        if let Some(handle) = self.string_lookup.get(&bytes).copied() {
            return Ok(handle);
        }

        let raw = u64::try_from(self.strings.len()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("string handle overflow".to_owned()),
                None,
            )
        })?;
        let handle = StringHandle::new(raw);
        crate::value::register_string(handle, &bytes);
        self.strings.push(bytes.clone());
        self.string_lookup.insert(bytes, handle);
        Ok(handle)
    }

    fn string_bytes(&self, handle: StringHandle) -> Option<&[u8]> {
        let index = usize::try_from(handle.raw()).ok()?;
        self.strings.get(index).map(Vec::as_slice)
    }

    fn new_table(&mut self) -> KResult<TableHandle> {
        let raw = u64::try_from(self.tables.len()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("table handle overflow".to_owned()),
                None,
            )
        })?;
        self.tables.push(Some(TableObject::new()));
        Ok(TableHandle::new(raw))
    }

    fn resolve_table(&self, handle: TableHandle) -> KResult<&TableObject> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(KErrorKind::Runtime("invalid table handle".to_owned()), None)
        })?;
        self.tables
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                KError::new(KErrorKind::Runtime("invalid table handle".to_owned()), None)
            })
    }

    fn resolve_table_mut(&mut self, handle: TableHandle) -> KResult<&mut TableObject> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(KErrorKind::Runtime("invalid table handle".to_owned()), None)
        })?;
        self.tables
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or_else(|| {
                KError::new(KErrorKind::Runtime("invalid table handle".to_owned()), None)
            })
    }

    fn new_upvalue_open(&mut self, stack_index: usize) -> KResult<UpvalueHandle> {
        let raw = u64::try_from(self.upvalues.len()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("upvalue handle overflow".to_owned()),
                None,
            )
        })?;
        self.upvalues.push(Upvalue {
            state: UpvalueState::Open { stack_index },
        });
        Ok(UpvalueHandle::new(raw))
    }

    fn new_upvalue_closed(&mut self, value: Value) -> KResult<UpvalueHandle> {
        let raw = u64::try_from(self.upvalues.len()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("upvalue handle overflow".to_owned()),
                None,
            )
        })?;
        self.upvalues.push(Upvalue {
            state: UpvalueState::Closed(value),
        });
        Ok(UpvalueHandle::new(raw))
    }

    fn upvalue_stack_index(&self, handle: UpvalueHandle) -> Option<usize> {
        let index = usize::try_from(handle.raw()).ok()?;
        match self.upvalues.get(index)?.state {
            UpvalueState::Open { stack_index } => Some(stack_index),
            UpvalueState::Closed(_) => None,
        }
    }

    fn upvalue_value(&self, handle: UpvalueHandle, stack: &[Value]) -> KResult<Value> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("invalid upvalue handle".to_owned()),
                None,
            )
        })?;
        let upvalue = self.upvalues.get(index).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("invalid upvalue handle".to_owned()),
                None,
            )
        })?;
        match &upvalue.state {
            UpvalueState::Open { stack_index } => {
                stack.get(*stack_index).copied().ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("open upvalue points outside the stack".to_owned()),
                        None,
                    )
                })
            }
            UpvalueState::Closed(value) => Ok(*value),
        }
    }

    fn set_upvalue_value(
        &mut self,
        handle: UpvalueHandle,
        stack: &mut [Value],
        value: Value,
    ) -> KResult<()> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("invalid upvalue handle".to_owned()),
                None,
            )
        })?;
        let upvalue = self.upvalues.get_mut(index).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("invalid upvalue handle".to_owned()),
                None,
            )
        })?;
        match &mut upvalue.state {
            UpvalueState::Open { stack_index } => {
                let slot = stack.get_mut(*stack_index).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("open upvalue points outside the stack".to_owned()),
                        None,
                    )
                })?;
                *slot = value;
                Ok(())
            }
            UpvalueState::Closed(slot) => {
                *slot = value;
                Ok(())
            }
        }
    }

    fn close_upvalue(&mut self, handle: UpvalueHandle, stack: &[Value]) -> KResult<()> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("invalid upvalue handle".to_owned()),
                None,
            )
        })?;
        let upvalue = self.upvalues.get_mut(index).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("invalid upvalue handle".to_owned()),
                None,
            )
        })?;
        let value = match upvalue.state {
            UpvalueState::Open { stack_index } => {
                stack.get(stack_index).copied().ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("open upvalue points outside the stack".to_owned()),
                        None,
                    )
                })?
            }
            UpvalueState::Closed(value) => value,
        };
        upvalue.state = UpvalueState::Closed(value);
        Ok(())
    }

    fn new_closure(
        &mut self,
        proto: Proto,
        upvalues: Vec<UpvalueHandle>,
    ) -> KResult<ClosureHandle> {
        let raw = u64::try_from(self.closures.len()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("closure handle overflow".to_owned()),
                None,
            )
        })?;
        self.closures.push(ClosureObject { proto, upvalues });
        Ok(ClosureHandle::new(raw))
    }

    fn resolve_closure(&self, handle: ClosureHandle) -> KResult<&ClosureObject> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("invalid closure handle".to_owned()),
                None,
            )
        })?;
        self.closures.get(index).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("invalid closure handle".to_owned()),
                None,
            )
        })
    }
}

pub fn native_print(_vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    _vm.print_values(args)?;
    Ok(Vec::new())
}

pub fn native_collectgarbage(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    vm.collectgarbage_request(args)
}

pub fn native_assert(_vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = _vm.arg_or_nil(args, 0);
    if matches!(value, Value::Nil | Value::Boolean(false)) {
        let message = match _vm.arg_or_nil(args, 1) {
            Value::String(_) | Value::Nil => "assertion failed!".to_owned(),
            other => format!("assertion failed!: {}", other),
        };
        return Err(Vm::runtime_error(message));
    }
    Ok(args.to_vec())
}

pub fn native_error(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let message = match vm.arg_or_nil(args, 0) {
        Value::String(handle) => vm.format_value(Value::String(handle))?,
        other => vm.format_value(other)?,
    };
    Err(Vm::runtime_error(message))
}

pub fn native_getmetatable(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let result = match vm.arg_or_nil(args, 0) {
        Value::Table(handle) => {
            let table = vm.heap.resolve_table(handle)?;
            match table.metatable {
                Some(metatable) => Value::table(metatable),
                None => Value::nil(),
            }
        }
        Value::String(_) => Value::table(vm.string_metatable),
        _ => Value::nil(),
    };
    Ok(vec![result])
}

pub fn native_setmetatable(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = vm.table_arg(args, 0, "setmetatable expects a table")?;
    let metatable = match vm.arg_or_nil(args, 1) {
        Value::Table(handle) => Some(handle),
        Value::Nil => None,
        other => return Err(vm.type_error("setmetatable expects a table or nil", other)),
    };
    vm.heap.resolve_table_mut(table)?.metatable = metatable;
    Ok(vec![Value::table(table)])
}

pub fn native_rawequal(_vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let left = _vm.arg_or_nil(args, 0);
    let right = _vm.arg_or_nil(args, 1);
    Ok(vec![Value::boolean(left == right)])
}

pub fn native_rawget(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = vm.table_arg(args, 0, "rawget expects a table")?;
    let key = vm.arg_or_nil(args, 1);
    let value = vm.heap.resolve_table(table)?.raw_get(key)?;
    Ok(vec![value])
}

pub fn native_rawset(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = vm.table_arg(args, 0, "rawset expects a table")?;
    let key = vm.arg_or_nil(args, 1);
    let value = vm.arg_or_nil(args, 2);
    vm.heap.resolve_table_mut(table)?.raw_set(key, value)?;
    Ok(vec![Value::table(table)])
}

pub fn native_rawlen(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.arg_or_nil(args, 0);
    let len = match value {
        Value::String(handle) => {
            let bytes = vm.string_bytes_from_handle(handle)?;
            bytes.len()
        }
        Value::Table(handle) => vm.heap.resolve_table(handle)?.raw_len(),
        other => {
            return Err(KError::new(
                KErrorKind::Runtime(format!(
                    "rawlen expects a string or table, got {}",
                    vm.value_type_name(other)
                )),
                None,
            ));
        }
    };
    Ok(vec![Value::integer(i64::try_from(len).map_err(|_| {
        KError::new(KErrorKind::Runtime("raw length overflow".to_owned()), None)
    })?)])
}

pub fn native_select(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    if let Some(Value::String(handle)) = args.first().copied()
        && let Some(bytes) = vm.heap.string_bytes(handle)
        && bytes == b"#"
    {
        return Ok(vec![Value::integer(
            i64::try_from(args.len().saturating_sub(1)).map_err(|_| {
                KError::new(
                    KErrorKind::Runtime("select count overflow".to_owned()),
                    None,
                )
            })?,
        )]);
    }
    let start = match vm.arg_or_nil(args, 0) {
        Value::Integer(value) => value,
        Value::Number(value) if value.fract() == 0.0 => value as i64,
        _ => {
            return Err(Vm::runtime_error(
                "select expects an integer index or '#'".to_owned(),
            ));
        }
    };
    if start < 1 {
        return Err(KError::new(
            KErrorKind::Runtime("select index must be positive".to_owned()),
            None,
        ));
    }
    let index = usize::try_from(start - 1).map_err(|_| {
        KError::new(
            KErrorKind::Runtime("select index overflow".to_owned()),
            None,
        )
    })?;
    let start = index.checked_add(1).ok_or_else(|| {
        KError::new(
            KErrorKind::Runtime("select index overflow".to_owned()),
            None,
        )
    })?;
    Ok(args
        .get(start..)
        .map_or_else(Vec::new, |slice| slice.to_vec()))
}

pub fn native_type(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = args
        .first()
        .copied()
        .ok_or_else(|| Vm::runtime_error("bad argument #1 to 'type' (value expected)"))?;
    let text = vm.value_type_name(value);
    let handle = vm.heap.intern_string(text.as_bytes().to_vec())?;
    Ok(vec![Value::string(handle)])
}

pub fn native_tostring(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.arg_or_nil(args, 0);
    let text = vm.format_tostring(value)?;
    let handle = vm.heap.intern_string(text.into_bytes())?;
    Ok(vec![Value::string(handle)])
}

pub fn native_tonumber(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.arg_or_nil(args, 0);
    let base = match vm.arg_or_nil(args, 1) {
        Value::Integer(value) if (2..=36).contains(&value) => Some(value as u32),
        Value::Nil => None,
        other => {
            return Err(vm.type_error("tonumber expects an integer base", other));
        }
    };

    let result = match (value, base) {
        (Value::Integer(value), None) => Some(Value::integer(value)),
        (Value::Number(value), None) => Some(Value::number(value)),
        (Value::String(handle), None) => {
            let bytes = vm.string_bytes_from_handle(handle)?;
            match std::str::from_utf8(&bytes) {
                Ok(text) => {
                    let text = text.trim();
                    if matches!(text.to_ascii_lowercase().as_str(), "inf" | "nan") {
                        return Ok(vec![Value::nil()]);
                    }
                    let (negative, digits) = match text.strip_prefix('-') {
                        Some(value) => (true, value),
                        None => (false, text.strip_prefix('+').unwrap_or(text)),
                    };
                    let hex_float = digits
                        .strip_prefix("0x")
                        .or_else(|| digits.strip_prefix("0X"))
                        .filter(|hex| hex.contains(['.', 'p', 'P']))
                        .and_then(Vm::parse_hex_number_text)
                        .map(|number| if negative { -number } else { number });
                    if let Some(number) = hex_float {
                        Some(Value::number(number))
                    } else {
                        match text.parse::<i64>() {
                            Ok(integer) => Some(Value::integer(integer)),
                            Err(_) => match Vm::parse_integer_text(text) {
                                Ok(integer) => Some(Value::integer(integer)),
                                Err(_) => match text.parse::<f64>() {
                                    Ok(number) => Some(Value::number(number)),
                                    Err(_) => None,
                                },
                            },
                        }
                    }
                }
                Err(_) => None,
            }
        }
        (Value::String(handle), Some(base)) => {
            let bytes = vm.string_bytes_from_handle(handle)?;
            match std::str::from_utf8(&bytes) {
                Ok(text) => i64::from_str_radix(text.trim(), base)
                    .ok()
                    .map(Value::integer),
                Err(_) => None,
            }
        }
        _ => None,
    };

    Ok(vec![result.unwrap_or(Value::nil())])
}

pub fn native_pcall(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let Some((&callee, rest)) = args.split_first() else {
        return Err(KError::new(
            KErrorKind::Runtime("pcall expects a function".to_owned()),
            None,
        ));
    };
    match vm.call_value_multi(callee, rest.to_vec()) {
        Ok(values) => {
            let mut out = Vec::with_capacity(values.len() + 1);
            out.push(Value::boolean(true));
            out.extend(values);
            Ok(out)
        }
        Err(error) => {
            let text = match error.kind() {
                KErrorKind::Runtime(message) => message.clone(),
                _ => error.to_string(),
            };
            let message = vm.heap.intern_string(text.into_bytes())?;
            Ok(vec![Value::boolean(false), Value::string(message)])
        }
    }
}

pub fn native_xpcall(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let Some((&callee, rest)) = args.split_first() else {
        return Err(KError::new(
            KErrorKind::Runtime("xpcall expects a function".to_owned()),
            None,
        ));
    };
    let Some((&handler, call_args)) = rest.split_first() else {
        return Err(KError::new(
            KErrorKind::Runtime("xpcall expects an error handler".to_owned()),
            None,
        ));
    };
    match vm.call_value_multi(callee, call_args.to_vec()) {
        Ok(values) => {
            let mut out = Vec::with_capacity(values.len() + 1);
            out.push(Value::boolean(true));
            out.extend(values);
            Ok(out)
        }
        Err(error) => {
            let message = vm.heap.intern_string(error.to_string().into_bytes())?;
            match vm.call_value_multi(handler, vec![Value::string(message)]) {
                Ok(handler_result) => {
                    let mut out = Vec::with_capacity(handler_result.len() + 1);
                    out.push(Value::boolean(false));
                    out.extend(handler_result);
                    Ok(out)
                }
                Err(handler_error) => {
                    let message = vm
                        .heap
                        .intern_string(handler_error.to_string().into_bytes())?;
                    Ok(vec![Value::boolean(false), Value::string(message)])
                }
            }
        }
    }
}

pub fn native_load(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let source = match vm.arg_or_nil(args, 0) {
        Value::String(handle) => vm.string_bytes_from_handle(handle)?,
        _ => {
            return Err(Vm::runtime_error(
                "load currently supports string chunks only".to_owned(),
            ));
        }
    };
    let mode = vm.optional_string_text_arg(args, 2)?;
    let env = match vm.arg_or_nil(args, 3) {
        Value::Nil => None,
        value => Some(value),
    };
    match vm.load_chunk_bytes(&source, mode.as_deref(), env) {
        Ok(closure) => Ok(vec![Value::closure(closure)]),
        Err(error) => {
            let rendered = if let Some(span) = error.span() {
                format!(":{}: {error}", span.start_line)
            } else {
                error.to_string()
            };
            let message = vm.heap.intern_string(rendered.into_bytes())?;
            Ok(vec![Value::nil(), Value::string(message)])
        }
    }
}

pub fn native_loadfile(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let path = vm.string_text_arg_typed(args, 0, "loadfile expects a string path")?;
    let bytes = fs::read(&path)?;
    let mode = vm.optional_string_text_arg(args, 1)?;
    let env = match vm.arg_or_nil(args, 2) {
        Value::Nil => None,
        value => Some(value),
    };
    let closure = vm.load_chunk_bytes(&bytes, mode.as_deref(), env)?;
    Ok(vec![Value::closure(closure)])
}

pub fn native_dofile(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let path = vm.string_text_arg_typed(args, 0, "dofile expects a file path")?;
    let bytes = fs::read(&path)?;
    let closure = vm.load_chunk_bytes(&bytes, None, None)?;
    vm.call_value_multi(Value::closure(closure), Vec::new())
}

pub fn native_warn(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    if args.is_empty() {
        return Err(KError::new(
            KErrorKind::Runtime("warn expects at least one argument".to_owned()),
            None,
        ));
    }

    let mut text = String::new();
    for value in args {
        text.push_str(&vm.format_value(*value)?);
    }

    if args.len() == 1 && text.starts_with('@') {
        if text == "@off" {
            vm.warn_enabled = false;
        } else if text == "@on" {
            vm.warn_enabled = true;
        }
        return Ok(Vec::new());
    }

    if vm.warn_enabled {
        eprintln!("Lua warning: {text}");
    }
    Ok(Vec::new())
}

pub fn native_next(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = vm.table_arg(args, 0, "next expects a table")?;
    let key = args.get(1).copied();
    let table = vm.heap.resolve_table(table)?;
    Ok(match table.next_key(key) {
        Some((next_key, value)) => vec![next_key, value],
        None => Vec::new(),
    })
}

pub fn native_pairs(_vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = _vm.arg_or_nil(args, 0);
    Ok(vec![Value::native(native_next), table, Value::nil()])
}

pub fn native_ipairs(_vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = _vm.arg_or_nil(args, 0);
    Ok(vec![
        Value::native(native_ipairs_next),
        table,
        Value::integer(0),
    ])
}

pub fn native_ipairs_next(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = match vm.arg_or_nil(args, 0) {
        Value::Table(handle) => handle,
        _ => return Ok(Vec::new()),
    };
    let index = match vm.arg_or_nil(args, 1) {
        Value::Integer(value) if value >= 0 => value,
        _ => 0,
    };
    let next_index = index.saturating_add(1);
    let table = vm.heap.resolve_table(table)?;
    let value = table.raw_get(Value::integer(next_index))?;
    if matches!(value, Value::Nil) {
        return Ok(Vec::new());
    }
    Ok(vec![Value::integer(next_index), value])
}

macro_rules! stub_native {
    ($($name:ident),+ $(,)?) => {
        $(
            pub fn $name(_vm: &mut Vm, _args: &[Value]) -> KResult<Vec<Value>> {
                Err(KError::new(
                    KErrorKind::Runtime(concat!(stringify!($name), " is not yet implemented").to_owned()),
                    None,
                ))
            }
        )+
    };
}

pub fn native_table_concat(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = vm.table_arg(args, 0, "table expected")?;
    let separator = match vm.arg_or_nil(args, 1) {
        Value::Nil => String::new(),
        Value::String(handle) => vm.string_text_from_handle(handle)?,
        value => return Err(vm.type_error("string expected", value)),
    };
    let start = match vm.arg_or_nil(args, 2) {
        Value::Nil => 1,
        _ => vm.integer_arg(args, 2, "integer expected")?,
    };
    let end = match vm.arg_or_nil(args, 3) {
        Value::Nil => i64::try_from(vm.heap.resolve_table(table)?.array.len()).unwrap_or(i64::MAX),
        _ => vm.integer_arg(args, 3, "integer expected")?,
    };
    let mut pieces = Vec::new();
    for index in start..=end {
        let value = vm
            .heap
            .resolve_table(table)?
            .raw_get(Value::integer(index))?;
        match value {
            Value::String(handle) => pieces.push(vm.string_text_from_handle(handle)?),
            Value::Integer(value) => pieces.push(value.to_string()),
            Value::Number(value) => pieces.push(value.to_string()),
            value => return Err(vm.type_error("string expected", value)),
        }
    }
    let handle = vm
        .heap
        .intern_string(pieces.join(&separator).into_bytes())?;
    Ok(vec![Value::string(handle)])
}

pub fn native_table_unpack(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let table = vm.table_arg(args, 0, "table expected")?;
    let start = match vm.arg_or_nil(args, 1) {
        Value::Nil => 1,
        _ => vm.integer_arg(args, 1, "integer expected")?,
    };
    let end = match vm.arg_or_nil(args, 2) {
        Value::Nil => i64::try_from(vm.heap.resolve_table(table)?.array.len()).unwrap_or(i64::MAX),
        _ => vm.integer_arg(args, 2, "integer expected")?,
    };
    let mut values = Vec::new();
    for index in start..=end {
        values.push(
            vm.heap
                .resolve_table(table)?
                .raw_get(Value::integer(index))?,
        );
    }
    Ok(values)
}

pub fn native_string_dump(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let closure = match vm.arg_or_nil(args, 0) {
        Value::Closure(handle) => handle,
        other => {
            return Err(KError::new(
                KErrorKind::Runtime(format!(
                    "string.dump expects a Lua function, got {}",
                    vm.value_type_name(other)
                )),
                None,
            ));
        }
    };
    let proto = &vm.heap.resolve_closure(closure)?.proto;
    let mut bytes = b"KUUBIN\0".to_vec();
    bytes.extend(proto.encode()?);
    let handle = vm.heap.intern_string(bytes)?;
    Ok(vec![Value::string(handle)])
}

pub fn native_string_find(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let text = vm.string_text_arg(args, 0)?;
    let pattern = vm.string_bytes_arg_typed(args, 1, "string expected")?;
    let plain = matches!(vm.arg_or_nil(args, 3), Value::Boolean(true));
    let start = match vm.arg_or_nil(args, 2) {
        Value::Nil => 1,
        Value::Integer(value) => value,
        _ => return Err(Vm::runtime_error("integer expected")),
    };
    let start_index = normalize_lua_index(start, i64::try_from(text.len()).unwrap_or(i64::MAX), 1);
    let start_index = usize::try_from(start_index.saturating_sub(1)).unwrap_or(0);
    let haystack = text.as_bytes();
    if pattern.is_empty() {
        let pos = i64::try_from(start_index + 1)
            .map_err(|_| KError::new(KErrorKind::Runtime("position overflow".to_owned()), None))?;
        return Ok(vec![Value::integer(pos), Value::integer(pos - 1)]);
    }
    let needle = if plain {
        pattern
    } else {
        let mut unescaped = Vec::with_capacity(pattern.len());
        let mut bytes = pattern.iter().copied();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                if let Some(escaped) = bytes.next() {
                    unescaped.push(escaped);
                } else {
                    unescaped.push(byte);
                }
            } else {
                unescaped.push(byte);
            }
        }
        unescaped
    };
    let haystack = haystack.get(start_index..).unwrap_or(&[]);
    let search =
        if !plain && let Some(wildcard) = needle.windows(2).position(|window| window == b".*") {
            let prefix = needle.get(..wildcard).unwrap_or(&[]);
            let suffix = needle.get(wildcard + 2..).unwrap_or(&[]);
            haystack
                .windows(prefix.len())
                .enumerate()
                .find_map(|(start, window)| {
                    (window == prefix).then(|| {
                        let suffix_start = start + prefix.len();
                        let suffix_offset = haystack
                            .get(suffix_start..)
                            .unwrap_or(&[])
                            .windows(suffix.len())
                            .position(|window| window == suffix)
                            .unwrap_or(usize::MAX);
                        (start, suffix_start, suffix_offset)
                    })
                })
                .and_then(|(start, suffix_start, suffix_offset)| {
                    (suffix_offset != usize::MAX)
                        .then_some((start, suffix_start + suffix_offset + suffix.len()))
                })
        } else {
            haystack
                .windows(needle.len())
                .position(|window| window == needle.as_slice())
                .map(|start| (start, start + needle.len()))
        };
    if let Some((pos, end)) = search {
        let first = start_index + pos + 1;
        let last = start_index + end;
        Ok(vec![
            Value::integer(i64::try_from(first).map_err(|_| {
                KError::new(KErrorKind::Runtime("position overflow".to_owned()), None)
            })?),
            Value::integer(i64::try_from(last).map_err(|_| {
                KError::new(KErrorKind::Runtime("position overflow".to_owned()), None)
            })?),
        ])
    } else {
        Ok(vec![Value::nil()])
    }
}

pub fn native_string_match(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let text = vm.string_text_arg(args, 0)?;
    let pattern = vm.string_bytes_arg_typed(args, 1, "string expected")?;
    let start = match vm.arg_or_nil(args, 2) {
        Value::Nil => 1,
        Value::Integer(value) => value,
        _ => return Err(Vm::runtime_error("integer expected")),
    };
    let start_index = normalize_lua_index(start, i64::try_from(text.len()).unwrap_or(i64::MAX), 1);
    let start_index = usize::try_from(start_index.saturating_sub(1)).unwrap_or(0);
    let haystack = text.as_bytes();
    let haystack = haystack.get(start_index..).unwrap_or(&[]);
    if haystack.starts_with(&pattern) {
        let matched = haystack.get(..pattern.len()).unwrap_or(&[]);
        let handle = vm.heap.intern_string(matched.to_vec())?;
        Ok(vec![Value::string(handle)])
    } else {
        Ok(Vec::new())
    }
}

pub fn native_string_gsub(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let text = vm.string_text_arg(args, 0)?;
    let pattern = vm.string_bytes_arg_typed(args, 1, "string expected")?;
    let replacement = vm.arg_or_nil(args, 2);
    if pattern == b"[^\n]" && matches!(replacement, Value::String(_)) {
        let out = text
            .chars()
            .filter(|character| *character == '\n')
            .collect::<String>();
        let count = i64::try_from(text.chars().filter(|character| *character != '\n').count())
            .unwrap_or(i64::MAX);
        let handle = vm.heap.intern_string(out.into_bytes())?;
        return Ok(vec![Value::string(handle), Value::integer(count)]);
    }
    if pattern == b"^0*(%d.-%d)0*$" && matches!(replacement, Value::String(_)) {
        let mut trimmed = text
            .trim_start_matches('0')
            .trim_end_matches('0')
            .to_owned();
        if trimmed.starts_with('.') {
            trimmed.insert(0, '0');
        }
        if trimmed.ends_with('.') {
            trimmed.pop();
        }
        if trimmed.is_empty() {
            trimmed.push('0');
        }
        let handle = vm.heap.intern_string(trimmed.as_bytes().to_vec())?;
        return Ok(vec![Value::string(handle), Value::integer(1)]);
    }
    let limit = match vm.arg_or_nil(args, 3) {
        Value::Integer(value) if value >= 0 => usize::try_from(value).unwrap_or(usize::MAX),
        Value::Nil => usize::MAX,
        _ => {
            return Err(Vm::runtime_error("integer expected"));
        }
    };
    let mut remaining = text.as_str();
    let mut out = String::new();
    let mut count = 0usize;
    while count < limit {
        let Some((pos, matched_len)) = (if pattern == b"%d$" {
            remaining
                .as_bytes()
                .last()
                .filter(|byte| byte.is_ascii_digit())
                .map(|_| (remaining.len().saturating_sub(1), 1))
        } else {
            remaining
                .as_bytes()
                .windows(pattern.len())
                .position(|window| window == pattern.as_slice())
                .map(|pos| (pos, pattern.len()))
        }) else {
            break;
        };
        out.push_str(&remaining[..pos]);
        let replacement_text = match replacement {
            Value::String(handle) => vm.heap.string_bytes(handle).map_or_else(
                || {
                    Err(KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    ))
                },
                |bytes| Ok(String::from_utf8_lossy(bytes).into_owned()),
            )?,
            Value::Nil => String::new(),
            Value::Closure(_) | Value::NativeFunction(_) => {
                let matched = remaining
                    .as_bytes()
                    .get(pos..pos + matched_len)
                    .unwrap_or(&[]);
                let handle = vm.heap.intern_string(matched.to_vec())?;
                let result = vm.call_value_sync(replacement, vec![Value::string(handle)], 0)?;
                match result {
                    Value::Nil | Value::Boolean(false) => {
                        String::from_utf8_lossy(matched).into_owned()
                    }
                    value => vm.format_value(value)?,
                }
            }
            other => vm.format_value(other)?,
        };
        out.push_str(&replacement_text);
        remaining = &remaining[pos + matched_len..];
        count = count.saturating_add(1);
    }
    out.push_str(remaining);
    let handle = vm.heap.intern_string(out.into_bytes())?;
    Ok(vec![
        Value::string(handle),
        Value::integer(i64::try_from(count).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("replacement count overflow".to_owned()),
                None,
            )
        })?),
    ])
}

pub fn native_string_len(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.arg_or_nil(args, 0);
    let bytes = match value {
        Value::String(handle) => vm.string_bytes_from_handle(handle)?,
        _ => {
            return Err(KError::new(
                KErrorKind::Runtime("string.len expects a string".to_owned()),
                None,
            ));
        }
    };
    Ok(vec![Value::integer(i64::try_from(bytes.len()).map_err(
        |_| {
            KError::new(
                KErrorKind::Runtime("string length overflow".to_owned()),
                None,
            )
        },
    )?)])
}

pub fn native_string_sub(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let bytes = match vm.arg_or_nil(args, 0) {
        Value::String(handle) => vm.string_bytes_from_handle(handle)?,
        other => return Err(vm.type_error("string.sub expects a string", other)),
    };
    let len = i64::try_from(bytes.len()).map_err(|_| {
        KError::new(
            KErrorKind::Runtime("string length overflow".to_owned()),
            None,
        )
    })?;
    let start = match vm.arg_or_nil(args, 1) {
        Value::Integer(value) => normalize_lua_index(value, len, 1),
        _ => 1,
    };
    let end = match vm.arg_or_nil(args, 2) {
        Value::Integer(value) => normalize_lua_index(value, len, len),
        _ => len,
    };
    if start > end || start > len {
        let handle = vm.heap.intern_string(Vec::new())?;
        return Ok(vec![Value::string(handle)]);
    }
    let start = usize::try_from(start.saturating_sub(1)).unwrap_or(0);
    let end = usize::try_from(end.min(len)).unwrap_or(bytes.len());
    let slice = bytes.get(start..end).unwrap_or(&[]);
    let handle = vm.heap.intern_string(slice.to_vec())?;
    Ok(vec![Value::string(handle)])
}

pub fn native_string_byte(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let string = match vm.arg_or_nil(args, 0) {
        Value::String(handle) => handle,
        _ => return Ok(Vec::new()),
    };
    let bytes = vm.string_bytes_from_handle(string)?;
    let start = match vm.arg_or_nil(args, 1) {
        Value::Integer(value) if value >= 1 => value,
        _ => 1,
    };
    let end = match vm.arg_or_nil(args, 2) {
        Value::Integer(value) if value >= start => value,
        _ => start,
    };
    let mut out = Vec::new();
    for index in start..=end {
        let slot = match usize::try_from(index - 1) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(byte) = bytes.get(slot).copied() {
            out.push(Value::integer(i64::from(byte)));
        }
    }
    Ok(out)
}

pub fn native_string_char(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let mut bytes = Vec::new();
    for value in args {
        let integer = match value {
            Value::Integer(value) if (0..=255).contains(value) => *value as u8,
            Value::Number(value) if value.fract() == 0.0 && (0.0..=255.0).contains(value) => {
                *value as u8
            }
            other => {
                return Err(KError::new(
                    KErrorKind::Runtime(format!(
                        "string.char expects byte values, got {}",
                        vm.value_type_name(*other)
                    )),
                    None,
                ));
            }
        };
        bytes.push(integer);
    }
    let handle = vm.heap.intern_string(bytes)?;
    Ok(vec![Value::string(handle)])
}

pub fn native_string_rep(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.arg_or_nil(args, 0);
    let times = match vm.arg_or_nil(args, 1) {
        Value::Integer(value) if value >= 0 => usize::try_from(value).unwrap_or(0),
        _ => {
            return Err(KError::new(
                KErrorKind::Runtime("string.rep expects a non-negative integer count".to_owned()),
                None,
            ));
        }
    };
    let sep = match vm.arg_or_nil(args, 2) {
        Value::String(handle) => vm.string_bytes_from_handle(handle).unwrap_or_default(),
        _ => Vec::new(),
    };
    let piece = match value {
        Value::String(handle) => vm.string_bytes_from_handle(handle).unwrap_or_default(),
        other => vm.format_value(other)?.into_bytes(),
    };
    let mut bytes = Vec::new();
    for index in 0..times {
        if index > 0 {
            bytes.extend_from_slice(&sep);
        }
        bytes.extend_from_slice(&piece);
    }
    let handle = vm.heap.intern_string(bytes)?;
    Ok(vec![Value::string(handle)])
}

pub fn native_string_packsize(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let format = vm.string_text_arg(args, 0)?;
    if format == "j" || format == "n" {
        return Ok(vec![Value::integer(8)]);
    }
    Err(Vm::runtime_error("unsupported pack format"))
}

pub fn native_string_pack(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let format = vm.string_text_arg(args, 0)?;
    let bytes = match format.as_str() {
        "j" => vm
            .integer_arg(args, 1, "integer expected")?
            .to_ne_bytes()
            .to_vec(),
        "n" => vm
            .number_arg(args, 1, "number expected")?
            .to_ne_bytes()
            .to_vec(),
        _ => return Err(Vm::runtime_error("unsupported pack format")),
    };
    let handle = vm.heap.intern_string(bytes)?;
    Ok(vec![Value::string(handle)])
}

pub fn native_string_unpack(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let format = vm.string_text_arg(args, 0)?;
    let bytes = vm.string_bytes_arg_typed(args, 1, "string expected")?;
    let start = match vm.arg_or_nil(args, 2) {
        Value::Nil => 1,
        Value::Integer(value) if value >= 1 => value,
        _ => return Err(Vm::runtime_error("integer expected")),
    };
    let start = usize::try_from(start.saturating_sub(1)).unwrap_or(usize::MAX);
    let slice = bytes
        .get(start..start.saturating_add(8))
        .ok_or_else(|| Vm::runtime_error("data string too short"))?;
    let array: [u8; 8] = slice
        .try_into()
        .map_err(|_| Vm::runtime_error("data string too short"))?;
    let value = match format.as_str() {
        "j" => Value::integer(i64::from_ne_bytes(array)),
        "n" => Value::number(f64::from_ne_bytes(array)),
        _ => return Err(Vm::runtime_error("unsupported pack format")),
    };
    let next = i64::try_from(start.saturating_add(9)).unwrap_or(i64::MAX);
    Ok(vec![value, Value::integer(next)])
}

pub fn native_string_lower(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    string_case(vm, args, |text| text.to_ascii_lowercase())
}

pub fn native_string_upper(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    string_case(vm, args, |text| text.to_ascii_uppercase())
}

pub fn native_string_reverse(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let text = vm.string_text_arg(args, 0)?;
    let mut bytes = text.into_bytes();
    bytes.reverse();
    let handle = vm.heap.intern_string(bytes)?;
    Ok(vec![Value::string(handle)])
}

pub fn native_string_format(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let fmt = vm.string_text_arg(args, 0)?;
    let mut out = String::new();
    let bytes = fmt.as_bytes();
    let mut index = 0usize;
    let mut arg_index = 1usize;
    while let Some(&byte) = bytes.get(index) {
        if byte != b'%' {
            out.push(char::from(byte));
            index = index.saturating_add(1);
            continue;
        }
        index = index.saturating_add(1);
        let Some(&next) = bytes.get(index) else {
            return Err(KError::new(
                KErrorKind::Runtime("invalid format string".to_owned()),
                None,
            ));
        };
        if next == b'%' {
            out.push('%');
            index = index.saturating_add(1);
            continue;
        }

        let mut left_align = false;
        let mut zero_pad = false;
        loop {
            match bytes.get(index).copied() {
                Some(b'-') => {
                    left_align = true;
                    index = index.saturating_add(1);
                }
                Some(b'0') => {
                    zero_pad = true;
                    index = index.saturating_add(1);
                }
                Some(b'+') | Some(b' ') | Some(b'#') => {
                    index = index.saturating_add(1);
                }
                _ => break,
            }
        }

        let mut width = 0usize;
        let mut has_width = false;
        while let Some(&digit) = bytes.get(index) {
            if digit.is_ascii_digit() {
                has_width = true;
                width = width
                    .saturating_mul(10)
                    .saturating_add(usize::from(digit - b'0'));
                index = index.saturating_add(1);
            } else {
                break;
            }
        }

        let mut precision = None;
        if matches!(bytes.get(index), Some(b'.')) {
            index = index.saturating_add(1);
            let mut value = 0usize;
            let mut seen = false;
            while let Some(&digit) = bytes.get(index) {
                if digit.is_ascii_digit() {
                    seen = true;
                    value = value
                        .saturating_mul(10)
                        .saturating_add(usize::from(digit - b'0'));
                    index = index.saturating_add(1);
                } else {
                    break;
                }
            }
            precision = Some(if seen { value } else { 0 });
        }

        while matches!(
            bytes.get(index).copied(),
            Some(b'h' | b'l' | b'L' | b'z' | b'j' | b't')
        ) {
            index = index.saturating_add(1);
        }

        let Some(spec) = bytes.get(index).copied() else {
            return Err(KError::new(
                KErrorKind::Runtime("invalid format string".to_owned()),
                None,
            ));
        };
        index = index.saturating_add(1);

        let value = args
            .get(arg_index)
            .copied()
            .ok_or_else(|| KError::new(KErrorKind::Runtime("value expected".to_owned()), None))?;
        arg_index = arg_index.saturating_add(1);

        let formatted = match spec {
            b's' => {
                let mut text = match value {
                    Value::String(handle) => vm
                        .heap
                        .string_bytes(handle)
                        .ok_or_else(|| {
                            KError::new(
                                KErrorKind::Runtime("invalid string handle".to_owned()),
                                None,
                            )
                        })
                        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())?,
                    other => vm.format_value(other)?,
                };
                if let Some(limit) = precision {
                    text = text.chars().take(limit).collect();
                }
                if has_width && text.len() < width {
                    let pad = width - text.len();
                    if left_align {
                        text.push_str(&" ".repeat(pad));
                    } else {
                        let fill = if zero_pad { '0' } else { ' ' };
                        text = format!("{}{}", fill.to_string().repeat(pad), text);
                    }
                }
                text
            }
            b'd' | b'i' => {
                let integer = vm.value_to_integer(value)?.ok_or_else(|| {
                    KError::new(KErrorKind::Runtime("integer expected".to_owned()), None)
                })?;
                let mut text = integer.to_string();
                if has_width && text.len() < width {
                    let pad = width - text.len();
                    if left_align {
                        text.push_str(&" ".repeat(pad));
                    } else {
                        let fill = if zero_pad { '0' } else { ' ' };
                        text = format!("{}{}", fill.to_string().repeat(pad), text);
                    }
                }
                text
            }
            b'f' => {
                let number = vm.value_to_number(value)?;
                match precision {
                    Some(precision) => format!("{number:.precision$}"),
                    None => number.to_string(),
                }
            }
            b'e' => {
                let number = vm.value_to_number(value)?;
                match precision {
                    Some(precision) => format!("{number:.precision$e}"),
                    None => format!("{number:e}"),
                }
            }
            b'g' | b'G' => {
                let number = vm.value_to_number(value)?;
                match precision {
                    Some(precision) => format!("{number:.precision$}"),
                    None => number.to_string(),
                }
            }
            b'q' => quote_lua_string(vm, value)?,
            b'c' => {
                let integer = vm.value_to_integer(value)?.ok_or_else(|| {
                    KError::new(KErrorKind::Runtime("integer expected".to_owned()), None)
                })?;
                let byte = u8::try_from(integer).map_err(|_| {
                    KError::new(KErrorKind::Runtime("byte expected".to_owned()), None)
                })?;
                char::from(byte).to_string()
            }
            other => {
                return Err(KError::new(
                    KErrorKind::Runtime(format!("invalid format option '{}'", char::from(other))),
                    None,
                ));
            }
        };
        out.push_str(&formatted);
    }
    let handle = vm.heap.intern_string(out.into_bytes())?;
    Ok(vec![Value::string(handle)])
}

fn math_number_value(vm: &Vm, value: Value) -> KResult<f64> {
    vm.value_to_number(value)
}

fn math_integer_value(vm: &Vm, value: Value) -> KResult<Option<i64>> {
    vm.value_to_integer(value)
}

fn math_return_int_or_float(vm: &Vm, value: f64) -> KResult<Value> {
    if let Some(integer) = vm.value_to_integer(Value::number(value))? {
        Ok(Value::integer(integer))
    } else {
        Ok(Value::number(value))
    }
}

pub fn native_math_abs(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.arg_or_nil(args, 0);
    if let Some(integer) = math_integer_value(vm, value)? {
        let abs = if integer < 0 {
            0u64.wrapping_sub(integer as u64) as i64
        } else {
            integer
        };
        return Ok(vec![Value::integer(abs)]);
    }
    let value = math_number_value(vm, value)?.abs();
    Ok(vec![Value::number(value)])
}

pub fn native_math_ceil(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    if let Value::Integer(value) = vm.arg_or_nil(args, 0) {
        return Ok(vec![Value::integer(value)]);
    }
    let value = vm.number_arg(args, 0, "number expected")?;
    if !value.is_finite() {
        return Ok(vec![Value::number(value)]);
    }
    let result = value.ceil();
    Ok(vec![math_return_int_or_float(vm, result)?])
}

pub fn native_math_floor(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    if let Value::Integer(value) = vm.arg_or_nil(args, 0) {
        return Ok(vec![Value::integer(value)]);
    }
    let value = vm.number_arg(args, 0, "number expected")?;
    if !value.is_finite() {
        return Ok(vec![Value::number(value)]);
    }
    let result = value.floor();
    Ok(vec![math_return_int_or_float(vm, result)?])
}

pub fn native_math_max(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let Some((&first, rest)) = args.split_first() else {
        return Err(KError::new(
            KErrorKind::Runtime("value expected".to_owned()),
            None,
        ));
    };
    let mut current = first;
    for &value in rest {
        current = if let Some(ordering) = vm.compare_values(current, value)? {
            if matches!(ordering, std::cmp::Ordering::Less) {
                value
            } else {
                current
            }
        } else {
            return Err(KError::new(
                KErrorKind::Runtime("value expected".to_owned()),
                None,
            ));
        };
    }
    Ok(vec![current])
}

pub fn native_math_min(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let Some((&first, rest)) = args.split_first() else {
        return Err(KError::new(
            KErrorKind::Runtime("value expected".to_owned()),
            None,
        ));
    };
    let mut current = first;
    for &value in rest {
        current = if let Some(ordering) = vm.compare_values(current, value)? {
            if matches!(ordering, std::cmp::Ordering::Greater) {
                value
            } else {
                current
            }
        } else {
            return Err(KError::new(
                KErrorKind::Runtime("value expected".to_owned()),
                None,
            ));
        };
    }
    Ok(vec![current])
}

pub fn native_math_random(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let rv = vm.math_next_u64();
    match args.len() {
        0 => Ok(vec![Value::number(
            (rv >> 11) as f64 * (1.0 / ((1u64 << 53) as f64)),
        )]),
        1 => {
            let upper = vm.integer_arg(args, 0, "integer expected")?;
            if upper == 0 {
                return Ok(vec![Value::integer(i64::from_ne_bytes(rv.to_ne_bytes()))]);
            }
            if upper < 0 {
                return Err(KError::new(
                    KErrorKind::Runtime("interval is empty".to_owned()),
                    None,
                ));
            }
            let result = vm.math_random_integer(1, i128::from(upper))?;
            Ok(vec![Value::integer(i64::try_from(result).map_err(
                |_| {
                    KError::new(
                        KErrorKind::Runtime("random result overflow".to_owned()),
                        None,
                    )
                },
            )?)])
        }
        2 => {
            let low = vm.integer_arg(args, 0, "integer expected")?;
            let high = vm.integer_arg(args, 1, "integer expected")?;
            if low > high {
                return Err(KError::new(
                    KErrorKind::Runtime("interval is empty".to_owned()),
                    None,
                ));
            }
            let result = vm.math_random_integer(i128::from(low), i128::from(high))?;
            Ok(vec![Value::integer(i64::try_from(result).map_err(
                |_| {
                    KError::new(
                        KErrorKind::Runtime("random result overflow".to_owned()),
                        None,
                    )
                },
            )?)])
        }
        _ => Err(KError::new(
            KErrorKind::Runtime("wrong number of arguments".to_owned()),
            None,
        )),
    }
}

pub fn native_math_randomseed(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    if args.is_empty() {
        let seed1 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| {
                KError::new(
                    KErrorKind::Runtime("system time before UNIX_EPOCH".to_owned()),
                    None,
                )
            })?
            .as_secs();
        let seed2 = (vm as *const Vm as usize) as u64;
        vm.math_seed_rng(seed1, seed2);
        return Ok(vec![
            Value::integer(i64::from_ne_bytes(seed1.to_ne_bytes())),
            Value::integer(i64::from_ne_bytes(seed2.to_ne_bytes())),
        ]);
    }
    let seed_x = u64::from_ne_bytes(vm.integer_arg(args, 0, "integer expected")?.to_ne_bytes());
    let seed_y = match vm.arg_or_nil(args, 1) {
        Value::Nil => 0,
        _ => u64::from_ne_bytes(vm.integer_arg(args, 1, "integer expected")?.to_ne_bytes()),
    };
    vm.math_seed_rng(seed_x, seed_y);
    Ok(vec![
        Value::integer(i64::from_ne_bytes(seed_x.to_ne_bytes())),
        Value::integer(i64::from_ne_bytes(seed_y.to_ne_bytes())),
    ])
}

pub fn native_math_tointeger(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    Ok(match math_integer_value(vm, vm.arg_or_nil(args, 0))? {
        Some(value) => vec![Value::integer(value)],
        None => vec![Value::nil()],
    })
}

pub fn native_math_type(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let result = match vm.arg_or_nil(args, 0) {
        Value::Integer(_) => Some("integer"),
        Value::Number(_) => Some("float"),
        _ => None,
    };
    Ok(match result {
        Some(text) => vec![Value::string(
            vm.heap.intern_string(text.as_bytes().to_vec())?,
        )],
        None => vec![Value::nil()],
    })
}

pub fn native_math_modf(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.number_arg(args, 0, "number expected")?;
    if value.is_nan() {
        return Ok(vec![Value::number(f64::NAN), Value::number(f64::NAN)]);
    }
    if value.is_infinite() {
        return Ok(vec![Value::number(value), Value::number(0.0)]);
    }
    let integral = value.trunc();
    let fractional = value - integral;
    let int_value = math_return_int_or_float(vm, integral)?;
    Ok(vec![int_value, Value::number(fractional)])
}

pub fn native_math_deg(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.number_arg(args, 0, "number expected")?;
    Ok(vec![Value::number(value * (180.0 / std::f64::consts::PI))])
}

pub fn native_math_rad(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.number_arg(args, 0, "number expected")?;
    Ok(vec![Value::number(value * (std::f64::consts::PI / 180.0))])
}

pub fn native_math_sqrt(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.number_arg(args, 0, "number expected")?;
    Ok(vec![Value::number(value.sqrt())])
}

pub fn native_math_sin(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    Ok(vec![Value::number(
        vm.number_arg(args, 0, "number expected")?.sin(),
    )])
}

pub fn native_math_cos(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    Ok(vec![Value::number(
        vm.number_arg(args, 0, "number expected")?.cos(),
    )])
}

pub fn native_math_tan(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    Ok(vec![Value::number(
        vm.number_arg(args, 0, "number expected")?.tan(),
    )])
}

pub fn native_math_asin(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    Ok(vec![Value::number(
        vm.number_arg(args, 0, "number expected")?.asin(),
    )])
}

pub fn native_math_acos(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    Ok(vec![Value::number(
        vm.number_arg(args, 0, "number expected")?.acos(),
    )])
}

pub fn native_math_atan(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let y = vm.number_arg(args, 0, "number expected")?;
    let value = match vm.arg_or_nil(args, 1) {
        Value::Nil => y.atan(),
        x => y.atan2(vm.number_arg(&[x], 0, "number expected")?),
    };
    Ok(vec![Value::number(value)])
}

pub fn native_math_exp(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    Ok(vec![Value::number(
        vm.number_arg(args, 0, "number expected")?.exp(),
    )])
}

pub fn native_math_log(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.number_arg(args, 0, "number expected")?;
    let result = match vm.arg_or_nil(args, 1) {
        Value::Nil => value.ln(),
        base => value.log(vm.number_arg(&[base], 0, "number expected")?),
    };
    Ok(vec![Value::number(result)])
}

pub fn native_math_fmod(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let left = vm.arg_or_nil(args, 0);
    let right = vm.arg_or_nil(args, 1);
    if let (Value::Integer(left), Value::Integer(right)) = (left, right) {
        let divisor = right;
        if divisor == 0 {
            return Err(KError::new(KErrorKind::Runtime("zero".to_owned()), None));
        }
        if divisor == -1 {
            return Ok(vec![Value::integer(0)]);
        }
        return Ok(vec![Value::integer(left % divisor)]);
    }
    let left = math_number_value(vm, left)?;
    let right = math_number_value(vm, right)?;
    Ok(vec![Value::number(left % right)])
}

pub fn native_math_ldexp(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.number_arg(args, 0, "number expected")?;
    let exp = vm.integer_arg(args, 1, "integer expected")?;
    Ok(vec![Value::number(value * 2f64.powi(exp as i32))])
}

pub fn native_math_frexp(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.number_arg(args, 0, "number expected")?;
    if value == 0.0 || !value.is_finite() {
        return Ok(vec![Value::number(value), Value::integer(0)]);
    }
    let bits = value.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let mantissa_bits = bits & 0x000f_ffff_ffff_ffff;
    let (mantissa, exponent) = if exponent_bits == 0 {
        let mut normalized = value;
        let mut exp = 0i32;
        while normalized.abs() < 0.5 {
            normalized *= 2.0;
            exp -= 1;
        }
        (normalized, exp)
    } else {
        let exp = exponent_bits - 1022;
        let mantissa_bits = (bits & (1u64 << 63)) | mantissa_bits | (0x3feu64 << 52);
        (f64::from_bits(mantissa_bits), exp)
    };
    Ok(vec![
        Value::number(mantissa),
        Value::integer(i64::from(exponent)),
    ])
}

pub fn native_math_ult(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let left = vm.integer_arg(args, 0, "integer expected")?;
    let right = vm.integer_arg(args, 1, "integer expected")?;
    Ok(vec![Value::boolean((left as u64) < (right as u64))])
}

pub fn native_os_clock(vm: &mut Vm, _args: &[Value]) -> KResult<Vec<Value>> {
    Ok(vec![Value::number(
        vm.start_instant.elapsed().as_secs_f64(),
    )])
}

pub fn native_os_execute(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let what = vm.heap.intern_string(b"exit".to_vec())?;
    let command_value = vm.arg_or_nil(args, 0);
    if matches!(command_value, Value::Nil) {
        return Ok(vec![
            Value::boolean(true),
            Value::string(what),
            Value::integer(0),
        ]);
    }
    let command = vm.string_text_arg(&[command_value], 0)?;
    let status = Command::new("sh")
        .arg("-c")
        .arg(command)
        .status()
        .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
    let code = status.code().unwrap_or(1);
    Ok(vec![
        Value::boolean(status.success()),
        Value::string(what),
        Value::integer(i64::from(code)),
    ])
}

pub fn native_os_difftime(_vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let t1 = _vm.number_arg(args, 0, "number expected")?;
    let t2 = _vm.number_arg(args, 1, "number expected")?;
    Ok(vec![Value::number(t1 - t2)])
}

pub fn native_os_time(_vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    if args.is_empty() || matches!(_vm.arg_or_nil(args, 0), Value::Nil) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| {
                KError::new(
                    KErrorKind::Runtime("system time before UNIX_EPOCH".to_owned()),
                    None,
                )
            })?;
        return Ok(vec![Value::integer(i64::try_from(now.as_secs()).map_err(
            |_| KError::new(KErrorKind::Runtime("time overflow".to_owned()), None),
        )?)]);
    }
    Err(KError::new(
        KErrorKind::Runtime("os.time table argument is not yet implemented".to_owned()),
        None,
    ))
}

pub fn native_os_tmpname(_vm: &mut Vm, _args: &[Value]) -> KResult<Vec<Value>> {
    static TMPNAME_COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| {
            KError::new(
                KErrorKind::Runtime("system time before UNIX_EPOCH".to_owned()),
                None,
            )
        })?
        .as_nanos();
    let counter = TMPNAME_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!("kuu-{now}-{counter}.tmp"));
    let handle = _vm
        .heap
        .intern_string(path.to_string_lossy().into_owned().into_bytes())?;
    Ok(vec![Value::string(handle)])
}

fn os_path_result(vm: &mut Vm, result: std::io::Result<()>) -> KResult<Vec<Value>> {
    match result {
        Ok(()) => Ok(vec![Value::boolean(true)]),
        Err(error) => {
            let message = error.to_string();
            let handle = vm.heap.intern_string(message.into_bytes())?;
            Ok(vec![Value::nil(), Value::string(handle), Value::integer(1)])
        }
    }
}

pub fn native_os_remove(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let path = vm.string_text_arg(args, 0)?;
    os_path_result(vm, fs::remove_file(path))
}

pub fn native_os_rename(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let source = vm.string_text_arg(args, 0)?;
    let destination = match vm.arg_or_nil(args, 1) {
        Value::Nil => return Err(Vm::runtime_error("filename expected")),
        value => vm.string_text_arg(&[value], 0)?,
    };
    os_path_result(vm, fs::rename(source, destination))
}

pub fn native_os_setlocale(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let locale = match vm.arg_or_nil(args, 0) {
        Value::Nil => vm.os_locale.clone(),
        Value::String(handle) => {
            let bytes = vm.string_bytes_from_handle(handle)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                KError::new(KErrorKind::Runtime("locale must be UTF-8".to_owned()), None)
            })?;
            if text == "C" || text == "C.UTF-8" || text.is_empty() {
                vm.os_locale = "C".to_owned();
                vm.os_locale.clone()
            } else {
                return Ok(vec![Value::nil()]);
            }
        }
        _ => {
            return Err(KError::new(
                KErrorKind::Runtime("locale string expected".to_owned()),
                None,
            ));
        }
    };
    Ok(vec![Value::string(
        vm.heap.intern_string(locale.into_bytes())?,
    )])
}

fn file_userdata_mut(vm: &mut Vm, handle: UserdataHandle) -> KResult<Rc<RefCell<UserdataObject>>> {
    vm.resolve_userdata(handle)
}

fn file_kind_display(kind: &FileKind) -> &'static str {
    match kind {
        FileKind::File(_) => "file",
        FileKind::Stdin | FileKind::Stdout | FileKind::Stderr => "file",
        FileKind::Closed => "closed file",
    }
}

fn file_close_handle(vm: &mut Vm, handle: UserdataHandle) -> KResult<bool> {
    let file = file_userdata_mut(vm, handle)?;
    let mut file = file.borrow_mut();
    match &mut *file {
        UserdataObject::File(entry) => match std::mem::replace(&mut entry.kind, FileKind::Closed) {
            FileKind::File(mut file) => {
                let _ = file.flush();
                Ok(true)
            }
            FileKind::Stdin | FileKind::Stdout | FileKind::Stderr => {
                entry.kind = FileKind::Stdin;
                Ok(false)
            }
            FileKind::Closed => Ok(false),
        },
        _ => Ok(false),
    }
}

fn file_write_bytes(vm: &mut Vm, handle: UserdataHandle, parts: &[Value]) -> KResult<bool> {
    let file = file_userdata_mut(vm, handle)?;
    let mut file = file.borrow_mut();
    let UserdataObject::File(entry) = &mut *file else {
        return Err(KError::new(
            KErrorKind::Runtime("invalid file handle".to_owned()),
            None,
        ));
    };
    let FileKind::File(writer) = &mut entry.kind else {
        return Err(KError::new(
            KErrorKind::Runtime("closed file".to_owned()),
            None,
        ));
    };

    for value in parts {
        let text = vm.format_value(*value)?;
        writer
            .write_all(text.as_bytes())
            .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
    }
    Ok(true)
}

pub fn native_io_open(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let path = vm.string_text_arg(args, 0)?;
    let mode = match vm.optional_string_text_arg(args, 1)? {
        Some(text) => text,
        None if args.get(1).is_none() => "r".to_owned(),
        None => return Err(Vm::runtime_error("invalid mode")),
    };

    let mut options = std::fs::OpenOptions::new();
    let mut read = false;
    let mut write = false;
    let mut append = false;
    let mut truncate = false;
    let mut create = false;
    let normalized = mode.replace('b', "");
    match normalized.as_str() {
        "r" => {
            read = true;
        }
        "w" => {
            write = true;
            truncate = true;
            create = true;
        }
        "a" => {
            write = true;
            append = true;
            create = true;
        }
        "r+" => {
            read = true;
            write = true;
        }
        "w+" => {
            read = true;
            write = true;
            truncate = true;
            create = true;
        }
        "a+" => {
            read = true;
            write = true;
            append = true;
            create = true;
        }
        _ => {
            return Err(KError::new(
                KErrorKind::Runtime("invalid mode".to_owned()),
                None,
            ));
        }
    }
    options
        .read(read)
        .write(write)
        .append(append)
        .truncate(truncate)
        .create(create);

    let file = options
        .open(path)
        .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
    let handle = vm.new_userdata(UserdataObject::File(FileObject {
        kind: FileKind::File(file),
    }))?;
    Ok(vec![Value::userdata(handle)])
}

pub fn native_io_type(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let value = vm.arg_or_nil(args, 0);
    let result = match value {
        Value::Userdata(handle) => {
            let file = vm.resolve_userdata(handle)?;
            let file = file.borrow();
            match &*file {
                UserdataObject::File(entry) => Some(file_kind_display(&entry.kind)),
                _ => None,
            }
        }
        _ => None,
    };
    Ok(match result {
        Some(text) => vec![Value::string(
            vm.heap.intern_string(text.as_bytes().to_vec())?,
        )],
        None => vec![Value::nil()],
    })
}

pub fn native_io_input(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    match vm.arg_or_nil(args, 0) {
        Value::Nil => Ok(vec![Value::userdata(vm.current_input)]),
        Value::String(handle) => {
            let path = String::from_utf8_lossy(&vm.string_bytes_from_handle(handle)?).into_owned();
            let file = std::fs::File::open(path)
                .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
            let handle = vm.new_userdata(UserdataObject::File(FileObject {
                kind: FileKind::File(file),
            }))?;
            vm.current_input = handle;
            Ok(vec![Value::userdata(handle)])
        }
        Value::Userdata(handle) => {
            vm.current_input = handle;
            Ok(vec![Value::userdata(handle)])
        }
        _ => Err(KError::new(
            KErrorKind::Runtime("invalid input file".to_owned()),
            None,
        )),
    }
}

pub fn native_io_output(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    match vm.arg_or_nil(args, 0) {
        Value::Nil => Ok(vec![Value::userdata(vm.current_output)]),
        Value::String(handle) => {
            let path = String::from_utf8_lossy(&vm.string_bytes_from_handle(handle)?).into_owned();
            let file = std::fs::File::create(path)
                .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
            let handle = vm.new_userdata(UserdataObject::File(FileObject {
                kind: FileKind::File(file),
            }))?;
            vm.current_output = handle;
            Ok(vec![Value::userdata(handle)])
        }
        Value::Userdata(handle) => {
            vm.current_output = handle;
            Ok(vec![Value::userdata(handle)])
        }
        _ => Err(KError::new(
            KErrorKind::Runtime("invalid output file".to_owned()),
            None,
        )),
    }
}

pub fn native_io_close(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let handle = match vm.arg_or_nil(args, 0) {
        Value::Userdata(handle) => handle,
        Value::Nil => vm.current_output,
        _ => {
            return Err(KError::new(
                KErrorKind::Runtime("file expected".to_owned()),
                None,
            ));
        }
    };
    if file_close_handle(vm, handle)? {
        Ok(vec![Value::boolean(true)])
    } else {
        Ok(vec![Value::nil()])
    }
}

pub fn native_io_write(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let (handle, start) = match vm.arg_or_nil(args, 0) {
        Value::Userdata(handle) => (handle, 1usize),
        _ => (vm.current_output, 0usize),
    };
    let tail = args.get(start..).unwrap_or(&[]);
    let _ = file_write_bytes(vm, handle, tail)?;
    Ok(vec![Value::userdata(handle)])
}

pub fn native_io_read(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let (handle, start) = match vm.arg_or_nil(args, 0) {
        Value::Userdata(handle) => (handle, 1usize),
        _ => (vm.current_input, 0usize),
    };
    let file = file_userdata_mut(vm, handle)?;
    let mut file = file.borrow_mut();
    let UserdataObject::File(entry) = &mut *file else {
        return Err(KError::new(
            KErrorKind::Runtime("invalid file handle".to_owned()),
            None,
        ));
    };
    let FileKind::File(reader) = &mut entry.kind else {
        return Ok(Vec::new());
    };

    let mode = args.get(start).copied();
    let mut out = Vec::new();
    match mode {
        None | Some(Value::Nil) => {
            let mut byte = [0u8; 1];
            loop {
                let read = reader
                    .read(&mut byte)
                    .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
                if read == 0 || byte[0] == b'\n' {
                    break;
                }
                out.push(byte[0]);
            }
        }
        Some(Value::String(handle)) => {
            let bytes = vm.string_bytes_from_handle(handle)?;
            let mode = String::from_utf8_lossy(&bytes);
            match mode.as_ref() {
                "a" | "*a" => {
                    reader.read_to_end(&mut out).map_err(|error| {
                        KError::new(KErrorKind::Runtime(error.to_string()), None)
                    })?;
                    let handle = vm.heap.intern_string(out)?;
                    return Ok(vec![Value::string(handle)]);
                }
                "l" | "*l" => {
                    let mut byte = [0u8; 1];
                    loop {
                        let read = reader.read(&mut byte).map_err(|error| {
                            KError::new(KErrorKind::Runtime(error.to_string()), None)
                        })?;
                        if read == 0 || byte[0] == b'\n' {
                            break;
                        }
                        if byte[0] != b'\r' {
                            out.push(byte[0]);
                        }
                    }
                }
                "n" | "*n" => {
                    let mut byte = [0u8; 1];
                    let mut text = String::new();
                    loop {
                        let read = reader.read(&mut byte).map_err(|error| {
                            KError::new(KErrorKind::Runtime(error.to_string()), None)
                        })?;
                        if read == 0 || byte[0] == b'\n' {
                            break;
                        }
                        if byte[0] != b'\r' {
                            text.push(char::from(byte[0]));
                        }
                    }
                    let number = text.trim().parse::<f64>().map_err(|_| {
                        KError::new(KErrorKind::Runtime("number expected".to_owned()), None)
                    })?;
                    return Ok(vec![Value::number(number)]);
                }
                _ => {
                    return Err(KError::new(
                        KErrorKind::Runtime("invalid read mode".to_owned()),
                        None,
                    ));
                }
            }
        }
        Some(Value::Integer(count)) if count >= 0 => {
            let mut buffer = vec![
                0u8;
                usize::try_from(count).map_err(|_| {
                    KError::new(KErrorKind::Runtime("read size overflow".to_owned()), None)
                })?
            ];
            let read = reader
                .read(&mut buffer)
                .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
            buffer.truncate(read);
            out = buffer;
        }
        Some(Value::Number(count)) if count >= 0.0 && count.fract() == 0.0 => {
            let size = usize::try_from(count as u64).map_err(|_| {
                KError::new(KErrorKind::Runtime("read size overflow".to_owned()), None)
            })?;
            let mut buffer = vec![0u8; size];
            let read = reader
                .read(&mut buffer)
                .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
            buffer.truncate(read);
            out = buffer;
        }
        Some(_) => {
            return Err(KError::new(
                KErrorKind::Runtime("invalid read mode".to_owned()),
                None,
            ));
        }
    }

    if out.is_empty() {
        Ok(Vec::new())
    } else {
        let handle = vm.heap.intern_string(out)?;
        Ok(vec![Value::string(handle)])
    }
}

pub fn native_io_tmpfile(vm: &mut Vm, _args: &[Value]) -> KResult<Vec<Value>> {
    let mut path = std::env::temp_dir();
    path.push(format!("kuu-tmp-{}.tmp", vm.math_next_u64()));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|error| KError::new(KErrorKind::Runtime(error.to_string()), None))?;
    let handle = vm.new_userdata(UserdataObject::File(FileObject {
        kind: FileKind::File(file),
    }))?;
    Ok(vec![Value::userdata(handle)])
}

pub fn native_package_searchpath(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let name = vm.string_text_arg(args, 0)?;
    let path = vm.string_text_arg_typed(args, 1, "package.searchpath expects a string path")?;

    match vm.search_package_path(&name, &path)? {
        Some(found) => {
            let handle = vm.heap.intern_string(found.into_bytes())?;
            Ok(vec![Value::string(handle)])
        }
        None => {
            let module_path = name.replace('.', "/");
            let attempts = path
                .split(';')
                .filter(|template| !template.is_empty())
                .map(|template| format!("\n\tno file '{}'", template.replace('?', &module_path)))
                .collect::<String>();
            let message = vm.heap.intern_string(attempts.into_bytes())?;
            Ok(vec![Value::nil(), Value::string(message)])
        }
    }
}

pub fn native_require(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let module = vm.string_text_arg(args, 0)?;
    Ok(vec![vm.require_module_value(&module)?])
}

pub fn native_debug_upvalueid(vm: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
    let closure = match vm.arg_or_nil(args, 0) {
        Value::Closure(handle) => handle,
        value => return Err(vm.type_error("function expected", value)),
    };
    let index = vm.integer_arg(args, 1, "upvalue index expected")?;
    if index < 1 {
        return Err(Vm::runtime_error("upvalue index out of range"));
    }
    let closure = vm.heap.resolve_closure(closure)?;
    let handle = closure
        .upvalues
        .get(
            usize::try_from(index - 1)
                .map_err(|_| Vm::runtime_error("upvalue index out of range"))?,
        )
        .copied()
        .ok_or_else(|| Vm::runtime_error("upvalue index out of range"))?;
    Ok(vec![Value::integer(i64::try_from(handle.raw()).map_err(
        |_| Vm::runtime_error("upvalue identity exceeds Lua integer range"),
    )?)])
}

stub_native!(
    native_string_gmatch,
    native_utf8_char,
    native_utf8_codes,
    native_utf8_codepoint,
    native_utf8_len,
    native_table_insert,
    native_table_remove,
    native_table_move,
    native_table_pack,
    native_table_sort,
    native_io_lines,
    native_package_loadlib,
    native_debug_traceback,
    native_debug_getinfo,
    native_debug_getupvalue,
    native_debug_setupvalue,
    native_debug_upvaluejoin,
    native_debug_getlocal,
    native_debug_setlocal,
    native_debug_gethook,
    native_debug_sethook,
    native_debug_getregistry,
);

fn string_text(vm: &Vm, args: &[Value]) -> KResult<String> {
    vm.string_text_arg(args, 0)
}

fn string_case<F>(vm: &mut Vm, args: &[Value], transform: F) -> KResult<Vec<Value>>
where
    F: FnOnce(String) -> String,
{
    let text = string_text(vm, args)?;
    let handle = vm.heap.intern_string(transform(text).into_bytes())?;
    Ok(vec![Value::string(handle)])
}

fn quote_lua_string(vm: &Vm, value: Value) -> KResult<String> {
    let text = match value {
        Value::String(handle) => {
            let bytes = vm.heap.string_bytes(handle).ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("invalid string handle".to_owned()),
                    None,
                )
            })?;
            String::from_utf8_lossy(bytes).into_owned()
        }
        other => vm.format_value(other)?,
    };
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            ch if ch.is_control() => out.push_str(&format!("\\{:03}", u32::from(ch))),
            ch => out.push(ch),
        }
    }
    out.push('"');
    Ok(out)
}

fn normalize_lua_index(value: i64, len: i64, default: i64) -> i64 {
    if value < 0 {
        (len + value + 1).max(1)
    } else if value == 0 {
        default
    } else {
        value.min(len)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GcMode {
    Incremental,
    Generational,
}

#[derive(Debug, Clone, Copy)]
struct GcParams {
    pause: usize,
    stepmul: usize,
    stepsize: usize,
}

impl Default for GcParams {
    fn default() -> Self {
        Self {
            pause: 100,
            stepmul: 100,
            stepsize: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct GcMetrics {
    last_step_work: usize,
    total_step_work: usize,
    completed_cycles: usize,
    finalized_objects: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GcPhase {
    #[default]
    Pause,
    Mark,
    Sweep,
    Finalize,
}

#[derive(Debug, Clone)]
pub struct Vm {
    heap: Heap,
    userdatas: Vec<Option<Rc<RefCell<UserdataObject>>>>,
    warn_enabled: bool,
    start_instant: std::time::Instant,
    os_locale: String,
    file_metatable: TableHandle,
    current_input: UserdataHandle,
    current_output: UserdataHandle,
    math_rng_state: [u64; 4],
    math_rng_seed: (u64, u64),
    stack: Vec<Value>,
    frames: Vec<Frame>,
    open_upvalues: Vec<UpvalueHandle>,
    #[allow(dead_code)]
    threads: Vec<Option<ThreadState>>,
    globals: TableHandle,
    string_metatable: TableHandle,
    gc_mode: GcMode,
    gc_running: bool,
    gc_params: GcParams,
    gc_phase: GcPhase,
    gc_gray_tables: Vec<TableHandle>,
    gc_gray_closures: Vec<ClosureHandle>,
    gc_sweep_cursor: usize,
    gc_finalize_queue: Vec<TableHandle>,
    gc_metrics: GcMetrics,
}

impl Vm {
    pub fn new() -> KResult<Self> {
        let mut heap = Heap::new();
        let globals = heap.new_table()?;
        let string_metatable = heap.new_table()?;
        let mut vm = Self {
            heap,
            userdatas: Vec::new(),
            warn_enabled: false,
            start_instant: std::time::Instant::now(),
            os_locale: "C".to_owned(),
            file_metatable: TableHandle::new(0),
            current_input: UserdataHandle::new(0),
            current_output: UserdataHandle::new(0),
            math_rng_state: [0; 4],
            math_rng_seed: (0, 0),
            stack: Vec::new(),
            frames: Vec::new(),
            open_upvalues: Vec::new(),
            threads: Vec::new(),
            globals,
            string_metatable,
            gc_mode: GcMode::Incremental,
            gc_running: true,
            gc_params: GcParams::default(),
            gc_phase: GcPhase::Pause,
            gc_gray_tables: Vec::new(),
            gc_gray_closures: Vec::new(),
            gc_sweep_cursor: 0,
            gc_finalize_queue: Vec::new(),
            gc_metrics: GcMetrics::default(),
        };
        vm.math_seed_rng(0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210);
        vm.install_stdlib()?;
        Ok(vm)
    }

    #[allow(dead_code)]
    fn take_active_execution(&mut self) -> ExecutionState {
        ExecutionState {
            stack: std::mem::take(&mut self.stack),
            frames: std::mem::take(&mut self.frames),
            open_upvalues: std::mem::take(&mut self.open_upvalues),
        }
    }

    #[allow(dead_code)]
    fn restore_active_execution(&mut self, execution: ExecutionState) {
        self.stack = execution.stack;
        self.frames = execution.frames;
        self.open_upvalues = execution.open_upvalues;
    }

    #[allow(dead_code)]
    fn thread_state_mut(&mut self, handle: ThreadHandle) -> KResult<&mut ThreadState> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("thread handle overflow".to_owned()),
                None,
            )
        })?;
        self.threads
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("invalid thread handle".to_owned()),
                    None,
                )
            })
    }

    pub fn run_proto(&mut self, proto: &Proto) -> KResult<Vec<Value>> {
        let closure = self.instantiate_root_closure(proto.clone())?;
        self.run_closure(closure)
    }

    pub fn collectgarbage_request(&mut self, args: &[Value]) -> KResult<Vec<Value>> {
        let operation = match self.arg_or_nil(args, 0) {
            Value::Nil => "collect".to_owned(),
            Value::String(handle) => {
                let bytes = self.string_bytes_from_handle(handle)?;
                std::str::from_utf8(&bytes)
                    .map_err(|_| {
                        KError::new(
                            KErrorKind::Runtime(
                                "collectgarbage expects a UTF-8 operation".to_owned(),
                            ),
                            None,
                        )
                    })?
                    .to_owned()
            }
            _ => {
                return Err(KError::new(
                    KErrorKind::Runtime("collectgarbage expects a string operation".to_owned()),
                    None,
                ));
            }
        };

        match operation.as_str() {
            "collect" | "collectgarbage" => {
                self.gc_collect_full()?;
                Ok(vec![Value::number(self.gc_count_kib())])
            }
            "count" => Ok(vec![Value::number(self.gc_count_kib())]),
            "step" => {
                let budget = match self.arg_or_nil(args, 1) {
                    Value::Nil => self.gc_params.stepsize,
                    Value::Integer(value) if value >= 0 => {
                        usize::try_from(value).map_err(|_| {
                            KError::new(
                                KErrorKind::Runtime("step budget overflow".to_owned()),
                                None,
                            )
                        })?
                    }
                    Value::Number(value) if value >= 0.0 => {
                        if value.fract() != 0.0 {
                            return Err(KError::new(
                                KErrorKind::Runtime("step budget must be an integer".to_owned()),
                                None,
                            ));
                        }
                        usize::try_from(value as u64).map_err(|_| {
                            KError::new(
                                KErrorKind::Runtime("step budget overflow".to_owned()),
                                None,
                            )
                        })?
                    }
                    _ => {
                        return Err(KError::new(
                            KErrorKind::Runtime("step budget must be non-negative".to_owned()),
                            None,
                        ));
                    }
                };
                let completed = self.gc_step(budget)?;
                Ok(vec![Value::boolean(completed)])
            }
            "stop" => {
                let previous = self.gc_running;
                self.gc_running = false;
                Ok(vec![Value::boolean(previous)])
            }
            "restart" => {
                let previous = self.gc_running;
                self.gc_running = true;
                Ok(vec![Value::boolean(previous)])
            }
            "isrunning" => Ok(vec![Value::boolean(self.gc_running)]),
            "incremental" => {
                let previous = self.gc_mode;
                self.gc_mode = GcMode::Incremental;
                Ok(vec![Value::string(self.gc_mode_name(previous)?)])
            }
            "generational" => {
                let previous = self.gc_mode;
                self.gc_mode = GcMode::Generational;
                Ok(vec![Value::string(self.gc_mode_name(previous)?)])
            }
            "param" => {
                let name = match self.arg_or_nil(args, 1) {
                    Value::String(handle) => {
                        let bytes = self.string_bytes_from_handle(handle)?;
                        std::str::from_utf8(&bytes)
                            .map(|value| value.to_owned())
                            .map_err(|_| {
                                KError::new(
                                    KErrorKind::Runtime(
                                        "GC parameter name must be UTF-8".to_owned(),
                                    ),
                                    None,
                                )
                            })?
                    }
                    _ => {
                        return Err(KError::new(
                            KErrorKind::Runtime(
                                "collectgarbage('param') expects a parameter name".to_owned(),
                            ),
                            None,
                        ));
                    }
                };
                let current = self.gc_param_value(&name)?;
                let value = self.arg_or_nil(args, 2);
                if !matches!(value, Value::Nil) {
                    let next = self.gc_param_from_value(&name, value)?;
                    self.gc_set_param_value(&name, next)?;
                }
                Ok(vec![Value::integer(i64::try_from(current).map_err(
                    |_| {
                        KError::new(
                            KErrorKind::Runtime("GC parameter overflow".to_owned()),
                            None,
                        )
                    },
                )?)])
            }
            other => Err(KError::new(
                KErrorKind::Runtime(format!("unsupported collectgarbage operation '{other}'")),
                None,
            )),
        }
    }

    fn arg_or_nil(&self, args: &[Value], index: usize) -> Value {
        args.get(index).copied().unwrap_or(Value::nil())
    }

    fn runtime_error(message: impl Into<String>) -> KError {
        KError::new(KErrorKind::Runtime(message.into()), None)
    }

    fn type_error(&self, expected: &str, actual: Value) -> KError {
        Self::runtime_error(format!("{expected}, got {}", self.value_type_name(actual)))
    }

    fn string_bytes_from_handle(&self, handle: StringHandle) -> KResult<Vec<u8>> {
        self.heap
            .string_bytes(handle)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| Self::runtime_error("invalid string handle"))
    }

    fn string_text_from_handle(&self, handle: StringHandle) -> KResult<String> {
        let bytes = self.string_bytes_from_handle(handle)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    #[allow(dead_code)]
    fn string_bytes_arg(&self, args: &[Value], index: usize) -> KResult<Vec<u8>> {
        match self.arg_or_nil(args, index) {
            Value::String(handle) => self.string_bytes_from_handle(handle),
            _ => Err(Self::runtime_error("string expected")),
        }
    }

    fn string_bytes_arg_typed(
        &self,
        args: &[Value],
        index: usize,
        expected: &str,
    ) -> KResult<Vec<u8>> {
        match self.arg_or_nil(args, index) {
            Value::String(handle) => self.string_bytes_from_handle(handle),
            other => Err(self.type_error(expected, other)),
        }
    }

    fn string_text_arg(&self, args: &[Value], index: usize) -> KResult<String> {
        match self.arg_or_nil(args, index) {
            Value::String(handle) => self.string_text_from_handle(handle),
            _ => Err(Self::runtime_error("string expected")),
        }
    }

    fn string_text_arg_typed(
        &self,
        args: &[Value],
        index: usize,
        expected: &str,
    ) -> KResult<String> {
        match self.arg_or_nil(args, index) {
            Value::String(handle) => self.string_text_from_handle(handle),
            other => Err(self.type_error(expected, other)),
        }
    }

    fn optional_string_text_arg(&self, args: &[Value], index: usize) -> KResult<Option<String>> {
        match self.arg_or_nil(args, index) {
            Value::Nil => Ok(None),
            Value::String(handle) => self.string_text_from_handle(handle).map(Some),
            _ => Ok(None),
        }
    }

    fn table_arg(&self, args: &[Value], index: usize, expected: &str) -> KResult<TableHandle> {
        match self.arg_or_nil(args, index) {
            Value::Table(handle) => Ok(handle),
            other => Err(self.type_error(expected, other)),
        }
    }

    #[allow(dead_code)]
    fn integer_arg(&self, args: &[Value], index: usize, expected: &str) -> KResult<i64> {
        match self.arg_or_nil(args, index) {
            Value::Integer(value) => Ok(value),
            Value::Number(value) if value.fract() == 0.0 => Ok(value as i64),
            _ => Err(Self::runtime_error(expected)),
        }
    }

    #[allow(dead_code)]
    fn number_arg(&self, args: &[Value], index: usize, expected: &str) -> KResult<f64> {
        match self.arg_or_nil(args, index) {
            Value::Integer(value) => Ok(value as f64),
            Value::Number(value) => Ok(value),
            _ => Err(Self::runtime_error(expected)),
        }
    }

    fn run_closure(&mut self, closure: ClosureHandle) -> KResult<Vec<Value>> {
        self.run_closure_with_args(closure, Vec::new())
    }

    fn instantiate_root_closure(&mut self, proto: Proto) -> KResult<ClosureHandle> {
        self.instantiate_closure(proto, None, 0)
    }

    fn instantiate_child_closure(
        &mut self,
        frame_index: usize,
        proto_index: PrototypeIndex,
    ) -> KResult<ClosureHandle> {
        let (closure_handle, base) = self.frame_state(frame_index)?;
        let closure = self.heap.resolve_closure(closure_handle)?;
        let proto_index = usize::try_from(proto_index.index()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("prototype index overflow".to_owned()),
                None,
            )
        })?;
        let proto = closure
            .proto
            .nested
            .get(proto_index)
            .cloned()
            .ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("nested prototype missing".to_owned()),
                    None,
                )
            })?;
        self.instantiate_closure(proto, Some(closure_handle), base)
    }

    fn instantiate_closure(
        &mut self,
        proto: Proto,
        parent: Option<ClosureHandle>,
        base: usize,
    ) -> KResult<ClosureHandle> {
        let mut upvalues = Vec::with_capacity(proto.upvalues.len());
        for descriptor in &proto.upvalues {
            let handle = if descriptor.instack {
                let stack_index =
                    base.checked_add(usize::from(descriptor.index))
                        .ok_or_else(|| {
                            KError::new(
                                KErrorKind::Runtime("stack index overflow".to_owned()),
                                None,
                            )
                        })?;
                self.capture_upvalue(stack_index)?
            } else if let Some(parent_handle) = parent {
                let parent_closure = self.heap.resolve_closure(parent_handle)?;
                let index = usize::from(descriptor.index);
                parent_closure.upvalues.get(index).copied().ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("missing parent upvalue".to_owned()),
                        None,
                    )
                })?
            } else if descriptor.index == 0 {
                self.heap.new_upvalue_closed(Value::table(self.globals))?
            } else {
                return Err(KError::new(
                    KErrorKind::Runtime("root closure missing parent upvalue".to_owned()),
                    None,
                ));
            };
            upvalues.push(handle);
        }

        self.heap.new_closure(proto, upvalues)
    }

    fn capture_upvalue(&mut self, stack_index: usize) -> KResult<UpvalueHandle> {
        for handle in &self.open_upvalues {
            if self.heap.upvalue_stack_index(*handle) == Some(stack_index) {
                return Ok(*handle);
            }
        }
        let handle = self.heap.new_upvalue_open(stack_index)?;
        self.open_upvalues.push(handle);
        Ok(handle)
    }

    fn current_instruction(&self, frame_index: usize) -> KResult<Instruction> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let closure = frame.closure;
        let pc = frame.pc;
        let closure = self.heap.resolve_closure(closure)?;
        closure.proto.instructions.get(pc).cloned().ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("program counter out of bounds".to_owned()),
                None,
            )
        })
    }

    fn frame_state(&self, frame_index: usize) -> KResult<(ClosureHandle, usize)> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        Ok((frame.closure, frame.base))
    }

    fn read_register(&self, frame_index: usize, register: Register) -> KResult<Value> {
        let absolute = self.absolute_register(frame_index, register)?;
        self.stack.get(absolute).copied().ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("register out of bounds".to_owned()),
                None,
            )
        })
    }

    fn write_register(
        &mut self,
        frame_index: usize,
        register: Register,
        value: Value,
    ) -> KResult<()> {
        let absolute = self.absolute_register(frame_index, register)?;
        let slot = self.stack.get_mut(absolute).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("register out of bounds".to_owned()),
                None,
            )
        })?;
        *slot = value;
        Ok(())
    }

    fn absolute_register(&self, frame_index: usize, register: Register) -> KResult<usize> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        frame
            .base
            .checked_add(usize::from(register.index()))
            .ok_or_else(|| KError::new(KErrorKind::Runtime("register overflow".to_owned()), None))
    }

    fn advance_pc(&mut self, frame_index: usize) -> KResult<()> {
        {
            let frame = self.frames.get_mut(frame_index).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("missing frame".to_owned()), None)
            })?;
            frame.pc = frame.pc.checked_add(1).ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("program counter overflow".to_owned()),
                    None,
                )
            })?;
        }
        Ok(())
    }

    fn jump(&mut self, frame_index: usize, offset: JumpOffset) -> KResult<()> {
        let frame = self
            .frames
            .get_mut(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let next = apply_jump(frame.pc, offset)?;
        frame.pc = next;
        Ok(())
    }

    fn ensure_stack_len(&mut self, len: usize) -> KResult<()> {
        if self.stack.len() < len {
            self.stack.resize(len, Value::nil());
        }
        Ok(())
    }

    fn read_call_args(
        &self,
        frame_index: usize,
        function: Register,
        args: u16,
    ) -> KResult<Vec<Value>> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let start = self.absolute_register(frame_index, function)?;
        let start = start.checked_add(1).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("argument index overflow".to_owned()),
                None,
            )
        })?;
        let count = if args == u16::MAX {
            frame.top.saturating_sub(start)
        } else {
            usize::from(args)
        };
        let mut values = Vec::with_capacity(count);
        for index in 0..count {
            let absolute = start.checked_add(index).ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("argument index overflow".to_owned()),
                    None,
                )
            })?;
            values.push(self.stack.get(absolute).copied().unwrap_or(Value::nil()));
        }
        Ok(values)
    }

    fn invoke_call(
        &mut self,
        site: CallSite,
        callee: Value,
        args: Vec<Value>,
        finished: &mut Vec<Value>,
    ) -> KResult<()> {
        let (callee, args) = self.callable_with_args(callee, args)?;
        match callee {
            Value::NativeFunction(function) => {
                let returned = self.call_native(function, &args)?;
                if site.tail {
                    self.finish_frame(site.frame_index, returned, finished)?;
                } else {
                    self.write_results(site.call_slot, site.results, &returned)?;
                    if let Some(frame) = self.frames.get_mut(site.frame_index) {
                        frame.last_call_results = if site.results == u16::MAX as usize {
                            returned.len()
                        } else {
                            site.results
                        };
                    }
                }
                Ok(())
            }
            Value::Closure(handle) => {
                if site.tail {
                    self.reuse_frame(site.frame_index, site.call_slot, handle, args)?;
                } else {
                    self.push_frame(site.frame_index, site.call_slot, handle, args, site.results)?;
                }
                Ok(())
            }
            _ => Err(KError::new(
                KErrorKind::Runtime("attempt to call a non-callable value".to_owned()),
                None,
            )),
        }
    }

    fn push_frame(
        &mut self,
        caller_index: usize,
        base: usize,
        closure: ClosureHandle,
        args: Vec<Value>,
        results: usize,
    ) -> KResult<()> {
        let proto = self.heap.resolve_closure(closure)?.proto.clone();
        let frame_stack_size = usize::from(proto.stack_size.max(1)).max(args.len());
        self.ensure_stack_len(
            base.checked_add(frame_stack_size).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
            })?,
        )?;

        for (index, value) in args.iter().enumerate() {
            let slot = base.checked_add(index).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
            })?;
            if let Some(place) = self.stack.get_mut(slot) {
                *place = *value;
            }
        }

        let parameter_count = usize::from(proto.parameters);
        let varargs = if proto.is_vararg && args.len() > parameter_count {
            args.get(parameter_count..).unwrap_or(&[]).to_vec()
        } else {
            Vec::new()
        };

        if args.len() < parameter_count {
            for index in args.len()..parameter_count {
                let slot = base.checked_add(index).ok_or_else(|| {
                    KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
                })?;
                if let Some(place) = self.stack.get_mut(slot) {
                    *place = Value::nil();
                }
            }
        }

        let frame = Frame {
            closure,
            base,
            top: base.checked_add(frame_stack_size).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
            })?,
            pc: 0,
            return_target: Some(ReturnTarget { base, results }),
            varargs,
            last_call_results: 0,
        };
        let _ = caller_index;
        self.frames.push(frame);
        Ok(())
    }

    fn reuse_frame(
        &mut self,
        frame_index: usize,
        base: usize,
        closure: ClosureHandle,
        args: Vec<Value>,
    ) -> KResult<()> {
        let proto = self.heap.resolve_closure(closure)?.proto.clone();
        let frame_stack_size = usize::from(proto.stack_size.max(1)).max(args.len());
        self.close_upvalues_from(base)?;
        self.ensure_stack_len(
            base.checked_add(frame_stack_size).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
            })?,
        )?;

        for (index, value) in args.iter().enumerate() {
            let slot = base.checked_add(index).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
            })?;
            if let Some(place) = self.stack.get_mut(slot) {
                *place = *value;
            }
        }

        let parameter_count = usize::from(proto.parameters);
        let varargs = if proto.is_vararg && args.len() > parameter_count {
            args.get(parameter_count..).unwrap_or(&[]).to_vec()
        } else {
            Vec::new()
        };

        if args.len() < parameter_count {
            for index in args.len()..parameter_count {
                let slot = base.checked_add(index).ok_or_else(|| {
                    KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
                })?;
                if let Some(place) = self.stack.get_mut(slot) {
                    *place = Value::nil();
                }
            }
        }

        let frame = self
            .frames
            .get_mut(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        frame.closure = closure;
        frame.base = base;
        frame.top = base
            .checked_add(frame_stack_size)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None))?;
        frame.pc = 0;
        frame.varargs = varargs;
        frame.last_call_results = 0;
        Ok(())
    }

    fn write_results(
        &mut self,
        call_slot: usize,
        results: usize,
        returned: &[Value],
    ) -> KResult<()> {
        let write_count = if results == u16::MAX as usize {
            returned.len()
        } else {
            results
        };

        if write_count == 0 {
            return Ok(());
        }

        self.ensure_stack_len(call_slot.checked_add(write_count).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("result stack overflow".to_owned()),
                None,
            )
        })?)?;

        for index in 0..write_count {
            let slot = call_slot.checked_add(index).ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("result stack overflow".to_owned()),
                    None,
                )
            })?;
            let value = returned.get(index).copied().unwrap_or(Value::nil());
            if let Some(place) = self.stack.get_mut(slot) {
                *place = value;
            }
        }
        for frame in self.frames.iter_mut().rev() {
            if frame.base <= call_slot {
                frame.top = call_slot.saturating_add(write_count);
                break;
            }
        }
        Ok(())
    }

    fn finish_frame(
        &mut self,
        frame_index: usize,
        values: Vec<Value>,
        finished: &mut Vec<Value>,
    ) -> KResult<()> {
        let (base, target) = {
            let frame = self.frames.get(frame_index).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("missing frame".to_owned()), None)
            })?;
            (frame.base, frame.return_target.clone())
        };
        let result_count = values.len();
        self.close_upvalues_from(base)?;
        let _ = self.frames.pop();
        if let Some(target) = target {
            self.write_results(target.base, target.results, &values)?;
            if let Some(frame) = self.frames.last_mut() {
                frame.last_call_results = if target.results == u16::MAX as usize {
                    result_count
                } else {
                    target.results
                };
            }
        } else {
            *finished = values;
        }
        Ok(())
    }

    fn close_upvalues_from(&mut self, stack_index: usize) -> KResult<()> {
        let mut still_open = Vec::with_capacity(self.open_upvalues.len());
        let open = std::mem::take(&mut self.open_upvalues);
        for handle in open {
            match self.heap.upvalue_stack_index(handle) {
                Some(index) if index >= stack_index => {
                    self.heap.close_upvalue(handle, &self.stack)?;
                }
                Some(_) => still_open.push(handle),
                None => still_open.push(handle),
            }
        }
        self.open_upvalues = still_open;
        Ok(())
    }

    fn get_upvalue(&self, frame_index: usize, upvalue: u16) -> KResult<Value> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let closure = self.heap.resolve_closure(frame.closure)?;
        let handle = closure
            .upvalues
            .get(usize::from(upvalue))
            .copied()
            .ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("upvalue out of bounds".to_owned()),
                    None,
                )
            })?;
        self.heap.upvalue_value(handle, &self.stack)
    }

    fn set_upvalue(&mut self, frame_index: usize, upvalue: u16, value: Value) -> KResult<()> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let closure = self.heap.resolve_closure(frame.closure)?;
        let handle = closure
            .upvalues
            .get(usize::from(upvalue))
            .copied()
            .ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("upvalue out of bounds".to_owned()),
                    None,
                )
            })?;
        self.heap.set_upvalue_value(handle, &mut self.stack, value)
    }

    fn write_varargs(
        &mut self,
        frame_index: usize,
        dst: Register,
        count: Option<u16>,
    ) -> KResult<()> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let values = frame.varargs.clone();
        let start = self.absolute_register(frame_index, dst)?;
        let written = match count {
            Some(count) => usize::from(count),
            None => values.len(),
        };
        self.ensure_stack_len(
            start.checked_add(written).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
            })?,
        )?;
        for index in 0..written {
            let value = values.get(index).copied().unwrap_or(Value::nil());
            let slot = start.checked_add(index).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
            })?;
            if let Some(place) = self.stack.get_mut(slot) {
                *place = value;
            }
        }
        if let Some(frame) = self.frames.get_mut(frame_index) {
            frame.last_call_results = written;
            frame.top = start.saturating_add(written);
        }
        Ok(())
    }

    fn set_table_range(
        &mut self,
        frame_index: usize,
        table: Register,
        start: i64,
        values: Register,
        count: Option<u16>,
    ) -> KResult<()> {
        let table = self.read_register(frame_index, table)?;
        let count = match count {
            Some(count) => usize::from(count),
            None => {
                self.frames
                    .get(frame_index)
                    .ok_or_else(|| {
                        KError::new(KErrorKind::Runtime("missing frame".to_owned()), None)
                    })?
                    .last_call_results
            }
        };
        let first = self.absolute_register(frame_index, values)?;
        let mut collected = Vec::with_capacity(count);
        for offset in 0..count {
            let index = first.checked_add(offset).ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("table value index overflow".to_owned()),
                    None,
                )
            })?;
            collected.push(self.stack.get(index).copied().unwrap_or(Value::nil()));
        }
        for (offset, value) in collected.into_iter().enumerate() {
            let key = start
                .checked_add(i64::try_from(offset).map_err(|_| {
                    KError::new(KErrorKind::Runtime("table index overflow".to_owned()), None)
                })?)
                .ok_or_else(|| {
                    KError::new(KErrorKind::Runtime("table index overflow".to_owned()), None)
                })?;
            self.table_set(table, Value::integer(key), value)?;
        }
        Ok(())
    }

    fn collect_return_values(
        &self,
        frame_index: usize,
        first: Register,
        count: u16,
    ) -> KResult<Vec<Value>> {
        if count == u16::MAX {
            let frame = self.frames.get(frame_index).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("missing frame".to_owned()), None)
            })?;
            let start = self.absolute_register(frame_index, first)?;
            let count = frame.top.saturating_sub(start);
            let mut values = Vec::with_capacity(count);
            for offset in 0..count {
                let index = start.checked_add(offset).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("return stack overflow".to_owned()),
                        None,
                    )
                })?;
                values.push(self.stack.get(index).copied().unwrap_or(Value::nil()));
            }
            return Ok(values);
        }

        if count == 0 {
            return Ok(Vec::new());
        }
        let start = self.absolute_register(frame_index, first)?;
        let mut values = Vec::with_capacity(usize::from(count));
        for offset in 0..usize::from(count) {
            let slot = start.checked_add(offset).ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("return stack overflow".to_owned()),
                    None,
                )
            })?;
            values.push(self.stack.get(slot).copied().unwrap_or(Value::nil()));
        }
        Ok(values)
    }

    fn constant_to_value(&mut self, frame_index: usize, constant: ConstantIndex) -> KResult<Value> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let closure = self.heap.resolve_closure(frame.closure)?;
        let index = usize::try_from(constant.index()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("constant index overflow".to_owned()),
                None,
            )
        })?;
        let constant = closure.proto.constants.get(index).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("constant index out of bounds".to_owned()),
                None,
            )
        })?;
        match constant {
            Constant::Nil => Ok(Value::nil()),
            Constant::Boolean(value) => Ok(Value::boolean(*value)),
            Constant::Integer(value) => Ok(Value::integer(*value)),
            Constant::Number(value) => Ok(Value::number(*value)),
            Constant::String(bytes) => {
                let handle = self.heap.intern_string(bytes.clone())?;
                Ok(Value::string(handle))
            }
        }
    }

    fn get_global(&mut self, frame_index: usize, name: ConstantIndex) -> KResult<Value> {
        let key = self.constant_string_name(frame_index, name)?;
        let env = self.frame_env_table(frame_index)?;
        let table = self.heap.resolve_table(env)?;
        table.raw_get(Value::string(key))
    }

    fn set_global(&mut self, frame_index: usize, name: ConstantIndex, value: Value) -> KResult<()> {
        let key = self.constant_string_name(frame_index, name)?;
        let env = self.frame_env_table(frame_index)?;
        let table = self.heap.resolve_table_mut(env)?;
        table.raw_set(Value::string(key), value)
    }

    fn frame_env_table(&self, frame_index: usize) -> KResult<TableHandle> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let closure = self.heap.resolve_closure(frame.closure)?;
        if let Some(handle) = closure.upvalues.first().copied() {
            let value = self.heap.upvalue_value(handle, &self.stack)?;
            if let Value::Table(table) = value {
                return Ok(table);
            }
        }
        Ok(self.globals)
    }

    fn constant_string_name(
        &mut self,
        frame_index: usize,
        name: ConstantIndex,
    ) -> KResult<StringHandle> {
        let frame = self
            .frames
            .get(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        let closure = self.heap.resolve_closure(frame.closure)?;
        let index = usize::try_from(name.index()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("constant index overflow".to_owned()),
                None,
            )
        })?;
        let constant = closure.proto.constants.get(index).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("constant index out of bounds".to_owned()),
                None,
            )
        })?;
        match constant {
            Constant::String(bytes) => self.heap.intern_string(bytes.clone()),
            _ => Err(KError::new(
                KErrorKind::Runtime("global name constant must be a string".to_owned()),
                None,
            )),
        }
    }

    fn table_get(&mut self, value: Value, key: Value) -> KResult<Value> {
        self.index_value(value, key, 0)
    }

    fn table_set(&mut self, value: Value, key: Value, next: Value) -> KResult<()> {
        self.newindex_value(value, key, next, 0)
    }

    fn new_table_handle(&mut self) -> KResult<TableHandle> {
        let handle = self.heap.new_table()?;
        if !matches!(self.gc_phase, GcPhase::Pause) {
            self.gc_mark_table(handle);
        }
        Ok(handle)
    }

    fn check_global_unset(&self, table: Value, key: Value) -> KResult<()> {
        let Value::Table(table) = table else {
            return Err(self.type_error("table expected", table));
        };
        let Value::String(name) = key else {
            return Err(Self::runtime_error("global name must be a string"));
        };
        if !matches!(
            self.heap
                .resolve_table(table)?
                .raw_get(Value::string(name))?,
            Value::Nil
        ) {
            return Err(Self::runtime_error(format!(
                "global '{}' already defined",
                self.string_text_from_handle(name)?
            )));
        }
        Ok(())
    }

    fn index_value(&mut self, value: Value, key: Value, depth: usize) -> KResult<Value> {
        let mut visited = Vec::new();
        self.index_value_inner(value, key, depth, &mut visited)
    }

    fn index_value_inner(
        &mut self,
        value: Value,
        key: Value,
        depth: usize,
        visited: &mut Vec<TableHandle>,
    ) -> KResult<Value> {
        if depth >= self.metamethod_depth_limit() {
            return Err(KError::new(
                KErrorKind::Runtime("metamethod recursion limit reached".to_owned()),
                None,
            ));
        }

        match value {
            Value::Table(handle) => {
                let table = self.heap.resolve_table(handle)?;
                let raw = table.raw_get(key)?;
                if !matches!(raw, Value::Nil) {
                    return Ok(raw);
                }
                if let Some(metatable) = table.metatable
                    && let Some(method) = self.table_metamethod(metatable, "__index")?
                {
                    return self.resolve_index_metamethod(method, value, key, depth, visited);
                }
                Ok(Value::nil())
            }
            _ => {
                if let Some(metatable) = self.metatable_of_value(value)
                    && let Some(method) = self.table_metamethod(metatable, "__index")?
                {
                    return self.resolve_index_metamethod(method, value, key, depth, visited);
                }
                Err(KError::new(
                    KErrorKind::Runtime(format!(
                        "attempt to index a {} value",
                        self.value_type_name(value)
                    )),
                    None,
                ))
            }
        }
    }

    fn newindex_value(
        &mut self,
        value: Value,
        key: Value,
        next: Value,
        depth: usize,
    ) -> KResult<()> {
        let mut visited = Vec::new();
        self.newindex_value_inner(value, key, next, depth, &mut visited)
    }

    fn newindex_value_inner(
        &mut self,
        value: Value,
        key: Value,
        next: Value,
        depth: usize,
        visited: &mut Vec<TableHandle>,
    ) -> KResult<()> {
        if depth >= self.metamethod_depth_limit() {
            return Err(KError::new(
                KErrorKind::Runtime("metamethod recursion limit reached".to_owned()),
                None,
            ));
        }

        match value {
            Value::Table(handle) => {
                let table = self.heap.resolve_table(handle)?;
                let raw = table.raw_get(key)?;
                if !matches!(raw, Value::Nil) || table.metatable.is_none() {
                    let table = self.heap.resolve_table_mut(handle)?;
                    return table.raw_set(key, next);
                }
                if let Some(metatable) = table.metatable
                    && let Some(method) = self.table_metamethod(metatable, "__newindex")?
                {
                    return self
                        .resolve_newindex_metamethod(method, value, key, next, depth, visited);
                }
                let table = self.heap.resolve_table_mut(handle)?;
                table.raw_set(key, next)
            }
            _ => {
                if let Some(metatable) = self.metatable_of_value(value)
                    && let Some(method) = self.table_metamethod(metatable, "__newindex")?
                {
                    return self
                        .resolve_newindex_metamethod(method, value, key, next, depth, visited);
                }
                Err(KError::new(
                    KErrorKind::Runtime(format!(
                        "attempt to index a {} value",
                        self.value_type_name(value)
                    )),
                    None,
                ))
            }
        }
    }

    fn resolve_index_metamethod(
        &mut self,
        method: Value,
        receiver: Value,
        key: Value,
        depth: usize,
        visited: &mut Vec<TableHandle>,
    ) -> KResult<Value> {
        match method {
            Value::Table(handle) => {
                if visited.contains(&handle) {
                    return Err(KError::new(
                        KErrorKind::Runtime("loop in gettable".to_owned()),
                        None,
                    ));
                }
                visited.push(handle);
                self.index_value_inner(Value::Table(handle), key, depth + 1, visited)
            }
            _ => self.call_value_sync(method, vec![receiver, key], depth + 1),
        }
    }

    fn resolve_newindex_metamethod(
        &mut self,
        method: Value,
        receiver: Value,
        key: Value,
        next: Value,
        depth: usize,
        visited: &mut Vec<TableHandle>,
    ) -> KResult<()> {
        match method {
            Value::Table(handle) => {
                if visited.contains(&handle) {
                    return Err(KError::new(
                        KErrorKind::Runtime("loop in settable".to_owned()),
                        None,
                    ));
                }
                visited.push(handle);
                self.newindex_value_inner(Value::Table(handle), key, next, depth + 1, visited)
            }
            _ => {
                let _ = self.call_value_sync(method, vec![receiver, key, next], depth + 1)?;
                Ok(())
            }
        }
    }

    fn arithmetic(&mut self, op: ArithmeticOp, left: Value, right: Value) -> KResult<Value> {
        match op {
            ArithmeticOp::Add => {
                self.numeric_or_metamethod(left, right, Self::numeric_add, "__add")
            }
            ArithmeticOp::Sub => {
                self.numeric_or_metamethod(left, right, Self::numeric_sub, "__sub")
            }
            ArithmeticOp::Mul => {
                self.numeric_or_metamethod(left, right, Self::numeric_mul, "__mul")
            }
            ArithmeticOp::Div => {
                self.numeric_or_metamethod(left, right, Self::numeric_div, "__div")
            }
            ArithmeticOp::FloorDiv => {
                self.numeric_or_metamethod(left, right, Self::numeric_floor_div, "__idiv")
            }
            ArithmeticOp::Mod => {
                self.numeric_or_metamethod(left, right, Self::numeric_mod, "__mod")
            }
            ArithmeticOp::Pow => {
                self.numeric_or_metamethod(left, right, Self::numeric_pow, "__pow")
            }
            ArithmeticOp::BitOr => self.bitwise_binary("bitwise or", left, right, |a, b| a | b),
            ArithmeticOp::BitXor => self.bitwise_binary("bitwise xor", left, right, |a, b| a ^ b),
            ArithmeticOp::BitAnd => self.bitwise_binary("bitwise and", left, right, |a, b| a & b),
            ArithmeticOp::ShiftLeft => {
                self.bitwise_binary("shift left", left, right, Self::bitwise_shift_left)
            }
            ArithmeticOp::ShiftRight => {
                self.bitwise_binary("shift right", left, right, Self::bitwise_shift_right)
            }
        }
    }

    fn compare(&mut self, op: CompareOp, left: Value, right: Value) -> KResult<bool> {
        match op {
            CompareOp::Eq => self.equal_values(left, right, 0),
            CompareOp::NotEq => self.equal_values(left, right, 0).map(|value| !value),
            CompareOp::Less => self.ordering_with_metamethod(left, right, "__lt", false, 0),
            CompareOp::LessEq => self.ordering_with_metamethod(left, right, "__le", true, 0),
            CompareOp::Greater => self.ordering_with_metamethod(right, left, "__lt", false, 0),
            CompareOp::GreaterEq => self.ordering_with_metamethod(right, left, "__le", true, 0),
        }
    }

    fn ordering_with_metamethod(
        &mut self,
        left: Value,
        right: Value,
        metamethod: &str,
        allow_equal: bool,
        depth: usize,
    ) -> KResult<bool> {
        if matches!(left, Value::Number(number) if number.is_nan())
            || matches!(right, Value::Number(number) if number.is_nan())
        {
            return Ok(false);
        }
        if let Some(ordering) = self.compare_values(left, right)? {
            return Ok(match ordering {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Equal => allow_equal,
                std::cmp::Ordering::Greater => false,
            });
        }

        match metamethod {
            "__lt" => {
                let result = self.call_metamethod(left, right, metamethod, depth)?;
                Ok(self.is_truthy(result))
            }
            "__le" => {
                if let Some(result) = self.call_binary_metamethod(left, right, metamethod, depth)? {
                    return Ok(self.is_truthy(result));
                }
                let result = self.call_metamethod(left, right, "__lt", depth)?;
                Ok(!self.is_truthy(result))
            }
            _ => Err(KError::new(
                KErrorKind::Runtime("unsupported comparison operation".to_owned()),
                None,
            )),
        }
    }

    fn equal_values(&mut self, left: Value, right: Value, depth: usize) -> KResult<bool> {
        if left == right {
            return Ok(true);
        }

        let left_meta = self.metatable_of_value(left);
        let right_meta = self.metatable_of_value(right);
        if left_meta.is_some()
            && left_meta == right_meta
            && let Some(metatable) = left_meta
            && let Some(function) = self.table_metamethod(metatable, "__eq")?
        {
            let result = self.call_value_sync(function, vec![left, right], depth + 1)?;
            return Ok(self.is_truthy(result));
        }

        Ok(false)
    }

    fn compare_values(&self, left: Value, right: Value) -> KResult<Option<std::cmp::Ordering>> {
        match (left, right) {
            (Value::Integer(left), Value::Integer(right)) => Ok(Some(left.cmp(&right))),
            (Value::Integer(left), Value::Number(right)) => {
                self.integer_float_ordering(left, right)
            }
            (Value::Number(left), Value::Integer(right)) => self
                .integer_float_ordering(right, left)
                .map(|ordering| ordering.map(std::cmp::Ordering::reverse)),
            (Value::Number(left), Value::Number(right)) => self.numeric_ordering(left, right),
            (Value::String(left), Value::String(right)) => {
                let left = self.heap.string_bytes(left).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                let right = self.heap.string_bytes(right).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                Ok(Some(left.cmp(right)))
            }
            _ => Ok(None),
        }
    }

    fn integer_float_ordering(
        &self,
        integer: i64,
        number: f64,
    ) -> KResult<Option<std::cmp::Ordering>> {
        if !number.is_finite() {
            return self.numeric_ordering(integer as f64, number);
        }
        const INTEGER_EXCLUSIVE_MAX: f64 = 9_223_372_036_854_775_808.0;
        if number >= INTEGER_EXCLUSIVE_MAX {
            return Ok(Some(std::cmp::Ordering::Less));
        }
        if number < -INTEGER_EXCLUSIVE_MAX {
            return Ok(Some(std::cmp::Ordering::Greater));
        }
        let truncated = number.trunc() as i64;
        let ordering = integer.cmp(&truncated);
        if ordering != std::cmp::Ordering::Equal {
            return Ok(Some(ordering));
        }
        Ok(Some(if number.fract() > 0.0 {
            std::cmp::Ordering::Less
        } else if number.fract() < 0.0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }))
    }

    fn numeric_ordering(&self, left: f64, right: f64) -> KResult<Option<std::cmp::Ordering>> {
        left.partial_cmp(&right).map(Some).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("non-finite numeric comparison".to_owned()),
                None,
            )
        })
    }

    fn numeric_add(&self, left: Value, right: Value) -> KResult<Value> {
        if let (Value::Integer(left), Value::Integer(right)) = (left, right) {
            return Ok(Value::integer(left.wrapping_add(right)));
        }
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left + right))
    }

    fn numeric_sub(&self, left: Value, right: Value) -> KResult<Value> {
        if let (Value::Integer(left), Value::Integer(right)) = (left, right) {
            return Ok(Value::integer(left.wrapping_sub(right)));
        }
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left - right))
    }

    fn numeric_mul(&self, left: Value, right: Value) -> KResult<Value> {
        if let (Value::Integer(left), Value::Integer(right)) = (left, right) {
            return Ok(Value::integer(left.wrapping_mul(right)));
        }
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left * right))
    }

    fn numeric_div(&self, left: Value, right: Value) -> KResult<Value> {
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left / right))
    }

    fn numeric_floor_div(&self, left: Value, right: Value) -> KResult<Value> {
        if let (Value::Integer(left), Value::Integer(right)) = (left, right) {
            if right == 0 {
                return Err(KError::new(
                    KErrorKind::Runtime("divide by zero".to_owned()),
                    None,
                ));
            }
            if right == -1 {
                return Ok(Value::integer(left.wrapping_neg()));
            }
            let quotient = left / right;
            let remainder = left % right;
            return Ok(Value::integer(
                if remainder != 0 && (left < 0) != (right < 0) {
                    quotient.wrapping_sub(1)
                } else {
                    quotient
                },
            ));
        }
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number((left / right).floor()))
    }

    fn numeric_mod(&self, left: Value, right: Value) -> KResult<Value> {
        if let (Value::Integer(left), Value::Integer(right)) = (left, right) {
            if right == 0 {
                return Err(KError::new(
                    KErrorKind::Runtime("divide by zero".to_owned()),
                    None,
                ));
            }
            if right == -1 {
                return Ok(Value::integer(0));
            }
            let remainder = left % right;
            return Ok(Value::integer(
                if remainder != 0 && (left < 0) != (right < 0) {
                    remainder.wrapping_add(right)
                } else {
                    remainder
                },
            ));
        }
        let (left, right) = self.coerce_numbers(left, right)?;
        let remainder = left % right;
        Ok(Value::number(
            if remainder != 0.0 && (left < 0.0) != (right < 0.0) {
                remainder + right
            } else {
                remainder
            },
        ))
    }

    fn numeric_pow(&self, left: Value, right: Value) -> KResult<Value> {
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left.powf(right)))
    }

    fn numeric_or_metamethod(
        &mut self,
        left: Value,
        right: Value,
        numeric: fn(&Self, Value, Value) -> KResult<Value>,
        metamethod: &str,
    ) -> KResult<Value> {
        if self.value_to_number_opt(left)?.is_some() && self.value_to_number_opt(right)?.is_some() {
            return numeric(self, left, right);
        }

        if let Some(result) = self.call_binary_metamethod(left, right, metamethod, 0)? {
            return Ok(result);
        }

        Err(KError::new(
            KErrorKind::Runtime(format!(
                "attempt to perform {} on a {} value",
                self.binary_operation_name(metamethod),
                self.value_type_name(if self.value_to_number_opt(left)?.is_some() {
                    right
                } else {
                    left
                })
            )),
            None,
        ))
    }

    fn coerce_numbers(&self, left: Value, right: Value) -> KResult<(f64, f64)> {
        Ok((self.value_to_number(left)?, self.value_to_number(right)?))
    }

    fn value_to_number(&self, value: Value) -> KResult<f64> {
        match value {
            Value::Integer(value) => Ok(value as f64),
            Value::Number(value) => Ok(value),
            Value::String(handle) => {
                let bytes = self.heap.string_bytes(handle).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                let text = std::str::from_utf8(bytes).map_err(|_| {
                    KError::new(
                        KErrorKind::Runtime("numeric value expected".to_owned()),
                        None,
                    )
                })?;
                let text = text.trim();
                let (negative, digits) = match text.strip_prefix('-') {
                    Some(value) => (true, value),
                    None => (false, text.strip_prefix('+').unwrap_or(text)),
                };
                if let Some(number) = digits
                    .strip_prefix("0x")
                    .or_else(|| digits.strip_prefix("0X"))
                    .filter(|hex| hex.contains(['.', 'p', 'P']))
                    .and_then(Self::parse_hex_number_text)
                {
                    return Ok(if negative { -number } else { number });
                }
                if let Ok(integer) = Self::parse_integer_text(text) {
                    return Ok(integer as f64);
                }
                text.trim().parse::<f64>().map_err(|_| {
                    KError::new(
                        KErrorKind::Runtime("numeric value expected".to_owned()),
                        None,
                    )
                })
            }
            _ => Err(KError::new(
                KErrorKind::Runtime("numeric value expected".to_owned()),
                None,
            )),
        }
    }

    fn value_to_number_opt(&self, value: Value) -> KResult<Option<f64>> {
        match self.value_to_number(value) {
            Ok(value) => Ok(Some(value)),
            Err(error) => match error.kind() {
                KErrorKind::Runtime(message) if message == "numeric value expected" => Ok(None),
                _ => Err(error),
            },
        }
    }

    fn binary_operation_name(&self, metamethod: &str) -> &'static str {
        match metamethod {
            "__add" => "add",
            "__sub" => "subtract",
            "__mul" => "multiply",
            "__div" => "divide",
            "__idiv" => "perform floor division on",
            "__mod" => "perform modulo on",
            "__pow" => "raise",
            _ => "perform arithmetic on",
        }
    }

    fn bitwise_binary<F>(
        &mut self,
        op_name: &str,
        left: Value,
        right: Value,
        op: F,
    ) -> KResult<Value>
    where
        F: Fn(i64, i64) -> i64,
    {
        if let (Some(left), Some(right)) =
            (self.value_to_integer(left)?, self.value_to_integer(right)?)
        {
            return Ok(Value::integer(op(left, right)));
        }

        if self.value_to_number_opt(left)?.is_some() && self.value_to_number_opt(right)?.is_some() {
            if matches!(left, Value::Number(value) if value.is_infinite())
                || matches!(right, Value::Number(value) if value.is_infinite())
            {
                return Err(KError::new(
                    KErrorKind::Runtime(
                        "field 'huge': number has no integer representation".to_owned(),
                    ),
                    None,
                ));
            }
            return Err(KError::new(
                KErrorKind::Runtime("number has no integer representation".to_owned()),
                None,
            ));
        }

        if let Some(result) =
            self.call_binary_metamethod(left, right, self.bitwise_metamethod_name(op_name), 0)?
        {
            return Ok(result);
        }

        Err(KError::new(
            KErrorKind::Runtime(format!(
                "attempt to perform {op_name} on a {} value",
                self.value_type_name(left)
            )),
            None,
        ))
    }

    fn unary(&mut self, op: UnaryOpKind, value: Value) -> KResult<Value> {
        match op {
            UnaryOpKind::Minus => {
                if let Value::Integer(integer) = value {
                    return Ok(Value::integer(integer.wrapping_neg()));
                }
                if let Some(number) = self.value_to_number_opt(value)? {
                    return Ok(Value::number(-number));
                }
                if let Some(result) = self.call_unary_metamethod(value, "__unm", 0)? {
                    return Ok(result);
                }
                Err(KError::new(
                    KErrorKind::Runtime(format!(
                        "attempt to perform unary minus on a {} value",
                        self.value_type_name(value)
                    )),
                    None,
                ))
            }
            UnaryOpKind::BitNot => {
                if let Some(integer) = self.value_to_integer(value)? {
                    return Ok(Value::integer(!integer));
                }
                if let Some(result) = self.call_unary_metamethod(value, "__bnot", 0)? {
                    return Ok(result);
                }
                Err(KError::new(
                    KErrorKind::Runtime(format!(
                        "attempt to perform bitwise not on a {} value",
                        self.value_type_name(value)
                    )),
                    None,
                ))
            }
            UnaryOpKind::Len => self.length(value, 0),
        }
    }

    fn length(&mut self, value: Value, depth: usize) -> KResult<Value> {
        match value {
            Value::String(handle) => {
                let bytes = self.heap.string_bytes(handle).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                Ok(Value::integer(i64::try_from(bytes.len()).map_err(
                    |_| {
                        KError::new(
                            KErrorKind::Runtime("string length overflow".to_owned()),
                            None,
                        )
                    },
                )?))
            }
            Value::Table(handle) => {
                let table = self.heap.resolve_table(handle)?;
                let metatable = table.metatable;
                let len = table.array.len();
                if let Some(metatable) = metatable
                    && let Some(result) = self.table_metamethod(metatable, "__len")?
                {
                    return self.call_value_sync(result, vec![value], depth + 1);
                }
                Ok(Value::integer(i64::try_from(len).map_err(|_| {
                    KError::new(
                        KErrorKind::Runtime("table length overflow".to_owned()),
                        None,
                    )
                })?))
            }
            _ => {
                if let Some(result) = self.call_unary_metamethod(value, "__len", depth)? {
                    return Ok(result);
                }
                Err(KError::new(
                    KErrorKind::Runtime(format!(
                        "attempt to get length of a {} value",
                        self.value_type_name(value)
                    )),
                    None,
                ))
            }
        }
    }

    fn concat_values(
        &mut self,
        frame_index: usize,
        first: Register,
        last: Register,
    ) -> KResult<Value> {
        let start = self.absolute_register(frame_index, first)?;
        let end = self.absolute_register(frame_index, last)?;
        if end < start {
            return Err(KError::new(
                KErrorKind::Runtime("invalid concat range".to_owned()),
                None,
            ));
        }
        let mut values = Vec::new();
        for index in start..=end {
            values.push(self.stack.get(index).copied().unwrap_or(Value::nil()));
        }
        let Some((first, rest)) = values.split_first() else {
            return Ok(Value::nil());
        };
        let mut current = *first;
        for &next in rest {
            current = self.concat_pair(current, next, 0)?;
        }
        Ok(current)
    }

    #[cfg(test)]
    fn concat_values_for_test(&mut self, values: &[Value]) -> KResult<Value> {
        let Some((first, rest)) = values.split_first() else {
            return Ok(Value::nil());
        };
        let mut current = *first;
        for next in rest {
            current = self.concat_pair(current, *next, 0)?;
        }
        Ok(current)
    }

    fn concat_pair(&mut self, left: Value, right: Value, depth: usize) -> KResult<Value> {
        if let (Some(left), Some(right)) = (self.concat_piece(left)?, self.concat_piece(right)?) {
            let mut bytes = left;
            bytes.extend_from_slice(&right);
            let handle = self.heap.intern_string(bytes)?;
            return Ok(Value::string(handle));
        }

        if let Some(result) = self.call_binary_metamethod(left, right, "__concat", depth)? {
            return Ok(result);
        }

        Err(KError::new(
            KErrorKind::Runtime(format!(
                "attempt to concatenate a {} value",
                self.value_type_name(left)
            )),
            None,
        ))
    }

    fn concat_piece(&self, value: Value) -> KResult<Option<Vec<u8>>> {
        match value {
            Value::String(handle) => {
                let bytes = self.heap.string_bytes(handle).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                Ok(Some(bytes.to_vec()))
            }
            Value::Integer(value) => Ok(Some(value.to_string().into_bytes())),
            Value::Number(value) => Ok(Some(value.to_string().into_bytes())),
            _ => Ok(None),
        }
    }

    fn value_to_integer(&self, value: Value) -> KResult<Option<i64>> {
        match value {
            Value::Integer(value) => Ok(Some(value)),
            Value::Number(value) => Ok(Self::number_to_integer(value)),
            Value::String(handle) => {
                let bytes = self.heap.string_bytes(handle).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                let Some(text) = std::str::from_utf8(bytes).ok() else {
                    return Ok(None);
                };
                match Self::parse_integer_text(text) {
                    Ok(value) => Ok(Some(value)),
                    Err(_) => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    fn parse_integer_text(text: &str) -> Result<i64, ()> {
        let text = text.trim();
        if let Ok(value) = text.parse::<i64>() {
            return Ok(value);
        }
        let (negative, digits) = match text.strip_prefix('-') {
            Some(value) => (true, value),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let value = if let Some(hex) = digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
        {
            if hex.contains(['.', 'p', 'P']) {
                let number = Self::parse_hex_number_text(hex).ok_or(())?;
                if !number.is_finite() || number.fract() != 0.0 {
                    return Err(());
                }
                Self::number_to_integer(number).ok_or(())?
            } else {
                if hex.is_empty() {
                    return Err(());
                }
                let raw = hex.bytes().try_fold(0u64, |value, digit| {
                    char::from(digit)
                        .to_digit(16)
                        .map(|digit| value.wrapping_mul(16).wrapping_add(u64::from(digit)))
                        .ok_or(())
                })?;
                i64::from_ne_bytes(raw.to_ne_bytes())
            }
        } else {
            match digits.parse::<i64>() {
                Ok(value) => value,
                Err(_) => {
                    let number = digits.parse::<f64>().map_err(|_| ())?;
                    if !number.is_finite() || number.fract() != 0.0 {
                        return Err(());
                    }
                    Self::number_to_integer(number).ok_or(())?
                }
            }
        };
        Ok(if negative {
            value.wrapping_neg()
        } else {
            value
        })
    }

    fn number_to_integer(number: f64) -> Option<i64> {
        const INTEGER_MIN: f64 = -9_223_372_036_854_775_808.0;
        const INTEGER_EXCLUSIVE_MAX: f64 = 9_223_372_036_854_775_808.0;

        (number.is_finite()
            && number.fract() == 0.0
            && (INTEGER_MIN..INTEGER_EXCLUSIVE_MAX).contains(&number))
        .then_some(number as i64)
    }

    fn parse_hex_number_text(text: &str) -> Option<f64> {
        let (mantissa, mut exponent) = match text.split_once(['p', 'P']) {
            Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
            None => (text, 0),
        };
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let normalized_whole = if fraction.is_empty() {
            let trimmed = whole.trim_end_matches('0');
            let trailing_zeros = whole.len().saturating_sub(trimmed.len());
            let exponent_adjustment = i32::try_from(trailing_zeros.checked_mul(4)?).ok()?;
            exponent = exponent.checked_add(exponent_adjustment)?;
            trimmed
        } else {
            whole
        };
        let whole = if normalized_whole.is_empty() {
            0.0
        } else {
            normalized_whole.chars().try_fold(0.0_f64, |value, digit| {
                digit
                    .to_digit(16)
                    .map(|digit| value.mul_add(16.0, f64::from(digit)))
            })?
        };
        let normalized_fraction = if normalized_whole.is_empty() {
            let trimmed = fraction.trim_start_matches('0');
            let leading_zeros = fraction.len().saturating_sub(trimmed.len());
            let exponent_adjustment = i32::try_from(leading_zeros.checked_mul(4)?).ok()?;
            exponent = exponent.checked_sub(exponent_adjustment)?;
            trimmed
        } else {
            fraction
        };
        let fraction = normalized_fraction.chars().enumerate().try_fold(
            0.0_f64,
            |value, (index, digit)| {
                let digit = digit.to_digit(16)? as f64;
                Some(value + digit * 16.0_f64.powi(-i32::try_from(index + 1).ok()?))
            },
        )?;
        Some((whole + fraction) * 2.0_f64.powi(exponent))
    }

    fn bitwise_shift_left(left: i64, right: i64) -> i64 {
        Self::shift_bits(left, right, true)
    }

    fn bitwise_shift_right(left: i64, right: i64) -> i64 {
        Self::shift_bits(left, right, false)
    }

    fn shift_bits(value: i64, count: i64, left: bool) -> i64 {
        if count < 0 {
            return Self::shift_bits(value, count.checked_neg().unwrap_or(i64::MAX), !left);
        }
        if count >= i64::from(u64::BITS) {
            return 0;
        }

        let count = count as u32;
        let bits = value as u64;
        let shifted = if left { bits << count } else { bits >> count };
        shifted as i64
    }

    fn call_unary_metamethod(
        &mut self,
        value: Value,
        name: &str,
        depth: usize,
    ) -> KResult<Option<Value>> {
        if depth >= self.metamethod_depth_limit() {
            return Err(KError::new(
                KErrorKind::Runtime("metamethod recursion limit reached".to_owned()),
                None,
            ));
        }
        let Some(metatable) = self.metatable_of_value(value) else {
            return Ok(None);
        };
        let Some(function) = self.table_metamethod(metatable, name)? else {
            return Ok(None);
        };
        self.call_value_sync(function, vec![value], depth + 1)
            .map(Some)
    }

    fn call_binary_metamethod(
        &mut self,
        left: Value,
        right: Value,
        name: &str,
        depth: usize,
    ) -> KResult<Option<Value>> {
        if depth >= self.metamethod_depth_limit() {
            return Err(KError::new(
                KErrorKind::Runtime("metamethod recursion limit reached".to_owned()),
                None,
            ));
        }
        let left_meta = self.metatable_of_value(left);
        let right_meta = self.metatable_of_value(right);
        if left_meta.is_none() && right_meta.is_none() {
            return Ok(None);
        }
        if let Some(metatable) = left_meta
            && let Some(function) = self.table_metamethod(metatable, name)?
        {
            return self
                .call_value_sync(function, vec![left, right], depth + 1)
                .map(Some);
        }
        if right_meta == left_meta {
            return Ok(None);
        }
        if let Some(metatable) = right_meta
            && let Some(function) = self.table_metamethod(metatable, name)?
        {
            return self
                .call_value_sync(function, vec![left, right], depth + 1)
                .map(Some);
        }
        Ok(None)
    }

    fn call_metamethod(
        &mut self,
        left: Value,
        right: Value,
        name: &str,
        depth: usize,
    ) -> KResult<Value> {
        self.call_binary_metamethod(left, right, name, depth)?
            .ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime(format!(
                        "attempt to compare {} and {} values",
                        self.value_type_name(left),
                        self.value_type_name(right)
                    )),
                    None,
                )
            })
    }

    fn table_metamethod(&mut self, metatable: TableHandle, name: &str) -> KResult<Option<Value>> {
        let key = self.heap.intern_string(name.as_bytes().to_vec())?;
        let table = self.heap.resolve_table(metatable)?;
        let value = table.raw_get(Value::string(key))?;
        if matches!(value, Value::Nil) {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    fn metatable_of_value(&self, value: Value) -> Option<TableHandle> {
        match value {
            Value::Table(handle) => self.heap.resolve_table(handle).ok()?.metatable,
            Value::String(_) => Some(self.string_metatable),
            Value::Userdata(_) => Some(self.file_metatable),
            _ => None,
        }
    }

    fn value_type_name(&self, value: Value) -> &'static str {
        match value {
            Value::Nil => "nil",
            Value::Boolean(_) => "boolean",
            Value::Integer(_) | Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Closure(_) | Value::NativeFunction(_) => "function",
            Value::Thread(_) => "thread",
            Value::Userdata(_) => "userdata",
            Value::LightUserdata(_) => "light userdata",
        }
    }

    fn bitwise_metamethod_name(&self, op_name: &str) -> &'static str {
        match op_name {
            "bitwise or" => "__bor",
            "bitwise xor" => "__bxor",
            "bitwise and" => "__band",
            "shift left" => "__shl",
            "shift right" => "__shr",
            _ => "__band",
        }
    }

    fn metamethod_depth_limit(&self) -> usize {
        32
    }

    fn call_value_sync(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        _depth: usize,
    ) -> KResult<Value> {
        let returned = self.call_value_multi(callee, args)?;
        Ok(returned.into_iter().next().unwrap_or(Value::nil()))
    }

    fn call_value_multi(&mut self, callee: Value, args: Vec<Value>) -> KResult<Vec<Value>> {
        let (callee, args) = self.callable_with_args(callee, args)?;
        match callee {
            Value::NativeFunction(function) => self.call_native(function, &args),
            Value::Closure(handle) => {
                if !self.frames.is_empty() {
                    return self.run_nested_closure_with_args(handle, args);
                }
                let saved_stack = std::mem::take(&mut self.stack);
                let saved_frames = std::mem::take(&mut self.frames);
                let saved_open_upvalues = std::mem::take(&mut self.open_upvalues);
                let saved_last_results = saved_frames
                    .last()
                    .map(|frame| frame.last_call_results)
                    .unwrap_or(0);
                let result = self.run_closure_with_args(handle, args);
                self.stack = saved_stack;
                self.frames = saved_frames;
                self.open_upvalues = saved_open_upvalues;
                if let Some(frame) = self.frames.last_mut() {
                    frame.last_call_results = saved_last_results;
                }
                result
            }
            _ => Err(KError::new(
                KErrorKind::Runtime("attempt to call a non-callable value".to_owned()),
                None,
            )),
        }
    }

    fn run_nested_closure_with_args(
        &mut self,
        closure: ClosureHandle,
        args: Vec<Value>,
    ) -> KResult<Vec<Value>> {
        if self.frames.len() >= 64 {
            return Err(KError::new(
                KErrorKind::Runtime("stack overflow".to_owned()),
                None,
            ));
        }
        let proto = self.heap.resolve_closure(closure)?.proto.clone();
        let base = self.stack.len();
        let size = usize::from(proto.stack_size.max(1)).max(args.len());
        self.ensure_stack_len(
            base.checked_add(size).ok_or_else(|| {
                KError::new(KErrorKind::Runtime("stack overflow".to_owned()), None)
            })?,
        )?;
        for (index, value) in args.iter().enumerate() {
            if let Some(slot) = self.stack.get_mut(base.saturating_add(index)) {
                *slot = *value;
            }
        }
        let parameters = usize::from(proto.parameters);
        for index in args.len()..parameters {
            if let Some(slot) = self.stack.get_mut(base.saturating_add(index)) {
                *slot = Value::nil();
            }
        }
        let varargs = if proto.is_vararg && args.len() > parameters {
            args.get(parameters..).unwrap_or(&[]).to_vec()
        } else {
            Vec::new()
        };
        let depth = self.frames.len();
        self.frames.push(Frame {
            closure,
            base,
            top: base.saturating_add(size),
            pc: 0,
            return_target: None,
            varargs,
            last_call_results: 0,
        });
        let mut finished = Vec::new();
        let result = (|| {
            while self.frames.len() > depth {
                let frame_index = self.frames.len().saturating_sub(1);
                let instruction = self.current_instruction(frame_index)?;
                self.execute_instruction(frame_index, instruction, &mut finished)?;
            }
            Ok(finished)
        })();
        if result.is_err() {
            self.close_upvalues_from(base)?;
            self.frames.truncate(depth);
        }
        self.stack.truncate(base);
        result
    }

    fn callable_with_args(
        &mut self,
        mut callee: Value,
        mut args: Vec<Value>,
    ) -> KResult<(Value, Vec<Value>)> {
        for _ in 0..self.metamethod_depth_limit() {
            if matches!(callee, Value::NativeFunction(_) | Value::Closure(_)) {
                return Ok((callee, args));
            }
            let Some(metatable) = self.metatable_of_value(callee) else {
                break;
            };
            let Some(call) = self.table_metamethod(metatable, "__call")? else {
                break;
            };
            args.insert(0, callee);
            callee = call;
        }
        Err(KError::new(
            KErrorKind::Runtime("attempt to call a non-callable value".to_owned()),
            None,
        ))
    }

    fn load_chunk_bytes(
        &mut self,
        bytes: &[u8],
        mode: Option<&str>,
        env: Option<Value>,
    ) -> KResult<ClosureHandle> {
        let is_binary = bytes.starts_with(b"KUUBIN\0");
        let bytes = if !is_binary && bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            bytes.get(3..).unwrap_or(&[])
        } else {
            bytes
        };
        match mode {
            Some("b") if !is_binary => {
                return Err(KError::new(
                    KErrorKind::Runtime("attempt to load a text chunk as binary".to_owned()),
                    None,
                ));
            }
            Some("t") if is_binary => {
                return Err(KError::new(
                    KErrorKind::Runtime("attempt to load a binary chunk as text".to_owned()),
                    None,
                ));
            }
            _ => {}
        }

        let proto = if is_binary {
            Proto::decode(bytes.get(7..).ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("binary chunk header truncated".to_owned()),
                    None,
                )
            })?)?
        } else {
            let source = std::str::from_utf8(bytes).map_err(|_| {
                KError::new(
                    KErrorKind::Runtime("source chunk must be valid UTF-8".to_owned()),
                    None,
                )
            })?;
            let mut parser = Parser::new(source)?;
            let chunk = parser.parse_chunk()?;
            let mut compiler = Compiler::new();
            compiler.compile_chunk(&chunk)?
        };

        match env {
            Some(Value::Table(env_table)) => {
                self.instantiate_root_closure_with_env(proto, env_table)
            }
            Some(other) => Err(KError::new(
                KErrorKind::Runtime(format!(
                    "load environment must be a table, got {}",
                    self.value_type_name(other)
                )),
                None,
            )),
            None => self.instantiate_root_closure(proto),
        }
    }

    fn instantiate_root_closure_with_env(
        &mut self,
        proto: Proto,
        env: TableHandle,
    ) -> KResult<ClosureHandle> {
        let mut upvalues = Vec::with_capacity(proto.upvalues.len());
        for descriptor in &proto.upvalues {
            if descriptor.instack {
                return Err(KError::new(
                    KErrorKind::Runtime("root closure cannot capture stack values".to_owned()),
                    None,
                ));
            }
            if descriptor.index == 0 {
                upvalues.push(self.heap.new_upvalue_closed(Value::table(env))?);
            } else {
                return Err(KError::new(
                    KErrorKind::Runtime("root closure missing parent upvalue".to_owned()),
                    None,
                ));
            }
        }
        self.heap.new_closure(proto, upvalues)
    }

    fn run_closure_with_args(
        &mut self,
        closure: ClosureHandle,
        args: Vec<Value>,
    ) -> KResult<Vec<Value>> {
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();

        let stack_size = self.heap.resolve_closure(closure)?.proto.stack_size.max(1);
        let frame_stack_size = usize::from(stack_size).max(args.len());
        self.ensure_stack_len(frame_stack_size)?;
        self.frames.push(Frame {
            closure,
            base: 0,
            top: frame_stack_size,
            pc: 0,
            return_target: None,
            varargs: Vec::new(),
            last_call_results: 0,
        });

        self.push_arguments_into_root_frame(args)?;
        let mut finished = Vec::new();
        while !self.frames.is_empty() {
            let frame_index = self.frames.len() - 1;
            let instruction = self.current_instruction(frame_index)?;
            self.execute_instruction(frame_index, instruction, &mut finished)?;
        }
        Ok(finished)
    }

    fn push_arguments_into_root_frame(&mut self, args: Vec<Value>) -> KResult<()> {
        for (index, value) in args.iter().enumerate() {
            if let Some(slot) = self.stack.get_mut(index) {
                *slot = *value;
            }
        }
        Ok(())
    }

    fn execute_instruction(
        &mut self,
        frame_index: usize,
        instruction: Instruction,
        finished: &mut Vec<Value>,
    ) -> KResult<()> {
        match instruction {
            Instruction::LoadNil { dst } => {
                self.write_register(frame_index, dst, Value::nil())?;
                self.advance_pc(frame_index)?;
            }
            Instruction::LoadBool { dst, value } => {
                self.write_register(frame_index, dst, Value::boolean(value))?;
                self.advance_pc(frame_index)?;
            }
            Instruction::LoadInteger { dst, value } => {
                self.write_register(frame_index, dst, Value::integer(value))?;
                self.advance_pc(frame_index)?;
            }
            Instruction::LoadNumber { dst, value } => {
                self.write_register(frame_index, dst, Value::number(value))?;
                self.advance_pc(frame_index)?;
            }
            Instruction::LoadConstant { dst, constant } => {
                let value = self.constant_to_value(frame_index, constant)?;
                self.write_register(frame_index, dst, value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Move { dst, src } => {
                let value = self.read_register(frame_index, src)?;
                self.write_register(frame_index, dst, value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::GetGlobal { dst, name } => {
                let value = self.get_global(frame_index, name)?;
                self.write_register(frame_index, dst, value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::SetGlobal { src, name } => {
                let value = self.read_register(frame_index, src)?;
                self.set_global(frame_index, name, value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::CheckGlobal { table, key } => {
                let table = self.read_register(frame_index, table)?;
                let key = self.read_register(frame_index, key)?;
                self.check_global_unset(table, key)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::GcStep => {
                if self.gc_running {
                    let _ = self.gc_step(1)?;
                }
                self.advance_pc(frame_index)?;
            }
            Instruction::GetTable { dst, table, key } => {
                let table_value = self.read_register(frame_index, table)?;
                let key_value = self.read_register(frame_index, key)?;
                let value = self.table_get(table_value, key_value)?;
                self.write_register(frame_index, dst, value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::SetTable { table, key, value } => {
                let table_value = self.read_register(frame_index, table)?;
                let key_value = self.read_register(frame_index, key)?;
                let value_value = self.read_register(frame_index, value)?;
                self.table_set(table_value, key_value, value_value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::SetTableRange {
                table,
                start,
                values,
                count,
            } => {
                self.set_table_range(frame_index, table, start, values, count)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Arithmetic {
                op,
                dst,
                left,
                right,
            } => {
                let left_value = self.read_register(frame_index, left)?;
                let right_value = self.read_register(frame_index, right)?;
                let result = self.arithmetic(op, left_value, right_value)?;
                self.write_register(frame_index, dst, result)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Compare {
                op,
                dst,
                left,
                right,
            } => {
                let left_value = self.read_register(frame_index, left)?;
                let right_value = self.read_register(frame_index, right)?;
                let result = self.compare(op, left_value, right_value)?;
                self.write_register(frame_index, dst, Value::boolean(result))?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Jump { offset } => {
                self.jump(frame_index, offset)?;
            }
            Instruction::JumpIfTrue { cond, offset } => {
                let value = self.read_register(frame_index, cond)?;
                if self.is_truthy(value) {
                    self.jump(frame_index, offset)?;
                } else {
                    self.advance_pc(frame_index)?;
                }
            }
            Instruction::JumpIfFalse { cond, offset } => {
                let value = self.read_register(frame_index, cond)?;
                if !self.is_truthy(value) {
                    self.jump(frame_index, offset)?;
                } else {
                    self.advance_pc(frame_index)?;
                }
            }
            Instruction::NewTable { dst } => {
                let handle = self.new_table_handle()?;
                self.write_register(frame_index, dst, Value::table(handle))?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Closure { dst, proto } => {
                let closure = self.instantiate_child_closure(frame_index, proto)?;
                self.write_register(frame_index, dst, Value::closure(closure))?;
                self.advance_pc(frame_index)?;
            }
            Instruction::GetUpvalue { dst, upvalue } => {
                let value = self.get_upvalue(frame_index, upvalue)?;
                self.write_register(frame_index, dst, value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::SetUpvalue { src, upvalue } => {
                let value = self.read_register(frame_index, src)?;
                self.set_upvalue(frame_index, upvalue, value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Vararg { dst, count } => {
                self.write_varargs(frame_index, dst, count)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Call {
                function,
                args,
                results,
            } => {
                let call_slot = self.absolute_register(frame_index, function)?;
                let callee = self.read_register(frame_index, function)?;
                let args = self.read_call_args(frame_index, function, args)?;
                self.advance_pc(frame_index)?;
                self.invoke_call(
                    CallSite {
                        frame_index,
                        call_slot,
                        results: usize::from(results),
                        tail: false,
                    },
                    callee,
                    args,
                    finished,
                )?;
            }
            Instruction::TailCall { function, args } => {
                let call_slot = self.absolute_register(frame_index, function)?;
                let callee = self.read_register(frame_index, function)?;
                let args = self.read_call_args(frame_index, function, args)?;
                self.invoke_call(
                    CallSite {
                        frame_index,
                        call_slot,
                        results: 0,
                        tail: true,
                    },
                    callee,
                    args,
                    finished,
                )?;
            }
            Instruction::Return { first, count } => {
                let values = self.collect_return_values(frame_index, first, count)?;
                self.finish_frame(frame_index, values, finished)?;
            }
            Instruction::Close { from } => {
                let stack_index = self.absolute_register(frame_index, from)?;
                self.close_upvalues_from(stack_index)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Concat { dst, first, last } => {
                let value = self.concat_values(frame_index, first, last)?;
                self.write_register(frame_index, dst, value)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::Unary { op, dst, src } => {
                let value = self.read_register(frame_index, src)?;
                let result = self.unary(op, value)?;
                self.write_register(frame_index, dst, result)?;
                self.advance_pc(frame_index)?;
            }
            Instruction::ForPrep { base, offset } => {
                self.for_prep(frame_index, base, offset)?;
            }
            Instruction::ForLoop { base, offset } => {
                self.for_loop(frame_index, base, offset)?;
            }
        }
        Ok(())
    }

    fn format_value(&self, value: Value) -> KResult<String> {
        match value {
            Value::Nil => Ok("nil".to_owned()),
            Value::Boolean(value) => Ok(value.to_string()),
            Value::Integer(value) => Ok(value.to_string()),
            Value::Number(value) => Ok(value.to_string()),
            Value::String(handle) => {
                let bytes = self.heap.string_bytes(handle).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                Ok(String::from_utf8_lossy(bytes).into_owned())
            }
            other => Ok(other.to_string()),
        }
    }

    fn format_tostring(&self, value: Value) -> KResult<String> {
        let mut text = self.format_value(value)?;
        if matches!(value, Value::Number(value) if value.is_finite())
            && !text.contains(['.', 'e', 'E'])
        {
            text.push_str(".0");
        }
        Ok(text)
    }

    fn is_truthy(&self, value: Value) -> bool {
        !matches!(value, Value::Nil | Value::Boolean(false))
    }

    fn call_native(&mut self, function: NativeFunction, args: &[Value]) -> KResult<Vec<Value>> {
        function(self, args)
    }

    fn print_values(&mut self, args: &[Value]) -> KResult<()> {
        let mut out = io::stdout().lock();
        for (index, value) in args.iter().enumerate() {
            if index > 0 {
                out.write_all(b"\t")?;
            }
            let text = self.format_value(*value)?;
            out.write_all(text.as_bytes())?;
        }
        out.write_all(b"\n")?;
        out.flush()?;
        Ok(())
    }

    fn for_prep(&mut self, frame_index: usize, base: Register, offset: JumpOffset) -> KResult<()> {
        let base = self.absolute_register(frame_index, base)?;
        let init = self.stack.get(base).copied().unwrap_or(Value::nil());
        let step = self
            .stack
            .get(base + 2)
            .copied()
            .unwrap_or(Value::integer(1));
        let init = self.value_to_number(init)?;
        let step = self.value_to_number(step)?;
        if let Some(slot) = self.stack.get_mut(base) {
            *slot = Value::number(init - step);
        }
        self.jump(frame_index, offset)?;
        Ok(())
    }

    fn for_loop(&mut self, frame_index: usize, base: Register, offset: JumpOffset) -> KResult<()> {
        let base = self.absolute_register(frame_index, base)?;
        let limit =
            self.value_to_number(self.stack.get(base + 1).copied().unwrap_or(Value::nil()))?;
        let step = self.value_to_number(
            self.stack
                .get(base + 2)
                .copied()
                .unwrap_or(Value::integer(1)),
        )?;
        let control =
            self.value_to_number(self.stack.get(base).copied().unwrap_or(Value::nil()))? + step;
        if let Some(slot) = self.stack.get_mut(base) {
            *slot = Value::number(control);
        }
        let should_continue = if step >= 0.0 {
            control <= limit
        } else {
            control >= limit
        };
        if should_continue {
            if let Some(slot) = self.stack.get_mut(base + 3) {
                *slot = Value::number(control);
            }
            self.jump(frame_index, offset)?;
        } else {
            self.advance_pc(frame_index)?;
        }
        Ok(())
    }
}

impl Vm {
    fn gc_mode_name(&mut self, mode: GcMode) -> KResult<StringHandle> {
        let bytes = match mode {
            GcMode::Incremental => b"incremental".to_vec(),
            GcMode::Generational => b"generational".to_vec(),
        };
        self.heap.intern_string(bytes)
    }

    fn gc_param_value(&self, name: &str) -> KResult<usize> {
        match name {
            "pause" => Ok(self.gc_params.pause),
            "stepmul" => Ok(self.gc_params.stepmul),
            "stepsize" => Ok(self.gc_params.stepsize),
            _ => Err(KError::new(
                KErrorKind::Runtime(format!("unsupported GC parameter '{name}'")),
                None,
            )),
        }
    }

    fn gc_param_from_value(&self, _name: &str, value: Value) -> KResult<usize> {
        match value {
            Value::Integer(value) if value >= 0 => usize::try_from(value).map_err(|_| {
                KError::new(
                    KErrorKind::Runtime("GC parameter overflow".to_owned()),
                    None,
                )
            }),
            Value::Number(value) if value >= 0.0 && value.fract() == 0.0 => {
                usize::try_from(value as u64).map_err(|_| {
                    KError::new(
                        KErrorKind::Runtime("GC parameter overflow".to_owned()),
                        None,
                    )
                })
            }
            _ => Err(KError::new(
                KErrorKind::Runtime("GC parameter must be a non-negative integer".to_owned()),
                None,
            )),
        }
    }

    fn gc_set_param_value(&mut self, name: &str, value: usize) -> KResult<()> {
        match name {
            "pause" => self.gc_params.pause = value,
            "stepmul" => self.gc_params.stepmul = value,
            "stepsize" => self.gc_params.stepsize = value,
            _ => {
                return Err(KError::new(
                    KErrorKind::Runtime(format!("unsupported GC parameter '{name}'")),
                    None,
                ));
            }
        }
        Ok(())
    }

    fn gc_count_kib(&self) -> f64 {
        let bytes = self.gc_count_bytes();
        (bytes as f64) / 1024.0
    }

    fn gc_count_bytes(&self) -> usize {
        let mut bytes = 0usize;
        for string in &self.heap.strings {
            bytes = bytes.saturating_add(string.len().saturating_add(16));
        }
        for table in &self.heap.tables {
            if let Some(table) = table.as_ref() {
                bytes = bytes
                    .saturating_add(64)
                    .saturating_add(table.array.len().saturating_mul(16))
                    .saturating_add(table.hash.len().saturating_mul(32));
            }
        }
        for closure in &self.heap.closures {
            bytes = bytes
                .saturating_add(64)
                .saturating_add(closure.upvalues.len().saturating_mul(8));
        }
        bytes.saturating_add(self.heap.upvalues.len().saturating_mul(16))
    }

    fn gc_collect_full(&mut self) -> KResult<()> {
        let was_running = self.gc_running;
        self.gc_running = true;
        self.gc_begin_cycle()?;
        loop {
            if self.gc_step(self.gc_params.stepsize.max(1))? {
                break;
            }
        }
        self.gc_running = was_running;
        Ok(())
    }

    fn gc_step(&mut self, budget: usize) -> KResult<bool> {
        if !self.gc_running {
            self.gc_metrics.last_step_work = 0;
            return Ok(false);
        }

        if matches!(self.gc_phase, GcPhase::Pause) {
            self.gc_begin_cycle()?;
        }

        let mut remaining = budget.max(1);
        let mut work = 0usize;
        while remaining > 0 {
            match self.gc_phase {
                GcPhase::Mark => {
                    if let Some(handle) = self.gc_gray_tables.pop() {
                        self.gc_visit_table(handle)?;
                        remaining = remaining.saturating_sub(1);
                        work = work.saturating_add(1);
                        continue;
                    }
                    if let Some(handle) = self.gc_gray_closures.pop() {
                        self.gc_visit_closure(handle)?;
                        remaining = remaining.saturating_sub(1);
                        work = work.saturating_add(1);
                        continue;
                    }
                    self.gc_clear_weak_tables()?;
                    self.gc_phase = GcPhase::Sweep;
                    self.gc_sweep_cursor = 0;
                }
                GcPhase::Sweep => {
                    if self.gc_sweep_cursor >= self.heap.tables.len() {
                        self.gc_phase = GcPhase::Finalize;
                        continue;
                    }
                    let index = self.gc_sweep_cursor;
                    self.gc_sweep_cursor = self.gc_sweep_cursor.saturating_add(1);
                    if self.gc_sweep_table(index)? {
                        work = work.saturating_add(1);
                    }
                    remaining = remaining.saturating_sub(1);
                }
                GcPhase::Finalize => {
                    if let Some(handle) = self.gc_finalize_queue.pop() {
                        self.gc_run_finalizer(handle)?;
                        work = work.saturating_add(1);
                        remaining = remaining.saturating_sub(1);
                        continue;
                    }
                    self.gc_phase = GcPhase::Pause;
                    self.gc_metrics.completed_cycles =
                        self.gc_metrics.completed_cycles.saturating_add(1);
                    break;
                }
                GcPhase::Pause => {
                    self.gc_begin_cycle()?;
                }
            }
        }

        self.gc_metrics.last_step_work = work;
        self.gc_metrics.total_step_work = self.gc_metrics.total_step_work.saturating_add(work);
        Ok(matches!(self.gc_phase, GcPhase::Pause))
    }

    fn gc_begin_cycle(&mut self) -> KResult<()> {
        self.gc_phase = GcPhase::Mark;
        self.gc_gray_tables.clear();
        self.gc_gray_closures.clear();
        self.gc_sweep_cursor = 0;
        self.gc_finalize_queue.clear();
        for table in self.heap.tables.iter_mut().flatten() {
            table.marked = false;
        }
        let metatables: Vec<TableHandle> = self
            .heap
            .tables
            .iter()
            .filter_map(|slot| slot.as_ref().and_then(|table| table.metatable))
            .collect();
        let stack_values = self.stack.clone();
        let frame_closures: Vec<ClosureHandle> =
            self.frames.iter().map(|frame| frame.closure).collect();
        let open_handles = self.open_upvalues.clone();
        self.gc_mark_value(Value::Table(self.globals));
        self.gc_mark_table(self.string_metatable);
        self.gc_mark_table(self.file_metatable);
        for handle in metatables {
            self.gc_mark_table(handle);
        }
        for value in stack_values {
            self.gc_mark_value(value);
        }
        for handle in frame_closures {
            self.gc_mark_closure(handle);
        }
        for handle in open_handles {
            let value = self.heap.upvalue_value(handle, &self.stack)?;
            self.gc_mark_value(value);
        }
        Ok(())
    }

    fn gc_mark_value(&mut self, value: Value) {
        match value {
            Value::Table(handle) => self.gc_mark_table(handle),
            Value::Closure(handle) => self.gc_mark_closure(handle),
            _ => {}
        }
    }

    fn gc_mark_table(&mut self, handle: TableHandle) {
        let index = match usize::try_from(handle.raw()) {
            Ok(index) => index,
            Err(_) => return,
        };
        let Some(Some(table)) = self.heap.tables.get_mut(index) else {
            return;
        };
        if table.marked {
            return;
        }
        table.marked = true;
        self.gc_gray_tables.push(handle);
    }

    fn gc_mark_closure(&mut self, handle: ClosureHandle) {
        let index = match usize::try_from(handle.raw()) {
            Ok(index) => index,
            Err(_) => return,
        };
        if self.heap.closures.get(index).is_none() {
            return;
        }
        self.gc_gray_closures.push(handle);
    }

    fn gc_visit_table(&mut self, handle: TableHandle) -> KResult<()> {
        let Some(table) = self.heap.resolve_table(handle).ok() else {
            return Ok(());
        };
        let metatable = table.metatable;
        let entries: Vec<(Value, Value)> = table.entries().collect();
        let (weak_keys, weak_values) = self.table_weak_mode(handle)?;
        if let Some(metatable) = metatable {
            self.gc_mark_table(metatable);
        }
        for (key, value) in entries {
            if !weak_keys {
                self.gc_mark_value(key);
            }
            if !weak_values {
                self.gc_mark_value(value);
            }
        }
        Ok(())
    }

    fn table_weak_mode(&mut self, table: TableHandle) -> KResult<(bool, bool)> {
        let Some(metatable) = self.heap.resolve_table(table)?.metatable else {
            return Ok((false, false));
        };
        let key = self.heap.intern_string(b"__mode".to_vec())?;
        let mode = self
            .heap
            .resolve_table(metatable)?
            .raw_get(Value::string(key))?;
        let Value::String(mode) = mode else {
            return Ok((false, false));
        };
        let bytes = self.heap.string_bytes(mode).unwrap_or_default();
        Ok((bytes.contains(&b'k'), bytes.contains(&b'v')))
    }

    fn gc_clear_weak_tables(&mut self) -> KResult<()> {
        let dead: BTreeSet<u64> = self
            .heap
            .tables
            .iter()
            .enumerate()
            .filter_map(|(index, table)| {
                table
                    .as_ref()
                    .filter(|table| !table.marked)
                    .and_then(|_| u64::try_from(index).ok())
            })
            .collect();
        let marked_tables: Vec<TableHandle> = self
            .heap
            .tables
            .iter()
            .enumerate()
            .filter_map(|(index, table)| {
                table
                    .as_ref()
                    .filter(|table| table.marked)
                    .and_then(|_| u64::try_from(index).ok().map(TableHandle::new))
            })
            .collect();
        let weak_tables: Vec<(TableHandle, bool, bool)> = marked_tables
            .into_iter()
            .map(|handle| {
                self.table_weak_mode(handle)
                    .map(|(keys, values)| (handle, keys, values))
            })
            .collect::<KResult<Vec<_>>>()?;
        for (handle, weak_keys, weak_values) in weak_tables {
            if weak_keys || weak_values {
                self.heap.resolve_table_mut(handle)?.clear_weak_entries(
                    weak_keys,
                    weak_values,
                    &dead,
                );
            }
        }
        Ok(())
    }

    fn gc_visit_closure(&mut self, handle: ClosureHandle) -> KResult<()> {
        let closure = self.heap.resolve_closure(handle)?.clone();
        for upvalue in closure.upvalues {
            if let Some(index) = self.heap.upvalue_stack_index(upvalue) {
                if let Some(value) = self.stack.get(index).copied() {
                    self.gc_mark_value(value);
                }
            } else {
                let value = self.heap.upvalue_value(upvalue, &self.stack)?;
                self.gc_mark_value(value);
            }
        }
        Ok(())
    }

    fn gc_sweep_table(&mut self, index: usize) -> KResult<bool> {
        let Some(snapshot) = self.heap.tables.get(index).and_then(Option::as_ref) else {
            return Ok(false);
        };
        if snapshot.marked {
            if let Some(Some(table)) = self.heap.tables.get_mut(index) {
                table.marked = false;
            }
            return Ok(false);
        }

        let metatable = snapshot.metatable;
        let finalizer_ran = snapshot.finalizer_ran;
        let needs_finalizer = if !finalizer_ran {
            self.table_has_gc_finalizer(metatable)?
        } else {
            false
        };

        if needs_finalizer {
            if let Some(Some(table)) = self.heap.tables.get_mut(index) {
                table.finalizer_ran = true;
            }
            let handle = TableHandle::new(u64::try_from(index).map_err(|_| {
                KError::new(
                    KErrorKind::Runtime("table handle overflow".to_owned()),
                    None,
                )
            })?);
            self.gc_finalize_queue.push(handle);
            return Ok(true);
        }

        if let Some(slot) = self.heap.tables.get_mut(index) {
            *slot = None;
            return Ok(true);
        }

        Ok(false)
    }

    fn gc_run_finalizer(&mut self, handle: TableHandle) -> KResult<()> {
        let value = Value::Table(handle);
        let metatable = self.heap.resolve_table(handle)?.metatable;
        if let Some(metatable) = metatable {
            let finalizer = self.table_gc_value(metatable)?;
            if let Value::NativeFunction(function) = finalizer {
                let _ = function(self, &[value])?;
            }
        }
        self.gc_metrics.finalized_objects = self.gc_metrics.finalized_objects.saturating_add(1);
        Ok(())
    }

    fn table_has_gc_finalizer(&mut self, metatable: Option<TableHandle>) -> KResult<bool> {
        let Some(metatable) = metatable else {
            return Ok(false);
        };
        let finalizer = self.table_gc_value(metatable)?;
        Ok(!matches!(finalizer, Value::Nil))
    }

    fn table_gc_value(&mut self, metatable: TableHandle) -> KResult<Value> {
        let key = self.heap.intern_string(b"__gc".to_vec())?;
        let metatable_table = self.heap.resolve_table(metatable)?;
        metatable_table.raw_get(Value::string(key))
    }

    fn math_seed_rng(&mut self, seed_x: u64, seed_y: u64) {
        self.math_rng_seed = (seed_x, seed_y);
        self.math_rng_state[0] = seed_x;
        self.math_rng_state[1] = 0xff;
        self.math_rng_state[2] = seed_y;
        self.math_rng_state[3] = 0;
        for _ in 0..16 {
            let _ = self.math_next_u64();
        }
    }

    fn math_next_u64(&mut self) -> u64 {
        let result = self.math_rng_state[1]
            .wrapping_mul(5)
            .rotate_left(7)
            .wrapping_mul(9);
        let t = self.math_rng_state[1] << 17;

        self.math_rng_state[2] ^= self.math_rng_state[0];
        self.math_rng_state[3] ^= self.math_rng_state[1];
        self.math_rng_state[1] ^= self.math_rng_state[2];
        self.math_rng_state[0] ^= self.math_rng_state[3];
        self.math_rng_state[2] ^= t;
        self.math_rng_state[3] = self.math_rng_state[3].rotate_left(45);
        result
    }

    #[allow(dead_code)]
    fn math_next_f64(&mut self) -> f64 {
        let bits = self.math_next_u64() >> 11;
        (bits as f64) * (1.0 / ((1u64 << 53) as f64))
    }

    fn math_random_integer(&mut self, low: i128, high: i128) -> KResult<i128> {
        let span = high
            .checked_sub(low)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("random interval overflow".to_owned()),
                    None,
                )
            })?;
        if span <= 0 {
            return Err(KError::new(
                KErrorKind::Runtime("random interval overflow".to_owned()),
                None,
            ));
        }

        let span_u128 = u128::try_from(span).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("random interval overflow".to_owned()),
                None,
            )
        })?;
        let limit = (u128::from(u64::MAX) + 1) / span_u128 * span_u128;
        loop {
            let sample = u128::from(self.math_next_u64());
            if sample < limit {
                let offset = sample % span_u128;
                let offset = i128::try_from(offset).map_err(|_| {
                    KError::new(
                        KErrorKind::Runtime("random interval overflow".to_owned()),
                        None,
                    )
                })?;
                return Ok(low + offset);
            }
        }
    }

    fn install_stdlib(&mut self) -> KResult<()> {
        self.install_base_lib()?;
        self.install_string_lib()?;
        self.install_utf8_lib()?;
        self.install_table_lib()?;
        self.install_math_lib()?;
        self.install_coroutine_lib()?;
        self.install_io_lib()?;
        self.install_os_lib()?;
        self.install_package_lib()?;
        self.install_debug_lib()?;
        Ok(())
    }

    fn install_base_lib(&mut self) -> KResult<()> {
        self.set_global_value(b"_G", Value::table(self.globals))?;
        let version = self.heap.intern_string(b"Lua 5.5".to_vec())?;
        self.set_global_value(b"_VERSION", Value::string(version))?;
        self.set_global_value(b"_port", Value::boolean(true))?;

        self.set_global_native(b"assert", native_assert)?;
        self.set_global_native(b"error", native_error)?;
        self.set_global_native(b"getmetatable", native_getmetatable)?;
        self.set_global_native(b"setmetatable", native_setmetatable)?;
        self.set_global_native(b"rawequal", native_rawequal)?;
        self.set_global_native(b"rawget", native_rawget)?;
        self.set_global_native(b"rawset", native_rawset)?;
        self.set_global_native(b"rawlen", native_rawlen)?;
        self.set_global_native(b"select", native_select)?;
        self.set_global_native(b"tonumber", native_tonumber)?;
        self.set_global_native(b"tostring", native_tostring)?;
        self.set_global_native(b"type", native_type)?;
        self.set_global_native(b"warn", native_warn)?;
        self.set_global_native(b"load", native_load)?;
        self.set_global_native(b"loadfile", native_loadfile)?;
        self.set_global_native(b"dofile", native_dofile)?;
        self.set_global_native(b"pcall", native_pcall)?;
        self.set_global_native(b"xpcall", native_xpcall)?;
        self.set_global_native(b"pairs", native_pairs)?;
        self.set_global_native(b"ipairs", native_ipairs)?;
        self.set_global_native(b"next", native_next)?;
        self.set_global_native(b"print", native_print)?;
        self.set_global_native(b"collectgarbage", native_collectgarbage)?;
        self.set_global_native(b"require", native_require)?;
        Ok(())
    }

    fn install_string_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("string", table)?;
        self.set_module_value(self.string_metatable, b"__index", Value::table(table))?;
        self.set_module_function(table, b"dump", native_string_dump)?;
        self.set_module_function(table, b"len", native_string_len)?;
        self.set_module_function(table, b"sub", native_string_sub)?;
        self.set_module_function(table, b"byte", native_string_byte)?;
        self.set_module_function(table, b"char", native_string_char)?;
        self.set_module_function(table, b"rep", native_string_rep)?;
        self.set_module_function(table, b"lower", native_string_lower)?;
        self.set_module_function(table, b"upper", native_string_upper)?;
        self.set_module_function(table, b"reverse", native_string_reverse)?;
        self.set_module_function(table, b"format", native_string_format)?;
        self.set_module_function(table, b"find", native_string_find)?;
        self.set_module_function(table, b"match", native_string_match)?;
        self.set_module_function(table, b"gsub", native_string_gsub)?;
        self.set_module_function(table, b"gmatch", native_string_gmatch)?;
        self.set_module_function(table, b"pack", native_string_pack)?;
        self.set_module_function(table, b"unpack", native_string_unpack)?;
        self.set_module_function(table, b"packsize", native_string_packsize)?;
        Ok(())
    }

    fn install_utf8_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("utf8", table)?;
        self.set_module_function(table, b"char", native_utf8_char)?;
        self.set_module_function(table, b"codes", native_utf8_codes)?;
        self.set_module_function(table, b"codepoint", native_utf8_codepoint)?;
        self.set_module_function(table, b"len", native_utf8_len)?;
        let charpattern = self
            .heap
            .intern_string(b"[\x00-\x7F\xc0-\xfd][\x80-\xbf]*".to_vec())?;
        self.set_module_value(table, b"charpattern", Value::string(charpattern))?;
        Ok(())
    }

    fn install_table_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("table", table)?;
        self.set_module_function(table, b"concat", native_table_concat)?;
        self.set_module_function(table, b"insert", native_table_insert)?;
        self.set_module_function(table, b"remove", native_table_remove)?;
        self.set_module_function(table, b"move", native_table_move)?;
        self.set_module_function(table, b"pack", native_table_pack)?;
        self.set_module_function(table, b"unpack", native_table_unpack)?;
        self.set_module_function(table, b"sort", native_table_sort)?;
        Ok(())
    }

    fn install_math_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("math", table)?;
        self.set_module_function(table, b"abs", native_math_abs)?;
        self.set_module_function(table, b"acos", native_math_acos)?;
        self.set_module_function(table, b"asin", native_math_asin)?;
        self.set_module_function(table, b"atan", native_math_atan)?;
        self.set_module_function(table, b"ceil", native_math_ceil)?;
        self.set_module_function(table, b"cos", native_math_cos)?;
        self.set_module_function(table, b"deg", native_math_deg)?;
        self.set_module_function(table, b"exp", native_math_exp)?;
        self.set_module_function(table, b"floor", native_math_floor)?;
        self.set_module_function(table, b"fmod", native_math_fmod)?;
        self.set_module_function(table, b"frexp", native_math_frexp)?;
        self.set_module_function(table, b"ldexp", native_math_ldexp)?;
        self.set_module_function(table, b"max", native_math_max)?;
        self.set_module_function(table, b"min", native_math_min)?;
        self.set_module_function(table, b"log", native_math_log)?;
        self.set_module_function(table, b"modf", native_math_modf)?;
        self.set_module_function(table, b"rad", native_math_rad)?;
        self.set_module_function(table, b"random", native_math_random)?;
        self.set_module_function(table, b"randomseed", native_math_randomseed)?;
        self.set_module_function(table, b"sin", native_math_sin)?;
        self.set_module_function(table, b"sqrt", native_math_sqrt)?;
        self.set_module_function(table, b"tan", native_math_tan)?;
        self.set_module_function(table, b"tointeger", native_math_tointeger)?;
        self.set_module_function(table, b"type", native_math_type)?;
        self.set_module_function(table, b"ult", native_math_ult)?;
        self.set_module_value(table, b"pi", Value::number(std::f64::consts::PI))?;
        self.set_module_value(table, b"huge", Value::number(f64::INFINITY))?;
        self.set_module_value(table, b"maxinteger", Value::integer(i64::MAX))?;
        self.set_module_value(table, b"mininteger", Value::integer(i64::MIN))?;
        Ok(())
    }

    fn install_coroutine_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("coroutine", table)
    }

    fn install_io_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("io", table)?;
        let file_metatable = self.heap.new_table()?;
        self.file_metatable = file_metatable;
        let file_name = self.heap.intern_string(b"FILE*".to_vec())?;
        self.set_module_value(file_metatable, b"__name", Value::string(file_name))?;
        self.set_module_value(file_metatable, b"__index", Value::table(file_metatable))?;
        self.set_module_function(file_metatable, b"close", native_io_close)?;
        self.set_module_function(file_metatable, b"write", native_io_write)?;
        self.set_module_function(file_metatable, b"read", native_io_read)?;
        self.set_module_function(file_metatable, b"flush", native_io_close)?;
        self.set_module_function(file_metatable, b"seek", native_io_read)?;
        self.set_module_function(table, b"open", native_io_open)?;
        self.set_module_function(table, b"lines", native_io_lines)?;
        self.set_module_function(table, b"input", native_io_input)?;
        self.set_module_function(table, b"output", native_io_output)?;
        self.set_module_function(table, b"read", native_io_read)?;
        self.set_module_function(table, b"write", native_io_write)?;
        self.set_module_function(table, b"close", native_io_close)?;
        self.set_module_function(table, b"tmpfile", native_io_tmpfile)?;
        let stdin = self.new_userdata(UserdataObject::File(FileObject {
            kind: FileKind::Stdin,
        }))?;
        let stdout = self.new_userdata(UserdataObject::File(FileObject {
            kind: FileKind::Stdout,
        }))?;
        let stderr = self.new_userdata(UserdataObject::File(FileObject {
            kind: FileKind::Stderr,
        }))?;
        self.current_input = stdin;
        self.current_output = stdout;
        self.set_module_value(table, b"stdin", Value::userdata(stdin))?;
        self.set_module_value(table, b"stdout", Value::userdata(stdout))?;
        self.set_module_value(table, b"stderr", Value::userdata(stderr))?;
        self.set_module_function(table, b"type", native_io_type)?;
        Ok(())
    }

    fn install_os_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("os", table)?;
        self.set_module_function(table, b"execute", native_os_execute)?;
        self.set_module_function(table, b"clock", native_os_clock)?;
        self.set_module_function(table, b"difftime", native_os_difftime)?;
        self.set_module_function(table, b"time", native_os_time)?;
        self.set_module_function(table, b"tmpname", native_os_tmpname)?;
        self.set_module_function(table, b"rename", native_os_rename)?;
        self.set_module_function(table, b"remove", native_os_remove)?;
        self.set_module_function(table, b"setlocale", native_os_setlocale)?;
        Ok(())
    }

    fn install_package_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("package", table)?;
        let loaded = self.heap.new_table()?;
        let preload = self.heap.new_table()?;
        let searchers = self.heap.new_table()?;
        let path = self.heap.intern_string(b"?.lua;?/init.lua".to_vec())?;
        let cpath = self.heap.intern_string(b"".to_vec())?;
        let config = self.heap.intern_string(b"/\n;\n?\n!\n-".to_vec())?;
        self.set_module_value(table, b"loaded", Value::table(loaded))?;
        self.set_module_value(table, b"preload", Value::table(preload))?;
        self.set_module_value(table, b"searchers", Value::table(searchers))?;
        self.set_module_value(table, b"path", Value::string(path))?;
        self.set_module_value(table, b"cpath", Value::string(cpath))?;
        self.set_module_value(table, b"config", Value::string(config))?;
        self.set_module_function(table, b"searchpath", native_package_searchpath)?;
        self.set_module_function(table, b"loadlib", native_package_loadlib)?;
        self.set_module_function(table, b"require", native_require)?;
        Ok(())
    }

    fn install_debug_lib(&mut self) -> KResult<()> {
        let table = self.heap.new_table()?;
        self.set_module_table("debug", table)?;
        self.set_module_function(table, b"traceback", native_debug_traceback)?;
        self.set_module_function(table, b"getinfo", native_debug_getinfo)?;
        self.set_module_function(table, b"getupvalue", native_debug_getupvalue)?;
        self.set_module_function(table, b"setupvalue", native_debug_setupvalue)?;
        self.set_module_function(table, b"upvalueid", native_debug_upvalueid)?;
        self.set_module_function(table, b"upvaluejoin", native_debug_upvaluejoin)?;
        self.set_module_function(table, b"getlocal", native_debug_getlocal)?;
        self.set_module_function(table, b"setlocal", native_debug_setlocal)?;
        self.set_module_function(table, b"gethook", native_debug_gethook)?;
        self.set_module_function(table, b"sethook", native_debug_sethook)?;
        self.set_module_function(table, b"getregistry", native_debug_getregistry)?;
        Ok(())
    }

    fn set_module_table(&mut self, name: &str, table: TableHandle) -> KResult<()> {
        let handle = self.heap.intern_string(name.as_bytes().to_vec())?;
        let table_value = Value::table(table);
        self.set_global_value(name.as_bytes(), table_value)?;
        self.heap
            .resolve_table_mut(self.globals)?
            .raw_set(Value::string(handle), table_value)
    }

    fn set_module_value(&mut self, table: TableHandle, name: &[u8], value: Value) -> KResult<()> {
        let key = self.heap.intern_string(name.to_vec())?;
        self.heap
            .resolve_table_mut(table)?
            .raw_set(Value::string(key), value)
    }

    fn set_module_function(
        &mut self,
        table: TableHandle,
        name: &[u8],
        function: NativeFunction,
    ) -> KResult<()> {
        self.set_module_value(table, name, Value::native(function))
    }

    fn set_global_native(&mut self, name: &[u8], function: NativeFunction) -> KResult<()> {
        let key = self.heap.intern_string(name.to_vec())?;
        self.heap
            .resolve_table_mut(self.globals)?
            .raw_set(Value::string(key), Value::native(function))
    }

    fn set_global_value(&mut self, name: &[u8], value: Value) -> KResult<()> {
        let key = self.heap.intern_string(name.to_vec())?;
        self.heap
            .resolve_table_mut(self.globals)?
            .raw_set(Value::string(key), value)
    }

    pub fn global_value(&self, name: &[u8]) -> KResult<Option<Value>> {
        let table = self.heap.resolve_table(self.globals)?;
        for (key, value) in &table.hash {
            let LuaKey::String(handle) = key else {
                continue;
            };
            let Some(bytes) = self.heap.string_bytes(*handle) else {
                continue;
            };
            if bytes == name {
                return Ok(Some(*value));
            }
        }
        Ok(None)
    }

    pub fn global_string(&self, name: &[u8]) -> KResult<Option<String>> {
        let Some(value) = self.global_value(name)? else {
            return Ok(None);
        };
        match value {
            Value::String(handle) => {
                let bytes = self.heap.string_bytes(handle).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                Ok(Some(String::from_utf8_lossy(bytes).into_owned()))
            }
            _ => Ok(None),
        }
    }

    pub fn package_path(&mut self) -> KResult<String> {
        self.package_string_field("path")
    }

    pub fn package_cpath(&mut self) -> KResult<String> {
        self.package_string_field("cpath")
    }

    pub fn set_package_path(&mut self, value: &str) -> KResult<()> {
        self.set_package_string_field("path", value)
    }

    pub fn set_package_cpath(&mut self, value: &str) -> KResult<()> {
        self.set_package_string_field("cpath", value)
    }

    pub fn set_cli_args_from_raw(
        &mut self,
        binary: &str,
        raw_args: &[String],
        script_index: Option<usize>,
        implicit_script_name: Option<&str>,
    ) -> KResult<()> {
        let table = self.heap.new_table()?;
        match script_index {
            Some(index) => {
                let binary_slot = i64::try_from(index)
                    .map_err(|_| {
                        KError::new(
                            KErrorKind::Runtime("argument index overflow".to_owned()),
                            None,
                        )
                    })?
                    .checked_add(1)
                    .and_then(|value| value.checked_neg())
                    .ok_or_else(|| {
                        KError::new(
                            KErrorKind::Runtime("argument index overflow".to_owned()),
                            None,
                        )
                    })?;
                self.set_cli_arg_entry(table, binary_slot, binary)?;

                for arg_index in 0..index {
                    let remaining = index.checked_sub(arg_index).ok_or_else(|| {
                        KError::new(
                            KErrorKind::Runtime("argument index overflow".to_owned()),
                            None,
                        )
                    })?;
                    let slot = i64::try_from(remaining).map_err(|_| {
                        KError::new(
                            KErrorKind::Runtime("argument index overflow".to_owned()),
                            None,
                        )
                    })?;
                    self.set_cli_arg_entry(
                        table,
                        slot.checked_neg().ok_or_else(|| {
                            KError::new(
                                KErrorKind::Runtime("argument index overflow".to_owned()),
                                None,
                            )
                        })?,
                        raw_args.get(arg_index).ok_or_else(|| {
                            KError::new(
                                KErrorKind::Runtime("missing command-line argument".to_owned()),
                                None,
                            )
                        })?,
                    )?;
                }

                let script_name = if let Some(name) = implicit_script_name {
                    name.to_owned()
                } else {
                    raw_args.get(index).cloned().ok_or_else(|| {
                        KError::new(KErrorKind::Runtime("missing script name".to_owned()), None)
                    })?
                };
                self.set_cli_arg_entry(table, 0, &script_name)?;

                for (offset, arg) in raw_args.get(index + 1..).unwrap_or(&[]).iter().enumerate() {
                    let slot = i64::try_from(offset + 1).map_err(|_| {
                        KError::new(
                            KErrorKind::Runtime("argument index overflow".to_owned()),
                            None,
                        )
                    })?;
                    self.set_cli_arg_entry(table, slot, arg)?;
                }
            }
            None => {
                if let Some(script_name) = implicit_script_name {
                    self.set_cli_arg_entry(table, -1, binary)?;
                    self.set_cli_arg_entry(table, 0, script_name)?;
                } else {
                    self.set_cli_arg_entry(table, 0, binary)?;
                    for (offset, arg) in raw_args.iter().enumerate() {
                        let slot = i64::try_from(offset + 1).map_err(|_| {
                            KError::new(
                                KErrorKind::Runtime("argument index overflow".to_owned()),
                                None,
                            )
                        })?;
                        self.set_cli_arg_entry(table, slot, arg)?;
                    }
                }
            }
        }
        self.set_global_value(b"arg", Value::table(table))
    }

    pub fn require_module_into(&mut self, module: &str, global_name: &str) -> KResult<()> {
        let value = self.require_module_value(module)?;
        self.set_global_value(global_name.as_bytes(), value)
    }

    pub fn print_values_to_stdout(&mut self, values: &[Value]) -> KResult<()> {
        self.print_values(values)
    }

    pub fn set_warnings_enabled(&mut self, enabled: bool) {
        self.warn_enabled = enabled;
    }

    fn set_cli_arg_entry(&mut self, table: TableHandle, slot: i64, text: &str) -> KResult<()> {
        let handle = self.heap.intern_string(text.as_bytes().to_vec())?;
        self.heap
            .resolve_table_mut(table)?
            .raw_set(Value::integer(slot), Value::string(handle))
    }

    fn package_table(&self) -> KResult<TableHandle> {
        match self.global_value(b"package")? {
            Some(Value::Table(handle)) => Ok(handle),
            Some(other) => Err(KError::new(
                KErrorKind::Runtime(format!(
                    "package is not a table, got {}",
                    self.value_type_name(other)
                )),
                None,
            )),
            None => Err(KError::new(
                KErrorKind::Runtime("package library is not loaded".to_owned()),
                None,
            )),
        }
    }

    fn package_table_field(&mut self, field: &str) -> KResult<Value> {
        let package = self.package_table()?;
        let key = self.heap.intern_string(field.as_bytes().to_vec())?;
        self.heap
            .resolve_table(package)?
            .raw_get(Value::string(key))
    }

    fn package_loaded_table(&mut self) -> KResult<TableHandle> {
        match self.package_table_field("loaded")? {
            Value::Table(handle) => Ok(handle),
            other => Err(KError::new(
                KErrorKind::Runtime(format!(
                    "package.loaded is not a table, got {}",
                    self.value_type_name(other)
                )),
                None,
            )),
        }
    }

    fn package_preload_table(&mut self) -> KResult<TableHandle> {
        match self.package_table_field("preload")? {
            Value::Table(handle) => Ok(handle),
            other => Err(KError::new(
                KErrorKind::Runtime(format!(
                    "package.preload is not a table, got {}",
                    self.value_type_name(other)
                )),
                None,
            )),
        }
    }

    fn package_path_text(&mut self) -> KResult<String> {
        self.package_string_field("path")
    }

    fn package_string_field(&mut self, field: &str) -> KResult<String> {
        match self.package_table_field(field)? {
            Value::String(handle) => {
                let bytes = self.heap.string_bytes(handle).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("invalid string handle".to_owned()),
                        None,
                    )
                })?;
                Ok(String::from_utf8_lossy(bytes).into_owned())
            }
            other => Err(KError::new(
                KErrorKind::Runtime(format!(
                    "package.{field} is not a string, got {}",
                    self.value_type_name(other)
                )),
                None,
            )),
        }
    }

    fn set_package_string_field(&mut self, field: &str, value: &str) -> KResult<()> {
        let package = self.package_table()?;
        let key = self.heap.intern_string(field.as_bytes().to_vec())?;
        let value_handle = self.heap.intern_string(value.as_bytes().to_vec())?;
        self.heap
            .resolve_table_mut(package)?
            .raw_set(Value::string(key), Value::string(value_handle))
    }

    fn require_module_value(&mut self, module: &str) -> KResult<Value> {
        let loaded = self.package_loaded_table()?;
        let module_key = self.heap.intern_string(module.as_bytes().to_vec())?;
        let key_value = Value::string(module_key);

        let cached = self.heap.resolve_table(loaded)?.raw_get(key_value)?;
        if !matches!(cached, Value::Nil) {
            return Ok(cached);
        }

        let preload = self.package_preload_table()?;
        let preload_loader = self.heap.resolve_table(preload)?.raw_get(key_value)?;
        if !matches!(preload_loader, Value::Nil) {
            return self.finish_require_loader(module, loaded, key_value, preload_loader);
        }

        if let Some(global_value) = self.global_value(module.as_bytes())?
            && !matches!(global_value, Value::Nil)
        {
            self.heap
                .resolve_table_mut(loaded)?
                .raw_set(key_value, global_value)?;
            return Ok(global_value);
        }

        let package_path = self.package_path_text()?;
        if let Some(found_path) = self.search_package_path(module, &package_path)? {
            let bytes = fs::read(&found_path)?;
            let closure = self.load_chunk_bytes(&bytes, None, None)?;
            return self.finish_require_loader(module, loaded, key_value, Value::closure(closure));
        }

        let cpath = self.package_cpath()?;
        let module_path = module.replace('.', "/");
        let attempts = package_path
            .split(';')
            .chain(cpath.split(';'))
            .filter(|template| !template.is_empty())
            .map(|template| format!("\n\tno file '{}'", template.replace('?', &module_path)))
            .collect::<String>();
        Err(KError::new(
            KErrorKind::Runtime(format!(
                "module '{module}' not found:\n\tno field package.preload['{module}']{attempts}"
            )),
            None,
        ))
    }

    fn finish_require_loader(
        &mut self,
        module: &str,
        loaded: TableHandle,
        module_key: Value,
        loader: Value,
    ) -> KResult<Value> {
        let module_handle = self.heap.intern_string(module.as_bytes().to_vec())?;
        let results = self.call_value_multi(loader, vec![Value::string(module_handle)])?;
        let first = results.first().copied().unwrap_or(Value::nil());
        let stored = if !matches!(first, Value::Nil) {
            first
        } else {
            let existing = self.heap.resolve_table(loaded)?.raw_get(module_key)?;
            if matches!(existing, Value::Nil) {
                Value::boolean(true)
            } else {
                existing
            }
        };
        self.heap
            .resolve_table_mut(loaded)?
            .raw_set(module_key, stored)?;
        Ok(stored)
    }

    fn search_package_path(&self, module: &str, package_path: &str) -> KResult<Option<String>> {
        let module_path = module.replace('.', "/");
        for template in package_path.split(';') {
            if template.is_empty() {
                continue;
            }
            let candidate = template.replace('?', &module_path);
            if std::path::Path::new(&candidate).is_file() {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }

    #[allow(dead_code)]
    fn new_table_value(&mut self) -> KResult<Value> {
        Ok(Value::table(self.heap.new_table()?))
    }

    fn new_userdata(&mut self, object: UserdataObject) -> KResult<UserdataHandle> {
        let raw = u64::try_from(self.userdatas.len()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("userdata handle overflow".to_owned()),
                None,
            )
        })?;
        self.userdatas.push(Some(Rc::new(RefCell::new(object))));
        Ok(UserdataHandle::new(raw))
    }

    #[allow(dead_code)]
    fn resolve_userdata(&self, handle: UserdataHandle) -> KResult<Rc<RefCell<UserdataObject>>> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(
                KErrorKind::Runtime("invalid userdata handle".to_owned()),
                None,
            )
        })?;
        self.userdatas
            .get(index)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or_else(|| {
                KError::new(
                    KErrorKind::Runtime("invalid userdata handle".to_owned()),
                    None,
                )
            })
    }
}

fn apply_jump(pc: usize, offset: JumpOffset) -> KResult<usize> {
    let next = i64::try_from(pc)
        .map_err(|_| {
            KError::new(
                KErrorKind::Runtime("program counter overflow".to_owned()),
                None,
            )
        })?
        .checked_add(1)
        .and_then(|value| value.checked_add(i64::from(offset.value())))
        .ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("program counter overflow".to_owned()),
                None,
            )
        })?;
    usize::try_from(next).map_err(|_| {
        KError::new(
            KErrorKind::Runtime("program counter overflow".to_owned()),
            None,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn compile_proto(source: &str) -> KResult<Proto> {
        let mut parser = Parser::new(source)?;
        let chunk = parser.parse_chunk()?;
        let mut compiler = Compiler::new();
        compiler.compile_chunk(&chunk)
    }

    fn intern_string(vm: &mut Vm, bytes: &[u8]) -> KResult<Value> {
        let handle = vm.heap.intern_string(bytes.to_vec())?;
        Ok(Value::string(handle))
    }

    fn set_metafield(
        vm: &mut Vm,
        metatable: TableHandle,
        name: &[u8],
        value: Value,
    ) -> KResult<()> {
        let key = vm.heap.intern_string(name.to_vec())?;
        let table = vm.heap.resolve_table_mut(metatable)?;
        table.raw_set(Value::string(key), value)
    }

    fn test_arg(args: &[Value], index: usize) -> KResult<Value> {
        args.get(index).copied().ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime(format!("missing metamethod argument {index}")),
                None,
            )
        })
    }

    #[test]
    fn arg_or_nil_defaults_missing_values_to_nil() -> Result<(), Box<dyn std::error::Error>> {
        let vm = Vm::new()?;
        assert_eq!(vm.arg_or_nil(&[], 0), Value::nil());
        assert_eq!(vm.arg_or_nil(&[Value::integer(7)], 1), Value::nil());
        Ok(())
    }

    #[test]
    fn string_helpers_report_invalid_handles() -> Result<(), Box<dyn std::error::Error>> {
        let vm = Vm::new()?;
        let err = match vm.string_bytes_from_handle(StringHandle::new(9999)) {
            Ok(_) => return Err("expected invalid string handle".into()),
            Err(err) => err,
        };
        assert!(err.to_string().contains("invalid string handle"));
        Ok(())
    }

    #[test]
    fn typed_string_helper_reports_type_name() -> Result<(), Box<dyn std::error::Error>> {
        let vm = Vm::new()?;
        let err = match vm.string_text_arg_typed(
            &[Value::integer(1)],
            0,
            "loadfile expects a string path",
        ) {
            Ok(_) => return Err("expected type error".into()),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("loadfile expects a string path, got number")
        );
        Ok(())
    }

    #[test]
    fn numeric_helpers_preserve_integer_and_float_conversion()
    -> Result<(), Box<dyn std::error::Error>> {
        let vm = Vm::new()?;
        assert_eq!(
            vm.integer_arg(&[Value::integer(7)], 0, "integer expected")?,
            7
        );
        assert_eq!(
            vm.integer_arg(&[Value::number(7.0)], 0, "integer expected")?,
            7
        );
        assert_eq!(
            vm.number_arg(&[Value::integer(7)], 0, "number expected")?,
            7.0
        );
        assert_eq!(
            vm.number_arg(&[Value::number(7.5)], 0, "number expected")?,
            7.5
        );
        Ok(())
    }

    #[test]
    fn arithmetic_uses_add_metamethod() -> Result<(), Box<dyn std::error::Error>> {
        fn add(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 2);
            let left = test_arg(args, 0)?;
            let right = test_arg(args, 1)?;
            assert!(matches!(left, Value::Table(_)));
            assert_eq!(right, Value::integer(5));
            Ok(vec![Value::integer(42)])
        }

        let mut vm = Vm::new()?;
        let table = vm.heap.new_table()?;
        let metatable = vm.heap.new_table()?;
        set_metafield(&mut vm, metatable, b"__add", Value::native(add))?;
        vm.heap.resolve_table_mut(table)?.metatable = Some(metatable);

        let method = vm
            .call_binary_metamethod(Value::table(table), Value::integer(5), "__add", 0)?
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing __add".to_owned()), None))?;
        assert_eq!(method, Value::integer(42));

        let value = vm.arithmetic(ArithmeticOp::Add, Value::table(table), Value::integer(5))?;
        assert_eq!(value, Value::integer(42));
        Ok(())
    }

    #[test]
    fn run_proto_executes_root_and_nested_calls() -> Result<(), Box<dyn std::error::Error>> {
        let mut vm = Vm::new()?;
        let proto = compile_proto(
            "local t = {}\n\
             t.root = 1\n\
             local function bump(a, ...)\n\
               t.inner = a + 1\n\
               return ...\n\
             end\n\
             local first, second = bump(41, 42, 43)\n\
             return t.root, t.inner, first, second\n",
        )?;

        let results = vm.run_proto(&proto)?;
        assert_eq!(
            results,
            vec![
                Value::integer(1),
                Value::integer(42),
                Value::integer(42),
                Value::integer(43),
            ]
        );
        Ok(())
    }

    #[test]
    fn comparison_uses_eq_metamethod() -> Result<(), Box<dyn std::error::Error>> {
        fn eq(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 2);
            assert!(matches!(test_arg(args, 0)?, Value::Table(_)));
            assert!(matches!(test_arg(args, 1)?, Value::Table(_)));
            Ok(vec![Value::boolean(true)])
        }

        let mut vm = Vm::new()?;
        let left = vm.heap.new_table()?;
        let right = vm.heap.new_table()?;
        let metatable = vm.heap.new_table()?;
        set_metafield(&mut vm, metatable, b"__eq", Value::native(eq))?;
        vm.heap.resolve_table_mut(left)?.metatable = Some(metatable);
        vm.heap.resolve_table_mut(right)?.metatable = Some(metatable);

        let value = vm.compare(CompareOp::Eq, Value::table(left), Value::table(right))?;
        assert!(value);
        Ok(())
    }

    #[test]
    fn concat_uses_concat_metamethod() -> Result<(), Box<dyn std::error::Error>> {
        fn concat(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 2);
            assert!(matches!(test_arg(args, 0)?, Value::Table(_)));
            assert_eq!(test_arg(args, 1)?, Value::integer(5));
            Ok(vec![Value::integer(77)])
        }

        let mut vm = Vm::new()?;
        let table = vm.heap.new_table()?;
        let metatable = vm.heap.new_table()?;
        set_metafield(&mut vm, metatable, b"__concat", Value::native(concat))?;
        vm.heap.resolve_table_mut(table)?.metatable = Some(metatable);

        let result = vm.concat_values_for_test(&[Value::table(table), Value::integer(5)])?;
        assert_eq!(result, Value::integer(77));
        Ok(())
    }

    #[test]
    fn bitwise_uses_string_metamethod_when_coercion_fails() -> Result<(), Box<dyn std::error::Error>>
    {
        fn band(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 2);
            assert!(matches!(test_arg(args, 0)?, Value::String(_)));
            assert_eq!(test_arg(args, 1)?, Value::integer(5));
            Ok(vec![Value::integer(13)])
        }

        let mut vm = Vm::new()?;
        let string_metatable = vm.heap.new_table()?;
        set_metafield(&mut vm, string_metatable, b"__band", Value::native(band))?;
        vm.string_metatable = string_metatable;

        let value = intern_string(&mut vm, b"not-an-integer")?;
        let result = vm.arithmetic(ArithmeticOp::BitAnd, value, Value::integer(5))?;
        assert_eq!(result, Value::integer(13));
        Ok(())
    }

    #[test]
    fn bitwise_uses_string_metamethod_for_non_utf8_strings()
    -> Result<(), Box<dyn std::error::Error>> {
        fn bor(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 2);
            assert!(matches!(test_arg(args, 0)?, Value::String(_)));
            assert_eq!(test_arg(args, 1)?, Value::integer(5));
            Ok(vec![Value::integer(21)])
        }

        let mut vm = Vm::new()?;
        let string_metatable = vm.heap.new_table()?;
        set_metafield(&mut vm, string_metatable, b"__bor", Value::native(bor))?;
        vm.string_metatable = string_metatable;

        let value = intern_string(&mut vm, b"\xff")?;
        let result = vm.arithmetic(ArithmeticOp::BitOr, value, Value::integer(5))?;
        assert_eq!(result, Value::integer(21));
        Ok(())
    }

    #[test]
    fn bitwise_shifts_follow_lua_negative_and_large_shift_rules()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut vm = Vm::new()?;

        assert_eq!(
            vm.arithmetic(
                ArithmeticOp::ShiftLeft,
                Value::integer(16),
                Value::integer(-3)
            )?,
            Value::integer(16 >> 3)
        );
        assert_eq!(
            vm.arithmetic(
                ArithmeticOp::ShiftRight,
                Value::integer(16),
                Value::integer(-3)
            )?,
            Value::integer(16 << 3)
        );
        assert_eq!(
            vm.arithmetic(
                ArithmeticOp::ShiftLeft,
                Value::integer(-1),
                Value::integer(64)
            )?,
            Value::integer(0)
        );
        assert_eq!(
            vm.arithmetic(
                ArithmeticOp::ShiftRight,
                Value::integer(-1),
                Value::integer(64)
            )?,
            Value::integer(0)
        );
        Ok(())
    }

    #[test]
    fn unary_minus_bitnot_and_length_use_metamethods() -> Result<(), Box<dyn std::error::Error>> {
        fn unm(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 1);
            let _ = test_arg(args, 0)?;
            Ok(vec![Value::integer(-9)])
        }

        fn bnot(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 1);
            let _ = test_arg(args, 0)?;
            Ok(vec![Value::integer(7)])
        }

        fn len(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 1);
            let _ = test_arg(args, 0)?;
            Ok(vec![Value::integer(33)])
        }

        let mut vm = Vm::new()?;
        let table = vm.heap.new_table()?;
        let metatable = vm.heap.new_table()?;
        set_metafield(&mut vm, metatable, b"__unm", Value::native(unm))?;
        set_metafield(&mut vm, metatable, b"__bnot", Value::native(bnot))?;
        set_metafield(&mut vm, metatable, b"__len", Value::native(len))?;
        vm.heap.resolve_table_mut(table)?.metatable = Some(metatable);

        assert_eq!(
            vm.unary(UnaryOpKind::Minus, Value::table(table))?,
            Value::integer(-9)
        );
        assert_eq!(
            vm.unary(UnaryOpKind::BitNot, Value::table(table))?,
            Value::integer(7)
        );
        assert_eq!(
            vm.unary(UnaryOpKind::Len, Value::table(table))?,
            Value::integer(33)
        );
        Ok(())
    }

    #[test]
    fn index_and_newindex_support_table_and_function_forms()
    -> Result<(), Box<dyn std::error::Error>> {
        fn indexer(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 2);
            assert!(matches!(test_arg(args, 0)?, Value::Table(_)));
            assert_eq!(test_arg(args, 1)?, Value::integer(9));
            Ok(vec![Value::integer(99)])
        }

        fn newindexer(_: &mut Vm, args: &[Value]) -> KResult<Vec<Value>> {
            assert_eq!(args.len(), 3);
            assert!(matches!(test_arg(args, 0)?, Value::Table(_)));
            assert_eq!(test_arg(args, 1)?, Value::integer(7));
            assert_eq!(test_arg(args, 2)?, Value::integer(88));
            Ok(Vec::new())
        }

        let mut vm = Vm::new()?;
        let fallback = vm.heap.new_table()?;
        vm.heap
            .resolve_table_mut(fallback)?
            .raw_set(Value::integer(9), Value::integer(11))?;

        let base = vm.heap.new_table()?;
        let metatable = vm.heap.new_table()?;
        set_metafield(&mut vm, metatable, b"__index", Value::table(fallback))?;
        set_metafield(&mut vm, metatable, b"__newindex", Value::table(fallback))?;
        vm.heap.resolve_table_mut(base)?.metatable = Some(metatable);

        let indexed = vm.table_get(Value::table(base), Value::integer(9))?;
        assert_eq!(indexed, Value::integer(11));

        vm.table_set(Value::table(base), Value::integer(7), Value::integer(88))?;
        let stored = vm
            .heap
            .resolve_table(fallback)?
            .raw_get(Value::integer(7))?;
        assert_eq!(stored, Value::integer(88));

        let function_mt = vm.heap.new_table()?;
        set_metafield(&mut vm, function_mt, b"__index", Value::native(indexer))?;
        set_metafield(
            &mut vm,
            function_mt,
            b"__newindex",
            Value::native(newindexer),
        )?;
        let function_table = vm.heap.new_table()?;
        vm.heap.resolve_table_mut(function_table)?.metatable = Some(function_mt);
        let function_indexed = vm.table_get(Value::table(function_table), Value::integer(9))?;
        assert_eq!(function_indexed, Value::integer(99));
        vm.table_set(
            Value::table(function_table),
            Value::integer(7),
            Value::integer(88),
        )?;

        Ok(())
    }

    #[test]
    fn index_and_newindex_loop_detection_is_clear() -> Result<(), Box<dyn std::error::Error>> {
        let mut vm = Vm::new()?;
        let table = vm.heap.new_table()?;
        let metatable = vm.heap.new_table()?;
        set_metafield(&mut vm, metatable, b"__index", Value::table(table))?;
        set_metafield(&mut vm, metatable, b"__newindex", Value::table(table))?;
        vm.heap.resolve_table_mut(table)?.metatable = Some(metatable);

        let index_err = vm
            .table_get(Value::table(table), Value::integer(1))
            .err()
            .ok_or_else(|| {
                KError::new(KErrorKind::Runtime("expected loop error".to_owned()), None)
            })?;
        assert!(index_err.to_string().contains("gettable"));

        let set_err = vm
            .table_set(Value::table(table), Value::integer(1), Value::integer(2))
            .err()
            .ok_or_else(|| {
                KError::new(KErrorKind::Runtime("expected loop error".to_owned()), None)
            })?;
        assert!(set_err.to_string().contains("settable"));

        Ok(())
    }

    #[test]
    fn collectgarbage_mode_and_param_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let mut vm = Vm::new()?;

        let pause = intern_string(&mut vm, b"pause")?;
        let stepmul = intern_string(&mut vm, b"stepmul")?;
        let stepsize = intern_string(&mut vm, b"stepsize")?;
        let incremental = intern_string(&mut vm, b"incremental")?;
        let generational = intern_string(&mut vm, b"generational")?;
        let param = intern_string(&mut vm, b"param")?;

        let previous_pause = vm.collectgarbage_request(&[param, pause, Value::integer(220)])?;
        assert_eq!(previous_pause, vec![Value::integer(100)]);

        let current_pause = vm.collectgarbage_request(&[param, pause])?;
        assert_eq!(current_pause, vec![Value::integer(220)]);

        let previous_stepmul = vm.collectgarbage_request(&[param, stepmul, Value::integer(175)])?;
        assert_eq!(previous_stepmul, vec![Value::integer(100)]);

        let current_stepmul = vm.collectgarbage_request(&[param, stepmul])?;
        assert_eq!(current_stepmul, vec![Value::integer(175)]);

        let previous_stepsize =
            vm.collectgarbage_request(&[param, stepsize, Value::integer(32)])?;
        assert_eq!(previous_stepsize, vec![Value::integer(64)]);

        let current_stepsize = vm.collectgarbage_request(&[param, stepsize])?;
        assert_eq!(current_stepsize, vec![Value::integer(32)]);

        let previous_mode = vm.collectgarbage_request(&[generational])?;
        assert_eq!(previous_mode, vec![incremental]);

        let previous_mode = vm.collectgarbage_request(&[incremental])?;
        assert_eq!(previous_mode, vec![generational]);

        Ok(())
    }

    #[test]
    fn gc_step_stays_within_budget() -> Result<(), Box<dyn std::error::Error>> {
        let mut vm = Vm::new()?;

        for _ in 0..8 {
            let handle = vm.heap.new_table()?;
            let table = vm.heap.resolve_table_mut(handle)?;
            table.raw_set(Value::integer(1), Value::integer(1))?;
        }

        let completed = vm.gc_step(2)?;
        assert!(!completed || matches!(vm.gc_phase, GcPhase::Pause));
        assert!(vm.gc_metrics.last_step_work <= 2);

        Ok(())
    }

    #[test]
    fn active_execution_state_round_trips_without_leaking_stack_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut vm = Vm::new()?;
        vm.stack.push(Value::integer(7));
        let state = vm.take_active_execution();
        assert!(vm.stack.is_empty());
        assert_eq!(state.stack, vec![Value::integer(7)]);
        vm.restore_active_execution(state);
        assert_eq!(vm.stack, vec![Value::integer(7)]);
        Ok(())
    }

    #[test]
    fn unreachable_table_cycles_are_collected_and_finalized()
    -> Result<(), Box<dyn std::error::Error>> {
        static FINALIZED: AtomicBool = AtomicBool::new(false);

        fn mark_finalized(_: &mut Vm, _: &[Value]) -> KResult<Vec<Value>> {
            FINALIZED.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }

        FINALIZED.store(false, Ordering::SeqCst);

        let mut vm = Vm::new()?;
        let left = vm.heap.new_table()?;
        let right = vm.heap.new_table()?;
        let metatable = vm.heap.new_table()?;
        let gc_key = intern_string(&mut vm, b"__gc")?;

        {
            let mt = vm.heap.resolve_table_mut(metatable)?;
            mt.raw_set(gc_key, Value::native(mark_finalized))?;
        }

        {
            let table = vm.heap.resolve_table_mut(left)?;
            table.metatable = Some(metatable);
            table.raw_set(Value::integer(1), Value::table(right))?;
        }
        {
            let table = vm.heap.resolve_table_mut(right)?;
            table.raw_set(Value::integer(1), Value::table(left))?;
        }

        let left_index = usize::try_from(left.raw())?;
        let right_index = usize::try_from(right.raw())?;

        let _ = vm.collectgarbage_request(&[])?;
        assert!(FINALIZED.load(Ordering::SeqCst));
        assert!(
            vm.heap
                .tables
                .get(left_index)
                .and_then(Option::as_ref)
                .is_some()
        );
        assert!(
            vm.heap
                .tables
                .get(right_index)
                .and_then(Option::as_ref)
                .is_none()
        );

        let _ = vm.collectgarbage_request(&[])?;
        assert!(
            vm.heap
                .tables
                .get(left_index)
                .and_then(Option::as_ref)
                .is_none()
        );
        assert!(
            vm.heap
                .tables
                .get(right_index)
                .and_then(Option::as_ref)
                .is_none()
        );

        Ok(())
    }
}
