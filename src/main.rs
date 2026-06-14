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
use kuu::error::{KError, KErrorKind, KResult};
use kuu::parser::Parser;
use kuu::vm::Vm;
use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::process;

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupAction {
    PrintVersion,
    EnableWarnings,
    RunChunk(String),
    LoadModule {
        global_name: String,
        module_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptSource {
    File(String),
    Stdin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliPlan {
    raw_args: Vec<String>,
    actions: Vec<StartupAction>,
    script_source: Option<ScriptSource>,
    script_index: Option<usize>,
    implicit_script_name: Option<String>,
    interactive: bool,
    ignore_environment: bool,
}

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}

fn run() -> KResult<i32> {
    let mut args = env::args_os();
    let Some(binary_path) = args.next() else {
        return Err(KError::bytecode("usage: kuu [options] [script [args]]"));
    };
    let raw_args: Vec<String> = args.map(|arg| arg.to_string_lossy().into_owned()).collect();
    let plan = parse_cli(raw_args)?;

    let mut vm = Vm::new()?;
    let binary_name = binary_path.to_string_lossy().into_owned();
    vm.set_cli_args_from_raw(
        &binary_name,
        &plan.raw_args,
        plan.script_index,
        plan.implicit_script_name.as_deref(),
    )?;
    configure_startup_environment(&mut vm, plan.ignore_environment)?;
    run_lua_init(&mut vm, plan.ignore_environment)?;

    for action in &plan.actions {
        match action {
            StartupAction::PrintVersion => println!("Lua 5.5"),
            StartupAction::EnableWarnings => vm.set_warnings_enabled(true),
            StartupAction::RunChunk(source) => {
                let _ = execute_source(&mut vm, source)?;
            }
            StartupAction::LoadModule {
                global_name,
                module_name,
            } => vm.require_module_into(module_name, global_name)?,
        }
    }

    if let Some(script_source) = &plan.script_source {
        let source = match script_source {
            ScriptSource::File(path) => fs::read_to_string(path)?,
            ScriptSource::Stdin => read_stdin_to_string()?,
        };
        let _ = execute_source(&mut vm, &source)?;
    }

    if plan.interactive {
        run_repl(&mut vm)?;
    }

    Ok(0)
}

fn parse_cli(raw_args: Vec<String>) -> KResult<CliPlan> {
    if raw_args.is_empty() {
        if io::stdin().is_terminal() {
            return Ok(CliPlan {
                raw_args,
                actions: vec![StartupAction::PrintVersion],
                script_source: None,
                script_index: None,
                implicit_script_name: None,
                interactive: true,
                ignore_environment: false,
            });
        }
        return Ok(CliPlan {
            raw_args,
            actions: Vec::new(),
            script_source: Some(ScriptSource::Stdin),
            script_index: None,
            implicit_script_name: Some("-".to_owned()),
            interactive: false,
            ignore_environment: false,
        });
    }

    let mut plan = CliPlan {
        raw_args,
        actions: Vec::new(),
        script_source: None,
        script_index: None,
        implicit_script_name: None,
        interactive: false,
        ignore_environment: false,
    };

    let mut index = 0usize;
    while let Some(arg) = plan.raw_args.get(index) {
        if arg == "--" {
            index = index.saturating_add(1);
            break;
        }
        if arg == "-" {
            plan.script_source = Some(ScriptSource::Stdin);
            plan.script_index = Some(index);
            index = index.saturating_add(1);
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            plan.script_source = Some(ScriptSource::File(arg.clone()));
            plan.script_index = Some(index);
            index = index.saturating_add(1);
            break;
        }

        if let Some(chunk) = arg.strip_prefix("-e") {
            let next = parse_option_argument("-e", chunk, &plan.raw_args, index)?;
            plan.actions.push(StartupAction::RunChunk(next.value));
            index = next.next_index;
            continue;
        }

        if let Some(module) = arg.strip_prefix("-l") {
            let next = parse_option_argument("-l", module, &plan.raw_args, index)?;
            let (global_name, module_name) = parse_module_binding(&next.value)?;
            plan.actions.push(StartupAction::LoadModule {
                global_name,
                module_name,
            });
            index = next.next_index;
            continue;
        }

        match arg.as_str() {
            "-i" => {
                plan.interactive = true;
                index = index.saturating_add(1);
            }
            "-v" => {
                plan.actions.push(StartupAction::PrintVersion);
                index = index.saturating_add(1);
            }
            "-W" => {
                plan.actions.push(StartupAction::EnableWarnings);
                index = index.saturating_add(1);
            }
            "-E" => {
                plan.ignore_environment = true;
                index = index.saturating_add(1);
            }
            _ => {
                return Err(KError::bytecode(format!("unrecognized option '{arg}'")));
            }
        }
    }

    if plan.script_source.is_none() {
        if let Some(arg) = plan.raw_args.get(index) {
            plan.script_source = Some(ScriptSource::File(arg.clone()));
            plan.script_index = Some(index);
        } else {
            let has_code_action = plan.actions.iter().any(|action| {
                matches!(
                    action,
                    StartupAction::PrintVersion
                        | StartupAction::RunChunk(_)
                        | StartupAction::LoadModule { .. }
                )
            });
            if !plan.interactive && !has_code_action && !io::stdin().is_terminal() {
                plan.script_source = Some(ScriptSource::Stdin);
                plan.implicit_script_name = Some("-".to_owned());
            }
        }
    }

    Ok(plan)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedOptionArgument {
    value: String,
    next_index: usize,
}

fn parse_option_argument(
    flag: &str,
    inline: &str,
    raw_args: &[String],
    index: usize,
) -> KResult<ParsedOptionArgument> {
    if !inline.is_empty() {
        return Ok(ParsedOptionArgument {
            value: inline.to_owned(),
            next_index: index.saturating_add(1),
        });
    }
    let next_index = index.saturating_add(1);
    let value = raw_args
        .get(next_index)
        .cloned()
        .ok_or_else(|| KError::bytecode(format!("'{flag}' needs argument")))?;
    Ok(ParsedOptionArgument {
        value,
        next_index: next_index.saturating_add(1),
    })
}

fn parse_module_binding(spec: &str) -> KResult<(String, String)> {
    if spec.trim().is_empty() {
        return Err(KError::bytecode("'-l' needs argument"));
    }
    if let Some((global_name, module_name)) = spec.split_once('=') {
        if global_name.trim().is_empty() || module_name.trim().is_empty() {
            return Err(KError::bytecode("'-l' needs argument"));
        }
        return Ok((global_name.trim().to_owned(), module_name.trim().to_owned()));
    }
    Ok((spec.trim().to_owned(), spec.trim().to_owned()))
}

fn read_stdin_to_string() -> KResult<String> {
    let mut stdin = io::stdin().lock();
    let mut source = String::new();
    stdin.read_to_string(&mut source)?;
    Ok(source)
}

fn execute_source(vm: &mut Vm, source: &str) -> KResult<Vec<kuu::value::Value>> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut parser = Parser::new(source)?;
    let chunk = parser.parse_chunk()?;
    let mut compiler = Compiler::new();
    let proto = compiler.compile_chunk(&chunk)?;
    vm.run_proto(&proto)
}

fn configure_startup_environment(vm: &mut Vm, ignore_environment: bool) -> KResult<()> {
    if ignore_environment {
        return Ok(());
    }

    let default_path = vm.package_path()?;
    if let Some(path) = preferred_env("LUA_PATH_5_5", "LUA_PATH") {
        vm.set_package_path(&expand_default_path(&path, &default_path))?;
    }

    let default_cpath = vm.package_cpath()?;
    if let Some(cpath) = preferred_env("LUA_CPATH_5_5", "LUA_CPATH") {
        vm.set_package_cpath(&expand_default_path(&cpath, &default_cpath))?;
    }

    Ok(())
}

fn run_lua_init(vm: &mut Vm, ignore_environment: bool) -> KResult<()> {
    if ignore_environment {
        return Ok(());
    }

    let Some(init) = preferred_env("LUA_INIT_5_5", "LUA_INIT") else {
        return Ok(());
    };

    if let Some(path) = init.strip_prefix('@') {
        let source = fs::read_to_string(path)?;
        let _ = execute_source(vm, &source)
            .map_err(|error| KError::bytecode(format!("LUA_INIT: {error}")))?;
        return Ok(());
    }

    let _ = execute_source(vm, &init)
        .map_err(|error| KError::bytecode(format!("LUA_INIT: {error}")))?;
    Ok(())
}

fn preferred_env(versioned: &str, plain: &str) -> Option<String> {
    env::var_os(versioned)
        .map(|value| value.to_string_lossy().into_owned())
        .or_else(|| env::var_os(plain).map(|value| value.to_string_lossy().into_owned()))
}

fn expand_default_path(value: &str, default: &str) -> String {
    if !value.contains(";;") {
        return value.to_owned();
    }

    let expanded = value.replace(";;", &format!(";{default};"));
    expanded
        .trim_start_matches(';')
        .trim_end_matches(';')
        .to_owned()
}

fn run_repl(vm: &mut Vm) -> KResult<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut pending = String::new();

    loop {
        let prompt = if pending.is_empty() {
            vm.global_string(b"_PROMPT")?
                .unwrap_or_else(|| "> ".to_owned())
        } else {
            vm.global_string(b"_PROMPT2")?
                .unwrap_or_else(|| ">> ".to_owned())
        };
        writer.write_all(prompt.as_bytes())?;
        writer.flush()?;

        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            if pending.trim().is_empty() {
                break;
            }
            if matches!(
                try_run_repl_chunk(vm, &pending, &mut writer)?,
                ReplChunkResult::Incomplete
            ) {
                let _ = execute_source(vm, &pending)?;
            }
            break;
        }
        if repl_line_starts_with_local(&line) {
            eprintln!("warning: locals do not survive across lines in interactive mode");
        }
        pending.push_str(&line);

        if pending.trim().is_empty() {
            pending.clear();
            continue;
        }

        match try_run_repl_chunk(vm, &pending, &mut writer)? {
            ReplChunkResult::Executed => pending.clear(),
            ReplChunkResult::Incomplete => {}
        }
    }

    Ok(())
}

