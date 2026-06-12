use crate::value::{LuaFunction, TableId, Value};
use crate::vm::{CallFrame, Error, VM};

fn lua_string(vm: &mut VM, bytes: impl Into<Vec<u8>>) -> Value {
    Value::String(vm.gc.alloc_string(bytes.into()))
}

fn set_table_field(vm: &mut VM, table_id: TableId, name: &str, value: Value) {
    let key = vm.gc.alloc_string(name.as_bytes().to_vec());
    vm.gc
        .get_table_mut(table_id)
        .hash
        .insert(Value::String(key), value);
}

fn set_global(vm: &mut VM, name: &str, value: Value) {
    set_table_field(vm, vm.globals, name, value);
}

fn register_module(
    vm: &mut VM,
    name: &str,
    funcs: &[(&str, crate::value::RustFunction)],
) -> TableId {
    let table_id = vm.gc.alloc_table();
    for &(func_name, func) in funcs {
        set_table_field(vm, table_id, func_name, Value::LightFunction(func));
    }
    set_global(vm, name, Value::Table(table_id));
    table_id
}

fn table_sequence_len(vm: &VM, table_id: TableId) -> i64 {
    let table = vm.gc.get_table(table_id);
    let mut i = 1;
    loop {
        let from_array = (i as usize) <= table.array.len()
            && table.array[i as usize - 1] != Value::Nil;
        let from_hash = table
            .hash
            .get(&Value::Integer(i))
            .is_some_and(|value| *value != Value::Nil);
        if !from_array && !from_hash {
            return i - 1;
        }
        i += 1;
    }
}

pub fn register_stdlib(vm: &mut VM) {
    let base_funcs = [
        ("print", std_print as crate::value::RustFunction),
        ("assert", std_assert as crate::value::RustFunction),
        ("type", std_type as crate::value::RustFunction),
        ("error", std_error as crate::value::RustFunction),
        ("setmetatable", std_setmetatable as crate::value::RustFunction),
        ("getmetatable", std_getmetatable as crate::value::RustFunction),
        ("pcall", std_pcall as crate::value::RustFunction),
        ("xpcall", std_xpcall as crate::value::RustFunction),
        ("select", std_select as crate::value::RustFunction),
        ("tonumber", std_tonumber as crate::value::RustFunction),
        ("tostring", std_tostring as crate::value::RustFunction),
        ("loadfile", std_loadfile as crate::value::RustFunction),
        ("load", std_load as crate::value::RustFunction),
        ("rawget", std_rawget as crate::value::RustFunction),
        ("rawset", std_rawset as crate::value::RustFunction),
        ("rawequal", std_rawequal as crate::value::RustFunction),
        ("next", std_next as crate::value::RustFunction),
        ("pairs", std_pairs as crate::value::RustFunction),
        ("ipairs", std_ipairs as crate::value::RustFunction),
        ("rawlen", std_rawlen as crate::value::RustFunction),
        ("collectgarbage", std_collectgarbage as crate::value::RustFunction),
        ("warn", std_warn as crate::value::RustFunction),
    ];

    for &(name, func) in &base_funcs {
        set_global(vm, name, Value::LightFunction(func));
    }

    let version = lua_string(vm, b"Lua 5.5".to_vec());
    set_global(vm, "_VERSION", version);
    set_global(vm, "_G", Value::Table(vm.globals));

    let math_funcs = [
        ("tointeger", std_math_tointeger as crate::value::RustFunction),
        ("type", std_math_type as crate::value::RustFunction),
        ("abs", std_math_abs as crate::value::RustFunction),
        ("min", std_math_min as crate::value::RustFunction),
        ("max", std_math_max as crate::value::RustFunction),
        ("floor", std_math_floor as crate::value::RustFunction),
        ("ceil", std_math_ceil as crate::value::RustFunction),
        ("fmod", std_math_fmod as crate::value::RustFunction),
        ("sin", std_math_sin as crate::value::RustFunction),
        ("cos", std_math_cos as crate::value::RustFunction),
        ("sqrt", std_math_sqrt as crate::value::RustFunction),
        ("log", std_math_log as crate::value::RustFunction),
        ("randomseed", std_math_randomseed as crate::value::RustFunction),
        ("random", std_math_random as crate::value::RustFunction),
    ];
    let math_tbl = register_module(vm, "math", &math_funcs);
    set_table_field(vm, math_tbl, "mininteger", Value::Integer(i64::MIN));
    set_table_field(vm, math_tbl, "maxinteger", Value::Integer(i64::MAX));
    set_table_field(vm, math_tbl, "huge", Value::Number(f64::INFINITY));
    set_table_field(vm, math_tbl, "pi", Value::Number(std::f64::consts::PI));

    let string_funcs = [
        ("packsize", std_string_packsize as crate::value::RustFunction),
        ("format", std_string_format as crate::value::RustFunction),
        ("find", std_string_find as crate::value::RustFunction),
        ("sub", std_string_sub as crate::value::RustFunction),
        ("len", std_string_len as crate::value::RustFunction),
        ("lower", std_string_lower as crate::value::RustFunction),
        ("upper", std_string_upper as crate::value::RustFunction),
        ("rep", std_string_rep as crate::value::RustFunction),
        ("byte", std_string_byte as crate::value::RustFunction),
        ("char", std_string_char as crate::value::RustFunction),
    ];
    let string_tbl = register_module(vm, "string", &string_funcs);

    // Set string metatable
    let string_meta = vm.gc.alloc_table();
    set_table_field(vm, string_meta, "__index", Value::Table(string_tbl));
    vm.string_metatable = Some(string_meta);

    let table_funcs = [
        ("pack", std_table_pack as crate::value::RustFunction),
        ("unpack", std_table_unpack as crate::value::RustFunction),
        ("concat", std_table_concat as crate::value::RustFunction),
        ("insert", std_table_insert as crate::value::RustFunction),
        ("remove", std_table_remove as crate::value::RustFunction),
    ];
    let table_tbl = register_module(vm, "table", &table_funcs);
    set_global(
        vm,
        "unpack",
        Value::LightFunction(std_table_unpack as crate::value::RustFunction),
    );

    let io_funcs = [
        ("write", std_io_write as crate::value::RustFunction),
        ("read", std_io_read as crate::value::RustFunction),
        ("open", std_io_open as crate::value::RustFunction),
    ];
    let io_tbl = register_module(vm, "io", &io_funcs);
    let stdout_tbl = vm.gc.alloc_table();
    set_table_field(
        vm,
        stdout_tbl,
        "write",
        Value::LightFunction(std_io_write as crate::value::RustFunction),
    );
    let stderr_tbl = vm.gc.alloc_table();
    set_table_field(
        vm,
        stderr_tbl,
        "write",
        Value::LightFunction(std_io_write as crate::value::RustFunction),
    );
    set_table_field(vm, io_tbl, "stdout", Value::Table(stdout_tbl));
    set_table_field(vm, io_tbl, "stderr", Value::Table(stderr_tbl));
    let stdin_tbl = vm.gc.alloc_table();
    set_table_field(vm, io_tbl, "stdin", Value::Table(stdin_tbl));

    let os_funcs = [
        ("clock", std_os_clock as crate::value::RustFunction),
        ("time", std_os_time as crate::value::RustFunction),
        ("difftime", std_os_difftime as crate::value::RustFunction),
        ("setlocale", std_os_setlocale as crate::value::RustFunction),
    ];
    let os_tbl = register_module(vm, "os", &os_funcs);

    let debug_funcs = [
        ("getregistry", std_debug_getregistry as crate::value::RustFunction),
        ("getmetatable", std_debug_getmetatable as crate::value::RustFunction),
        ("setmetatable", std_debug_setmetatable as crate::value::RustFunction),
    ];
    let debug_tbl = register_module(vm, "debug", &debug_funcs);

    let package_tbl = vm.gc.alloc_table();
    let loaded_tbl = vm.gc.alloc_table();
    set_table_field(vm, loaded_tbl, "math", Value::Table(math_tbl));
    set_table_field(vm, loaded_tbl, "string", Value::Table(string_tbl));
    set_table_field(vm, loaded_tbl, "table", Value::Table(table_tbl));
    set_table_field(vm, loaded_tbl, "io", Value::Table(io_tbl));
    set_table_field(vm, loaded_tbl, "os", Value::Table(os_tbl));
    set_table_field(vm, loaded_tbl, "debug", Value::Table(debug_tbl));
    set_table_field(vm, package_tbl, "loaded", Value::Table(loaded_tbl));
    let preload_tbl = vm.gc.alloc_table();
    set_table_field(vm, package_tbl, "preload", Value::Table(preload_tbl));
    let package_path = lua_string(vm, b"./?.lua;/Users/josh/lua-5.5.0-tests/?.lua".to_vec());
    set_table_field(vm, package_tbl, "path", package_path);
    set_global(vm, "package", Value::Table(package_tbl));

    let init_lua = r#"
        function require(modname)
            if package.loaded[modname] then
                return package.loaded[modname]
            end
            local loader = package.preload[modname]
            local err = nil
            if not loader then
                loader, err = loadfile(modname .. '.lua')
            end
            if not loader then
                loader, err = loadfile('/Users/josh/lua-5.5.0-tests/' .. modname .. '.lua')
            end
            if not loader then
                error(err or ('module \'' .. modname .. '\' not found'))
            end
            local res = loader()
            if res == nil then
                res = true
            end
            package.loaded[modname] = res
            return res
        end
        function dofile(filename)
            local f, err = loadfile(filename)
            if not f then
                error(err)
            end
            return f()
        end
    "#;
    let lex = crate::lexer::Lexer::new(init_lua.as_bytes());
    let mut parser = crate::parser::Parser::new(lex);
    if let Ok(proto) = parser.parse_chunk().map_err(|_| ()).and_then(|block| crate::compiler::Compiler::compile_chunk(&block).map_err(|_| ())) {
        let _ = vm.execute(proto);
    }
}

