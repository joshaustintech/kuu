#![forbid(unsafe_code)]

use kuu::{compiler, lexer, parser, vm};

fn compile_source(source: &[u8]) -> Result<compiler::Proto, String> {
    let lex = lexer::Lexer::new(source);
    let mut parser = parser::Parser::new(lex);
    let block = parser
        .parse_chunk()
        .map_err(|err| format!("parse error: {err:?}"))?;
    compiler::Compiler::compile_chunk(&block).map_err(|err| format!("compile error: {err:?}"))
}

fn run_source(source: &[u8]) -> Result<(), String> {
    let proto = compile_source(source)?;
    let mut vm = vm::VM::new();
    vm.execute(proto)
        .map(|_| ())
        .map_err(|err| format!("runtime error: {err:?}"))
}

fn run_file(path: &str) -> Result<(), String> {
    let source =
        std::fs::read(path).map_err(|err| format!("could not read '{path}': {err}"))?;
    run_source(&source)
}

fn usage(program: &str) {
    eprintln!("usage: {program} [-e chunk] [script.lua]");
}

fn main() {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "kuu".to_string());

    let result = match args.next().as_deref() {
        Some("-e") => match args.next() {
            Some(chunk) => run_source(chunk.as_bytes()),
            None => {
                usage(&program);
                Err("missing chunk after -e".to_string())
            }
        },
        Some(path) => run_file(path),
        None => {
            usage(&program);
            Ok(())
        }
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
