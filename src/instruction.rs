#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    // R(A) := R(B)
    Move { dst: u8, src: u8 },
    // R(A) := K(Bx)  (load constant)
    LoadK { dst: u8, const_idx: u16 },
    // R(A) := nil
    LoadNil { dst: u8, count: u8 },
    // R(A) := boolean
    LoadBool { dst: u8, val: bool, skip_next: bool },
    
    // Upvalue operations
    GetUpval { dst: u8, upval_idx: u8 },
    SetUpval { upval_idx: u8, src: u8 },
    
    // Global operations (via _ENV or global table)
    GetTabUp { dst: u8, upval_idx: u8, key_const: u16 },
    SetTabUp { upval_idx: u8, key_const: u16, src: u8 },
    
    // Table operations
    GetTable { dst: u8, tbl: u8, key: u8 },
    SetTable { tbl: u8, key: u8, val: u8 },
    NewTable { dst: u8, array_size: u16, hash_size: u16 },
    SetList { tbl: u8, count: u8, start_idx: u32 }, // sets a range of array elements from registers starting at tbl+1
    
    // Arithmetic & Bitwise
    Add { dst: u8, lhs: u8, rhs: u8 },
    Sub { dst: u8, lhs: u8, rhs: u8 },
    Mul { dst: u8, lhs: u8, rhs: u8 },
    Div { dst: u8, lhs: u8, rhs: u8 },
    Mod { dst: u8, lhs: u8, rhs: u8 },
    Pow { dst: u8, lhs: u8, rhs: u8 },
    IDiv { dst: u8, lhs: u8, rhs: u8 },
    BAnd { dst: u8, lhs: u8, rhs: u8 },
    BOr { dst: u8, lhs: u8, rhs: u8 },
    BXor { dst: u8, lhs: u8, rhs: u8 },
    Shl { dst: u8, lhs: u8, rhs: u8 },
    Shr { dst: u8, lhs: u8, rhs: u8 },
    UNeg { dst: u8, src: u8 },
    UNot { dst: u8, src: u8 },
    ULen { dst: u8, src: u8 },
    UBNot { dst: u8, src: u8 },
    Concat { dst: u8, start: u8, count: u8 },
    
    // Jump & Control Flow
    Jmp { offset: i32 },
    Eq { lhs: u8, rhs: u8, eq: bool },
    Lt { lhs: u8, rhs: u8, eq: bool },
    Le { lhs: u8, rhs: u8, eq: bool },
    Test { reg: u8, cond: bool },
    
    // Calls & Returns
    Call { func: u8, num_args: u8, num_results: u8 },
    Return { start: u8, count: u8 },
    
    // Loops
    ForPrep { reg: u8, offset: i32 },
    ForLoop { reg: u8, offset: i32 },
    
    // Closures
    Closure { dst: u8, proto_idx: u16 },
    // Vararg
    Vararg { dst: u8, count: u8 },
}
