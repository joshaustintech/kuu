use crate::value::{Value, LuaFunction, LuaClosure, TableId, UpvalueId, Upvalue, UpvalueState};
use crate::compiler::{Proto, Constant};
use crate::instruction::Instruction;

pub struct CallFrame {
    pub proto: Proto,
    pub base: usize,
    pub ip: usize,
    pub protected: bool,
    pub error_handler: Option<Value>,
    pub is_error_handler: bool,
    pub varargs: Vec<Value>,
    pub num_args: Option<usize>,
}

impl Clone for CallFrame {
    fn clone(&self) -> Self {
        Self {
            proto: self.proto.clone(),
            base: self.base,
            ip: self.ip,
            protected: self.protected,
            error_handler: self.error_handler,
            is_error_handler: self.is_error_handler,
            varargs: self.varargs.clone(),
            num_args: self.num_args,
        }
    }
}

impl CallFrame {
    pub fn new(proto: Proto, base: usize) -> Self {
        Self {
            proto,
            base,
            ip: 0,
            protected: false,
            error_handler: None,
            is_error_handler: false,
            varargs: Vec::new(),
            num_args: None,
        }
    }
}

pub struct VM {
    pub gc: crate::gc::GcHeap,
    pub globals: TableId,
    pub registry: TableId,
    pub stack: Vec<Value>,
    pub frames: Vec<CallFrame>,
    pub open_upvalues: Vec<UpvalueId>,
    pub string_metatable: Option<TableId>,
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

impl VM {
    pub fn new() -> Self {
        let mut gc = crate::gc::GcHeap::new();
        let globals = gc.alloc_table();
        let registry = gc.alloc_table();
        let mut vm = Self {
            gc,
            globals,
            registry,
            stack: Vec::new(),
            frames: Vec::new(),
            open_upvalues: Vec::new(),
            string_metatable: None,
        };
        crate::stdlib::register_stdlib(&mut vm);
        vm
    }

    pub fn get_arg(&self, idx: usize) -> Value {
        let frame = self.frames.last().expect("no active call frame");
        if frame.num_args.is_some_and(|limit| idx >= limit) {
            return Value::Nil;
        }
        let arg_idx = frame.base + idx;
        if arg_idx < self.stack.len() {
            self.stack[arg_idx]
        } else {
            Value::Nil
        }
    }

    pub fn get_arg_count(&self) -> usize {
        let frame = self.frames.last().expect("no active call frame");
        if let Some(limit) = frame.num_args {
            limit
        } else {
            self.stack.len().saturating_sub(frame.base)
        }
    }

    pub fn get_upvalue(&self, idx: usize) -> Value {
        let frame = self.frames.last().expect("no active call frame");
        let func_val = self.stack[frame.base - 1];
        if let Value::Function(func_id) = func_val {
            match self.gc.get_function(func_id) {
                LuaFunction::Rust(rust_closure) if idx < rust_closure.upvalues.len() => {
                    return rust_closure.upvalues[idx];
                }
                _ => {}
            }
        }
        Value::Nil
    }

    pub fn push_value(&mut self, val: Value) {
        self.stack.push(val);
    }

    pub fn value_to_string(&self, val: Value) -> String {
        match val {
            Value::Nil => "nil".to_string(),
            Value::Boolean(b) => b.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(f) => f.to_string(),
            Value::String(id) => {
                let bytes = &self.gc.get_string(id).data;
                String::from_utf8_lossy(bytes).into_owned()
            }
            Value::Table(id) => format!("table: {:#x}", id.0),
            Value::Function(id) => format!("function: {:#x}", id.0),
            Value::LightFunction(f) => format!("function: {:?}", f),
            Value::Thread(id) => format!("thread: {:#x}", id.0),
            Value::Userdata(id) => format!("userdata: {:#x}", id.0),
            Value::LightUserdata(u) => format!("lightuserdata: {:#x}", u),
        }
    }

    fn is_valid_table_key(key: Value) -> bool {
        match key {
            Value::Nil => false,
            Value::Number(n) => !n.is_nan(),
            _ => true,
        }
    }

    fn array_index(key: Value) -> Option<usize> {
        let idx = key.to_integer()?;
        if idx > 0 {
            usize::try_from(idx).ok()
        } else {
            None
        }
    }

    fn table_raw_get(&self, tbl_id: TableId, key: Value) -> Value {
        let table = self.gc.get_table(tbl_id);
        if let Some(idx) = Self::array_index(key)
            && idx <= table.array.len()
        {
            let val = table.array[idx - 1];
            if val != Value::Nil {
                return val;
            }
        }
        table.hash.get(&key).copied().unwrap_or(Value::Nil)
    }

    fn table_raw_set(&mut self, tbl_id: TableId, key: Value, val: Value) -> Result<(), Error> {
        if !Self::is_valid_table_key(key) {
            return Err(Error::Runtime("table index is nil".to_string()));
        }

        let table = self.gc.get_table_mut(tbl_id);
        if let Some(idx) = Self::array_index(key)
            && idx <= table.array.len().saturating_add(1)
        {
            if idx > table.array.len() {
                table.array.resize(idx, Value::Nil);
            }
            table.array[idx - 1] = val;
            table.hash.remove(&key);
            return Ok(());
        }

        if val == Value::Nil {
            table.hash.remove(&key);
        } else {
            table.hash.insert(key, val);
        }
        Ok(())
    }

    fn start_lua_call(
        &mut self,
        caller: &mut CallFrame,
        func_id: crate::value::FunctionId,
        args: &[Value],
        protected: bool,
        error_handler: Option<Value>,
        is_error_handler: bool,
    ) -> Result<(), Error> {
        let proto = match self.gc.get_function(func_id) {
            LuaFunction::Lua(closure) => closure.proto.clone(),
            _ => return Err(Error::Runtime("attempt to call a non-Lua function as Lua".to_string())),
        };

        let new_base = self.stack.len() + 1;
        let required_len = new_base + usize::max(proto.max_stack_size as usize, args.len());
        self.stack.resize(required_len, Value::Nil);
        self.stack[new_base - 1] = Value::Function(func_id);
        for (i, arg) in args.iter().enumerate() {
            self.stack[new_base + i] = *arg;
        }

        let mut varargs = Vec::new();
        if proto.is_vararg {
            let num_params = proto.num_params as usize;
            if args.len() > num_params {
                varargs.extend_from_slice(&args[num_params..]);
            }
        }

        self.frames.push(caller.clone());
        *caller = CallFrame {
            proto,
            base: new_base,
            ip: 0,
            protected,
            error_handler,
            is_error_handler,
            varargs,
            num_args: None,
        };
        Ok(())
    }

    fn call_rust_callable(&mut self, callable: Value, args: &[Value]) -> Result<Vec<Value>, Error> {
        let rust_fn = match callable {
            Value::LightFunction(func) => func,
            Value::Function(func_id) => match self.gc.get_function(func_id) {
                LuaFunction::Rust(closure) => closure.func,
                LuaFunction::Lua(_) => {
                    return Err(Error::Runtime("attempt to call a Lua function as native".to_string()));
                }
            },
            _ => return Err(Error::Runtime("attempt to call a non-function value".to_string())),
        };

        let old_len = self.stack.len();
        let base = old_len + 1;
        self.stack.resize(base + args.len(), Value::Nil);
        self.stack[base - 1] = callable;
        for (i, arg) in args.iter().enumerate() {
            self.stack[base + i] = *arg;
        }

        self.frames.push(CallFrame {
            proto: Proto::new(),
            base,
            ip: 0,
            protected: false,
            error_handler: None,
            is_error_handler: false,
            varargs: Vec::new(),
            num_args: Some(args.len()),
        });

        let result = rust_fn(self);
        self.frames.pop();
        let num_ret = result?;
        if num_ret < 0 {
            return Err(Error::Runtime("native metamethod yielded unexpectedly".to_string()));
        }
        let ret_start = self.stack.len().saturating_sub(num_ret as usize);
        let returns = self.stack[ret_start..].to_vec();
        self.stack.truncate(old_len);
        Ok(returns)
    }

