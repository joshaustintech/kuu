use kuu::error::KResult;
use kuu::value::{
    ClosureHandle, LuaKey, NativeFunction, StringHandle, TableHandle, ThreadHandle, UserdataHandle,
    Value,
};

fn sample_native(_: &[Value]) -> KResult<Vec<Value>> {
    Ok(vec![Value::nil()])
}

fn other_native(_: &[Value]) -> KResult<Vec<Value>> {
    Ok(Vec::new())
}

#[test]
fn integer_and_number_equality_matches_lua_semantics() -> Result<(), String> {
    let integer = Value::integer(42);
    let number = Value::number(42.0);

    assert_eq!(integer, number);
    assert_eq!(integer.hash_key(), number.hash_key());
    assert_eq!(integer.hash_key(), Some(LuaKey::Integer(42)));
    Ok(())
}

#[test]
fn non_integral_numbers_keep_their_own_hash_key() -> Result<(), String> {
    let value = Value::number(3.5);

    assert_eq!(value.hash_key(), Some(LuaKey::Number(3.5f64.to_bits())));
    Ok(())
}

#[test]
fn signed_zero_is_canonicalized_for_hashing() -> Result<(), String> {
    let positive = Value::number(0.0);
    let negative = Value::number(-0.0);

    assert_eq!(positive, negative);
    assert_eq!(positive.hash_key(), Some(LuaKey::Integer(0)));
    assert_eq!(negative.hash_key(), Some(LuaKey::Integer(0)));
    Ok(())
}

#[test]
fn non_finite_numbers_are_not_hashable() -> Result<(), String> {
    assert!(Value::number(f64::NAN).hash_key().is_none());
    assert!(Value::number(f64::INFINITY).hash_key().is_none());
    Ok(())
}

#[test]
fn handle_values_compare_and_hash_by_identity() -> Result<(), String> {
    let string = Value::string(StringHandle::new(11));
    let table = Value::table(TableHandle::new(12));
    let closure = Value::closure(ClosureHandle::new(13));
    let thread = Value::thread(ThreadHandle::new(14));
    let userdata = Value::userdata(UserdataHandle::new(15));

    assert_eq!(string, Value::string(StringHandle::new(11)));
    assert_eq!(table, Value::table(TableHandle::new(12)));
    assert_eq!(closure, Value::closure(ClosureHandle::new(13)));
    assert_eq!(thread, Value::thread(ThreadHandle::new(14)));
    assert_eq!(userdata, Value::userdata(UserdataHandle::new(15)));

    assert_eq!(
        string.hash_key(),
        Some(LuaKey::String(StringHandle::new(11)))
    );
    assert_eq!(table.hash_key(), Some(LuaKey::Table(TableHandle::new(12))));
    assert_eq!(
        closure.hash_key(),
        Some(LuaKey::Closure(ClosureHandle::new(13)))
    );
    assert_eq!(
        thread.hash_key(),
        Some(LuaKey::Thread(ThreadHandle::new(14)))
    );
    assert_eq!(
        userdata.hash_key(),
        Some(LuaKey::Userdata(UserdataHandle::new(15)))
    );
    Ok(())
}

#[test]
fn native_function_identity_participates_in_equality() -> Result<(), String> {
    let left = Value::native(sample_native as NativeFunction);
    let same = Value::native(sample_native as NativeFunction);
    let other = Value::native(other_native as NativeFunction);

    assert_eq!(left, same);
    assert_ne!(left, other);
    assert!(left.hash_key().is_some());
    Ok(())
}
