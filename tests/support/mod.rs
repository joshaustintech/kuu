use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: ExitStatus,
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

pub fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn run_binary(script: &Path) -> Result<RunResult, HarnessError> {
    let binary = match env::var_os("CARGO_BIN_EXE_kuu") {
        Some(value) => PathBuf::from(value),
        None => return Err(HarnessError::MissingBinary),
    };

    let output = match Command::new(&binary).arg(script).output() {
        Ok(output) => output,
        Err(error) => {
            return Err(HarnessError::Spawn {
                binary,
                source: error.to_string(),
            });
        }
    };

    Ok(RunResult {
        stdout: output.stdout,
        stderr: output.stderr,
        status: output.status,
    })
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
