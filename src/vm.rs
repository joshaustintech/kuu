use crate::error::{KError, KErrorKind, KResult};
use crate::instruction::{
    ArithmeticOp, CompareOp, ConstantIndex, Instruction, JumpOffset, PrototypeIndex, Register,
    UnaryOpKind,
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

    fn iter_values(&self) -> impl Iterator<Item = Value> + '_ {
        self.array
            .iter()
            .copied()
            .chain(self.hash.values().copied())
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
    stack: Vec<Value>,
    frames: Vec<Frame>,
    open_upvalues: Vec<UpvalueHandle>,
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
        {
            let print_name = heap.intern_string(b"print".to_vec())?;
            let gc_name = heap.intern_string(b"collectgarbage".to_vec())?;
            let table = heap.resolve_table_mut(globals)?;
            table.raw_set(Value::string(print_name), Value::native(native_print))?;
            table.raw_set(Value::string(gc_name), Value::native(native_collectgarbage))?;
        }
        Ok(Self {
            heap,
            stack: Vec::new(),
            frames: Vec::new(),
            open_upvalues: Vec::new(),
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
        })
    }

    pub fn run_proto(&mut self, proto: &Proto) -> KResult<Vec<Value>> {
        let closure = self.instantiate_root_closure(proto.clone())?;
        self.run_closure(closure)
    }

    pub fn collectgarbage_request(&mut self, args: &[Value]) -> KResult<Vec<Value>> {
        let operation = match args.first() {
            Some(Value::String(handle)) => self.heap.string_bytes(*handle).map_or_else(
                || {
                    Err(KError::new(
                        KErrorKind::Runtime("invalid collectgarbage operation".to_owned()),
                        None,
                    ))
                },
                |bytes| {
                    std::str::from_utf8(bytes).map_err(|_| {
                        KError::new(
                            KErrorKind::Runtime(
                                "collectgarbage expects a UTF-8 operation".to_owned(),
                            ),
                            None,
                        )
                    })
                },
            )?,
            None => "collect",
            _ => {
                return Err(KError::new(
                    KErrorKind::Runtime("collectgarbage expects a string operation".to_owned()),
                    None,
                ));
            }
        };

        match operation {
            "collect" | "collectgarbage" => {
                self.gc_collect_full()?;
                Ok(vec![Value::number(self.gc_count_kib())])
            }
            "count" => Ok(vec![Value::number(self.gc_count_kib())]),
            "step" => {
                let budget = match args.get(1) {
                    Some(Value::Integer(value)) if *value >= 0 => {
                        usize::try_from(*value).map_err(|_| {
                            KError::new(
                                KErrorKind::Runtime("step budget overflow".to_owned()),
                                None,
                            )
                        })?
                    }
                    Some(Value::Number(value)) if *value >= 0.0 => {
                        if value.fract() != 0.0 {
                            return Err(KError::new(
                                KErrorKind::Runtime("step budget must be an integer".to_owned()),
                                None,
                            ));
                        }
                        usize::try_from(*value as u64).map_err(|_| {
                            KError::new(
                                KErrorKind::Runtime("step budget overflow".to_owned()),
                                None,
                            )
                        })?
                    }
                    Some(_) => {
                        return Err(KError::new(
                            KErrorKind::Runtime("step budget must be non-negative".to_owned()),
                            None,
                        ));
                    }
                    None => self.gc_params.stepsize,
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
                let name = if let Some(Value::String(handle)) = args.get(1) {
                    let bytes = self.heap.string_bytes(*handle).ok_or_else(|| {
                        KError::new(
                            KErrorKind::Runtime("invalid GC parameter name".to_owned()),
                            None,
                        )
                    })?;
                    std::str::from_utf8(bytes)
                        .map(|value| value.to_owned())
                        .map_err(|_| {
                            KError::new(
                                KErrorKind::Runtime("GC parameter name must be UTF-8".to_owned()),
                                None,
                            )
                        })?
                } else {
                    return Err(KError::new(
                        KErrorKind::Runtime(
                            "collectgarbage('param') expects a parameter name".to_owned(),
                        ),
                        None,
                    ));
                };
                let current = self.gc_param_value(&name)?;
                if let Some(value) = args.get(2) {
                    let next = self.gc_param_from_value(&name, *value)?;
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

    fn table_get(&mut self, value: Value, key: Value) -> KResult<Value> {
        self.index_value(value, key, 0)
    }

    fn table_set(&mut self, value: Value, key: Value, next: Value) -> KResult<()> {
        self.newindex_value(value, key, next, 0)
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
                text.parse::<f64>().map_err(|_| {
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
        let mut current = self.stack.get(start).copied().unwrap_or(Value::nil());
        for index in start + 1..=end {
            let next = self.stack.get(index).copied().unwrap_or(Value::nil());
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
            Value::Number(value) if value.is_finite() && value.fract() == 0.0 => {
                Ok(Some(value as i64))
            }
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
                match text.parse::<i64>() {
                    Ok(value) => Ok(Some(value)),
                    Err(_) => Ok(None),
                }
            }
            _ => Ok(None),
        }
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
        match callee {
            Value::NativeFunction(function) => {
                let returned = self.call_native(function, &args)?;
                Ok(returned.into_iter().next().unwrap_or(Value::nil()))
            }
            Value::Closure(handle) => {
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
                result.map(|values| values.into_iter().next().unwrap_or(Value::nil()))
            }
            _ => Err(KError::new(
                KErrorKind::Runtime("attempt to call a non-callable value".to_owned()),
                None,
            )),
        }
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
        let values: Vec<Value> = table.iter_values().collect();
        if let Some(metatable) = metatable {
            self.gc_mark_table(metatable);
        }
        for value in values {
            self.gc_mark_value(value);
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
