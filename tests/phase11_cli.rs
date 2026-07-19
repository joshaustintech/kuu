mod support;

use support::{ExpectedRun, RunOptions, TempDir, compare_run, run_binary_with};

#[test]
fn stdin_is_executed_as_a_file_without_arguments() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        stdin: b"print(10)\nprint(2)\n".to_vec(),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"10\n2\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn global_require_returns_loaded_module() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-e".to_owned(), "print(require('math').abs(-2))".to_owned()],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"2\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn load_uses_global_environment_when_env_is_nil() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-e".to_owned(), "print(load('return 2')())".to_owned()],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"2\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn global_declarations_reject_existing_environment_fields() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "global assert, load, pcall, string, print; local f = assert(load(\"global print = 10\")); local ok, msg = pcall(f); assert(not ok and string.find(msg, \"global 'print' already defined\")); local f = assert(load(\"local _ENV = {AA = false}; global AA = 10\")); local ok, msg = pcall(f); assert(not ok and string.find(msg, \"global 'AA' already defined\")); print('OK')".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"OK\n",
        stderr: b"",
        exit_code: Some(0),
    };
    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn load_returns_nil_and_error_for_invalid_source() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local f, err = load('goto l1; do ::l1:: end'); print(f == nil, err ~= nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn integer_overflow_wraps() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(math.type(math.maxinteger + 1), math.maxinteger + 1)".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"integer\t-9223372036854775808\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn arithmetic_coerces_whitespace_padded_numeric_strings() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print('2' + ' 3e0 ', '10' - ' 10  ', -'  10 ')".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"5\t0\t-10\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn arithmetic_accepts_tab_padding_but_rejects_internal_whitespace()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print('\\t4\\t' + 1); print(pcall(function () return '1 2' + 1 end))".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    assert!(actual.stdout.starts_with(b"5\nfalse\t"));
    assert_eq!(actual.stderr, b"");
    assert_eq!(actual.status.code(), Some(0));
    Ok(())
}

#[test]
fn method_closure_captures_shadowing_local_table() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "global<const> *; local a={i=10}; local self=20; function a:x(x) return x+self.i end; function a.y(x) return x+self end; a.t={i=-100}; a[\"t\"].x=function(self,a,b) return self.i+a+b end; do local a={x=0}; function a:add(x) self.x,a.y=self.x+x,20; return self end; local z=a:add(10):add(20):add(30); print(z.x,a.y) end".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"60\t20\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn tail_call_preserves_call_metamethod_receiver() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function foo(self, x) return self, x end; local t = setmetatable({}, {__call = foo}); local function foo2(x) return t(x) end; local a, b = foo2(100); print(a == t, b)".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"true\t100\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn trailing_vararg_expands_inside_table_constructor() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function foo(x, ...) local a = {...}; return x, a[1], a[2] end; print(foo(10, 100, 200))".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"10\t100\t200\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn non_trailing_vararg_stays_single_table_value() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function f(...) local t = {..., 99}; print(t[1], t[2]) end; f(10, 20)"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"10\t99\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn trailing_vararg_expands_after_fixed_return_value() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function f(x, ...) return x, ... end; print(f(10, 100, 200))".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"10\t100\t200\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn tail_call_forwards_all_varargs() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function sink(...) return ... end; local function forward(...) return sink(...) end; print(forward(10, 20, 30))".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"10\t20\t30\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn protected_recursive_closure_keeps_open_upvalues_valid() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function loop() assert(pcall(loop)) end; local ok, msg = xpcall(loop, loop); print(not ok, string.find(msg, 'error') ~= nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"true\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn omitted_function_parameters_are_nil() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function f(x) return x == nil end; print(f())".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn tail_method_call_passes_receiver() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local a = { value = 40 }; function a:add(n) return self.value + n end; local function f() return a:add(2) end; print(f())".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"42\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn generic_for_preserves_iterator_state_and_multi_results() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "for i, value in ipairs({ 10, 20 }) do print(i, value) end".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"1\t10\n2\t20\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn type_requires_a_value_argument() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-e".to_owned(), "print(pcall(type))".to_owned()],
        ..RunOptions::default()
    })?;

    assert!(actual.stdout.starts_with(b"false\t"));
    assert_eq!(actual.stderr, b"");
    assert_eq!(actual.status.code(), Some(0));
    Ok(())
}

#[test]
fn dotted_method_declaration_installs_on_full_prefix() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local a = { b = { c = {} } }; function a.b.c:set(key, value) self[key] = value end; a.b.c:set('answer', 42); print(a.b.c.answer)".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"42\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn string_packsize_reports_lua_integer_width() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-e".to_owned(), "print(string.packsize('j'))".to_owned()],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"8\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn bitwise_coerces_hex_integer_strings() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print('0xffffffffffffffff' | 0, '1234.0' << '5.0', '0xffff.0' ~ '0xAAAA', ~'0x0.000p4')".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"-1\t39488\t21845\t-1\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn bitwise_rejects_out_of_range_numeric_strings() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(pcall(function () return '0xffffffffffffffff.0' | 0 end))".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"false\t",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(actual.stdout.starts_with(expected.stdout));
    assert_eq!(actual.stderr, expected.stderr);
    assert_eq!(actual.status.code(), expected.exit_code);
    Ok(())
}