pub fn std_print(vm: &mut VM) -> Result<i32, Error> {
    let count = vm.get_arg_count();
    let mut s = String::new();
    for i in 0..count {
        if i > 0 {
            s.push('\t');
        }
        s.push_str(&vm.value_to_string(vm.get_arg(i)));
    }
    println!("{}", s);
    Ok(0)
}

pub fn std_assert(vm: &mut VM) -> Result<i32, Error> {
    let cond = vm.get_arg(0);
    let truthy = !matches!(cond, Value::Nil | Value::Boolean(false));
    if !truthy {
        let msg = vm.get_arg(1);
        let msg_str = match msg {
            Value::Nil => "assertion failed!".to_string(),
            _ => vm.value_to_string(msg),
        };
        return Err(Error::Runtime(msg_str));
    }
    let count = vm.get_arg_count();
    for i in 0..count {
        let val = vm.get_arg(i);
        vm.push_value(val);
    }
    Ok(count as i32)
}

pub fn std_type(vm: &mut VM) -> Result<i32, Error> {
    let val = vm.get_arg(0);
    let t_str = match val {
        Value::Nil => "nil",
        Value::Boolean(_) => "boolean",
        Value::Integer(_) | Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Table(_) => "table",
        Value::Function(_) | Value::LightFunction(_) => "function",
        Value::Thread(_) => "thread",
        Value::Userdata(_) | Value::LightUserdata(_) => "userdata",
    };
    let str_id = vm.gc.alloc_string(t_str.as_bytes().to_vec());
    vm.push_value(Value::String(str_id));
    Ok(1)
}

pub fn std_error(vm: &mut VM) -> Result<i32, Error> {
    let msg = vm.get_arg(0);
    let msg_str = vm.value_to_string(msg);
    Err(Error::Runtime(msg_str))
}

pub fn std_setmetatable(vm: &mut VM) -> Result<i32, Error> {
    let tbl_val = vm.get_arg(0);
    let meta_val = vm.get_arg(1);

    let tbl_id = match tbl_val {
        Value::Table(id) => id,
        _ => return Err(Error::Runtime("bad argument #1 to 'setmetatable' (table expected)".to_string())),
    };

    let meta_id = match meta_val {
        Value::Nil => None,
        Value::Table(id) => Some(id),
        _ => return Err(Error::Runtime("bad argument #2 to 'setmetatable' (nil or table expected)".to_string())),
    };

    if let Some(existing_meta) = vm.gc.get_table(tbl_id).metatable {
        let meta_table = vm.gc.get_table(existing_meta);
        for k in meta_table.hash.keys() {
            if let Value::String(s_id) = k {
                let is_protected = vm.gc.get_string(*s_id).data == b"__metatable";
                if is_protected {
                    return Err(Error::Runtime("cannot change a protected metatable".to_string()));
                }
            }
        }
    }

    vm.gc.get_table_mut(tbl_id).metatable = meta_id;
    vm.push_value(tbl_val);
    Ok(1)
}

pub fn std_getmetatable(vm: &mut VM) -> Result<i32, Error> {
    let val = vm.get_arg(0);
    let meta_id = vm.get_metatable(val);
    match meta_id {
        Some(m_id) => {
            let meta_table = vm.gc.get_table(m_id);
            let mut protected_val = None;
            for (k, v) in &meta_table.hash {
                if let Value::String(s_id) = k {
                    let is_protected = vm.gc.get_string(*s_id).data == b"__metatable";
                    if is_protected {
                        protected_val = Some(*v);
                        break;
                    }
                }
            }
            if let Some(pv) = protected_val {
                vm.push_value(pv);
            } else {
                vm.push_value(Value::Table(m_id));
            }
        }
        None => {
            vm.push_value(Value::Nil);
        }
    }
    Ok(1)
}

pub fn std_pcall(_vm: &mut VM) -> Result<i32, Error> {
    // pcall is handled as a special case inside VM's Call instruction.
    // This handler exists only to provide the function pointer identity.
    Ok(0)
}

pub fn std_xpcall(_vm: &mut VM) -> Result<i32, Error> {
    // xpcall is handled as a special case inside VM's Call instruction.
    // This handler exists only to provide the function pointer identity.
    Ok(0)
}

pub fn std_select(vm: &mut VM) -> Result<i32, Error> {
    let arg1 = vm.get_arg(0);
    let count = vm.get_arg_count();

    match arg1 {
        Value::String(id) if vm.gc.get_string(id).data == b"#" => {
            vm.push_value(Value::Integer(count.saturating_sub(1) as i64));
            Ok(1)
        }
        _ => {
            let idx = match arg1.to_integer() {
                Some(i) => i,
                None => return Err(Error::Runtime("bad argument #1 to 'select' (number expected)".to_string())),
            };
            let num_args = count as i64 - 1;
            let real_idx = if idx > 0 {
                idx
            } else if idx < 0 {
                num_args + idx + 1
            } else {
                return Err(Error::Runtime("bad argument #1 to 'select' (index out of range)".to_string()));
            };

            if real_idx < 1 || real_idx > num_args {
                return Err(Error::Runtime("bad argument #1 to 'select' (index out of range)".to_string()));
            }

            let start_idx = real_idx as usize;
            let ret_count = (num_args - real_idx + 1) as i32;
            for i in 0..ret_count as usize {
                vm.push_value(vm.get_arg(start_idx + i));
            }
            Ok(ret_count)
        }
    }
}

