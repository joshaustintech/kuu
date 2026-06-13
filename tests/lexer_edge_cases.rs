mod lexer_support;

use kuu::error::KSpan;
use kuu::lexer::Keyword;

use lexer_support::{ExpectedKind, assert_scan, assert_syntax_error, eof, token};

#[test]
fn shebang_is_ignored_at_chunk_start() -> Result<(), String> {
    assert_scan(
        "#!/usr/bin/env kuu\nreturn",
        &[
            token(
                ExpectedKind::Keyword(Keyword::Return),
                "return",
                KSpan::new(2, 1, 2, 6),
            ),
            eof(KSpan::new(2, 7, 2, 7)),
        ],
    )?;
    Ok(())
}

#[test]
fn crlf_counts_as_a_single_newline() -> Result<(), String> {
    assert_scan(
        "a\r\nb",
        &[
            token(ExpectedKind::Name, "a", KSpan::new(1, 1, 1, 1)),
            token(ExpectedKind::Name, "b", KSpan::new(2, 1, 2, 1)),
            eof(KSpan::new(2, 2, 2, 2)),
        ],
    )?;
    Ok(())
}

#[test]
fn short_comments_stop_at_newline() -> Result<(), String> {
    assert_scan(
        "foo-- comment\nbar",
        &[
            token(ExpectedKind::Name, "foo", KSpan::new(1, 1, 1, 3)),
            token(ExpectedKind::Name, "bar", KSpan::new(2, 1, 2, 3)),
            eof(KSpan::new(2, 4, 2, 4)),
        ],
    )?;
    Ok(())
}

#[test]
fn unterminated_long_bracket_string_errors() -> Result<(), String> {
    assert_syntax_error("[[abc", KSpan::new(1, 1, 1, 6))?;
    Ok(())
}

#[test]
fn unterminated_long_bracket_comment_errors() -> Result<(), String> {
    assert_syntax_error("--[[abc", KSpan::new(1, 1, 1, 8))?;
    Ok(())
}

#[test]
fn numeral_and_punctuator_boundaries_are_correct() -> Result<(), String> {
    assert_scan(
        "1..2",
        &[
            token(ExpectedKind::Integer, "1", KSpan::new(1, 1, 1, 1)),
            token(
                ExpectedKind::Punctuator(lexer_support::Punctuator::DotDot),
                "..",
                KSpan::new(1, 2, 1, 3),
            ),
            token(ExpectedKind::Integer, "2", KSpan::new(1, 4, 1, 4)),
            eof(KSpan::new(1, 5, 1, 5)),
        ],
    )?;

    assert_scan(
        "1...2",
        &[
            token(ExpectedKind::Integer, "1", KSpan::new(1, 1, 1, 1)),
            token(
                ExpectedKind::Punctuator(lexer_support::Punctuator::DotDotDot),
                "...",
                KSpan::new(1, 2, 1, 4),
            ),
            token(ExpectedKind::Integer, "2", KSpan::new(1, 5, 1, 5)),
            eof(KSpan::new(1, 6, 1, 6)),
        ],
    )?;

    Ok(())
}

#[test]
fn decimal_escape_overflow_is_rejected() -> Result<(), String> {
    assert_syntax_error("\"\\256\"", KSpan::new(1, 1, 1, 6))?;
    Ok(())
}

#[test]
fn hex_escape_requires_two_digits() -> Result<(), String> {
    assert_syntax_error("\"\\x4\"", KSpan::new(1, 1, 1, 5))?;
    Ok(())
}

#[test]
fn empty_unicode_escape_is_rejected() -> Result<(), String> {
    assert_syntax_error("\"\\u{}\"", KSpan::new(1, 1, 1, 5))?;
    Ok(())
}
