#![forbid(unsafe_code)]
#![deny(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::unwrap_used
)]

use kuu::compiler::Compiler;
use kuu::error::KResult;
use kuu::parser::Parser;
use kuu::vm::Vm;
use std::env;
use std::fs;
use std::process;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> KResult<()> {
    let mut args = env::args_os();
    let _binary = args.next();
    let Some(script_path) = args.next() else {
        return Err(kuu::error::KError::bytecode("usage: kuu <script.lua>"));
    };

    if args.next().is_some() {
        return Err(kuu::error::KError::bytecode("usage: kuu <script.lua>"));
    }

    let source = fs::read_to_string(&script_path)?;
    let mut parser = Parser::new(&source)?;
    let chunk = parser.parse_chunk()?;
    let mut compiler = Compiler::new();
    let proto = compiler.compile_chunk(&chunk)?;
    let mut vm = Vm::new()?;
    let _ = vm.run_proto(&proto)?;
    Ok(())
}