pub fn std_tonumber(vm: &mut VM) -> Result<i32, Error> {
    let val = vm.get_arg(0);
    let base_val = vm.get_arg(1);

    let base = match base_val {
        Value::Nil => 10,
        Value::Integer(b) => {
            if !(2..=36).contains(&b) {
                return Err(Error::Runtime("bad argument #2 to 'tonumber' (base out of range)".to_string()));
            }
            b
        }
        _ => 10,
    };

    match val {
        Value::Integer(i) => {
            if base == 10 {
                vm.push_value(Value::Integer(i));
                Ok(1)
            } else {
                Err(Error::Runtime("bad argument #1 to 'tonumber' (string expected, got number)".to_string()))
            }
        }
        Value::Number(f) => {
            if base == 10 {
                vm.push_value(Value::Number(f));
                Ok(1)
            } else {
                Err(Error::Runtime("bad argument #1 to 'tonumber' (string expected, got number)".to_string()))
            }
        }
        Value::String(id) => {
            let s_bytes = &vm.gc.get_string(id).data;
            let s = String::from_utf8_lossy(s_bytes).trim().to_string();
            if base == 10 {
                if let Ok(i) = s.parse::<i64>() {
                    vm.push_value(Value::Integer(i));
                    return Ok(1);
                }
                if let Ok(f) = s.parse::<f64>() {
                    vm.push_value(Value::Number(f));
                    return Ok(1);
                }
            } else {
                if let Ok(i) = i64::from_str_radix(&s, base as u32) {
                    vm.push_value(Value::Integer(i));
                    return Ok(1);
                }
            }
            vm.push_value(Value::Nil);
            Ok(1)
        }
        _ => {
            vm.push_value(Value::Nil);
            Ok(1)
        }
    }
}

pub fn std_tostring(vm: &mut VM) -> Result<i32, Error> {
    let val = vm.get_arg(0);
    if let Some(mm) = vm.get_metamethod(val, "__tostring") {
        match mm {
            Value::Function(func_id) => {
                let closure = match vm.gc.get_function(func_id) {
                    LuaFunction::Lua(c) => c,
                    _ => return Err(Error::Runtime("native metamethod calling not supported".to_string())),
                };

                vm.frames.pop(); // pop std_tostring Rust frame
                vm.frames.push(CallFrame::new(closure.proto.clone(), vm.stack.len() + 1));
                return Ok(-1);
            }
            _ => return Err(Error::Runtime("metamethod must be a function".to_string())),
        }
    }
    let s = vm.value_to_string(val);
    let str_id = vm.gc.alloc_string(s.into_bytes());
    vm.push_value(Value::String(str_id));
    Ok(1)
}

pub fn std_loadfile(vm: &mut VM) -> Result<i32, Error> {
    let filename_val = vm.get_arg(0);
    let filename = match filename_val {
        Value::String(s_id) => {
            let bytes = &vm.gc.get_string(s_id).data;
            String::from_utf8_lossy(bytes).into_owned()
        }
        _ => return Err(Error::Runtime("bad argument #1 to 'loadfile' (string expected)".to_string())),
    };
    let source = match std::fs::read(&filename) {
        Ok(s) => s,
        Err(_) => {
            vm.push_value(Value::Nil);
            let err_msg = vm.gc.alloc_string(format!("cannot open {}: No such file or directory", filename).into_bytes());
            vm.push_value(Value::String(err_msg));
            return Ok(2);
        }
    };
    let lex = crate::lexer::Lexer::new(&source);
    let mut parser = crate::parser::Parser::new(lex);
    let block = match parser.parse_chunk() {
        Ok(b) => b,
        Err(e) => {
            vm.push_value(Value::Nil);
            let err_msg = vm.gc.alloc_string(format!("{:?}", e).into_bytes());
            vm.push_value(Value::String(err_msg));
            return Ok(2);
        }
    };
    let proto = match crate::compiler::Compiler::compile_chunk(&block) {
        Ok(p) => p,
        Err(e) => {
            vm.push_value(Value::Nil);
            let err_msg = vm.gc.alloc_string(format!("{:?}", e).into_bytes());
            vm.push_value(Value::String(err_msg));
            return Ok(2);
        }
    };
    let mut upvalues = Vec::new();
    for up_desc in &proto.upvalues {
        let val = if up_desc.name.as_deref() == Some("_ENV") {
            Value::Table(vm.globals)
        } else {
            Value::Nil
        };
        let up_id = vm.gc.alloc_upvalue(crate::value::Upvalue {
            val: crate::value::UpvalueState::Closed(val),
        });
        upvalues.push(up_id);
    }
    let closure = crate::value::LuaClosure {
        proto,
        upvalues,
    };
    let func_id = vm.gc.alloc_function(crate::value::LuaFunction::Lua(closure));
    vm.push_value(Value::Function(func_id));
    Ok(1)
}

pub fn std_load(vm: &mut VM) -> Result<i32, Error> {
    let source_val = vm.get_arg(0);
    let source = match source_val {
        Value::String(s_id) => vm.gc.get_string(s_id).data.clone(),
        _ => return Err(Error::Runtime("bad argument #1 to 'load' (string expected)".to_string())),
    };
    let lex = crate::lexer::Lexer::new(&source);
    let mut parser = crate::parser::Parser::new(lex);
    let block = match parser.parse_chunk() {
        Ok(b) => b,
        Err(e) => {
            vm.push_value(Value::Nil);
            let err_msg = vm.gc.alloc_string(format!("{:?}", e).into_bytes());
            vm.push_value(Value::String(err_msg));
            return Ok(2);
        }
    };
    let proto = match crate::compiler::Compiler::compile_chunk(&block) {
        Ok(p) => p,
        Err(e) => {
            vm.push_value(Value::Nil);
            let err_msg = vm.gc.alloc_string(format!("{:?}", e).into_bytes());
            vm.push_value(Value::String(err_msg));
            return Ok(2);
        }
    };
    let mut upvalues = Vec::new();
    for up_desc in &proto.upvalues {
        let val = if up_desc.name.as_deref() == Some("_ENV") {
            Value::Table(vm.globals)
        } else {
            Value::Nil
        };
        let up_id = vm.gc.alloc_upvalue(crate::value::Upvalue {
            val: crate::value::UpvalueState::Closed(val),
        });
        upvalues.push(up_id);
    }
    let closure = crate::value::LuaClosure {
        proto,
        upvalues,
    };
    let func_id = vm.gc.alloc_function(crate::value::LuaFunction::Lua(closure));
    vm.push_value(Value::Function(func_id));
    Ok(1)
}

pub fn std_rawget(vm: &mut VM) -> Result<i32, Error> {
    let t = vm.get_arg(0);
    let k = vm.get_arg(1);
    if let Value::Table(id) = t {
        let table = vm.gc.get_table(id);
        let val = table.hash.get(&k).cloned().unwrap_or(Value::Nil);
        vm.push_value(val);
        Ok(1)
    } else {
        Err(Error::Runtime("bad argument #1 to 'rawget' (table expected)".to_string()))
    }
}

pub fn std_rawset(vm: &mut VM) -> Result<i32, Error> {
    let t = vm.get_arg(0);
    let k = vm.get_arg(1);
    let v = vm.get_arg(2);
    if let Value::Table(id) = t {
        if k == Value::Nil {
            return Err(Error::Runtime("table index is nil".to_string()));
        }
        let table = vm.gc.get_table_mut(id);
        table.hash.insert(k, v);
        vm.push_value(t);
        Ok(1)
    } else {
        Err(Error::Runtime("bad argument #1 to 'rawset' (table expected)".to_string()))
    }
}

pub fn std_rawequal(vm: &mut VM) -> Result<i32, Error> {
    let v1 = vm.get_arg(0);
    let v2 = vm.get_arg(1);
    vm.push_value(Value::Boolean(v1 == v2));
    Ok(1)
}

