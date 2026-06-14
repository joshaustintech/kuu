mod support;

use support::{ExpectedRun, compare_run, fixture_path, run_binary};

#[test]
fn hello_fixture_runs_through_the_binary() -> Result<(), Box<dyn std::error::Error>> {
    let script = fixture_path("hello.lua");
    let actual = run_binary(&script)?;

    let expected = ExpectedRun {
        stdout: b"hello\n",
        stderr: b"",
        exit_code: Some(0),
    };

    let result = compare_run(&actual, &expected);
    assert!(result.is_ok());
    Ok(())
}