enum ReplChunkResult {
    Executed,
    Incomplete,
}

fn try_run_repl_chunk(
    vm: &mut Vm,
    source: &str,
    writer: &mut impl Write,
) -> KResult<ReplChunkResult> {
    let expression_source = format!("return {}", source.trim_end());
    match execute_source(vm, &expression_source) {
        Ok(values) => {
            if !values.is_empty() {
                vm.print_values_to_stdout(&values)?;
            }
            writer.flush()?;
            return Ok(ReplChunkResult::Executed);
        }
        Err(error) => {
            if !matches!(error.kind(), KErrorKind::Syntax(_)) {
                return Err(error);
            }
        }
    }

    match execute_source(vm, source) {
        Ok(_) => {
            writer.flush()?;
            Ok(ReplChunkResult::Executed)
        }
        Err(error) if is_incomplete_repl_error(&error, source) => Ok(ReplChunkResult::Incomplete),
        Err(error) => Err(error),
    }
}

fn is_incomplete_repl_error(error: &KError, source: &str) -> bool {
    if !matches!(error.kind(), KErrorKind::Syntax(_)) {
        return false;
    }
    let Some(span) = error.span() else {
        return false;
    };
    let line_count = source.lines().count();
    span.start_line > line_count
}

fn repl_line_starts_with_local(line: &str) -> bool {
    let trimmed = line.trim_start_matches(|ch: char| ch.is_whitespace());
    let Some(rest) = trimmed.strip_prefix("local") else {
        return false;
    };
    match rest.chars().next() {
        None => true,
        Some(ch) => !ch.is_ascii_alphanumeric() && ch != '_',
    }
}