fn value_cmp(a: &Value, b: &Value, gc: &crate::gc::GcHeap) -> std::cmp::Ordering {
    let type_order = |v: &Value| match v {
        Value::Nil => 0,
        Value::Boolean(_) => 1,
        Value::Integer(_) => 2,
        Value::Number(_) => 3,
        Value::String(_) => 4,
        Value::Table(_) => 5,
        Value::Function(_) => 6,
        Value::LightFunction(_) => 7,
        Value::Thread(_) => 8,
        Value::Userdata(_) => 9,
        Value::LightUserdata(_) => 10,
    };
    let ta = type_order(a);
    let tb = type_order(b);
    if ta != tb {
        return ta.cmp(&tb);
    }
    match (a, b) {
        (Value::Boolean(x), Value::Boolean(y)) => x.cmp(y),
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => {
            x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::String(x), Value::String(y)) => {
            let sx = &gc.get_string(*x).data;
            let sy = &gc.get_string(*y).data;
            sx.cmp(sy)
        }
        (Value::Table(x), Value::Table(y)) => x.0.cmp(&y.0),
        (Value::Function(x), Value::Function(y)) => x.0.cmp(&y.0),
        (Value::LightFunction(x), Value::LightFunction(y)) => (*x as usize).cmp(&(*y as usize)),
        (Value::Thread(x), Value::Thread(y)) => x.0.cmp(&y.0),
        (Value::Userdata(x), Value::Userdata(y)) => x.0.cmp(&y.0),
        (Value::LightUserdata(x), Value::LightUserdata(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

pub fn std_next(vm: &mut VM) -> Result<i32, Error> {
    let t = vm.get_arg(0);
    let k = vm.get_arg(1);
    if let Value::Table(id) = t {
        let table = vm.gc.get_table(id);
        let mut keys: Vec<Value> = table.hash.iter()
            .filter(|(_, v)| **v != Value::Nil)
            .map(|(k, _)| *k)
            .collect();
        let gc_ref = &vm.gc;
        keys.sort_by(|a, b| value_cmp(a, b, gc_ref));

        if k == Value::Nil {
            if keys.is_empty() {
                vm.push_value(Value::Nil);
                Ok(1)
            } else {
                let next_k = keys[0];
                let next_v = table.hash.get(&next_k).cloned().unwrap_or(Value::Nil);
                vm.push_value(next_k);
                vm.push_value(next_v);
                Ok(2)
            }
        } else {
            if let Some(pos) = keys.iter().position(|x| *x == k) {
                if pos + 1 < keys.len() {
                    let next_k = keys[pos + 1];
                    let next_v = table.hash.get(&next_k).cloned().unwrap_or(Value::Nil);
                    vm.push_value(next_k);
                    vm.push_value(next_v);
                    Ok(2)
                } else {
                    vm.push_value(Value::Nil);
                    Ok(1)
                }
            } else {
                Err(Error::Runtime("invalid key to 'next'".to_string()))
            }
        }
    } else {
        Err(Error::Runtime("bad argument #1 to 'next' (table expected)".to_string()))
    }
}

pub fn std_pairs(vm: &mut VM) -> Result<i32, Error> {
    let t = vm.get_arg(0);
    let next_fn = Value::LightFunction(std_next as crate::value::RustFunction);
    vm.push_value(next_fn);
    vm.push_value(t);
    vm.push_value(Value::Nil);
    Ok(3)
}

pub fn std_ipairs_aux(vm: &mut VM) -> Result<i32, Error> {
    let t = vm.get_arg(0);
    let i = vm.get_arg(1);
    if let Value::Integer(idx) = i {
        let next_idx = idx + 1;
        if let Value::Table(id) = t {
            let table = vm.gc.get_table(id);
            let val = table.hash.get(&Value::Integer(next_idx)).cloned().unwrap_or(Value::Nil);
            if val != Value::Nil {
                vm.push_value(Value::Integer(next_idx));
                vm.push_value(val);
                return Ok(2);
            }
        }
        vm.push_value(Value::Nil);
        Ok(1)
    } else {
        Err(Error::Runtime("ipairs iterator index must be an integer".to_string()))
    }
}

pub fn std_ipairs(vm: &mut VM) -> Result<i32, Error> {
    let t = vm.get_arg(0);
    let iter_fn = Value::LightFunction(std_ipairs_aux as crate::value::RustFunction);
    vm.push_value(iter_fn);
    vm.push_value(t);
    vm.push_value(Value::Integer(0));
    Ok(3)
}

pub fn std_rawlen(vm: &mut VM) -> Result<i32, Error> {
    match vm.get_arg(0) {
        Value::String(id) => {
            vm.push_value(Value::Integer(vm.gc.get_string(id).data.len() as i64));
            Ok(1)
        }
        Value::Table(id) => {
            vm.push_value(Value::Integer(table_sequence_len(vm, id)));
            Ok(1)
        }
        _ => Err(Error::Runtime(
            "bad argument #1 to 'rawlen' (table or string expected)".to_string(),
        )),
    }
}

pub fn std_collectgarbage(vm: &mut VM) -> Result<i32, Error> {
    match vm.get_arg(0) {
        Value::String(id) if vm.gc.get_string(id).data == b"count" => {
            vm.push_value(Value::Number(0.0));
        }
        _ => {
            vm.push_value(Value::Integer(0));
        }
    }
    Ok(1)
}

pub fn std_warn(vm: &mut VM) -> Result<i32, Error> {
    let count = vm.get_arg_count();
    if count == 0 {
        return Ok(0);
    }

    let first = vm.get_arg(0);
    if let Value::String(id) = first {
        let bytes = &vm.gc.get_string(id).data;
        if bytes == b"@on" || bytes == b"@off" {
            return Ok(0);
        }
    }

    let mut message = String::new();
    for i in 0..count {
        message.push_str(&vm.value_to_string(vm.get_arg(i)));
    }
    eprintln!("{message}");
    Ok(0)
}

pub fn std_table_pack(vm: &mut VM) -> Result<i32, Error> {
    let count = vm.get_arg_count();
    let table_id = vm.gc.alloc_table();
    for i in 0..count {
        let value = vm.get_arg(i);
        vm.gc
            .get_table_mut(table_id)
            .hash
            .insert(Value::Integer(i as i64 + 1), value);
    }
    set_table_field(vm, table_id, "n", Value::Integer(count as i64));
    vm.push_value(Value::Table(table_id));
    Ok(1)
}

pub fn std_table_unpack(vm: &mut VM) -> Result<i32, Error> {
    let table_id = match vm.get_arg(0) {
        Value::Table(id) => id,
        _ => {
            return Err(Error::Runtime(
                "bad argument #1 to 'unpack' (table expected)".to_string(),
            ));
        }
    };
    let start = vm.get_arg(1).to_integer().unwrap_or(1);
    let end = vm
        .get_arg(2)
        .to_integer()
        .unwrap_or_else(|| table_sequence_len(vm, table_id));
    if end < start {
        return Ok(0);
    }

    let mut returned = 0;
    for idx in start..=end {
        let value = vm
            .gc
            .get_table(table_id)
            .hash
            .get(&Value::Integer(idx))
            .copied()
            .unwrap_or(Value::Nil);
        vm.push_value(value);
        returned += 1;
    }
    Ok(returned)
}

pub fn std_table_concat(vm: &mut VM) -> Result<i32, Error> {
    let table_id = match vm.get_arg(0) {
        Value::Table(id) => id,
        _ => {
            return Err(Error::Runtime(
                "bad argument #1 to 'concat' (table expected)".to_string(),
            ));
        }
    };
    let sep = match vm.get_arg(1) {
        Value::Nil => String::new(),
        value => vm.value_to_string(value),
    };
    let start = vm.get_arg(2).to_integer().unwrap_or(1);
    let end = vm
        .get_arg(3)
        .to_integer()
        .unwrap_or_else(|| table_sequence_len(vm, table_id));

    let mut out = String::new();
    for idx in start..=end {
        if idx > start {
            out.push_str(&sep);
        }
        let value = vm
            .gc
            .get_table(table_id)
            .hash
            .get(&Value::Integer(idx))
            .copied()
            .unwrap_or(Value::Nil);
        if matches!(value, Value::Nil) {
            return Err(Error::Runtime(
                "invalid value (nil) at index in table for 'concat'".to_string(),
            ));
        }
        out.push_str(&vm.value_to_string(value));
    }
    let result = lua_string(vm, out.into_bytes());
    vm.push_value(result);
    Ok(1)
}

pub fn std_table_insert(vm: &mut VM) -> Result<i32, Error> {
    let table_id = match vm.get_arg(0) {
        Value::Table(id) => id,
        _ => {
            return Err(Error::Runtime(
                "bad argument #1 to 'insert' (table expected)".to_string(),
            ));
        }
    };
    let count = vm.get_arg_count();
    let len = table_sequence_len(vm, table_id);
    let (pos, value) = if count >= 3 {
        (vm.get_arg(1).to_integer().unwrap_or(len + 1), vm.get_arg(2))
    } else {
        (len + 1, vm.get_arg(1))
    };
    for idx in (pos..=len).rev() {
        let old = vm
            .gc
            .get_table(table_id)
            .hash
            .get(&Value::Integer(idx))
            .copied()
            .unwrap_or(Value::Nil);
        vm.gc
            .get_table_mut(table_id)
            .hash
            .insert(Value::Integer(idx + 1), old);
    }
    vm.gc
        .get_table_mut(table_id)
        .hash
        .insert(Value::Integer(pos), value);
    Ok(0)
}

pub fn std_table_remove(vm: &mut VM) -> Result<i32, Error> {
    let table_id = match vm.get_arg(0) {
        Value::Table(id) => id,
        _ => {
            return Err(Error::Runtime(
                "bad argument #1 to 'remove' (table expected)".to_string(),
            ));
        }
    };
    let len = table_sequence_len(vm, table_id);
    let pos = vm.get_arg(1).to_integer().unwrap_or(len);
    let removed = vm
        .gc
        .get_table(table_id)
        .hash
        .get(&Value::Integer(pos))
        .copied()
        .unwrap_or(Value::Nil);
    for idx in pos..len {
        let next = vm
            .gc
            .get_table(table_id)
            .hash
            .get(&Value::Integer(idx + 1))
            .copied()
            .unwrap_or(Value::Nil);
        vm.gc
            .get_table_mut(table_id)
            .hash
            .insert(Value::Integer(idx), next);
    }
    vm.gc
        .get_table_mut(table_id)
        .hash
        .insert(Value::Integer(len), Value::Nil);
    vm.push_value(removed);
    Ok(1)
}

fn number_arg(vm: &VM, idx: usize, name: &str) -> Result<f64, Error> {
    vm.get_arg(idx).to_number().ok_or_else(|| {
        Error::Runtime(format!(
            "bad argument #{} to '{}' (number expected)",
            idx + 1,
            name
        ))
    })
}

pub fn std_math_tointeger(vm: &mut VM) -> Result<i32, Error> {
    let val = vm.get_arg(0);
    match val {
        Value::Integer(i) => {
            vm.push_value(Value::Integer(i));
            Ok(1)
        }
        Value::Number(f) => {
            if f.fract() == 0.0 && f >= (i64::MIN as f64) && f <= (i64::MAX as f64) {
                vm.push_value(Value::Integer(f as i64));
            } else {
                vm.push_value(Value::Nil);
            }
            Ok(1)
        }
        Value::String(s_id) => {
            let s_bytes = &vm.gc.get_string(s_id).data;
            if let Ok(s) = std::str::from_utf8(s_bytes) {
                let trimmed = s.trim();
                if let Ok(i) = trimmed.parse::<i64>() {
                    vm.push_value(Value::Integer(i));
                    return Ok(1);
                }
                if let Some(f) = trimmed.parse::<f64>().ok().filter(|&f| f.fract() == 0.0 && f >= (i64::MIN as f64) && f <= (i64::MAX as f64)) {
                    vm.push_value(Value::Integer(f as i64));
                    return Ok(1);
                }
            }
            vm.push_value(Value::Nil);
            Ok(1)
        }
        _ => {
            vm.push_value(Value::Nil);
            Ok(1)
        }
    }
}

pub fn std_math_type(vm: &mut VM) -> Result<i32, Error> {
    let result = match vm.get_arg(0) {
        Value::Integer(_) => Some("integer"),
        Value::Number(_) => Some("float"),
        _ => None,
    };
    if let Some(kind) = result {
        let value = lua_string(vm, kind.as_bytes().to_vec());
        vm.push_value(value);
    } else {
        vm.push_value(Value::Nil);
    }
    Ok(1)
}

pub fn std_math_abs(vm: &mut VM) -> Result<i32, Error> {
    match vm.get_arg(0) {
        Value::Integer(i) => vm.push_value(Value::Integer(i.abs())),
        value => vm.push_value(Value::Number(
            value.to_number().ok_or_else(|| {
                Error::Runtime("bad argument #1 to 'abs' (number expected)".to_string())
            })?.abs(),
        )),
    }
    Ok(1)
}

pub fn std_math_min(vm: &mut VM) -> Result<i32, Error> {
    let count = vm.get_arg_count();
    if count == 0 {
        return Err(Error::Runtime("bad argument #1 to 'min' (value expected)".to_string()));
    }
    let mut best = vm.get_arg(0);
    for i in 1..count {
        let candidate = vm.get_arg(i);
        if number_arg(vm, i, "min")? < best.to_number().unwrap_or(f64::INFINITY) {
            best = candidate;
        }
    }
    vm.push_value(best);
    Ok(1)
}

pub fn std_math_max(vm: &mut VM) -> Result<i32, Error> {
    let count = vm.get_arg_count();
    if count == 0 {
        return Err(Error::Runtime("bad argument #1 to 'max' (value expected)".to_string()));
    }
    let mut best = vm.get_arg(0);
    for i in 1..count {
        let candidate = vm.get_arg(i);
        if number_arg(vm, i, "max")? > best.to_number().unwrap_or(f64::NEG_INFINITY) {
            best = candidate;
        }
    }
    vm.push_value(best);
    Ok(1)
}

pub fn std_math_floor(vm: &mut VM) -> Result<i32, Error> {
    let val = vm.get_arg(0);
    match val {
        Value::Integer(i) => {
            vm.push_value(Value::Integer(i));
            Ok(1)
        }
        Value::Number(f) => {
            vm.push_value(Value::Integer(f.floor() as i64));
            Ok(1)
        }
        _ => Err(Error::Runtime("bad argument #1 to 'floor' (number expected)".to_string())),
    }
}

pub fn std_math_ceil(vm: &mut VM) -> Result<i32, Error> {
    let val = vm.get_arg(0);
    match val {
        Value::Integer(i) => vm.push_value(Value::Integer(i)),
        Value::Number(f) => vm.push_value(Value::Integer(f.ceil() as i64)),
        _ => {
            return Err(Error::Runtime(
                "bad argument #1 to 'ceil' (number expected)".to_string(),
            ));
        }
    }
    Ok(1)
}

pub fn std_math_fmod(vm: &mut VM) -> Result<i32, Error> {
    let x_val = vm.get_arg(0);
    let y_val = vm.get_arg(1);
    match (x_val, y_val) {
        (Value::Integer(x), Value::Integer(y)) => {
            if y == 0 {
                return Err(Error::Runtime("zero division in fmod".to_string()));
            }
            vm.push_value(Value::Integer(x % y));
            Ok(1)
        }
        _ => {
            let x = match x_val {
                Value::Integer(i) => i as f64,
                Value::Number(f) => f,
                _ => return Err(Error::Runtime("bad argument #1 to 'fmod' (number expected)".to_string())),
            };
            let y = match y_val {
                Value::Integer(i) => i as f64,
                Value::Number(f) => f,
                _ => return Err(Error::Runtime("bad argument #2 to 'fmod' (number expected)".to_string())),
            };
            vm.push_value(Value::Number(x % y));
            Ok(1)
        }
    }
}

pub fn std_math_sin(vm: &mut VM) -> Result<i32, Error> {
    vm.push_value(Value::Number(number_arg(vm, 0, "sin")?.sin()));
    Ok(1)
}

pub fn std_math_cos(vm: &mut VM) -> Result<i32, Error> {
    vm.push_value(Value::Number(number_arg(vm, 0, "cos")?.cos()));
    Ok(1)
}

pub fn std_math_sqrt(vm: &mut VM) -> Result<i32, Error> {
    vm.push_value(Value::Number(number_arg(vm, 0, "sqrt")?.sqrt()));
    Ok(1)
}

pub fn std_math_log(vm: &mut VM) -> Result<i32, Error> {
    let x = number_arg(vm, 0, "log")?;
    match vm.get_arg(1) {
        Value::Nil => vm.push_value(Value::Number(x.ln())),
        _ => vm.push_value(Value::Number(x.log(number_arg(vm, 1, "log")?))),
    }
    Ok(1)
}

pub fn std_math_randomseed(vm: &mut VM) -> Result<i32, Error> {
    vm.push_value(Value::Integer(0));
    vm.push_value(Value::Integer(0));
    Ok(2)
}

pub fn std_math_random(vm: &mut VM) -> Result<i32, Error> {
    match vm.get_arg_count() {
        0 => vm.push_value(Value::Number(0.5)),
        1 => {
            let upper = vm.get_arg(0).to_integer().ok_or_else(|| {
                Error::Runtime("bad argument #1 to 'random' (integer expected)".to_string())
            })?;
            vm.push_value(Value::Integer(upper.clamp(1, upper)));
        }
        _ => {
            let lower = vm.get_arg(0).to_integer().ok_or_else(|| {
                Error::Runtime("bad argument #1 to 'random' (integer expected)".to_string())
            })?;
            vm.push_value(Value::Integer(lower));
        }
    }
    Ok(1)
}

pub fn std_string_packsize(vm: &mut VM) -> Result<i32, Error> {
    let fmt_val = vm.get_arg(0);
    let fmt_bytes = match fmt_val {
        Value::String(s_id) => &vm.gc.get_string(s_id).data,
        _ => return Err(Error::Runtime("bad argument #1 to 'packsize' (string expected)".to_string())),
    };

    let mut size: usize = 0;
    let mut max_align: usize = 1;
    let mut i = 0;
    while i < fmt_bytes.len() {
        let opt = fmt_bytes[i];
        i += 1;

        match opt {
            b'<' | b'>' | b'=' => {}
            b'!' => {
                if i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                    let mut val = 0;
                    while i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                        val = val * 10 + (fmt_bytes[i] - b'0') as usize;
                        i += 1;
                    }
                    max_align = val;
                } else {
                    max_align = 8;
                }
            }
            b' ' => {}
            _ => {
                let (item_size, item_align) = match opt {
                    b'b' | b'B' => (1, 1),
                    b'h' | b'H' => (2, 2),
                    b'i' | b'I' => {
                        let mut sz = 4;
                        if i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                            sz = 0;
                            while i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                                sz = sz * 10 + (fmt_bytes[i] - b'0') as usize;
                                i += 1;
                            }
                        }
                        (sz, sz)
                    }
                    b'l' | b'L' => (8, 8),
                    b'j' | b'J' => (8, 8),
                    b'T' => (8, 8),
                    b'f' => (4, 4),
                    b'd' => (8, 8),
                    b'n' => (8, 8),
                    b'x' => (1, 1),
                    b'c' => {
                        let mut sz = 0;
                        while i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                            sz = sz * 10 + (fmt_bytes[i] - b'0') as usize;
                            i += 1;
                        }
                        (sz, 1)
                    }
                    b'X' => {
                        if i >= fmt_bytes.len() {
                            return Err(Error::Runtime("invalid format option 'X'".to_string()));
                        }
                        let next_opt = fmt_bytes[i];
                        i += 1;
                        let (_, next_align) = match next_opt {
                            b'b' | b'B' => (0, 1),
                            b'h' | b'H' => (0, 2),
                            b'i' | b'I' => {
                                let mut sz = 4;
                                if i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                                    sz = 0;
                                    while i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                                        sz = sz * 10 + (fmt_bytes[i] - b'0') as usize;
                                        i += 1;
                                    }
                                }
                                (0, sz)
                            }
                            b'l' | b'L' => (0, 8),
                            b'j' | b'J' => (0, 8),
                            b'T' => (0, 8),
                            b'f' => (0, 4),
                            b'd' => (0, 8),
                            b'n' => (0, 8),
                            _ => return Err(Error::Runtime("invalid format option after 'X'".to_string())),
                        };
                        let align_val = std::cmp::min(next_align, max_align);
                        if align_val > 0 {
                            size = size.div_ceil(align_val) * align_val;
                        }
                        continue;
                    }
                    b's' | b'z' => {
                        return Err(Error::Runtime("variable-length format".to_string()));
                    }
                    _ => return Err(Error::Runtime(format!("invalid format option '{}'", opt as char))),
                };

                let align_val = std::cmp::min(item_align, max_align);
                if align_val > 0 {
                    size = size.div_ceil(align_val) * align_val;
                }
                size += item_size;
            }
        }
    }

    vm.push_value(Value::Integer(size as i64));
    Ok(1)
}

