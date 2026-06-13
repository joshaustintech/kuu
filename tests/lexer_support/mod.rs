use kuu::error::{KErrorKind, KResult, KSpan};
use kuu::lexer::{Keyword, Lexer, Token, TokenKind};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedKind {
    Eof,
    Name,
    Integer,
    Float,
    String(&'static [u8]),
    Keyword(Keyword),
    Punctuator(Punctuator),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punctuator {
    Plus,
    Minus,
    Star,
    Slash,
    DoubleSlash,
    Percent,
    Caret,
    Hash,
    Ampersand,
    Tilde,
    Pipe,
    ShiftLeft,
    ShiftRight,
    EqEq,
    NotEq,
    LessEq,
    GreaterEq,
    Less,
    Greater,
    Assign,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    DoubleColon,
    Semicolon,
    Colon,
    Comma,
    Dot,
    DotDot,
    DotDotDot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedToken {
    pub kind: ExpectedKind,
    pub lexeme: &'static str,
    pub span: KSpan,
}

pub fn token(kind: ExpectedKind, lexeme: &'static str, span: KSpan) -> ExpectedToken {
    ExpectedToken { kind, lexeme, span }
}

pub fn eof(span: KSpan) -> ExpectedToken {
    token(ExpectedKind::Eof, "", span)
}

pub fn collect_tokens(source: &str) -> KResult<Vec<Token>> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();

    loop {
        let token = lexer.next_token()?;
        let end = matches!(token.kind, TokenKind::Eof);
        tokens.push(token);
        if end {
            break;
        }
    }

    Ok(tokens)
}

pub fn assert_scan(source: &str, expected: &[ExpectedToken]) -> Result<(), String> {
    let tokens = collect_tokens(source)
        .map_err(|error| format!("lexing failed for {:?}: {}", source, error))?;

    if tokens.len() != expected.len() {
        return Err(format!(
            "token count mismatch for {:?}: expected {}, actual {}",
            source,
            expected.len(),
            tokens.len()
        ));
    }

    for (actual, expected) in tokens.iter().zip(expected.iter()) {
        assert_token(actual, expected)?;
    }

    Ok(())
}

#[allow(dead_code)]
pub fn assert_syntax_error(source: &str, expected_span: KSpan) -> Result<(), String> {
    let mut lexer = Lexer::new(source);
    let err = match lexer.next_token() {
        Err(error) => error,
        Ok(token) => {
            return Err(format!(
                "expected syntax error for {:?}, got {:?}",
                source, token
            ));
        }
    };

    match err.kind() {
        KErrorKind::Syntax(_) => {}
        other => {
            return Err(format!(
                "expected syntax error for {:?}, got {:?}",
                source, other
            ));
        }
    }

    match err.span() {
        Some(actual_span)
            if actual_span.start_line == expected_span.start_line
                && actual_span.start_column == expected_span.start_column => {}
        actual => {
            return Err(format!(
                "span mismatch for {:?}: expected start {:?}, actual {:?}",
                source,
                (expected_span.start_line, expected_span.start_column),
                actual
            ));
        }
    }

    Ok(())
}

fn assert_token(actual: &Token, expected: &ExpectedToken) -> Result<(), String> {
    if actual.lexeme != expected.lexeme {
        return Err(format!(
            "lexeme mismatch: expected {:?}, actual {:?}",
            expected.lexeme, actual.lexeme
        ));
    }

    if actual.span != expected.span {
        return Err(format!(
            "span mismatch: expected {:?}, actual {:?}",
            expected.span, actual.span
        ));
    }

    match (&actual.kind, expected.kind) {
        (TokenKind::Eof, ExpectedKind::Eof) => Ok(()),
        (TokenKind::Name(actual_name), ExpectedKind::Name) => {
            if actual_name == &actual.lexeme {
                Ok(())
            } else {
                Err(format!(
                    "name payload mismatch: expected {:?}, actual {:?}",
                    actual.lexeme, actual_name
                ))
            }
        }
        (TokenKind::Integer(actual_text), ExpectedKind::Integer) => {
            if actual_text == &actual.lexeme {
                Ok(())
            } else {
                Err(format!(
                    "integer payload mismatch: expected {:?}, actual {:?}",
                    actual.lexeme, actual_text
                ))
            }
        }
        (TokenKind::Float(actual_text), ExpectedKind::Float) => {
            if actual_text == &actual.lexeme {
                Ok(())
            } else {
                Err(format!(
                    "float payload mismatch: expected {:?}, actual {:?}",
                    actual.lexeme, actual_text
                ))
            }
        }
        (TokenKind::String(actual_bytes), ExpectedKind::String(expected_bytes)) => {
            if actual_bytes == expected_bytes {
                Ok(())
            } else {
                Err(format!(
                    "string payload mismatch: expected {:?}, actual {:?}",
                    expected_bytes, actual_bytes
                ))
            }
        }
        (TokenKind::Keyword(actual_keyword), ExpectedKind::Keyword(expected_keyword)) => {
            if actual_keyword == &expected_keyword {
                Ok(())
            } else {
                Err(format!(
                    "keyword mismatch: expected {:?}, actual {:?}",
                    expected_keyword, actual_keyword
                ))
            }
        }
        (TokenKind::Plus, ExpectedKind::Punctuator(Punctuator::Plus))
        | (TokenKind::Minus, ExpectedKind::Punctuator(Punctuator::Minus))
        | (TokenKind::Star, ExpectedKind::Punctuator(Punctuator::Star))
        | (TokenKind::Slash, ExpectedKind::Punctuator(Punctuator::Slash))
        | (TokenKind::DoubleSlash, ExpectedKind::Punctuator(Punctuator::DoubleSlash))
        | (TokenKind::Percent, ExpectedKind::Punctuator(Punctuator::Percent))
        | (TokenKind::Caret, ExpectedKind::Punctuator(Punctuator::Caret))
        | (TokenKind::Hash, ExpectedKind::Punctuator(Punctuator::Hash))
        | (TokenKind::Ampersand, ExpectedKind::Punctuator(Punctuator::Ampersand))
        | (TokenKind::Tilde, ExpectedKind::Punctuator(Punctuator::Tilde))
        | (TokenKind::Pipe, ExpectedKind::Punctuator(Punctuator::Pipe))
        | (TokenKind::ShiftLeft, ExpectedKind::Punctuator(Punctuator::ShiftLeft))
        | (TokenKind::ShiftRight, ExpectedKind::Punctuator(Punctuator::ShiftRight))
        | (TokenKind::EqEq, ExpectedKind::Punctuator(Punctuator::EqEq))
        | (TokenKind::NotEq, ExpectedKind::Punctuator(Punctuator::NotEq))
        | (TokenKind::LessEq, ExpectedKind::Punctuator(Punctuator::LessEq))
        | (TokenKind::GreaterEq, ExpectedKind::Punctuator(Punctuator::GreaterEq))
        | (TokenKind::Less, ExpectedKind::Punctuator(Punctuator::Less))
        | (TokenKind::Greater, ExpectedKind::Punctuator(Punctuator::Greater))
        | (TokenKind::Assign, ExpectedKind::Punctuator(Punctuator::Assign))
        | (TokenKind::LParen, ExpectedKind::Punctuator(Punctuator::LParen))
        | (TokenKind::RParen, ExpectedKind::Punctuator(Punctuator::RParen))
        | (TokenKind::LBrace, ExpectedKind::Punctuator(Punctuator::LBrace))
        | (TokenKind::RBrace, ExpectedKind::Punctuator(Punctuator::RBrace))
        | (TokenKind::LBracket, ExpectedKind::Punctuator(Punctuator::LBracket))
        | (TokenKind::RBracket, ExpectedKind::Punctuator(Punctuator::RBracket))
        | (TokenKind::DoubleColon, ExpectedKind::Punctuator(Punctuator::DoubleColon))
        | (TokenKind::Semicolon, ExpectedKind::Punctuator(Punctuator::Semicolon))
        | (TokenKind::Colon, ExpectedKind::Punctuator(Punctuator::Colon))
        | (TokenKind::Comma, ExpectedKind::Punctuator(Punctuator::Comma))
        | (TokenKind::Dot, ExpectedKind::Punctuator(Punctuator::Dot))
        | (TokenKind::DotDot, ExpectedKind::Punctuator(Punctuator::DotDot))
        | (TokenKind::DotDotDot, ExpectedKind::Punctuator(Punctuator::DotDotDot)) => Ok(()),
        _ => Err(format!(
            "token mismatch: actual={:?}, expected={:?}",
            actual, expected
        )),
    }
}
