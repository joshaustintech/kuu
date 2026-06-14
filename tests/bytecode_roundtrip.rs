use kuu::error::KResult;
use kuu::instruction::{
    ArithmeticOp, CompareOp, ConstantIndex, Instruction, JumpOffset, PrototypeIndex, Register,
    UnaryOpKind,
};
use kuu::proto::{Constant, Proto, UpvalueDescriptor};

fn sample_instructions() -> KResult<Vec<Instruction>> {
    Ok(vec![
        Instruction::LoadNil {
            dst: Register::new(0),
        },
        Instruction::LoadBool {
            dst: Register::new(1),
            value: true,
        },
        Instruction::LoadInteger {
            dst: Register::new(2),
            value: -17,
        },
        Instruction::LoadNumber {
            dst: Register::new(3),
            value: 3.25,
        },
        Instruction::LoadConstant {
            dst: Register::new(4),
            constant: ConstantIndex::new(7),
        },
        Instruction::Move {
            dst: Register::new(5),
            src: Register::new(6),
        },
        Instruction::GetGlobal {
            dst: Register::new(7),
            name: ConstantIndex::new(8),
        },
        Instruction::SetGlobal {
            src: Register::new(8),
            name: ConstantIndex::new(9),
        },
        Instruction::GetTable {
            dst: Register::new(9),
            table: Register::new(10),
            key: Register::new(11),
        },
        Instruction::SetTable {
            table: Register::new(12),
            key: Register::new(13),
            value: Register::new(14),
        },
        Instruction::Arithmetic {
            op: ArithmeticOp::Add,
            dst: Register::new(15),
            left: Register::new(16),
            right: Register::new(17),
        },
        Instruction::Arithmetic {
            op: ArithmeticOp::BitAnd,
            dst: Register::new(15),
            left: Register::new(16),
            right: Register::new(17),
        },
        Instruction::Compare {
            op: CompareOp::LessEq,
            dst: Register::new(18),
            left: Register::new(19),
            right: Register::new(20),
        },
        Instruction::Unary {
            op: UnaryOpKind::Minus,
            dst: Register::new(18),
            src: Register::new(19),
        },
        Instruction::Jump {
            offset: JumpOffset::new(-12)?,
        },
        Instruction::Call {
            function: Register::new(21),
            args: 3,
            results: 1,
        },
        Instruction::Return {
            first: Register::new(22),
            count: 2,
        },
        Instruction::Closure {
            dst: Register::new(23),
            proto: PrototypeIndex::new(4),
        },
        Instruction::Vararg {
            dst: Register::new(24),
            count: Some(5),
        },
        Instruction::ForPrep {
            base: Register::new(25),
            offset: JumpOffset::new(9)?,
        },
        Instruction::ForLoop {
            base: Register::new(26),
            offset: JumpOffset::new(-7)?,
        },
        Instruction::Concat {
            dst: Register::new(27),
            first: Register::new(28),
            last: Register::new(29),
        },
        Instruction::Close {
            from: Register::new(30),
        },
        Instruction::NewTable {
            dst: Register::new(31),
        },
        Instruction::GetUpvalue {
            dst: Register::new(32),
            upvalue: 2,
        },
        Instruction::SetUpvalue {
            src: Register::new(33),
            upvalue: 3,
        },
        Instruction::JumpIfTrue {
            cond: Register::new(34),
            offset: JumpOffset::new(11)?,
        },
        Instruction::JumpIfFalse {
            cond: Register::new(35),
            offset: JumpOffset::new(-9)?,
        },
        Instruction::TailCall {
            function: Register::new(36),
            args: 4,
        },
    ])
}

fn sample_proto() -> KResult<Proto> {
    Ok(Proto {
        name: Some(b"chunk".to_vec()),
        parameters: 2,
        is_vararg: true,
        stack_size: 8,
        upvalues: vec![UpvalueDescriptor {
            instack: true,
            index: 1,
        }],
        constants: vec![
            Constant::Nil,
            Constant::Boolean(true),
            Constant::Integer(42),
            Constant::Number(3.5),
            Constant::String(b"hello".to_vec()),
        ],
        instructions: sample_instructions()?,
        nested: vec![Proto {
            name: Some(b"nested".to_vec()),
            parameters: 0,
            is_vararg: false,
            stack_size: 1,
            upvalues: Vec::new(),
            constants: vec![Constant::String(b"inner".to_vec())],
            instructions: vec![Instruction::NewTable {
                dst: Register::new(0),
            }],
            nested: Vec::new(),
        }],
    })
}

#[test]
fn instruction_roundtrip_covers_phase_five_opcodes() -> Result<(), String> {
    let instructions = sample_instructions().map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();

    for instruction in &instructions {
        bytes.extend_from_slice(&instruction.encode());
    }

    let decoded = Instruction::decode_all(&bytes).map_err(|error| error.to_string())?;
    assert_eq!(decoded, instructions);
    Ok(())
}

#[test]
fn proto_roundtrip_preserves_constants_and_nested_functions() -> Result<(), String> {
    let proto = sample_proto().map_err(|error| error.to_string())?;
    let encoded = proto.encode().map_err(|error| error.to_string())?;
    let decoded = Proto::decode(&encoded).map_err(|error| error.to_string())?;

    assert_eq!(decoded, proto);
    Ok(())
}

#[test]
fn invalid_offsets_and_truncated_payloads_are_rejected() -> Result<(), String> {
    assert!(JumpOffset::new(i64::from(i32::MAX) + 1).is_err());
    assert!(JumpOffset::new(i64::from(i32::MIN) - 1).is_err());

    let mut bytes = Instruction::Jump {
        offset: JumpOffset::from_i32(3),
    }
    .encode();
    bytes.pop();
    assert!(Instruction::decode(&bytes).is_err());

    assert!(Instruction::decode(&[255]).is_err());
    Ok(())
}