pub fn std_string_format(vm: &mut VM) -> Result<i32, Error> {
    let fmt_val = vm.get_arg(0);
    let fmt_bytes = match fmt_val {
        Value::String(s_id) => &vm.gc.get_string(s_id).data,
        _ => return Err(Error::Runtime("bad argument #1 to 'format' (string expected)".to_string())),
    };

    let mut result = Vec::new();
    let mut arg_idx = 1;
    let mut i = 0;
    while i < fmt_bytes.len() {
        if fmt_bytes[i] == b'%' {
            if i + 1 < fmt_bytes.len() && fmt_bytes[i + 1] == b'%' {
                result.push(b'%');
                i += 2;
                continue;
            }
            let start_spec = i;
            i += 1;
            while i < fmt_bytes.len() {
                let c = fmt_bytes[i];
                if c.is_ascii_digit() || c == b'.' || c == b'-' || c == b'+' || c == b' ' {
                    i += 1;
                } else {
                    break;
                }
            }
            if i >= fmt_bytes.len() {
                return Err(Error::Runtime("invalid format string".to_string()));
            }
            let spec_char = fmt_bytes[i];
            let raw_spec = &fmt_bytes[start_spec..=i];
            i += 1;

            let arg = vm.get_arg(arg_idx);
            arg_idx += 1;

            match spec_char {
                b'd' | b'i' => {
                    let val = match arg {
                        Value::Integer(n) => n,
                        Value::Number(f) => f as i64,
                        Value::String(s_id) => {
                            let s = String::from_utf8_lossy(&vm.gc.get_string(s_id).data).into_owned();
                            s.parse::<i64>().unwrap_or(0)
                        }
                        _ => 0,
                    };
                    let s = format!("{}", val);
                    result.extend_from_slice(s.as_bytes());
                }
                b'x' | b'X' => {
                    let val = match arg {
                        Value::Integer(n) => n,
                        Value::Number(f) => f as i64,
                        _ => 0,
                    };
                    let s = if spec_char == b'x' {
                        format!("{:x}", val)
                    } else {
                        format!("{:X}", val)
                    };
                    result.extend_from_slice(s.as_bytes());
                }
                b's' => {
                    let s = vm.value_to_string(arg);
                    result.extend_from_slice(s.as_bytes());
                }
                b'q' => {
                    let s = vm.value_to_string(arg);
                    let mut quoted = Vec::new();
                    quoted.push(b'"');
                    for &b in s.as_bytes() {
                        match b {
                            b'\n' => quoted.extend_from_slice(b"\\n"),
                            b'\r' => quoted.extend_from_slice(b"\\r"),
                            b'\\' => quoted.extend_from_slice(b"\\\\"),
                            b'"' => quoted.extend_from_slice(b"\\\""),
                            _ if b.is_ascii_control() || b > 127 => {
                                quoted.extend_from_slice(format!("\\{}", b).as_bytes());
                            }
                            _ => quoted.push(b),
                        }
                    }
                    quoted.push(b'"');
                    result.extend(quoted);
                }
                b'c' => {
                    let val = match arg {
                        Value::Integer(n) => n as u8,
                        Value::Number(f) => f as u8,
                        _ => 0,
                    };
                    result.push(val);
                }
                b'f' => {
                    let val = match arg {
                        Value::Number(f) => f,
                        Value::Integer(n) => n as f64,
                        _ => 0.0,
                    };
                    let s = format!("{}", val);
                    result.extend_from_slice(s.as_bytes());
                }
                _ => {
                    result.extend_from_slice(raw_spec);
                }
            }
        } else {
            result.push(fmt_bytes[i]);
            i += 1;
        }
    }

    let new_s_id = vm.gc.alloc_string(result);
    vm.push_value(Value::String(new_s_id));
    Ok(1)
}

