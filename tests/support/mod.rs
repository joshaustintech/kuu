use std::env;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: ExitStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunOptions {
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
    pub current_dir: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub clear_env: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedRun<'a> {
    pub stdout: &'a [u8],
    pub stderr: &'a [u8],
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessError {
    MissingBinary,
    Spawn {
        binary: PathBuf,
        source: String,
    },
    StdinUnavailable,
    StdinWrite {
        binary: PathBuf,
        source: String,
    },
    OutputMismatch {
        field: &'static str,
        expected: Vec<u8>,
        actual: Vec<u8>,
    },
    ExitCodeMismatch {
        expected: Option<i32>,
        actual: Option<i32>,
    },
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBinary => write!(f, "missing CARGO_BIN_EXE_kuu"),
            Self::Spawn { binary, source } => {
                write!(f, "failed to spawn {}: {}", binary.display(), source)
            }
            Self::StdinUnavailable => write!(f, "child stdin was not available"),
            Self::StdinWrite { binary, source } => {
                write!(
                    f,
                    "failed to write stdin for {}: {}",
                    binary.display(),
                    source
                )
            }
            Self::OutputMismatch {
                field,
                expected,
                actual,
            } => write!(
                f,
                "{} mismatch: expected {:?}, actual {:?}",
                field, expected, actual
            ),
            Self::ExitCodeMismatch { expected, actual } => {
                write!(
                    f,
                    "exit code mismatch: expected {:?}, actual {:?}",
                    expected, actual
                )
            }
        }
    }
}

impl std::error::Error for HarnessError {}

#[allow(dead_code)]
pub fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[allow(dead_code)]
pub fn run_binary(script: &Path) -> Result<RunResult, HarnessError> {
    run_binary_with(RunOptions {
        args: vec![script.display().to_string()],
        ..RunOptions::default()
    })
}

pub fn run_binary_with(options: RunOptions) -> Result<RunResult, HarnessError> {
    let binary = match env::var_os("CARGO_BIN_EXE_kuu") {
        Some(value) => PathBuf::from(value),
        None => return Err(HarnessError::MissingBinary),
    };

    let mut command = Command::new(&binary);
    command.args(&options.args);
    if let Some(current_dir) = &options.current_dir {
        command.current_dir(current_dir);
    }
    if options.clear_env {
        command.env_clear();
    }
    for (key, value) in &options.env {
        command.env(key, value);
    }
    if options.stdin.is_empty() {
        let output = match command.output() {
            Ok(output) => output,
            Err(error) => {
                return Err(HarnessError::Spawn {
                    binary,
                    source: error.to_string(),
                });
            }
        };

        return Ok(RunResult {
            stdout: output.stdout,
            stderr: output.stderr,
            status: output.status,
        });
    }

    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Err(HarnessError::Spawn {
                binary: binary.clone(),
                source: error.to_string(),
            });
        }
    };

    let mut stdin = child.stdin.take().ok_or(HarnessError::StdinUnavailable)?;
    stdin
        .write_all(&options.stdin)
        .map_err(|error| HarnessError::StdinWrite {
            binary: binary.clone(),
            source: error.to_string(),
        })?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|error| HarnessError::Spawn {
            binary,
            source: error.to_string(),
        })?;

    Ok(RunResult {
        stdout: output.stdout,
        stderr: output.stderr,
        status: output.status,
    })
}

#[allow(dead_code)]
static NEXT_TEMP_DIR_ID: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

#[allow(dead_code)]
impl TempDir {
    pub fn new(prefix: &str) -> Result<Self, std::io::Error> {
        let id = NEXT_TEMP_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("kuu-{prefix}-{}-{id}", std::process::id());
        let path = env::temp_dir().join(name);
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, name: &str, contents: &str) -> Result<PathBuf, std::io::Error> {
        let path = self.path.join(name);
        fs::write(&path, contents)?;
        Ok(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn compare_run(actual: &RunResult, expected: &ExpectedRun<'_>) -> Result<(), HarnessError> {
    if actual.stdout != expected.stdout {
        return Err(HarnessError::OutputMismatch {
            field: "stdout",
            expected: expected.stdout.to_vec(),
            actual: actual.stdout.clone(),
        });
    }

    if actual.stderr != expected.stderr {
        return Err(HarnessError::OutputMismatch {
            field: "stderr",
            expected: expected.stderr.to_vec(),
            actual: actual.stderr.clone(),
        });
    }

    let actual_code = actual.status.code();
    if actual_code != expected.exit_code {
        return Err(HarnessError::ExitCodeMismatch {
            expected: expected.exit_code,
            actual: actual_code,
        });
    }

    Ok(())
}
