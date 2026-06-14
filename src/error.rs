use std::fmt;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KSpan {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl KSpan {
    pub const fn new(
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Self {
        Self {
            start_line,
            start_column,
            end_line,
            end_column,
        }
    }
}

impl fmt::Display for KSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}-{}:{}",
            self.start_line, self.start_column, self.end_line, self.end_column
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KErrorKind {
    Syntax(String),
    Runtime(String),
    Bytecode(String),
    Io(String),
}

impl KErrorKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Syntax(_) => "syntax error",
            Self::Runtime(_) => "runtime error",
            Self::Bytecode(_) => "bytecode error",
            Self::Io(_) => "io error",
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Syntax(message)
            | Self::Runtime(message)
            | Self::Bytecode(message)
            | Self::Io(message) => message,
        }
    }
}

impl fmt::Display for KErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.label(), self.message())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KError {
    kind: KErrorKind,
    span: Option<KSpan>,
}

impl KError {
    pub fn new(kind: KErrorKind, span: Option<KSpan>) -> Self {
        Self { kind, span }
    }

    pub fn syntax(message: impl Into<String>, span: KSpan) -> Self {
        Self::new(KErrorKind::Syntax(message.into()), Some(span))
    }

    pub fn runtime(message: impl Into<String>, span: KSpan) -> Self {
        Self::new(KErrorKind::Runtime(message.into()), Some(span))
    }

    pub fn bytecode(message: impl Into<String>) -> Self {
        Self::new(KErrorKind::Bytecode(message.into()), None)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(KErrorKind::Io(message.into()), None)
    }

    pub fn kind(&self) -> &KErrorKind {
        &self.kind
    }

    pub fn span(&self) -> Option<KSpan> {
        self.span
    }
}

impl fmt::Display for KError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(span) => write!(
                f,
                "{} at {}: {}",
                self.kind.label(),
                span,
                self.kind.message()
            ),
            None => write!(f, "{}", self.kind),
        }
    }
}

impl std::error::Error for KError {}

pub type KResult<T> = Result<T, KError>;

impl From<io::Error> for KError {
    fn from(error: io::Error) -> Self {
        Self::io(error.to_string())
    }
}