pub fn std_string_find(vm: &mut VM) -> Result<i32, Error> {
    let s_val = vm.get_arg(0);
    let pat_val = vm.get_arg(1);
    let init_val = vm.get_arg(2);
    
    let s_bytes = match s_val {
        Value::String(s_id) => &vm.gc.get_string(s_id).data,
        _ => return Err(Error::Runtime("bad argument #1 to 'find' (string expected)".to_string())),
    };
    let pat_bytes = match pat_val {
        Value::String(s_id) => &vm.gc.get_string(s_id).data,
        _ => return Err(Error::Runtime("bad argument #2 to 'find' (string expected)".to_string())),
    };

    let s_len = s_bytes.len() as i64;
    let init = match init_val {
        Value::Integer(n) => {
            if n < 0 {
                std::cmp::max(0, s_len + n)
            } else if n > 0 {
                std::cmp::min(s_len, n - 1)
            } else {
                0
            }
        }
        _ => 0,
    };

    if init as usize > s_bytes.len() {
        vm.push_value(Value::Nil);
        return Ok(1);
    }

    let search_slice = &s_bytes[init as usize..];
    if let Some(pos) = search_slice.windows(pat_bytes.len()).position(|w| w == pat_bytes) {
        let start_idx = init + pos as i64 + 1;
        let end_idx = start_idx + pat_bytes.len() as i64 - 1;
        vm.push_value(Value::Integer(start_idx));
        vm.push_value(Value::Integer(end_idx));
        Ok(2)
    } else {
        vm.push_value(Value::Nil);
        Ok(1)
    }
}

