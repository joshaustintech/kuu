use crate::error::KResult;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};

static STRING_REGISTRY: OnceLock<Mutex<BTreeMap<u64, Vec<u8>>>> = OnceLock::new();

fn string_registry() -> &'static Mutex<BTreeMap<u64, Vec<u8>>> {
    STRING_REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(crate) fn register_string(handle: StringHandle, bytes: &[u8]) {
    if let Ok(mut registry) = string_registry().lock() {
        registry.insert(handle.raw(), bytes.to_vec());
    }
}

pub(crate) fn lookup_string(handle: StringHandle) -> Option<Vec<u8>> {
    let guard = string_registry().lock().ok()?;
    guard.get(&handle.raw()).cloned()
}

macro_rules! handle_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if let Some(bytes) = lookup_string(StringHandle::new(self.0)) {
                    write!(f, "{}", String::from_utf8_lossy(&bytes))
                } else {
                    write!(f, "{}({})", stringify!($name), self.0)
                }
            }
        }
    };
}

handle_type!(StringHandle);
handle_type!(TableHandle);
handle_type!(ClosureHandle);
handle_type!(ThreadHandle);
handle_type!(UserdataHandle);

pub type NativeFunction = fn(&[Value]) -> KResult<Vec<Value>>;

#[derive(Debug, Clone, Copy)]
pub enum Value {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(StringHandle),
    Table(TableHandle),
    Closure(ClosureHandle),
    NativeFunction(NativeFunction),
    Thread(ThreadHandle),
    Userdata(UserdataHandle),
    LightUserdata(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LuaKey {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(u64),
    String(StringHandle),
    Table(TableHandle),
    Closure(ClosureHandle),
    NativeFunction(usize),
    Thread(ThreadHandle),
    Userdata(UserdataHandle),
    LightUserdata(usize),
}

impl Value {
    pub const fn nil() -> Self {
        Self::Nil
    }

    pub const fn boolean(value: bool) -> Self {
        Self::Boolean(value)
    }

    pub const fn integer(value: i64) -> Self {
        Self::Integer(value)
    }

    pub const fn number(value: f64) -> Self {
        Self::Number(value)
    }

    pub const fn string(handle: StringHandle) -> Self {
        Self::String(handle)
    }

    pub const fn table(handle: TableHandle) -> Self {
        Self::Table(handle)
    }

    pub const fn closure(handle: ClosureHandle) -> Self {
        Self::Closure(handle)
    }

    pub const fn native(function: NativeFunction) -> Self {
        Self::NativeFunction(function)
    }

    pub const fn thread(handle: ThreadHandle) -> Self {
        Self::Thread(handle)
    }

    pub const fn userdata(handle: UserdataHandle) -> Self {
        Self::Userdata(handle)
    }

    pub const fn light_userdata(value: usize) -> Self {
        Self::LightUserdata(value)
    }

    pub fn lua_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Nil, Self::Nil) => true,
            (Self::Boolean(left), Self::Boolean(right)) => left == right,
            (Self::Integer(left), Self::Integer(right)) => left == right,
            (Self::Number(left), Self::Number(right)) => left == right,
            (Self::Integer(left), Self::Number(right)) => integer_number_eq(*left, *right),
            (Self::Number(left), Self::Integer(right)) => integer_number_eq(*right, *left),
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Table(left), Self::Table(right)) => left == right,
            (Self::Closure(left), Self::Closure(right)) => left == right,
            (Self::NativeFunction(left), Self::NativeFunction(right)) => {
                std::ptr::fn_addr_eq(*left, *right)
            }
            (Self::Thread(left), Self::Thread(right)) => left == right,
            (Self::Userdata(left), Self::Userdata(right)) => left == right,
            (Self::LightUserdata(left), Self::LightUserdata(right)) => left == right,
            _ => false,
        }
    }

    pub fn hash_key(&self) -> Option<LuaKey> {
        match self {
            Self::Nil => None,
            Self::Boolean(value) => Some(LuaKey::Boolean(*value)),
            Self::Integer(value) => Some(LuaKey::Integer(*value)),
            Self::Number(value) => number_key(*value),
            Self::String(handle) => Some(LuaKey::String(*handle)),
            Self::Table(handle) => Some(LuaKey::Table(*handle)),
            Self::Closure(handle) => Some(LuaKey::Closure(*handle)),
            Self::NativeFunction(function) => Some(LuaKey::NativeFunction(*function as usize)),
            Self::Thread(handle) => Some(LuaKey::Thread(*handle)),
            Self::Userdata(handle) => Some(LuaKey::Userdata(*handle)),
            Self::LightUserdata(value) => Some(LuaKey::LightUserdata(*value)),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.lua_eq(other)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nil => f.write_str("nil"),
            Self::Boolean(value) => write!(f, "{value}"),
            Self::Integer(value) => write!(f, "{value}"),
            Self::Number(value) => write!(f, "{value}"),
            Self::String(handle) => write!(f, "{handle}"),
            Self::Table(handle) => write!(f, "{handle}"),
            Self::Closure(handle) => write!(f, "{handle}"),
            Self::NativeFunction(function) => {
                write!(f, "NativeFunction({:p})", *function as *const ())
            }
            Self::Thread(handle) => write!(f, "{handle}"),
            Self::Userdata(handle) => write!(f, "{handle}"),
            Self::LightUserdata(value) => write!(f, "LightUserdata({value})"),
        }
    }
}

fn integer_number_eq(integer: i64, number: f64) -> bool {
    if !number.is_finite() || number.fract() != 0.0 {
        return false;
    }

    if number < i64::MIN as f64 || number > i64::MAX as f64 {
        return false;
    }

    (number as i64) == integer
}

fn number_key(value: f64) -> Option<LuaKey> {
    if !value.is_finite() {
        return None;
    }

    if value == 0.0 {
        return Some(LuaKey::Integer(0));
    }

    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        let integer = value as i64;
        if (integer as f64) == value {
            return Some(LuaKey::Integer(integer));
        }
    }

    let bits = if value == 0.0 {
        0.0f64.to_bits()
    } else {
        value.to_bits()
    };
    Some(LuaKey::Number(bits))
}