    fn call_metamethod_for_result(
        &mut self,
        dst_slot: usize,
        mm: Value,
        args: &[Value],
        frame: &mut CallFrame,
    ) -> Result<(), Error> {
        match mm {
            Value::Function(func_id) => match self.gc.get_function(func_id) {
                LuaFunction::Lua(_) => self.start_lua_call(frame, func_id, args, false, None, false),
                LuaFunction::Rust(_) => {
                    let returns = self.call_rust_callable(mm, args)?;
                    self.stack[dst_slot] = returns.first().copied().unwrap_or(Value::Nil);
                    Ok(())
                }
            },
            Value::LightFunction(_) => {
                let returns = self.call_rust_callable(mm, args)?;
                self.stack[dst_slot] = returns.first().copied().unwrap_or(Value::Nil);
                Ok(())
            }
            _ => Err(Error::Runtime("metamethod must be a function".to_string())),
        }
    }

    fn call_metamethod_for_effect(
        &mut self,
        mm: Value,
        args: &[Value],
        frame: &mut CallFrame,
    ) -> Result<(), Error> {
        match mm {
            Value::Function(func_id) => match self.gc.get_function(func_id) {
                LuaFunction::Lua(_) => self.start_lua_call(frame, func_id, args, false, None, false),
                LuaFunction::Rust(_) => {
                    let _ = self.call_rust_callable(mm, args)?;
                    Ok(())
                }
            },
            Value::LightFunction(_) => {
                let _ = self.call_rust_callable(mm, args)?;
                Ok(())
            }
            _ => Err(Error::Runtime("metamethod must be a function".to_string())),
        }
    }

    fn unwind_protected(&mut self, err: Error) -> Result<bool, Error> {
        let mut target_idx = None;
        for (i, frame) in self.frames.iter().enumerate().rev() {
            if frame.protected {
                target_idx = Some(i);
                break;
            }
        }

        if let Some(idx) = target_idx {
            let prot_frame = self.frames[idx].clone();
            self.close_upvalues(prot_frame.base);
            self.frames.truncate(idx);

            if let Some(parent_frame) = self.frames.last_mut() {
                let parent_ip = parent_frame.ip;
                let parent_inst = parent_frame.proto.instructions[parent_ip - 1];
                if let Instruction::Call { func, .. } = parent_inst {
                    let dst_start = parent_frame.base + func as usize;
                    self.stack[dst_start] = Value::Boolean(false);
                    
                    let err_msg = match err {
                        Error::Runtime(ref s) => s.clone(),
                    };
                    let err_str_id = self.gc.alloc_string(err_msg.into_bytes());
                    
                    if let Some(msgh) = prot_frame.error_handler {
                        match msgh {
                            Value::Function(msgh_func_id) => {
                                let closure = match self.gc.get_function(msgh_func_id) {
                                    LuaFunction::Lua(c) => c,
                                    _ => {
                                        self.stack[dst_start + 1] = Value::String(err_str_id);
                                        self.stack.truncate(dst_start + 2);
                                        return Ok(true);
                                    }
                                };
                                
                                let new_base = prot_frame.base;
                                self.stack.resize(new_base + 1, Value::Nil);
                                self.stack[new_base - 1] = msgh;
                                self.stack[new_base] = Value::String(err_str_id);
                                
                                self.frames.push(CallFrame {
                                    proto: closure.proto.clone(),
                                    base: new_base,
                                    ip: 0,
                                    protected: false,
                                    error_handler: None,
                                    is_error_handler: true,
                                    varargs: Vec::new(),
                                    num_args: None,
                                });
                            }
                            Value::LightFunction(msgh_rust_fn) => {
                                let new_base = prot_frame.base;
                                self.stack.resize(new_base + 1, Value::Nil);
                                self.stack[new_base] = Value::String(err_str_id);
                                
                                self.frames.push(CallFrame {
                                    proto: Proto::new(),
                                    base: new_base,
                                    ip: 0,
                                    protected: false,
                                    error_handler: None,
                                    is_error_handler: true,
                                    varargs: Vec::new(),
                                    num_args: Some(1),
                                });
                                
                                let num_ret = msgh_rust_fn(self)?;
                                self.frames.pop();
                                
                                if num_ret > 0 {
                                    self.stack[dst_start + 1] = self.stack[new_base];
                                } else {
                                    self.stack[dst_start + 1] = Value::Nil;
                                }
                                self.stack.truncate(dst_start + 2);
                            }
                            _ => {
                                self.stack[dst_start + 1] = Value::String(err_str_id);
                                self.stack.truncate(dst_start + 2);
                            }
                        }
                    } else {
                        self.stack[dst_start + 1] = Value::String(err_str_id);
                        self.stack.truncate(dst_start + 2);
                    }
                }
                Ok(true)
            } else {
                Err(err)
            }
        } else {
            Ok(false)
        }
    }

    pub fn execute(&mut self, proto: Proto) -> Result<Value, Error> {
        let mut upvalues = Vec::new();
        for up_desc in &proto.upvalues {
            let val = if up_desc.name.as_deref() == Some("_ENV") {
                Value::Table(self.globals)
            } else {
                Value::Nil
            };
            let up_id = self.gc.alloc_upvalue(Upvalue {
                val: UpvalueState::Closed(val),
            });
            upvalues.push(up_id);
        }

        let closure = LuaClosure {
            proto: proto.clone(),
            upvalues,
        };
        let func_id = self.gc.alloc_function(LuaFunction::Lua(closure));
        
        let required_len = proto.max_stack_size as usize + 1;
        self.stack.resize(required_len, Value::Nil);
        self.stack[0] = Value::Function(func_id);

        self.frames.push(CallFrame {
            proto,
            base: 1,
            ip: 0,
            protected: false,
            error_handler: None,
            is_error_handler: false,
            varargs: Vec::new(),
            num_args: None,
        });

        loop {
            match self.run() {
                Ok(()) => break,
                Err(e) => {
                    match self.unwind_protected(e.clone()) {
                        Ok(true) => {}
                        _ => return Err(e),
                    }
                }
            }
        }
        
        Ok(self.stack[0])
    }

    pub fn get_metatable(&self, val: Value) -> Option<TableId> {
        match val {
            Value::Table(id) => self.gc.get_table(id).metatable,
            Value::Userdata(id) => self.gc.get_userdata(id).metatable,
            Value::String(_) => self.string_metatable,
            _ => None,
        }
    }

    pub fn get_metamethod(&self, val: Value, event: &str) -> Option<Value> {
        let meta_id = self.get_metatable(val)?;
        let table = self.gc.get_table(meta_id);
        for (k, v) in &table.hash {
            if let Value::String(s_id) = k {
                let s_bytes = &self.gc.get_string(*s_id).data;
                if s_bytes == event.as_bytes() {
                    return Some(*v);
                }
            }
        }
        None
    }

