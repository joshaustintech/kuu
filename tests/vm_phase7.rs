mod support;

use support::{ExpectedRun, compare_run, fixture_path, run_binary};

#[test]
fn prints_the_hello_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("hello.lua"))?;
    let expected = ExpectedRun {
        stdout: b"hello\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tables_globals_and_identity_work() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("phase7_tables.lua"))?;
    let expected = ExpectedRun {
        stdout: b"1\t42\ttrue\tok\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn closures_and_closed_upvalues_survive_the_outer_return() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary(&fixture_path("phase7_upvalues.lua"))?;
    let expected = ExpectedRun {
        stdout: b"1\t2\t3\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn varargs_flow_through_lua_calls() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("phase7_varargs.lua"))?;
    let expected = ExpectedRun {
        stdout: b"1\t2\t3\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn forwarded_varargs_reach_the_next_call() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("phase7_vararg_forward.lua"))?;
    let expected = ExpectedRun {
        stdout: b"1\t2\t3\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn recursion_and_tail_calls_use_vm_frames() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("phase7_recursion.lua"))?;
    let expected = ExpectedRun {
        stdout: b"15\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}