pub fn std_string_sub(vm: &mut VM) -> Result<i32, Error> {
    let s_val = vm.get_arg(0);
    let i_val = vm.get_arg(1);
    let j_val = vm.get_arg(2);

    let bytes = match s_val {
        Value::String(s_id) => &vm.gc.get_string(s_id).data,
        _ => return Err(Error::Runtime("bad argument #1 to 'sub' (string expected)".to_string())),
    };

    let len = bytes.len() as i64;

    let i = match i_val {
        Value::Integer(n) => n,
        _ => return Err(Error::Runtime("bad argument #2 to 'sub' (integer expected)".to_string())),
    };

    let j = match j_val {
        Value::Integer(n) => n,
        Value::Nil => -1,
        _ => return Err(Error::Runtime("bad argument #3 to 'sub' (integer expected)".to_string())),
    };

    let start = if i < 0 {
        std::cmp::max(0, len + i)
    } else if i > 0 {
        std::cmp::min(len, i - 1)
    } else {
        0
    };

    let end = if j < 0 {
        std::cmp::max(0, len + j + 1)
    } else {
        std::cmp::min(len, j)
    };

    let sub_bytes = if start < end {
        bytes[start as usize..end as usize].to_vec()
    } else {
        Vec::new()
    };


    let new_s_id = vm.gc.alloc_string(sub_bytes);
    vm.push_value(Value::String(new_s_id));
    Ok(1)
}

// ── string.len ────────────────────────────────────────────────────────────────
pub fn std_string_len(vm: &mut VM) -> Result<i32, Error> {
    let s_val = vm.get_arg(0);
    let len = match s_val {
        Value::String(s_id) => vm.gc.get_string(s_id).data.len() as i64,
        _ => return Err(Error::Runtime("bad argument #1 to 'len' (string expected)".to_string())),
    };
    vm.push_value(Value::Integer(len));
    Ok(1)
}

// ── string.lower ─────────────────────────────────────────────────────────────
pub fn std_string_lower(vm: &mut VM) -> Result<i32, Error> {
    let s_val = vm.get_arg(0);
    let bytes = match s_val {
        Value::String(s_id) => vm.gc.get_string(s_id).data.clone(),
        _ => return Err(Error::Runtime("bad argument #1 to 'lower' (string expected)".to_string())),
    };
    let lower: Vec<u8> = bytes.iter().map(|b| b.to_ascii_lowercase()).collect();
    let id = vm.gc.alloc_string(lower);
    vm.push_value(Value::String(id));
    Ok(1)
}

// ── string.upper ─────────────────────────────────────────────────────────────
pub fn std_string_upper(vm: &mut VM) -> Result<i32, Error> {
    let s_val = vm.get_arg(0);
    let bytes = match s_val {
        Value::String(s_id) => vm.gc.get_string(s_id).data.clone(),
        _ => return Err(Error::Runtime("bad argument #1 to 'upper' (string expected)".to_string())),
    };
    let upper: Vec<u8> = bytes.iter().map(|b| b.to_ascii_uppercase()).collect();
    let id = vm.gc.alloc_string(upper);
    vm.push_value(Value::String(id));
    Ok(1)
}

// ── string.rep ───────────────────────────────────────────────────────────────
pub fn std_string_rep(vm: &mut VM) -> Result<i32, Error> {
    let s_val = vm.get_arg(0);
    let n_val = vm.get_arg(1);
    let sep_val = vm.get_arg(2);

    let bytes = match s_val {
        Value::String(s_id) => vm.gc.get_string(s_id).data.clone(),
        _ => return Err(Error::Runtime("bad argument #1 to 'rep' (string expected)".to_string())),
    };
    let n = match n_val {
        Value::Integer(n) => n,
        _ => return Err(Error::Runtime("bad argument #2 to 'rep' (integer expected)".to_string())),
    };
    let sep = match sep_val {
        Value::String(s_id) => vm.gc.get_string(s_id).data.clone(),
        Value::Nil => Vec::new(),
        _ => return Err(Error::Runtime("bad argument #3 to 'rep' (string expected)".to_string())),
    };

    if n <= 0 {
        let id = vm.gc.alloc_string(Vec::new());
        vm.push_value(Value::String(id));
        return Ok(1);
    }
    let n = n as usize;
    let mut result = Vec::with_capacity(bytes.len() * n + sep.len() * n.saturating_sub(1));
    for i in 0..n {
        if i > 0 {
            result.extend_from_slice(&sep);
        }
        result.extend_from_slice(&bytes);
    }
    let id = vm.gc.alloc_string(result);
    vm.push_value(Value::String(id));
    Ok(1)
}

