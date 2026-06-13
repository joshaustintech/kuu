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

pub mod error;

#[cfg(test)]
mod tests {
    use super::error::{KError, KErrorKind, KSpan};
    use std::io;

    #[test]
    fn compile_smoke() {
        let span = KSpan::new(1, 2, 1, 5);
        let err = KError::syntax("unexpected token", span);

        assert_eq!(
            err.kind(),
            &KErrorKind::Syntax("unexpected token".to_owned())
        );
        assert_eq!(err.span(), Some(span));
    }

    #[test]
    fn io_error_converts() {
        let err = KError::from(io::Error::other("disk full"));

        assert_eq!(err.kind(), &KErrorKind::Io("disk full".to_owned()));
        assert_eq!(err.span(), None);
    }

    #[test]
    fn error_display_includes_span_and_kind() {
        let span = KSpan::new(3, 4, 3, 9);
        let err = KError::runtime("bad value", span);

        assert_eq!(err.to_string(), "runtime error at 3:4-3:9: bad value");
    }
}
