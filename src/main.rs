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
use std::io::Read;
use std::process;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> KResult<()> {
    let mut args = env::args_os();
    let Some(binary_path) = args.next() else {
        return Err(kuu::error::KError::bytecode("usage: kuu <script.lua>"));
    };
    let raw_args: Vec<String> = args.map(|arg| arg.to_string_lossy().into_owned()).collect();
    if raw_args.len() == 1 && raw_args.first().is_some_and(|arg| arg == "-v") {
        println!("Lua 5.5");
        return Ok(());
    }

    let mut prelude = Vec::new();
    let mut script_path: Option<String> = None;
    let mut index = 0usize;
    while let Some(arg) = raw_args.get(index) {
        if arg == "--" {
            index = index.saturating_add(1);
            break;
        }
        if arg == "-" || !arg.starts_with('-') {
            break;
        }
        if let Some(chunk) = arg.strip_prefix("-e") {
            let inline = !chunk.is_empty();
            let chunk = if inline {
                chunk.to_owned()
            } else {
                raw_args.get(index + 1).cloned().unwrap_or_default()
            };
            if !chunk.trim().is_empty() {
                prelude.push(chunk);
            }
            index = if inline {
                index.saturating_add(1)
            } else {
                index.saturating_add(2)
            };
            continue;
        }
        if let Some(module) = arg.strip_prefix("-l") {
            let inline = !module.is_empty();
            let module = if inline {
                module.to_owned()
            } else {
                raw_args.get(index + 1).cloned().unwrap_or_default()
            };
            let module = module.trim();
            if !module.is_empty() {
                let source = if let Some((alias, name)) = module.split_once('=') {
                    format!("{alias} = {name}")
                } else {
                    format!("{module} = {module}")
                };
                prelude.push(source);
            }
            index = if inline {
                index.saturating_add(1)
            } else {
                index.saturating_add(2)
            };
            continue;
        }
        break;
    }
    if script_path.is_none() {
        script_path = raw_args.get(index).cloned();
        if script_path.is_some() {
            index = index.saturating_add(1);
        }
    }
    if let Some(script_path_ref) = script_path.as_ref() {
        let script_args = raw_args.get(index..).unwrap_or(&[]).to_vec();
        let mut vm = Vm::new()?;
        let binary_name = binary_path.to_string_lossy().into_owned();
        let script_name = script_path_ref.clone();
        vm.set_cli_args(&binary_name, &script_name, &script_args)?;
        for chunk_source in &prelude {
            run_source(&mut vm, chunk_source)?;
        }
        let source = if script_name == "-" {
            let mut stdin = std::io::stdin().lock();
            let mut source = String::new();
            stdin.read_to_string(&mut source)?;
            source
        } else {
            fs::read_to_string(&script_name)?
        };
        run_source(&mut vm, &source)?;
        return Ok(());
    }
    if prelude.is_empty() {
        return Err(kuu::error::KError::bytecode("usage: kuu <script.lua>"));
    }
    let mut vm = Vm::new()?;
    let binary_name = binary_path.to_string_lossy().into_owned();
    vm.set_cli_args(&binary_name, "-", &[])?;
    for chunk_source in &prelude {
        run_source(&mut vm, chunk_source)?;
    }
    Ok(())
}

fn run_source(vm: &mut Vm, source: &str) -> KResult<()> {
    let mut parser = Parser::new(source)?;
    let chunk = parser.parse_chunk()?;
    let mut compiler = Compiler::new();
    let proto = compiler.compile_chunk(&chunk)?;
    let _ = vm.run_proto(&proto)?;
    Ok(())
}