// ── string.byte ─────────────────────────────────────────────────────────────
pub fn std_string_byte(vm: &mut VM) -> Result<i32, Error> {
    let s_val = vm.get_arg(0);
    let i_val = vm.get_arg(1);
    let j_val = vm.get_arg(2);

    let bytes = match s_val {
        Value::String(s_id) => vm.gc.get_string(s_id).data.clone(),
        _ => return Err(Error::Runtime("bad argument #1 to 'byte' (string expected)".to_string())),
    };
    let len = bytes.len() as i64;

    let i = match i_val {
        Value::Integer(n) => n,
        Value::Nil => 1,
        _ => return Err(Error::Runtime("bad argument #2 to 'byte' (integer expected)".to_string())),
    };
    let j = match j_val {
        Value::Integer(n) => n,
        Value::Nil => i,
        _ => return Err(Error::Runtime("bad argument #3 to 'byte' (integer expected)".to_string())),
    };

    let start = if i < 0 { (len + i).max(0) } else { (i - 1).max(0) };
    let end   = if j < 0 { (len + j + 1).max(0) } else { j.min(len) };

    let mut count = 0i32;
    for idx in start..end {
        vm.push_value(Value::Integer(bytes[idx as usize] as i64));
        count += 1;
    }
    Ok(count)
}

// ── string.char ─────────────────────────────────────────────────────────────
pub fn std_string_char(vm: &mut VM) -> Result<i32, Error> {
    let nargs = vm.get_arg_count();
    let mut result = Vec::with_capacity(nargs);
    for i in 0..nargs {
        match vm.get_arg(i) {
            Value::Integer(n) => {
                if !(0..=255).contains(&n) {
                    return Err(Error::Runtime(format!("bad argument #{} to 'char' (value out of range)", i + 1)));
                }
                result.push(n as u8);
            }
            _ => return Err(Error::Runtime(format!("bad argument #{} to 'char' (integer expected)", i + 1))),
        }
    }
    let id = vm.gc.alloc_string(result);
    vm.push_value(Value::String(id));
    Ok(1)
}

// ── io.write ─────────────────────────────────────────────────────────────────
pub fn std_io_write(vm: &mut VM) -> Result<i32, Error> {
    use std::io::Write;
    let nargs = vm.get_arg_count();
    for i in 0..nargs {
        match vm.get_arg(i) {
            Value::String(s_id) => {
                let data = vm.gc.get_string(s_id).data.clone();
                std::io::stdout()
                    .write_all(&data)
                    .map_err(|e| Error::Runtime(e.to_string()))?;
            }
            Value::Integer(n) => {
                print!("{}", n);
            }
            Value::Number(f) => {
                print!("{}", f);
            }
            v => {
                let tname = match v {
                    Value::Nil => "nil",
                    Value::Boolean(_) => "boolean",
                    Value::Integer(_) | Value::Number(_) => "number",
                    Value::String(_) => "string",
                    Value::Table(_) => "table",
                    Value::Function(_) | Value::LightFunction(_) => "function",
                    Value::Thread(_) => "thread",
                    Value::Userdata(_) | Value::LightUserdata(_) => "userdata",
                };
                return Err(Error::Runtime(format!(
                    "bad argument to 'write': cannot write {}",
                    tname
                )))
            }
        }
    }
    std::io::stdout().flush().ok();
    Ok(0)
}

// ── io.read ──────────────────────────────────────────────────────────────────
pub fn std_io_read(vm: &mut VM) -> Result<i32, Error> {
    use std::io::BufRead;
    let fmt = vm.get_arg(0);
    let is_line = match &fmt {
        Value::Nil => true,
        Value::String(s_id) => {
            let s = String::from_utf8_lossy(&vm.gc.get_string(*s_id).data).to_string();
            matches!(s.as_str(), "*l" | "l" | "*L" | "L")
        }
        _ => true,
    };
    if is_line {
        let mut line = String::new();
        let read = std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| Error::Runtime(e.to_string()))?;
        if read == 0 {
            vm.push_value(Value::Nil);
        } else {
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            let id = vm.gc.alloc_string(line.into_bytes());
            vm.push_value(Value::String(id));
        }
        Ok(1)
    } else {
        vm.push_value(Value::Nil);
        Ok(1)
    }
}

// ── io.open ──────────────────────────────────────────────────────────────────
pub fn std_io_open(vm: &mut VM) -> Result<i32, Error> {
    let _ = vm.get_arg(0);
    vm.push_value(Value::Nil);
    let msg = vm.gc.alloc_string(b"io.open not fully implemented".to_vec());
    vm.push_value(Value::String(msg));
    Ok(2)
}

// ── os.clock ─────────────────────────────────────────────────────────────────
pub fn std_os_clock(vm: &mut VM) -> Result<i32, Error> {
    vm.push_value(Value::Number(0.0));
    Ok(1)
}

// ── os.time ──────────────────────────────────────────────────────────────────
pub fn std_os_time(vm: &mut VM) -> Result<i32, Error> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    vm.push_value(Value::Integer(secs));
    Ok(1)
}

// ── os.difftime ──────────────────────────────────────────────────────────────
pub fn std_os_difftime(vm: &mut VM) -> Result<i32, Error> {
    let t2 = match vm.get_arg(0) {
        Value::Integer(n) => n as f64,
        Value::Number(f) => f,
        _ => return Err(Error::Runtime("bad argument #1 to 'difftime'".to_string())),
    };
    let t1 = match vm.get_arg(1) {
        Value::Integer(n) => n as f64,
        Value::Number(f) => f,
        _ => return Err(Error::Runtime("bad argument #2 to 'difftime'".to_string())),
    };
    vm.push_value(Value::Number(t2 - t1));
    Ok(1)
}

// ── os.setlocale ─────────────────────────────────────────────────────────────
pub fn std_os_setlocale(vm: &mut VM) -> Result<i32, Error> {
    let locale_val = vm.get_arg(0);
    match locale_val {
        Value::Nil => {
            let id = vm.gc.alloc_string(b"C".to_vec());
            vm.push_value(Value::String(id));
        }
        Value::String(s_id) => {
            let data = vm.gc.get_string(s_id).data.clone();
            let id = vm.gc.alloc_string(data);
            vm.push_value(Value::String(id));
        }
        _ => vm.push_value(Value::Nil),
    }
    Ok(1)
}

// ── debug.getregistry ────────────────────────────────────────────────────────
pub fn std_debug_getregistry(vm: &mut VM) -> Result<i32, Error> {
    vm.push_value(Value::Table(vm.registry));
    Ok(1)
}

// ── debug.getmetatable ───────────────────────────────────────────────────────
pub fn std_debug_getmetatable(vm: &mut VM) -> Result<i32, Error> {
    let val = vm.get_arg(0);
    let mt = match val {
        Value::Table(tid) => vm.gc.get_table(tid).metatable,
        _ => None,
    };
    match mt {
        Some(mt_id) => vm.push_value(Value::Table(mt_id)),
        None => vm.push_value(Value::Nil),
    }
    Ok(1)
}

// ── debug.setmetatable ───────────────────────────────────────────────────────
pub fn std_debug_setmetatable(vm: &mut VM) -> Result<i32, Error> {
    let obj = vm.get_arg(0);
    let mt_val = vm.get_arg(1);
    let new_mt = match mt_val {
        Value::Table(tid) => Some(tid),
        Value::Nil => None,
        _ => return Err(Error::Runtime(
            "bad argument #2 to 'debug.setmetatable' (table or nil expected)".to_string(),
        )),
    };
    if let Value::Table(tid) = obj {
        vm.gc.get_table_mut(tid).metatable = new_mt;
    }
    vm.push_value(obj);
    Ok(1)
}
