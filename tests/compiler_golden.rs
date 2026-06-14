use kuu::compiler::Compiler;
use kuu::instruction::{ArithmeticOp, CompareOp, Instruction, JumpOffset, Register};
use kuu::parser::Parser;
use kuu::proto::Proto;

fn compile(source: &str) -> Result<Proto, String> {
    let mut parser = Parser::new(source).map_err(|error| error.to_string())?;
    let chunk = parser.parse_chunk().map_err(|error| error.to_string())?;
    let mut compiler = Compiler::new();
    compiler
        .compile_chunk(&chunk)
        .map_err(|error| error.to_string())
}

#[test]
fn simple_arithmetic_and_assignment_compile_to_expected_bytecode() -> Result<(), String> {
    let proto = compile(
        "local x = 1\n\
         x = x + 2\n\
         return x\n",
    )?;

    assert_eq!(proto.name, Some(b"chunk".to_vec()));
    assert_eq!(proto.parameters, 0);
    assert!(!proto.is_vararg);
    assert_eq!(
        proto.instructions,
        vec![
            Instruction::LoadInteger {
                dst: Register::new(1),
                value: 1,
            },
            Instruction::Move {
                dst: Register::new(0),
                src: Register::new(1),
            },
            Instruction::Move {
                dst: Register::new(3),
                src: Register::new(0),
            },
            Instruction::LoadInteger {
                dst: Register::new(4),
                value: 2,
            },
            Instruction::Arithmetic {
                op: ArithmeticOp::Add,
                dst: Register::new(2),
                left: Register::new(3),
                right: Register::new(4),
            },
            Instruction::Move {
                dst: Register::new(0),
                src: Register::new(2),
            },
            Instruction::Move {
                dst: Register::new(5),
                src: Register::new(0),
            },
            Instruction::Return {
                first: Register::new(5),
                count: 1,
            },
        ],
    );
    Ok(())
}

#[test]
fn control_flow_compiles_with_patched_jumps() -> Result<(), String> {
    let proto = compile(
        "local x = 0\n\
         while x < 3 do\n\
           x = x + 1\n\
         end\n\
         repeat\n\
           x = x - 1\n\
         until x == 0\n\
         ::done::\n\
         goto done\n\
         return x\n",
    )?;

    assert!(
        proto
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::JumpIfFalse { .. }))
    );
    assert!(
        proto
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Jump { .. }))
    );
    assert!(proto.instructions.iter().any(|instruction| matches!(
        instruction,
        Instruction::Compare {
            op: CompareOp::Less,
            ..
        }
    )));
    assert!(proto.instructions.iter().any(|instruction| matches!(
        instruction,
        Instruction::Compare {
            op: CompareOp::Eq,
            ..
        }
    )));
    assert!(
        proto
            .instructions
            .iter()
            .all(|instruction| match instruction {
                Instruction::Jump { offset }
                | Instruction::JumpIfTrue { offset, .. }
                | Instruction::JumpIfFalse { offset, .. } => offset != &JumpOffset::from_i32(0),
                _ => true,
            })
    );
    assert!(matches!(
        proto.instructions.last(),
        Some(Instruction::Return { .. })
    ));
    Ok(())
}

#[test]
fn closures_varargs_and_tail_calls_compile_into_nested_protos() -> Result<(), String> {
    let proto = compile(
        "local function outer(a, ...)\n\
           local function inner(b)\n\
             return a + b\n\
           end\n\
           return inner(...)\n\
         end\n",
    )?;

    assert_eq!(proto.nested.len(), 1);

    let outer = &proto.nested[0];
    assert_eq!(outer.name, Some(b"outer".to_vec()));
    assert_eq!(outer.parameters, 1);
    assert!(outer.is_vararg);
    assert!(
        outer
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::TailCall { .. }))
    );
    assert!(
        outer
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Closure { .. }))
    );

    assert_eq!(outer.nested.len(), 1);
    let inner = &outer.nested[0];
    assert_eq!(inner.name, Some(b"inner".to_vec()));
    assert_eq!(inner.parameters, 1);
    assert!(!inner.is_vararg);
    assert_eq!(inner.upvalues.len(), 1);
    assert!(
        inner
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetUpvalue { .. }))
    );
    assert!(inner.instructions.iter().any(|instruction| matches!(
        instruction,
        Instruction::Arithmetic {
            op: ArithmeticOp::Add,
            ..
        }
    )));

    Ok(())
}

#[test]
fn method_syntax_and_bytecode_encoding_roundtrip() -> Result<(), String> {
    let proto = compile(
        "local obj = {}\n\
         function obj:inc(x)\n\
           return x\n\
         end\n",
    )?;

    assert_eq!(proto.nested.len(), 1);
    let nested = &proto.nested[0];
    assert_eq!(nested.name, Some(b"obj:inc".to_vec()));
    assert_eq!(nested.parameters, 1);
    assert!(
        nested
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Return { count: 1, .. }))
    );

    let encoded = proto.encode().map_err(|error| error.to_string())?;
    let decoded = Proto::decode(&encoded).map_err(|error| error.to_string())?;
    assert_eq!(decoded, proto);
    Ok(())
}

#[test]
fn single_trailing_call_argument_preserves_all_results() -> Result<(), String> {
    let proto = compile(
        "local function pack(...)\n\
           return ...\n\
         end\n\
         print(pack(1, 2, 3))\n",
    )?;

    assert!(proto.instructions.iter().any(|instruction| matches!(
        instruction,
        Instruction::Call {
            results,
            ..
        } if *results == u16::MAX
    )));
    assert!(proto.instructions.iter().any(|instruction| matches!(
        instruction,
        Instruction::Call { args, .. } if *args == u16::MAX
    )));
    Ok(())
}
