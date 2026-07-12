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
