use crate::error::{KError, KErrorKind, KResult};
use crate::instruction::{
    ArithmeticOp, CompareOp, ConstantIndex, Instruction, JumpOffset, PrototypeIndex, Register,
};
use crate::proto::{Constant, Proto};
use crate::value::{ClosureHandle, LuaKey, NativeFunction, StringHandle, TableHandle, Value};
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::io::{self, Write};

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

#[derive(Debug, Clone)]
struct TableObject {
    array: Vec<Value>,
    hash: BTreeMap<LuaKey, Value>,
}

impl TableObject {
    fn new() -> Self {
        Self {
            array: Vec::new(),
            hash: BTreeMap::new(),
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
            None => Err(KError::new(
                KErrorKind::Runtime("table index is not hashable".to_owned()),
                None,
            )),
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
}

#[derive(Debug, Clone)]
struct Heap {
    strings: Vec<Vec<u8>>,
    string_lookup: BTreeMap<Vec<u8>, StringHandle>,
    tables: Vec<TableObject>,
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
        self.tables.push(TableObject::new());
        Ok(TableHandle::new(raw))
    }

    fn resolve_table(&self, handle: TableHandle) -> KResult<&TableObject> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(KErrorKind::Runtime("invalid table handle".to_owned()), None)
        })?;
        self.tables.get(index).ok_or_else(|| {
            KError::new(KErrorKind::Runtime("invalid table handle".to_owned()), None)
        })
    }

    fn resolve_table_mut(&mut self, handle: TableHandle) -> KResult<&mut TableObject> {
        let index = usize::try_from(handle.raw()).map_err(|_| {
            KError::new(KErrorKind::Runtime("invalid table handle".to_owned()), None)
        })?;
        self.tables.get_mut(index).ok_or_else(|| {
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

pub fn native_print(args: &[Value]) -> KResult<Vec<Value>> {
    let mut out = io::stdout().lock();
    for (index, value) in args.iter().enumerate() {
        if index > 0 {
            out.write_all(b"\t")?;
        }
        let text = value.to_string();
        out.write_all(text.as_bytes())?;
    }
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(Vec::new())
}

#[derive(Debug, Clone)]
pub struct Vm {
    heap: Heap,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    open_upvalues: Vec<UpvalueHandle>,
    globals: TableHandle,
}

impl Vm {
    pub fn new() -> KResult<Self> {
        let mut heap = Heap::new();
        let globals = heap.new_table()?;
        {
            let print_name = heap.intern_string(b"print".to_vec())?;
            let table = heap.resolve_table_mut(globals)?;
            table.raw_set(Value::string(print_name), Value::native(native_print))?;
        }
        Ok(Self {
            heap,
            stack: Vec::new(),
            frames: Vec::new(),
            open_upvalues: Vec::new(),
            globals,
        })
    }

    pub fn run_proto(&mut self, proto: &Proto) -> KResult<Vec<Value>> {
        let closure = self.instantiate_root_closure(proto.clone())?;
        self.run_closure(closure)
    }

    fn run_closure(&mut self, closure: ClosureHandle) -> KResult<Vec<Value>> {
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();

        self.stack.push(Value::closure(closure));
        let stack_size = self.heap.resolve_closure(closure)?.proto.stack_size.max(1);
        self.ensure_stack_len(usize::from(stack_size))?;
        self.frames.push(Frame {
            closure,
            base: 0,
            top: usize::from(stack_size),
            pc: 0,
            return_target: None,
            varargs: Vec::new(),
            last_call_results: 0,
        });

        let mut finished = Vec::new();
        while !self.frames.is_empty() {
            let frame_index = self.frames.len() - 1;
            let instruction = self.current_instruction(frame_index)?;
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
                    let handle = self.heap.new_table()?;
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
                        &mut finished,
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
                        &mut finished,
                    )?;
                }
                Instruction::Return { first, count } => {
                    let values = self.collect_return_values(frame_index, first, count)?;
                    self.finish_frame(frame_index, values, &mut finished)?;
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
                Instruction::ForPrep { base, offset } => {
                    self.for_prep(frame_index, base, offset)?;
                }
                Instruction::ForLoop { base, offset } => {
                    self.for_loop(frame_index, base, offset)?;
                }
            }
        }

        Ok(finished)
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
        let frame = self
            .frames
            .get_mut(frame_index)
            .ok_or_else(|| KError::new(KErrorKind::Runtime("missing frame".to_owned()), None))?;
        frame.pc = frame.pc.checked_add(1).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("program counter overflow".to_owned()),
                None,
            )
        })?;
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
            frame.last_call_results
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
        self.ensure_stack_len(
            base.checked_add(usize::from(proto.stack_size.max(1)))
                .ok_or_else(|| {
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
                let slot = base
                    .checked_add(1)
                    .and_then(|start| start.checked_add(index))
                    .ok_or_else(|| {
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
            top: base
                .checked_add(usize::from(proto.stack_size.max(1)))
                .ok_or_else(|| {
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
        self.close_upvalues_from(base)?;
        self.ensure_stack_len(
            base.checked_add(usize::from(proto.stack_size.max(1)))
                .ok_or_else(|| {
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
                let slot = base
                    .checked_add(1)
                    .and_then(|start| start.checked_add(index))
                    .ok_or_else(|| {
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
            .checked_add(usize::from(proto.stack_size.max(1)))
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
            return Ok(frame.varargs.clone());
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

    fn table_get(&mut self, table: Value, key: Value) -> KResult<Value> {
        let Value::Table(handle) = table else {
            return Err(KError::new(
                KErrorKind::Runtime("attempt to index a non-table value".to_owned()),
                None,
            ));
        };
        let table = self.heap.resolve_table(handle)?;
        table.raw_get(key)
    }

    fn table_set(&mut self, table: Value, key: Value, value: Value) -> KResult<()> {
        let Value::Table(handle) = table else {
            return Err(KError::new(
                KErrorKind::Runtime("attempt to index a non-table value".to_owned()),
                None,
            ));
        };
        let table = self.heap.resolve_table_mut(handle)?;
        table.raw_set(key, value)
    }

    fn arithmetic(&mut self, op: ArithmeticOp, left: Value, right: Value) -> KResult<Value> {
        match op {
            ArithmeticOp::Add => self.numeric_add(left, right),
            ArithmeticOp::Sub => self.numeric_sub(left, right),
            ArithmeticOp::Mul => self.numeric_mul(left, right),
            ArithmeticOp::Div => self.numeric_div(left, right),
            ArithmeticOp::FloorDiv => self.numeric_floor_div(left, right),
            ArithmeticOp::Mod => self.numeric_mod(left, right),
            ArithmeticOp::Pow => self.numeric_pow(left, right),
        }
    }

    fn compare(&mut self, op: CompareOp, left: Value, right: Value) -> KResult<bool> {
        match op {
            CompareOp::Eq => Ok(left == right),
            CompareOp::NotEq => Ok(left != right),
            CompareOp::Less => {
                self.ordering(left, right, |value| value == std::cmp::Ordering::Less)
            }
            CompareOp::LessEq => self.ordering(left, right, |value| {
                value == std::cmp::Ordering::Less || value == std::cmp::Ordering::Equal
            }),
            CompareOp::Greater => {
                self.ordering(left, right, |value| value == std::cmp::Ordering::Greater)
            }
            CompareOp::GreaterEq => self.ordering(left, right, |value| {
                value == std::cmp::Ordering::Greater || value == std::cmp::Ordering::Equal
            }),
        }
    }

    fn ordering<F>(&self, left: Value, right: Value, predicate: F) -> KResult<bool>
    where
        F: Fn(std::cmp::Ordering) -> bool,
    {
        if let Some(ordering) = self.compare_values(left, right)? {
            Ok(predicate(ordering))
        } else {
            Err(KError::new(
                KErrorKind::Runtime("incomparable values".to_owned()),
                None,
            ))
        }
    }

    fn compare_values(&self, left: Value, right: Value) -> KResult<Option<std::cmp::Ordering>> {
        match (left, right) {
            (Value::Integer(left), Value::Integer(right)) => Ok(Some(left.cmp(&right))),
            (Value::Integer(left), Value::Number(right)) => {
                self.numeric_ordering(left as f64, right)
            }
            (Value::Number(left), Value::Integer(right)) => {
                self.numeric_ordering(left, right as f64)
            }
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

    fn numeric_ordering(&self, left: f64, right: f64) -> KResult<Option<std::cmp::Ordering>> {
        left.partial_cmp(&right).map(Some).ok_or_else(|| {
            KError::new(
                KErrorKind::Runtime("non-finite numeric comparison".to_owned()),
                None,
            )
        })
    }

    fn numeric_add(&self, left: Value, right: Value) -> KResult<Value> {
        if let Some(value) = self.integer_op(left, right, i64::checked_add) {
            return value;
        }
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left + right))
    }

    fn numeric_sub(&self, left: Value, right: Value) -> KResult<Value> {
        if let Some(value) = self.integer_op(left, right, i64::checked_sub) {
            return value;
        }
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left - right))
    }

    fn numeric_mul(&self, left: Value, right: Value) -> KResult<Value> {
        if let Some(value) = self.integer_op(left, right, i64::checked_mul) {
            return value;
        }
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left * right))
    }

    fn numeric_div(&self, left: Value, right: Value) -> KResult<Value> {
        let (left, right) = self.coerce_numbers(left, right)?;
        if right == 0.0 {
            return Err(KError::new(
                KErrorKind::Runtime("division by zero".to_owned()),
                None,
            ));
        }
        Ok(Value::number(left / right))
    }

    fn numeric_floor_div(&self, left: Value, right: Value) -> KResult<Value> {
        let (left, right) = self.coerce_numbers(left, right)?;
        if right == 0.0 {
            return Err(KError::new(
                KErrorKind::Runtime("division by zero".to_owned()),
                None,
            ));
        }
        Ok(Value::number((left / right).floor()))
    }

    fn numeric_mod(&self, left: Value, right: Value) -> KResult<Value> {
        let (left, right) = self.coerce_numbers(left, right)?;
        if right == 0.0 {
            return Err(KError::new(
                KErrorKind::Runtime("division by zero".to_owned()),
                None,
            ));
        }
        Ok(Value::number(left % right))
    }

    fn numeric_pow(&self, left: Value, right: Value) -> KResult<Value> {
        let (left, right) = self.coerce_numbers(left, right)?;
        Ok(Value::number(left.powf(right)))
    }

    fn integer_op(
        &self,
        left: Value,
        right: Value,
        op: fn(i64, i64) -> Option<i64>,
    ) -> Option<KResult<Value>> {
        match (left, right) {
            (Value::Integer(left), Value::Integer(right)) => {
                Some(op(left, right).map(Value::integer).ok_or_else(|| {
                    KError::new(
                        KErrorKind::Runtime("integer arithmetic overflow".to_owned()),
                        None,
                    )
                }))
            }
            _ => None,
        }
    }

    fn coerce_numbers(&self, left: Value, right: Value) -> KResult<(f64, f64)> {
        Ok((self.value_to_number(left)?, self.value_to_number(right)?))
    }

    fn value_to_number(&self, value: Value) -> KResult<f64> {
        match value {
            Value::Integer(value) => Ok(value as f64),
            Value::Number(value) => Ok(value),
            _ => Err(KError::new(
                KErrorKind::Runtime("numeric value expected".to_owned()),
                None,
            )),
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
        let mut bytes = Vec::new();
        for index in start..=end {
            let value = self.stack.get(index).copied().unwrap_or(Value::nil());
            let text = self.format_value(value)?;
            bytes.extend_from_slice(text.as_bytes());
        }
        let handle = self.heap.intern_string(bytes)?;
        Ok(Value::string(handle))
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

    fn is_truthy(&self, value: Value) -> bool {
        !matches!(value, Value::Nil | Value::Boolean(false))
    }

    fn call_native(&mut self, function: NativeFunction, args: &[Value]) -> KResult<Vec<Value>> {
        if std::ptr::fn_addr_eq(function, native_print as NativeFunction) {
            self.print_values(args)?;
            Ok(Vec::new())
        } else {
            function(args)
        }
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
