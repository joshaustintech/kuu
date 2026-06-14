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

pub mod ast;
mod bytecode;
pub mod error;
pub mod instruction;
pub mod lexer;
pub mod parser;
pub mod proto;
pub mod resolve;
pub mod value;

#[cfg(test)]
mod tests {
    use super::error::{KError, KErrorKind, KSpan};
    use super::lexer::{Keyword, Lexer, TokenKind};
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

    #[test]
    fn lexer_smoke_test() {
        let mut lexer = Lexer::new("global answer = 42");

        let first = lexer.next_token();
        assert!(first.is_ok());
        let first = if let Ok(token) = first { token } else { return };
        assert_eq!(first.kind, TokenKind::Keyword(Keyword::Global));

        let second = lexer.next_token();
        assert!(second.is_ok());
        let second = if let Ok(token) = second {
            token
        } else {
            return;
        };
        assert_eq!(second.kind, TokenKind::Name("answer".to_owned()));

        let third = lexer.next_token();
        assert!(third.is_ok());
        let third = if let Ok(token) = third { token } else { return };
        assert_eq!(third.kind, TokenKind::Assign);

        let fourth = lexer.next_token();
        assert!(fourth.is_ok());
        let fourth = if let Ok(token) = fourth {
            token
        } else {
            return;
        };
        assert_eq!(fourth.kind, TokenKind::Integer("42".to_owned()));
    }
}