#[test]
fn dash_reads_stdin_and_stops_option_handling() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-".to_owned(), "-h".to_owned()],
        stdin: b"print(arg[1])\n".to_vec(),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"-h\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn arg_table_tracks_options_script_and_script_arguments() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = TempDir::new("phase11-arg")?;
    let script = dir.write(
        "arg.lua",
        "assert(arg[-2] == '-e')\nassert(arg[-1] == 'print(\"setup\")')\nassert(arg[0] == 'arg.lua')\nassert(arg[1] == 'a')\nassert(arg[2] == 'b')\nprint(arg[1], arg[2])\n",
    )?;
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(\"setup\")".to_owned(),
            "arg.lua".to_owned(),
            "a".to_owned(),
            "b".to_owned(),
        ],
        current_dir: Some(dir.path().to_path_buf()),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"setup\na\tb\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(
        compare_run(&actual, &expected).is_ok(),
        "script path: {}",
        script.display()
    );
    Ok(())
}

#[test]
fn dash_l_uses_require_and_assigns_the_loaded_value() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("phase11-module")?;
    dir.write(
        "mymod.lua",
        "print('loading module')\nreturn { answer = 42 }\n",
    )?;
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "package.path = '?.lua'".to_owned(),
            "-l".to_owned(),
            "mymod".to_owned(),
            "-e".to_owned(),
            "print(mymod.answer)".to_owned(),
        ],
        current_dir: Some(dir.path().to_path_buf()),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"loading module\n42\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn multiple_e_chunks_run_in_command_line_order() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-eprint(1)".to_owned(),
            "-ea=3".to_owned(),
            "-e".to_owned(),
            "print(a)".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"1\n3\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn no_script_calls_expose_the_interpreter_options_in_arg() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary_with(RunOptions {
        args: vec!["-e".to_owned(), "print(arg[1], arg[2])".to_owned()],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"-e\tprint(arg[1], arg[2])\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn dash_l_supports_explicit_global_aliases() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-l".to_owned(),
            "str=string".to_owned(),
            "-lm=math".to_owned(),
            "-e".to_owned(),
            "print(str.upper('alo alo'), m.max(10, 20))".to_owned(),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"ALO ALO\t20\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn package_path_can_come_from_environment() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("phase11-env-path")?;
    dir.write("show_path.lua", "print(package.path)\n")?;
    let actual = run_binary_with(RunOptions {
        args: vec!["--".to_owned(), "show_path.lua".to_owned()],
        current_dir: Some(dir.path().to_path_buf()),
        env: vec![
            ("LUA_INIT".to_owned(), String::new()),
            ("LUA_PATH".to_owned(), "x".to_owned()),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"x\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn versioned_env_paths_override_unversioned_values() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("phase11-env-versioned")?;
    dir.write(
        "show_path.lua",
        "print(package.path)\nprint(package.cpath)\n",
    )?;
    let actual = run_binary_with(RunOptions {
        args: vec!["show_path.lua".to_owned()],
        current_dir: Some(dir.path().to_path_buf()),
        env: vec![
            ("LUA_INIT".to_owned(), String::new()),
            ("LUA_PATH".to_owned(), "x".to_owned()),
            ("LUA_PATH_5_5".to_owned(), "y".to_owned()),
            ("LUA_CPATH".to_owned(), "xuxu".to_owned()),
            ("LUA_CPATH_5_5".to_owned(), "yacc".to_owned()),
        ],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"y\nyacc\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn lua_init_runs_before_the_script_and_can_see_arg() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("phase11-init")?;
    dir.write("show_x.lua", "print(X)\n")?;
    let actual = run_binary_with(RunOptions {
        args: vec!["show_x.lua".to_owned(), "3.2".to_owned()],
        current_dir: Some(dir.path().to_path_buf()),
        env: vec![("LUA_INIT".to_owned(), "X=tonumber(arg[1])".to_owned())],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"3.2\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn lua_init_can_execute_a_file_before_the_script() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("phase11-init-file")?;
    let init_path = dir.write("init.lua", "x = x or 10; print(x); x = x + 1\n")?;
    dir.write("show_x.lua", "print(x)\n")?;
    let actual = run_binary_with(RunOptions {
        args: vec!["show_x.lua".to_owned()],
        current_dir: Some(dir.path().to_path_buf()),
        env: vec![("LUA_INIT".to_owned(), format!("@{}", init_path.display()))],
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"10\n11\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn dash_e_ignores_environment_startup_and_paths() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("phase11-noenv")?;
    dir.write("show_paths.lua", "print(package.path, package.cpath)\n")?;
    let actual = run_binary_with(RunOptions {
        args: vec!["-E".to_owned(), "show_paths.lua".to_owned()],
        current_dir: Some(dir.path().to_path_buf()),
        env: vec![
            ("LUA_INIT".to_owned(), "error(10)".to_owned()),
            ("LUA_PATH".to_owned(), "xxx".to_owned()),
            ("LUA_CPATH".to_owned(), "xxx".to_owned()),
        ],
        ..RunOptions::default()
    })?;

    assert_eq!(actual.status.code(), Some(0));
    assert!(actual.stderr.is_empty());
    let stdout = String::from_utf8(actual.stdout)?;
    assert!(!stdout.contains("xxx"), "{stdout}");
    assert!(stdout.contains("?.lua;?/init.lua"), "{stdout}");
    Ok(())
}

#[test]
fn dash_e_without_a_script_still_consumes_piped_stdin() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-E".to_owned()],
        stdin: b"print(123)\n".to_vec(),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"123\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn interactive_mode_preserves_globals_and_prints_expression_results()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "_PROMPT='' _PROMPT2=''".to_owned(),
            "-i".to_owned(),
        ],
        stdin: b"x = 41\nx + 1\n".to_vec(),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"42\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn repl_warns_when_a_line_starts_with_local() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "_PROMPT='' _PROMPT2=''".to_owned(),
            "-i".to_owned(),
        ],
        stdin: b"  local x = 10\nprint(x)\n".to_vec(),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"nil\n",
        stderr: b"warning: locals do not survive across lines in interactive mode\n",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn repl_continues_incomplete_assignments_until_the_chunk_finishes()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "_PROMPT='' _PROMPT2=''".to_owned(),
            "-i".to_owned(),
        ],
        stdin: b"a =\n10\nprint(a)\na\n".to_vec(),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"10\n10\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn missing_e_argument_exits_non_zero() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-e".to_owned()],
        ..RunOptions::default()
    })?;

    assert_eq!(actual.status.code(), Some(1));
    let stderr = String::from_utf8(actual.stderr)?;
    assert!(stderr.contains("'-e' needs argument"), "{stderr}");
    Ok(())
}

#[test]
fn invalid_options_exit_non_zero() -> Result<(), Box<dyn std::error::Error>> {
    for args in [
        vec!["-h".to_owned()],
        vec!["---".to_owned()],
        vec!["-Ex".to_owned(), "--".to_owned()],
        vec!["-vv".to_owned()],
        vec!["-iv".to_owned()],
        vec!["-l".to_owned()],
    ] {
        let actual = run_binary_with(RunOptions {
            args,
            ..RunOptions::default()
        })?;
        assert_eq!(actual.status.code(), Some(1));
        assert!(!actual.stderr.is_empty());
    }
    Ok(())
}

#[test]
fn bom_prefixed_scripts_run_like_plain_text_files() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("phase11-bom")?;
    dir.write("bom.lua", "\u{feff}# comment!!\nprint(3)\n")?;
    let actual = run_binary_with(RunOptions {
        args: vec!["bom.lua".to_owned()],
        current_dir: Some(dir.path().to_path_buf()),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"3\n",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn repl_uses_custom_primary_and_secondary_prompts() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "_PROMPT='alo' _PROMPT2='cont'".to_owned(),
            "-i".to_owned(),
        ],
        stdin: b"a =\n10\n".to_vec(),
        ..RunOptions::default()
    })?;

    assert_eq!(actual.status.code(), Some(0));
    assert!(actual.stderr.is_empty());
    let stdout = String::from_utf8(actual.stdout)?;
    assert!(stdout.contains("alo"), "{stdout}");
    assert!(stdout.contains("cont"), "{stdout}");
    Ok(())
}

#[test]
fn warnings_are_silent_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        stdin: b"warn[[XXX]]\n".to_vec(),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"",
        stderr: b"",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}

#[test]
fn dash_w_enables_warning_output_and_controls() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("phase11-warn")?;
    dir.write(
        "warn.lua",
        "warn('@allow')\nwarn('@off', 'XXX', '@off')\nwarn('@off')\nwarn('@on', 'YYY', '@on')\nwarn('@off')\nwarn('@on')\nwarn('', '@on')\nwarn('@on')\nwarn('Z', 'Z', 'Z')\n",
    )?;
    let actual = run_binary_with(RunOptions {
        args: vec!["-W".to_owned(), "warn.lua".to_owned()],
        current_dir: Some(dir.path().to_path_buf()),
        ..RunOptions::default()
    })?;

    let expected = ExpectedRun {
        stdout: b"",
        stderr: b"Lua warning: @offXXX@off\nLua warning: @on\nLua warning: ZZZ\n",
        exit_code: Some(0),
    };

    assert!(compare_run(&actual, &expected).is_ok());
    Ok(())
}
