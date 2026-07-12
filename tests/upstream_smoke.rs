use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn scripts() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/upstream/lua-5.5.0-tests");
    let mut files = fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("lua"))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

#[test]
#[ignore = "full Lua suite is an expected-failure conformance gate"]
fn every_vendored_lua_smoke_test_runs() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/upstream/lua-5.5.0-tests");
    let binary = env!("CARGO_BIN_EXE_kuu");
    let mut failures = Vec::new();

    for script in scripts()? {
        let mut child = Command::new(binary)
            .arg(&script)
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                child.kill()?;
                break None;
            }
            thread::sleep(Duration::from_millis(25));
        };
        if status.is_none_or(|value| !value.success()) {
            failures.push(script.display().to_string());
        }
    }

    assert!(
        failures.is_empty(),
        "upstream smoke failures:\n{}",
        failures.join("\n")
    );
    Ok(())
}