    pub fn close_upvalues(&mut self, boundary: usize) {
        let mut i = 0;
        while i < self.open_upvalues.len() {
            let up_id = self.open_upvalues[i];
            let up = self.gc.get_upvalue_mut(up_id);
            match up.val {
                UpvalueState::Open { stack_idx, .. } if stack_idx >= boundary => {
                    let val = self.stack[stack_idx];
                    up.val = UpvalueState::Closed(val);
                    self.open_upvalues.remove(i);
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn value_to_string_bytes(&self, val: Value) -> Option<Vec<u8>> {
        match val {
            Value::String(id) => Some(self.gc.get_string(id).data.clone()),
            Value::Integer(i) => Some(i.to_string().into_bytes()),
            Value::Number(f) => Some(f.to_string().into_bytes()),
            _ => None,
        }
    }

    fn execute_binary_op(&mut self, dst_slot: usize, l: Value, r: Value, op_name: &str, inst: Instruction, frame: &mut CallFrame) -> Result<(), Error> {
        let is_arith = matches!(
            inst,
            Instruction::Add { .. } |
            Instruction::Sub { .. } |
            Instruction::Mul { .. } |
            Instruction::Div { .. } |
            Instruction::Mod { .. } |
            Instruction::Pow { .. } |
            Instruction::IDiv { .. }
        );

        if let (true, Some(num_l), Some(num_r)) = (is_arith, l.to_number(), r.to_number()) {
            let res = match inst {
                Instruction::Add { .. } => match (l, r) {
                    (Value::Integer(a), Value::Integer(b)) => Value::Integer(a.wrapping_add(b)),
                    _ => Value::Number(num_l + num_r),
                },
                Instruction::Sub { .. } => match (l, r) {
                    (Value::Integer(a), Value::Integer(b)) => Value::Integer(a.wrapping_sub(b)),
                    _ => Value::Number(num_l - num_r),
                },
                Instruction::Mul { .. } => match (l, r) {
                    (Value::Integer(a), Value::Integer(b)) => Value::Integer(a.wrapping_mul(b)),
                    _ => Value::Number(num_l * num_r),
                },
                Instruction::Div { .. } => {
                    Value::Number(num_l / num_r)
                }
                Instruction::Mod { .. } => match (l, r) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        if b == 0 {
                            return Err(Error::Runtime("attempt to perform 'n % 0'".to_string()));
                        }
                        Value::Integer(a.rem_euclid(b))
                    }
                    _ => {
                        Value::Number(num_l - (num_l / num_r).floor() * num_r)
                    }
                },
                Instruction::Pow { .. } => {
                    Value::Number(num_l.powf(num_r))
                }
                Instruction::IDiv { .. } => match (l, r) {
                    (Value::Integer(a), Value::Integer(b)) => {
                        if b == 0 {
                            return Err(Error::Runtime("attempt to divide by zero".to_string()));
                        }
                        let q = a / b;
                        let rem = a % b;
                        let res = if (rem != 0) && ((a < 0) ^ (b < 0)) { q - 1 } else { q };
                        Value::Integer(res)
                    }
                    _ => {
                        Value::Number((num_l / num_r).floor())
                    }
                },
                _ => unreachable!(),
            };
            self.stack[dst_slot] = res;
            return Ok(());
        }

        let is_bit = matches!(
            inst,
            Instruction::BAnd { .. } |
            Instruction::BOr { .. } |
            Instruction::BXor { .. } |
            Instruction::Shl { .. } |
            Instruction::Shr { .. }
        );

        if let (true, Some(a), Some(b)) = (is_bit, l.to_integer(), r.to_integer()) {
            let res = match inst {
                Instruction::BAnd { .. } => Value::Integer(a & b),
                Instruction::BOr { .. } => Value::Integer(a | b),
                Instruction::BXor { .. } => Value::Integer(a ^ b),
                Instruction::Shl { .. } => {
                    let shift = b;
                    let val = if shift >= 64 || shift <= -64 {
                        0
                    } else if shift >= 0 {
                        (a as u64).wrapping_shl(shift as u32) as i64
                    } else {
                        (a as u64).wrapping_shr((-shift) as u32) as i64
                    };
                    Value::Integer(val)
                }
                Instruction::Shr { .. } => {
                    let shift = b;
                    let val = if shift >= 64 || shift <= -64 {
                        0
                    } else if shift >= 0 {
                        (a as u64).wrapping_shr(shift as u32) as i64
                    } else {
                        (a as u64).wrapping_shl((-shift) as u32) as i64
                    };
                    Value::Integer(val)
                }
                _ => unreachable!(),
            };
            self.stack[dst_slot] = res;
            return Ok(());
        }

        if let (true, Some(s_l), Some(s_r)) = (
            matches!(inst, Instruction::Concat { .. }),
            self.value_to_string_bytes(l),
            self.value_to_string_bytes(r),
        ) {
            let mut new_bytes = s_l;
            new_bytes.extend_from_slice(&s_r);
            let new_str_id = self.gc.alloc_string(new_bytes);
            self.stack[dst_slot] = Value::String(new_str_id);
            return Ok(());
        }

        let mm_name = format!("__{}", op_name);
        if let Some(mm) = self.get_metamethod(l, &mm_name).or_else(|| self.get_metamethod(r, &mm_name)) {
            return self.call_metamethod_for_result(dst_slot, mm, &[l, r], frame);
        }

        Err(Error::Runtime(format!("attempt to perform binary op {} on invalid types: {:?}, {:?}", op_name, l, r)))
    }

    fn execute_unary_op(&mut self, dst_slot: usize, val: Value, op_name: &str, inst: Instruction, frame: &mut CallFrame) -> Result<(), Error> {
        match inst {
            Instruction::UNeg { .. } => {
                if let Some(n) = val.to_number() {
                    let res = match val {
                        Value::Integer(i) => Value::Integer(i.wrapping_neg()),
                        _ => Value::Number(-n),
                    };
                    self.stack[dst_slot] = res;
                    return Ok(());
                }
            }
            Instruction::UNot { .. } => {
                let truthy = !matches!(val, Value::Nil | Value::Boolean(false));
                self.stack[dst_slot] = Value::Boolean(!truthy);
                return Ok(());
            }
            Instruction::UBNot { .. } => {
                if let Some(i) = val.to_integer() {
                    self.stack[dst_slot] = Value::Integer(!i);
                    return Ok(());
                }
            }
            Instruction::ULen { .. } => {
                match val {
                    Value::String(str_id) => {
                        let len = self.gc.get_string(str_id).data.len() as i64;
                        self.stack[dst_slot] = Value::Integer(len);
                        return Ok(());
                    }
                    Value::Table(tbl_id) => {
                        if self.get_metamethod(val, "__len").is_none() {
                            let len;
                            let mut i = 1;
                            loop {
                                if self.table_raw_get(tbl_id, Value::Integer(i)) == Value::Nil {
                                    len = (i - 1) as usize;
                                    break;
                                }
                                i += 1;
                            }
                            self.stack[dst_slot] = Value::Integer(len as i64);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
            _ => unreachable!(),
        }

        let mm_name = format!("__{}", op_name);
        if let Some(mm) = self.get_metamethod(val, &mm_name) {
            return self.call_metamethod_for_result(dst_slot, mm, &[val], frame);
        }

        Err(Error::Runtime(format!("attempt to perform unary op {} on invalid type: {:?}", op_name, val)))
    }

    fn call_compare_metamethod(&mut self, mm: Value, lhs_val: Value, rhs_val: Value, frame: &mut CallFrame) -> Result<(), Error> {
        match mm {
            Value::Function(func_id) => match self.gc.get_function(func_id) {
                LuaFunction::Lua(_) => self.start_lua_call(frame, func_id, &[lhs_val, rhs_val], false, None, false),
                LuaFunction::Rust(_) => {
                    let returns = self.call_rust_callable(mm, &[lhs_val, rhs_val])?;
                    let truthy = returns
                        .first()
                        .is_some_and(|val| !matches!(val, Value::Nil | Value::Boolean(false)));
                    let parent_inst = frame.proto.instructions[frame.ip - 1];
                    if let Instruction::Eq { eq, .. } | Instruction::Lt { eq, .. } | Instruction::Le { eq, .. } = parent_inst
                        && truthy != eq
                    {
                        frame.ip += 1;
                    }
                    Ok(())
                }
            },
            Value::LightFunction(_) => {
                let returns = self.call_rust_callable(mm, &[lhs_val, rhs_val])?;
                let truthy = returns
                    .first()
                    .is_some_and(|val| !matches!(val, Value::Nil | Value::Boolean(false)));
                let parent_inst = frame.proto.instructions[frame.ip - 1];
                if let Instruction::Eq { eq, .. } | Instruction::Lt { eq, .. } | Instruction::Le { eq, .. } = parent_inst
                    && truthy != eq
                {
                    frame.ip += 1;
                }
                Ok(())
            }
            _ => Err(Error::Runtime("metamethod must be a function".to_string())),
        }
    }

    fn execute_eq(&mut self, lhs_val: Value, rhs_val: Value, eq: bool, frame: &mut CallFrame) -> Result<(), Error> {
        let is_equal = if lhs_val == rhs_val {
            true
        } else {
            match (lhs_val, rhs_val) {
                (Value::Table(_), Value::Table(_)) | (Value::Userdata(_), Value::Userdata(_)) => {
                    let mm_l = self.get_metamethod(lhs_val, "__eq");
                    let mm_r = self.get_metamethod(rhs_val, "__eq");
                    match (mm_l, mm_r) {
                        (Some(mm), Some(r)) if mm == r => {
                            self.call_compare_metamethod(mm, lhs_val, rhs_val, frame)?;
                            return Ok(());
                        }
                        _ => {}
                    }
                    false
                }
                _ => false,
            }
        };

        if is_equal != eq {
            frame.ip += 1;
        }
        Ok(())
    }

    fn execute_lt(&mut self, lhs_val: Value, rhs_val: Value, eq: bool, frame: &mut CallFrame) -> Result<(), Error> {
        match (lhs_val, rhs_val) {
            (Value::Integer(a), Value::Integer(b)) => {
                if (a < b) != eq { frame.ip += 1; }
            }
            (Value::Number(a), Value::Number(b)) => {
                if (a < b) != eq { frame.ip += 1; }
            }
            (Value::Integer(a), Value::Number(b)) => {
                if ((a as f64) < b) != eq { frame.ip += 1; }
            }
            (Value::Number(a), Value::Integer(b)) => {
                if (a < (b as f64)) != eq { frame.ip += 1; }
            }
            (Value::String(a), Value::String(b)) => {
                let s_a = &self.gc.get_string(a).data;
                let s_b = &self.gc.get_string(b).data;
                if (s_a < s_b) != eq { frame.ip += 1; }
            }
            _ => {
                if let Some(mm) = self.get_metamethod(lhs_val, "__lt").or_else(|| self.get_metamethod(rhs_val, "__lt")) {
                    self.call_compare_metamethod(mm, lhs_val, rhs_val, frame)?;
                    return Ok(());
                } else {
                    return Err(Error::Runtime(format!("attempt to compare < on invalid types: {:?}, {:?}", lhs_val, rhs_val)));
                }
            }
        }
        Ok(())
    }

    fn execute_le(&mut self, lhs_val: Value, rhs_val: Value, eq: bool, frame: &mut CallFrame) -> Result<(), Error> {
        match (lhs_val, rhs_val) {
            (Value::Integer(a), Value::Integer(b)) => {
                if (a <= b) != eq { frame.ip += 1; }
            }
            (Value::Number(a), Value::Number(b)) => {
                if (a <= b) != eq { frame.ip += 1; }
            }
            (Value::Integer(a), Value::Number(b)) => {
                if ((a as f64) <= b) != eq { frame.ip += 1; }
            }
            (Value::Number(a), Value::Integer(b)) => {
                if (a <= (b as f64)) != eq { frame.ip += 1; }
            }
            (Value::String(a), Value::String(b)) => {
                let s_a = &self.gc.get_string(a).data;
                let s_b = &self.gc.get_string(b).data;
                if (s_a <= s_b) != eq { frame.ip += 1; }
            }
            _ => {
                if let Some(mm) = self.get_metamethod(lhs_val, "__le").or_else(|| self.get_metamethod(rhs_val, "__le")) {
                    self.call_compare_metamethod(mm, lhs_val, rhs_val, frame)?;
                    return Ok(());
                } else {
                    return Err(Error::Runtime(format!("attempt to compare <= on invalid types: {:?}, {:?}", lhs_val, rhs_val)));
                }
            }
        }
        Ok(())
    }

    pub fn execute_get_table(&mut self, dst_slot: usize, tbl_val: Value, key_val: Value, frame: &mut CallFrame) -> Result<(), Error> {
        let mut current_tbl = tbl_val;
        for _ in 0..100 {
            match current_tbl {
                Value::Table(tbl_id) => {
                    let val = self.table_raw_get(tbl_id, key_val);
                    if val != Value::Nil {
                        self.stack[dst_slot] = val;
                        return Ok(());
                    }
                    
                    if let Some(mm) = self.get_metamethod(current_tbl, "__index") {
                        match mm {
                            Value::Table(next_tbl_id) => {
                                current_tbl = Value::Table(next_tbl_id);
                                continue;
                            }
                            Value::Function(_) | Value::LightFunction(_) => {
                                return self.call_metamethod_for_result(dst_slot, mm, &[tbl_val, key_val], frame);
                            }
                            _ => return Err(Error::Runtime("invalid __index metamethod type".to_string())),
                        }
                    }
                    self.stack[dst_slot] = Value::Nil;
                    return Ok(());
                }
                _ => {
                    if let Some(mm) = self.get_metamethod(current_tbl, "__index") {
                        match mm {
                            Value::Table(next_tbl_id) => {
                                current_tbl = Value::Table(next_tbl_id);
                                continue;
                            }
                            Value::Function(_) | Value::LightFunction(_) => {
                                return self.call_metamethod_for_result(dst_slot, mm, &[current_tbl, key_val], frame);
                            }
                            _ => return Err(Error::Runtime("invalid __index metamethod type".to_string())),
                        }
                    } else {
                        return Err(Error::Runtime("attempt to index a non-table value".to_string()));
                    }
                }
            }
        }
        Err(Error::Runtime("loop in gettable metamethods".to_string()))
    }

    fn execute_set_table(&mut self, tbl_val: Value, key_val: Value, val_val: Value, frame: &mut CallFrame) -> Result<(), Error> {
        let mut current_tbl = tbl_val;
        if !Self::is_valid_table_key(key_val) {
            return Err(Error::Runtime("table index is nil".to_string()));
        }
        for _ in 0..100 {
            match current_tbl {
                Value::Table(tbl_id) => {
                    if self.table_raw_get(tbl_id, key_val) != Value::Nil {
                        self.table_raw_set(tbl_id, key_val, val_val)?;
                        return Ok(());
                    }

                    if let Some(mm) = self.get_metamethod(current_tbl, "__newindex") {
                        match mm {
                            Value::Table(next_tbl_id) => {
                                current_tbl = Value::Table(next_tbl_id);
                                continue;
                            }
                            Value::Function(_) | Value::LightFunction(_) => {
                                return self.call_metamethod_for_effect(mm, &[tbl_val, key_val, val_val], frame);
                            }
                            _ => return Err(Error::Runtime("invalid __newindex metamethod type".to_string())),
                        }
                    }

                    self.table_raw_set(tbl_id, key_val, val_val)?;
                    return Ok(());
                }
                _ => {
                    if let Some(mm) = self.get_metamethod(current_tbl, "__newindex") {
                        match mm {
                            Value::Table(next_tbl_id) => {
                                current_tbl = Value::Table(next_tbl_id);
                                continue;
                            }
                            Value::Function(_) | Value::LightFunction(_) => {
                                return self.call_metamethod_for_effect(mm, &[current_tbl, key_val, val_val], frame);
                            }
                            _ => return Err(Error::Runtime("invalid __newindex metamethod type".to_string())),
                        }
                    } else {
                        return Err(Error::Runtime("attempt to index a non-table value".to_string()));
                    }
                }
            }
        }
        Err(Error::Runtime("loop in settable metamethods".to_string()))
    }

    pub fn run(&mut self) -> Result<(), Error> {
        macro_rules! vm_try {
            ($frame:expr, $expr:expr) => {
                match $expr {
                    Ok(value) => value,
                    Err(err) => {
                        if $frame.protected {
                            self.frames.push($frame);
                        }
                        return Err(err);
                    }
                }
            };
        }

        while let Some(mut frame) = self.frames.pop() {
            while frame.ip < frame.proto.instructions.len() {
                let inst = frame.proto.instructions[frame.ip];
                frame.ip += 1;

                match inst {
                    Instruction::Move { dst, src } => {
                        let val = self.stack[frame.base + src as usize];
                        self.stack[frame.base + dst as usize] = val;
                    }
                    Instruction::LoadK { dst, const_idx } => {
                        let k = &frame.proto.constants[const_idx as usize];
                        let val = match k {
                            Constant::Nil => Value::Nil,
                            Constant::Boolean(b) => Value::Boolean(*b),
                            Constant::Integer(i) => Value::Integer(*i),
                            Constant::Number(n) => Value::Number(*n),
                            Constant::String(s) => {
                                let id = self.gc.alloc_string(s.clone());
                                Value::String(id)
                            }
                        };
                        self.stack[frame.base + dst as usize] = val;
                    }
                    Instruction::LoadNil { dst, count } => {
                        for i in 0..count {
                            self.stack[frame.base + dst as usize + i as usize] = Value::Nil;
                        }
                    }
                    Instruction::LoadBool { dst, val, skip_next } => {
                        self.stack[frame.base + dst as usize] = Value::Boolean(val);
                        if skip_next {
                            frame.ip += 1;
                        }
                    }
                    Instruction::Add { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "add", inst, &mut frame));
                    }
                    Instruction::Sub { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "sub", inst, &mut frame));
                    }
                    Instruction::Mul { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "mul", inst, &mut frame));
                    }
                    Instruction::Div { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "div", inst, &mut frame));
                    }
                    Instruction::Mod { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "mod", inst, &mut frame));
                    }
                    Instruction::Pow { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "pow", inst, &mut frame));
                    }
                    Instruction::IDiv { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "idiv", inst, &mut frame));
                    }
                    Instruction::BAnd { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "band", inst, &mut frame));
                    }
                    Instruction::BOr { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "bor", inst, &mut frame));
                    }
                    Instruction::BXor { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "bxor", inst, &mut frame));
                    }
                    Instruction::Shl { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "shl", inst, &mut frame));
                    }
                    Instruction::Shr { dst, lhs, rhs } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, l, r, "shr", inst, &mut frame));
                    }
                    Instruction::Concat { dst, start, count } => {
                        let mut res = self.stack[frame.base + start as usize];
                        for i in 1..count as usize {
                            let next_val = self.stack[frame.base + start as usize + i];
                            vm_try!(frame, self.execute_binary_op(frame.base + dst as usize, res, next_val, "concat", inst, &mut frame));
                            res = self.stack[frame.base + dst as usize];
                        }
                    }
                    Instruction::UNeg { dst, src } => {
                        let val = self.stack[frame.base + src as usize];
                        vm_try!(frame, self.execute_unary_op(frame.base + dst as usize, val, "unm", inst, &mut frame));
                    }
                    Instruction::UNot { dst, src } => {
                        let val = self.stack[frame.base + src as usize];
                        vm_try!(frame, self.execute_unary_op(frame.base + dst as usize, val, "not", inst, &mut frame));
                    }
                    Instruction::ULen { dst, src } => {
                        let val = self.stack[frame.base + src as usize];
                        vm_try!(frame, self.execute_unary_op(frame.base + dst as usize, val, "len", inst, &mut frame));
                    }
                    Instruction::UBNot { dst, src } => {
                        let val = self.stack[frame.base + src as usize];
                        vm_try!(frame, self.execute_unary_op(frame.base + dst as usize, val, "bnot", inst, &mut frame));
                    }
                    Instruction::GetUpval { dst, upval_idx } => {
                        let current_func = self.stack[frame.base - 1];
                        if let Value::Function(func_id) = current_func {
                            if let LuaFunction::Lua(closure) = self.gc.get_function(func_id) {
                                let upval_id = closure.upvalues[upval_idx as usize];
                                let upval = self.gc.get_upvalue(upval_id);
                                let val = match upval.val {
                                    UpvalueState::Open { stack_idx, .. } => self.stack[stack_idx],
                                    UpvalueState::Closed(v) => v,
                                };
                                self.stack[frame.base + dst as usize] = val;
                            } else {
                                return Err(Error::Runtime("current function is not a Lua closure".to_string()));
                            }
                        } else {
                            return Err(Error::Runtime("current function not found on stack".to_string()));
                        }
                    }
                    Instruction::SetUpval { upval_idx, src } => {
                        let current_func = self.stack[frame.base - 1];
                        if let Value::Function(func_id) = current_func {
                            let upval_id = {
                                if let LuaFunction::Lua(closure) = self.gc.get_function(func_id) {
                                    closure.upvalues[upval_idx as usize]
                                } else {
                                    return Err(Error::Runtime("current function is not a Lua closure".to_string()));
                                }
                            };
                            let val = self.stack[frame.base + src as usize];
                            let upval = self.gc.get_upvalue_mut(upval_id);
                            match upval.val {
                                UpvalueState::Open { stack_idx, .. } => {
                                    self.stack[stack_idx] = val;
                                }
                                UpvalueState::Closed(ref mut v) => {
                                    *v = val;
                                }
                            }
                        } else {
                            return Err(Error::Runtime("current function not found on stack".to_string()));
                        }
                    }
                    Instruction::GetTabUp { dst, upval_idx, key_const } => {
                        let current_func = self.stack[frame.base - 1];
                        if let Value::Function(func_id) = current_func {
                            let upval_id = {
                                if let LuaFunction::Lua(closure) = self.gc.get_function(func_id) {
                                    closure.upvalues[upval_idx as usize]
                                } else {
                                    return Err(Error::Runtime("current function is not a Lua closure".to_string()));
                                }
                            };
                            let env_val = match self.gc.get_upvalue(upval_id).val {
                                UpvalueState::Open { stack_idx, .. } => self.stack[stack_idx],
                                UpvalueState::Closed(v) => v,
                            };
                            
                            let key_const_val = &frame.proto.constants[key_const as usize];
                            let key_val = match key_const_val {
                                Constant::String(s) => Value::String(self.gc.alloc_string(s.clone())),
                                _ => return Err(Error::Runtime("GetTabUp key must be a string".to_string())),
                            };

                            vm_try!(frame, self.execute_get_table(frame.base + dst as usize, env_val, key_val, &mut frame));
                        } else {
                            return Err(Error::Runtime("current function not found on stack".to_string()));
                        }
                    }
                    Instruction::SetTabUp { upval_idx, key_const, src } => {
                        let current_func = self.stack[frame.base - 1];
                        if let Value::Function(func_id) = current_func {
                            let upval_id = {
                                if let LuaFunction::Lua(closure) = self.gc.get_function(func_id) {
                                    closure.upvalues[upval_idx as usize]
                                } else {
                                    return Err(Error::Runtime("current function is not a Lua closure".to_string()));
                                }
                            };
                            let env_val = match self.gc.get_upvalue(upval_id).val {
                                UpvalueState::Open { stack_idx, .. } => self.stack[stack_idx],
                                UpvalueState::Closed(v) => v,
                            };
                            
                            let key_const_val = &frame.proto.constants[key_const as usize];
                            let key_val = match key_const_val {
                                Constant::String(s) => Value::String(self.gc.alloc_string(s.clone())),
                                _ => return Err(Error::Runtime("SetTabUp key must be a string".to_string())),
                            };

                            let val = self.stack[frame.base + src as usize];
                            vm_try!(frame, self.execute_set_table(env_val, key_val, val, &mut frame));
                        } else {
                            return Err(Error::Runtime("current function not found on stack".to_string()));
                        }
                    }
                    Instruction::Closure { dst, proto_idx } => {
                        let sub_proto = frame.proto.sub_protos[proto_idx as usize].clone();
                        let mut upvalues = Vec::new();
                        let current_func_val = self.stack[frame.base - 1];
                        
                        for up_desc in &sub_proto.upvalues {
                            if up_desc.in_stack {
                                let stack_idx = frame.base + up_desc.idx as usize;
                                let mut found_id = None;
                                for open_id in &self.open_upvalues {
                                    let open_up = self.gc.get_upvalue(*open_id);
                                    if matches!(open_up.val, UpvalueState::Open { stack_idx: s_idx, .. } if s_idx == stack_idx) {
                                        found_id = Some(*open_id);
                                        break;
                                    }
                                }
                                
                                let up_id = match found_id {
                                    Some(id) => id,
                                    None => {
                                        let id = self.gc.alloc_upvalue(Upvalue {
                                            val: UpvalueState::Open { thread_id: 0, stack_idx },
                                        });
                                        self.open_upvalues.push(id);
                                        id
                                    }
                                };
                                upvalues.push(up_id);
                            } else if let Value::Function(func_id) = current_func_val {
                                let up_id = if let LuaFunction::Lua(parent_closure) = self.gc.get_function(func_id) {
                                    parent_closure.upvalues[up_desc.idx as usize]
                                } else {
                                    return Err(Error::Runtime("parent closure is not a Lua closure".to_string()));
                                };
                                upvalues.push(up_id);
                            } else {
                                return Err(Error::Runtime("parent function not found on stack".to_string()));
                            }
                        }
                        
                        let closure = LuaClosure {
                            proto: sub_proto,
                            upvalues,
                        };
                        let closure_id = self.gc.alloc_function(LuaFunction::Lua(closure));
                        self.stack[frame.base + dst as usize] = Value::Function(closure_id);
                    }
                    Instruction::Call { func, num_args, num_results } => {
                        let val = self.stack[frame.base + func as usize];
                        
                        let mut is_pcall_like = None;
                        if let Value::LightFunction(f) = val {
                            if f as usize == crate::stdlib::std_pcall as crate::value::RustFunction as usize {
                                is_pcall_like = Some(false);
                            } else if f as usize == crate::stdlib::std_xpcall as crate::value::RustFunction as usize {
                                is_pcall_like = Some(true);
                            }
                        }

                        if let Some(is_xpcall) = is_pcall_like {
                            let target_f = self.stack[frame.base + func as usize + 1];
                            let error_handler = if is_xpcall {
                                Some(self.stack[frame.base + func as usize + 2])
                            } else {
                                None
                            };

                            let args_start_offset = if is_xpcall { 3 } else { 2 };
                            let num_args_to_target = num_args.saturating_sub(if is_xpcall { 2 } else { 1 });

                            eprintln!("[DEBUG] pcall dispatch: func={}, target_f={:?}, num_args={}, num_args_to_target={}", func, target_f, num_args, num_args_to_target);
                            match target_f {
                                Value::Function(target_func_id) => {
                                    match self.gc.get_function(target_func_id) {
                                        LuaFunction::Lua(closure) => {
                                            self.frames.push(frame.clone());
                                            let new_base = frame.base + func as usize + args_start_offset;
                                            let required_len = new_base + usize::max(
                                                closure.proto.max_stack_size as usize,
                                                num_args_to_target as usize,
                                            );
                                            if self.stack.len() < required_len {
                                                self.stack.resize(required_len, Value::Nil);
                                            }
                                            self.stack[new_base - 1] = target_f;
                                            let mut varargs = Vec::new();
                                            if closure.proto.is_vararg {
                                                let num_params = closure.proto.num_params as usize;
                                                if num_args_to_target as usize > num_params {
                                                    for i in num_params..num_args_to_target as usize {
                                                        varargs.push(self.stack[new_base + i]);
                                                    }
                                                }
                                            }
                                            frame = CallFrame {
                                                proto: closure.proto.clone(),
                                                base: new_base,
                                                ip: 0,
                                                protected: true,
                                                error_handler,
                                                is_error_handler: false,
                                                varargs,
                                                num_args: None,
                                            };
                                            eprintln!("[DEBUG] pcall Lua: new frame base={}, proto.instructions.len()={}, proto.max_stack_size={}", frame.base, frame.proto.instructions.len(), frame.proto.max_stack_size);
                                        }
                                        LuaFunction::Rust(rust_closure) => {
                                            let rust_fn = rust_closure.func;
                                            let new_base = frame.base + func as usize + args_start_offset;
                                            let ret_start = self.stack.len();
                                            let required_len = new_base + num_args_to_target as usize;
                                            if self.stack.len() < required_len {
                                                self.stack.resize(required_len, Value::Nil);
                                            }
                                            
                                            self.frames.push(frame.clone());
                                            self.frames.push(CallFrame {
                                                proto: Proto::new(),
                                                base: new_base,
                                                ip: 0,
                                                protected: true,
                                                error_handler,
                                                is_error_handler: false,
                                                varargs: Vec::new(),
                                                num_args: Some(num_args_to_target as usize),
                                            });
                                            
                                            let res = rust_fn(self);
                                            self.frames.pop();
                                            frame = self.frames.pop().unwrap();
                                            
                                            let dst_start = frame.base + func as usize;
                                            match res {
                                                Ok(num_ret) => {
                                                    self.stack[dst_start] = Value::Boolean(true);
                                                    for i in 0..num_ret as usize {
                                                        self.stack[dst_start + 1 + i] = self.stack[self.stack.len() - num_ret as usize + i];
                                                    }
                                                    self.stack.truncate(ret_start);
                                                }
                                                Err(e) => {
                                                    self.stack[dst_start] = Value::Boolean(false);
                                                    let Error::Runtime(err_msg) = e;
                                                    let err_str_id = self.gc.alloc_string(err_msg.into_bytes());
                                                    
                                                    if let Some(Value::LightFunction(msgh_rust_fn)) = error_handler {
                                                        self.stack.resize(new_base + 1, Value::Nil);
                                                        self.stack[new_base] = Value::String(err_str_id);
                                                        self.frames.push(frame.clone());
                                                        self.frames.push(CallFrame {
                                                            proto: Proto::new(),
                                                            base: new_base,
                                                            ip: 0,
                                                            protected: false,
                                                            error_handler: None,
                                                            is_error_handler: true,
                                                            varargs: Vec::new(),
                                                            num_args: Some(1),
                                                        });
                                                        let num_ret_msgh = vm_try!(frame, msgh_rust_fn(self));
                                                        self.frames.pop();
                                                        frame = self.frames.pop().unwrap();
                                                        if num_ret_msgh > 0 {
                                                            self.stack[dst_start + 1] = self.stack[self.stack.len() - num_ret_msgh as usize];
                                                        } else {
                                                            self.stack[dst_start + 1] = Value::Nil;
                                                        }
                                                    } else {
                                                        self.stack[dst_start + 1] = Value::String(err_str_id);
                                                    }
                                                    self.stack.truncate(ret_start);
                                                }
                                            }
                                        }
                                    }
                                }
                                Value::LightFunction(inner_rust_fn) => {
                                    let new_base = frame.base + func as usize + args_start_offset;
                                    let ret_start = self.stack.len();
                                    let required_len = new_base + num_args_to_target as usize;
                                    if self.stack.len() < required_len {
                                        self.stack.resize(required_len, Value::Nil);
                                    }
                                    
                                    self.frames.push(frame.clone());
                                    self.frames.push(CallFrame {
                                        proto: Proto::new(),
                                        base: new_base,
                                        ip: 0,
                                        protected: true,
                                        error_handler,
                                        is_error_handler: false,
                                        varargs: Vec::new(),
                                        num_args: Some(num_args_to_target as usize),
                                    });
                                    
                                    let res = inner_rust_fn(self);
                                    self.frames.pop();
                                    frame = self.frames.pop().unwrap();
                                    
                                    let dst_start = frame.base + func as usize;
                                    match res {
                                        Ok(num_ret) => {
                                            self.stack[dst_start] = Value::Boolean(true);
                                            for i in 0..num_ret as usize {
                                                self.stack[dst_start + 1 + i] = self.stack[self.stack.len() - num_ret as usize + i];
                                            }
                                            self.stack.truncate(ret_start);
                                        }
                                        Err(e) => {
                                            self.stack[dst_start] = Value::Boolean(false);
                                            let Error::Runtime(err_msg) = e;
                                            let err_str_id = self.gc.alloc_string(err_msg.into_bytes());
                                            
                                            if let Some(Value::LightFunction(msgh_rust_fn)) = error_handler {
                                                self.stack.resize(new_base + 1, Value::Nil);
                                                self.stack[new_base] = Value::String(err_str_id);
                                                self.frames.push(frame.clone());
                                                self.frames.push(CallFrame {
                                                    proto: Proto::new(),
                                                    base: new_base,
                                                    ip: 0,
                                                    protected: false,
                                                    error_handler: None,
                                                    is_error_handler: true,
                                                    varargs: Vec::new(),
                                                    num_args: Some(1),
                                                });
                                                let num_ret_msgh = vm_try!(frame, msgh_rust_fn(self));
                                                self.frames.pop();
                                                frame = self.frames.pop().unwrap();
                                                if num_ret_msgh > 0 {
                                                    self.stack[dst_start + 1] = self.stack[self.stack.len() - num_ret_msgh as usize];
                                                } else {
                                                    self.stack[dst_start + 1] = Value::Nil;
                                                }
                                            } else {
                                                self.stack[dst_start + 1] = Value::String(err_str_id);
                                            }
                                            self.stack.truncate(ret_start);
                                        }
                                    }
                                }
                                _ => {
                                    let dst_start = frame.base + func as usize;
                                    self.stack[dst_start] = Value::Boolean(false);
                                    let err_str_id = self.gc.alloc_string("attempt to call a non-function value".to_string().into_bytes());
                                    self.stack[dst_start + 1] = Value::String(err_str_id);
                                }
                            }
                        } else {
                            match val {
                                Value::Function(func_id) => {
                                    match self.gc.get_function(func_id) {
                                        LuaFunction::Lua(closure) => {
                                            self.frames.push(frame.clone());
                                            let new_base = frame.base + func as usize + 1;
                                            let required_len = new_base + usize::max(
                                                closure.proto.max_stack_size as usize,
                                                num_args as usize,
                                            );
                                            if self.stack.len() < required_len {
                                                self.stack.resize(required_len, Value::Nil);
                                            }
                                            for i in 0..num_args as usize {
                                                self.stack[new_base + i] = self.stack[frame.base + func as usize + 1 + i];
                                            }
                                            let mut varargs = Vec::new();
                                            if closure.proto.is_vararg {
                                                let num_params = closure.proto.num_params as usize;
                                                if num_args as usize > num_params {
                                                    for i in num_params..num_args as usize {
                                                        varargs.push(self.stack[frame.base + func as usize + 1 + i]);
                                                    }
                                                }
                                            }
                                            frame = CallFrame {
                                                proto: closure.proto.clone(),
                                                base: new_base,
                                                ip: 0,
                                                protected: false,
                                                error_handler: None,
                                                is_error_handler: false,
                                                varargs,
                                                num_args: None,
                                            };
                                        }
                                        LuaFunction::Rust(rust_closure) => {
                                            let rust_fn = rust_closure.func;
                                            let new_base = frame.base + func as usize + 1;
                                            let ret_start = self.stack.len();
                                            let required_len = new_base + num_args as usize;
                                            if self.stack.len() < required_len {
                                                self.stack.resize(required_len, Value::Nil);
                                            }
                                            
                                            self.frames.push(frame.clone());
                                            self.frames.push(CallFrame {
                                                proto: Proto::new(),
                                                base: new_base,
                                                ip: 0,
                                                protected: false,
                                                error_handler: None,
                                                is_error_handler: false,
                                                varargs: Vec::new(),
                                                num_args: Some(num_args as usize),
                                            });
                                            
                                            let num_ret = vm_try!(frame, rust_fn(self));
                                            
                                            if num_ret >= 0 {
                                                self.frames.pop();
                                                frame = self.frames.pop().unwrap();

                                                let dst_start = frame.base + func as usize;
                                                let copy_count = num_results as usize;
                                                for i in 0..copy_count {
                                                    let val = if i < num_ret as usize {
                                                        self.stack[self.stack.len() - num_ret as usize + i]
                                                    } else {
                                                        Value::Nil
                                                    };
                                                    self.stack[dst_start + i] = val;
                                                }
                                                self.stack.truncate(ret_start);
                                            } else {
                                                frame = self.frames.pop().unwrap();
                                            }
                                        }
                                    }
                                }
                                Value::LightFunction(rust_fn) => {
                                    let new_base = frame.base + func as usize + 1;
                                    let ret_start = self.stack.len();
                                    let required_len = new_base + num_args as usize;
                                    if self.stack.len() < required_len {
                                        self.stack.resize(required_len, Value::Nil);
                                    }
                                    
                                    self.frames.push(frame.clone());
                                    self.frames.push(CallFrame {
                                        proto: Proto::new(),
                                        base: new_base,
                                        ip: 0,
                                        protected: false,
                                        error_handler: None,
                                        is_error_handler: false,
                                        varargs: Vec::new(),
                                        num_args: Some(num_args as usize),
                                    });
                                    
                                    let num_ret = vm_try!(frame, rust_fn(self));
                                    
                                    if num_ret >= 0 {
                                        self.frames.pop();
                                        frame = self.frames.pop().unwrap();
                                        
                                        let dst_start = frame.base + func as usize;
                                        let copy_count = num_results as usize;
                                        for i in 0..copy_count {
                                            let val = if i < num_ret as usize {
                                                self.stack[self.stack.len() - num_ret as usize + i]
                                            } else {
                                                Value::Nil
                                            };
                                            self.stack[dst_start + i] = val;
                                        }
                                        self.stack.truncate(ret_start);
                                    } else {
                                        frame = self.frames.pop().unwrap();
                                    }
                                }
                                _ => {
                                    let inst = frame.proto.instructions[frame.ip - 1];
                                    return Err(Error::Runtime(format!(
                                        "attempt to call a non-function value: {:?} at instruction {:?} (ip: {}), constants: {:?}",
                                        val, inst, frame.ip - 1, frame.proto.constants
                                    )));
                                }
                            }
                        }
                    }
                    Instruction::Return { start, count } => {
                        self.close_upvalues(frame.base);
                        eprintln!("[DEBUG] Return: frame.base={}, frame.protected={}, start={}, count={}, frames.len()={}", frame.base, frame.protected, start, count, self.frames.len());

                        if let Some(parent_frame) = self.frames.pop() {
                            let parent_ip = parent_frame.ip;
                            let parent_inst = parent_frame.proto.instructions[parent_ip - 1];
                            let call_info = match parent_inst {
                                Instruction::Call { func, num_results, .. } => Some((func, num_results)),
                                Instruction::GetTable { dst, .. } | Instruction::GetTabUp { dst, .. } => Some((dst, 1)),
                                Instruction::Add { dst, .. } |
                                Instruction::Sub { dst, .. } |
                                Instruction::Mul { dst, .. } |
                                Instruction::Div { dst, .. } |
                                Instruction::Mod { dst, .. } |
                                Instruction::Pow { dst, .. } |
                                Instruction::IDiv { dst, .. } |
                                Instruction::BAnd { dst, .. } |
                                Instruction::BOr { dst, .. } |
                                Instruction::BXor { dst, .. } |
                                Instruction::Shl { dst, .. } |
                                Instruction::Shr { dst, .. } |
                                Instruction::Concat { dst, .. } => Some((dst, 1)),
                                Instruction::UNeg { dst, .. } |
                                Instruction::UNot { dst, .. } |
                                Instruction::ULen { dst, .. } |
                                Instruction::UBNot { dst, .. } => Some((dst, 1)),
                                _ => None,
                            };
                            
                            match parent_inst {
                                Instruction::Eq { eq, .. } | Instruction::Lt { eq, .. } | Instruction::Le { eq, .. } => {
                                    let ret_val = if count > 0 {
                                        self.stack[frame.base + start as usize]
                                    } else {
                                        Value::Nil
                                    };
                                    let truthy = !matches!(ret_val, Value::Nil | Value::Boolean(false));
                                    if truthy != eq {
                                        frame = parent_frame;
                                        frame.ip += 1;
                                    } else {
                                        frame = parent_frame;
                                    }
                                }
                                _ => {
                                    if let Some((dst, num_results)) = call_info {
                                        let dst_start = parent_frame.base + dst as usize;
                                        if frame.protected {
                                            eprintln!("[DEBUG] Protected Return: parent_frame.base={}, dst={}, dst_start={}, count={}, frame.base={}, start={}", parent_frame.base, dst, dst_start, count, frame.base, start);
                                            eprintln!("[DEBUG] stack.len()={}, stack[frame.base+start]={:?}", self.stack.len(), self.stack.get(frame.base + start as usize));
                                            self.stack[dst_start] = Value::Boolean(true);
                                            for i in 0..count as usize {
                                                self.stack[dst_start + 1 + i] = self.stack[frame.base + start as usize + i];
                                            }
                                            eprintln!("[DEBUG] After write: stack[{}]={:?}, stack[{}]={:?}", dst_start, self.stack[dst_start], dst_start+1, self.stack.get(dst_start+1));
                                        } else if frame.is_error_handler {
                                            if count > 0 {
                                                self.stack[dst_start + 1] = self.stack[frame.base + start as usize];
                                            } else {
                                                self.stack[dst_start + 1] = Value::Nil;
                                            }
                                        } else {
                                            let copy_count = num_results as usize;
                                            for i in 0..copy_count {
                                                let val = if i < count as usize {
                                                    self.stack[frame.base + start as usize + i]
                                                } else {
                                                    Value::Nil
                                                };
                                                self.stack[dst_start + i] = val;
                                            }
                                        }
                                    }
                                    frame = parent_frame;
                                }
                            }
                        } else {
                            if count > 0 {
                                self.stack[0] = self.stack[frame.base + start as usize];
                            } else {
                                self.stack[0] = Value::Nil;
                            }
                            return Ok(());
                        }
                    }
                    Instruction::Jmp { offset } => {
                        frame.ip = (frame.ip as i32 + offset) as usize;
                    }
                    Instruction::Eq { lhs, rhs, eq } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_eq(l, r, eq, &mut frame));
                    }
                    Instruction::Lt { lhs, rhs, eq } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_lt(l, r, eq, &mut frame));
                    }
                    Instruction::Le { lhs, rhs, eq } => {
                        let l = self.stack[frame.base + lhs as usize];
                        let r = self.stack[frame.base + rhs as usize];
                        vm_try!(frame, self.execute_le(l, r, eq, &mut frame));
                    }
                    Instruction::Test { reg, cond } => {
                        let val = self.stack[frame.base + reg as usize];
                        let is_truthy = !matches!(val, Value::Nil | Value::Boolean(false));
                        if is_truthy != cond {
                            frame.ip += 1;
                        }
                    }
                    Instruction::NewTable { dst, array_size, hash_size } => {
                        let tbl_id = self.gc.alloc_table();
                        {
                            let table = self.gc.get_table_mut(tbl_id);
                            table.array.reserve(array_size as usize);
                            table.hash.reserve(hash_size as usize);
                        }
                        self.stack[frame.base + dst as usize] = Value::Table(tbl_id);
                    }
                    Instruction::SetList { tbl, count, start_idx } => {
                        let tbl_val = self.stack[frame.base + tbl as usize];
                        let tbl_id = match tbl_val {
                            Value::Table(id) => id,
                            _ => return Err(Error::Runtime("attempt to set list on a non-table value".to_string())),
                        };

                        for i in 0..count as usize {
                            let key = Value::Integer(start_idx as i64 + i as i64 + 1);
                            let val = self.stack[frame.base + tbl as usize + 1 + i];
                            vm_try!(frame, self.table_raw_set(tbl_id, key, val));
                        }
                    }
                    Instruction::SetTable { tbl, key, val } => {
                        let tbl_val = self.stack[frame.base + tbl as usize];
                        let key_val = self.stack[frame.base + key as usize];
                        let val_val = self.stack[frame.base + val as usize];
                        vm_try!(frame, self.execute_set_table(tbl_val, key_val, val_val, &mut frame));
                    }
                    Instruction::GetTable { dst, tbl, key } => {
                        let tbl_val = self.stack[frame.base + tbl as usize];
                        let key_val = self.stack[frame.base + key as usize];
                        vm_try!(frame, self.execute_get_table(frame.base + dst as usize, tbl_val, key_val, &mut frame));
                    }
                    Instruction::Vararg { dst, count } => {
                        let num_to_copy = count as usize;
                        for i in 0..num_to_copy {
                            let val = if i < frame.varargs.len() {
                                frame.varargs[i]
                            } else {
                                Value::Nil
                            };
                            self.stack[frame.base + dst as usize + i] = val;
                        }
                    }
                    Instruction::ForPrep { reg, offset } => {
                        let init = self.stack[frame.base + reg as usize];
                        let step = self.stack[frame.base + reg as usize + 2];
                        match step {
                            Value::Integer(0) => {
                                return Err(Error::Runtime("'for' step is zero".to_string()));
                            }
                            Value::Number(0.0) => {
                                return Err(Error::Runtime("'for' step is zero".to_string()));
                            }
                            _ => {}
                        }
                        let init_next = match (init, step) {
                            (Value::Integer(i), Value::Integer(s)) => Value::Integer(i.wrapping_sub(s)),
                            (Value::Integer(i), Value::Number(s)) => Value::Number(i as f64 - s),
                            (Value::Number(i), Value::Integer(s)) => Value::Number(i - s as f64),
                            (Value::Number(i), Value::Number(s)) => Value::Number(i - s),
                            _ => return Err(Error::Runtime("loop values must be numbers".to_string())),
                        };
                        self.stack[frame.base + reg as usize] = init_next;
                        frame.ip = (frame.ip as i32 + offset) as usize;
                    }
                    Instruction::ForLoop { reg, offset } => {
                        let init = self.stack[frame.base + reg as usize];
                        let limit = self.stack[frame.base + reg as usize + 1];
                        let step = self.stack[frame.base + reg as usize + 2];

                        let (next_init, loop_again) = match (init, limit, step) {
                            (Value::Integer(i), Value::Integer(lim), Value::Integer(s)) => {
                                let next = i.wrapping_add(s);
                                let again = if s >= 0 { next <= lim } else { next >= lim };
                                (Value::Integer(next), again)
                            }
                            _ => {
                                let i_f = vm_try!(frame, init.to_number().ok_or_else(|| Error::Runtime("loop init must be a number".to_string())));
                                let lim_f = vm_try!(frame, limit.to_number().ok_or_else(|| Error::Runtime("loop limit must be a number".to_string())));
                                let s_f = vm_try!(frame, step.to_number().ok_or_else(|| Error::Runtime("loop step must be a number".to_string())));
                                let next = i_f + s_f;
                                let again = if s_f >= 0.0 { next <= lim_f } else { next >= lim_f };
                                (Value::Number(next), again)
                            }
                        };

                        self.stack[frame.base + reg as usize] = next_init;
                        if loop_again {
                            self.stack[frame.base + reg as usize + 3] = next_init;
                            frame.ip = (frame.ip as i32 + offset) as usize;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Error {
    Runtime(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Runtime(s) => write!(f, "runtime error: {}", s),
        }
    }
}

impl std::error::Error for Error {}
