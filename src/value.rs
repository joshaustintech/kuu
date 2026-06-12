use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StringId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserdataId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UpvalueId(pub usize);

// Type alias for native Rust functions that can be called from Lua
pub type RustFunction = fn(&mut crate::vm::VM) -> Result<i32, crate::vm::Error>;

#[derive(Debug, Clone, Copy)]
pub enum Value {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(StringId),
    Table(TableId),
    Function(FunctionId),
    LightFunction(RustFunction),
    Thread(ThreadId),
    Userdata(UserdataId),
    LightUserdata(usize),
}

impl Value {
    pub fn to_integer(&self) -> Option<i64> {
        match *self {
            Value::Integer(i) => Some(i),
            Value::Number(n) => {
                if n.is_finite() && n == (n as i64) as f64 {
                    Some(n as i64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn to_number(&self) -> Option<f64> {
        match *self {
            Value::Integer(i) => Some(i as f64),
            Value::Number(n) => Some(n),
            _ => None,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Integer(a), Value::Integer(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Integer(a), Value::Number(b)) => *a as f64 == *b,
            (Value::Number(a), Value::Integer(b)) => *a == *b as f64,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Table(a), Value::Table(b)) => a == b,
            (Value::Function(a), Value::Function(b)) => a == b,
            (Value::LightFunction(a), Value::LightFunction(b)) => *a as usize == *b as usize,
            (Value::Thread(a), Value::Thread(b)) => a == b,
            (Value::Userdata(a), Value::Userdata(b)) => a == b,
            (Value::LightUserdata(a), Value::LightUserdata(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Value::Nil => {
                0u8.hash(state);
            }
            Value::Boolean(b) => {
                1u8.hash(state);
                b.hash(state);
            }
            Value::Integer(i) => {
                2u8.hash(state);
                i.hash(state);
            }
            Value::Number(f) => {
                2u8.hash(state);
                if *f == (*f as i64) as f64 {
                    (*f as i64).hash(state);
                } else {
                    f.to_bits().hash(state);
                }
            }
            Value::String(s) => {
                3u8.hash(state);
                s.0.hash(state);
            }
            Value::Table(t) => {
                4u8.hash(state);
                t.0.hash(state);
            }
            Value::Function(f) => {
                5u8.hash(state);
                f.0.hash(state);
            }
            Value::LightFunction(f) => {
                6u8.hash(state);
                (*f as usize).hash(state);
            }
            Value::Thread(t) => {
                7u8.hash(state);
                t.0.hash(state);
            }
            Value::Userdata(u) => {
                8u8.hash(state);
                u.0.hash(state);
            }
            Value::LightUserdata(u) => {
                9u8.hash(state);
                u.hash(state);
            }
        }
    }
}

// Garbage Collected Objects

pub struct LuaString {
    pub data: Vec<u8>,
}

pub struct LuaTable {
    pub array: Vec<Value>,
    pub hash: std::collections::HashMap<Value, Value>,
    pub metatable: Option<TableId>,
}

pub struct LuaClosure {
    pub proto: crate::compiler::Proto,
    pub upvalues: Vec<UpvalueId>,
}

pub struct RustClosure {
    pub func: RustFunction,
    pub upvalues: Vec<Value>,
}

pub enum LuaFunction {
    Lua(LuaClosure),
    Rust(RustClosure),
}

pub struct LuaThread {
    pub stack: Vec<Value>,
}

pub struct LuaUserdata {
    pub data: Vec<u8>,
    pub metatable: Option<TableId>,
}

pub struct Upvalue {
    pub val: UpvalueState,
}

pub enum UpvalueState {
    Open { thread_id: usize, stack_idx: usize },
    Closed(Value),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};

    fn calculate_hash<T: Hash>(t: &T) -> u64 {
        let mut s = std::collections::hash_map::DefaultHasher::new();
        t.hash(&mut s);
        s.finish()
    }

    #[test]
    fn test_float_int_equality() {
        let int_val = Value::Integer(42);
        let float_val = Value::Number(42.0);
        let float_val_non_int = Value::Number(42.5);

        assert_eq!(int_val, float_val);
        assert_eq!(float_val, int_val);
        assert_ne!(int_val, float_val_non_int);
        assert_ne!(float_val_non_int, int_val);
    }

    #[test]
    fn test_float_int_hashing() {
        let int_val = Value::Integer(42);
        let float_val = Value::Number(42.0);

        assert_eq!(calculate_hash(&int_val), calculate_hash(&float_val));
    }

    #[test]
    fn test_hashmap_lookup() {
        let mut map = HashMap::new();
        map.insert(Value::Integer(42), "integer 42");

        assert_eq!(map.get(&Value::Number(42.0)), Some(&"integer 42"));

        map.insert(Value::Number(100.0), "float 100");
        assert_eq!(map.get(&Value::Integer(100)), Some(&"float 100"));
    }
}

