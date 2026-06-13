mod lexer_support;

use kuu::error::KSpan;
use kuu::lexer::Keyword;

use lexer_support::{ExpectedKind, ExpectedToken, assert_scan, assert_syntax_error, eof, token};

fn single_line_expected(source: &'static str, kind: ExpectedKind) -> Vec<ExpectedToken> {
    let end_column = source.chars().count();
    vec![
        token(kind, source, KSpan::new(1, 1, 1, end_column)),
        eof(KSpan::new(1, end_column + 1, 1, end_column + 1)),
    ]
}

#[test]
fn decimal_and_hex_numerals_match_manual_section_3_1() -> Result<(), String> {
    let cases = [
        ("3", ExpectedKind::Integer),
        ("345", ExpectedKind::Integer),
        ("3.0", ExpectedKind::Float),
        ("3.1416", ExpectedKind::Float),
        ("314.16e-2", ExpectedKind::Float),
        ("0.31416E1", ExpectedKind::Float),
        ("34e1", ExpectedKind::Float),
        ("0xff", ExpectedKind::Integer),
        ("0xBEBADA", ExpectedKind::Integer),
        ("0x0.1E", ExpectedKind::Float),
        ("0xA23p-4", ExpectedKind::Float),
        ("0X1.921FB54442D18P+1", ExpectedKind::Float),
    ];

    for (source, kind) in cases {
        assert_scan(source, &single_line_expected(source, kind))?;
    }

    Ok(())
}

#[test]
fn malformed_numerals_return_syntax_errors() -> Result<(), String> {
    let cases = [
        ("1e", KSpan::new(1, 1, 1, 3)),
        ("1e+", KSpan::new(1, 1, 1, 4)),
        ("0x", KSpan::new(1, 1, 1, 3)),
        ("0xg", KSpan::new(1, 1, 1, 3)),
        ("0x1p", KSpan::new(1, 1, 1, 5)),
        ("0x1p+", KSpan::new(1, 1, 1, 6)),
        ("0x1.2p", KSpan::new(1, 1, 1, 7)),
        ("0x1.2p-", KSpan::new(1, 1, 1, 8)),
    ];

    for (source, span) in cases {
        assert_syntax_error(source, span)?;
    }

    Ok(())
}

#[test]
fn short_strings_and_escapes_preserve_lexeme_and_bytes() -> Result<(), String> {
    let cases = [
        ("'alo\\n123\"'", ExpectedKind::String(b"alo\n123\"")),
        (
            "\"\\a\\b\\f\\n\\r\\t\\v\\\\\\\"\\'\"",
            ExpectedKind::String(b"\x07\x08\x0c\n\r\t\x0b\\\"'"),
        ),
        ("\"\\x41\\97\"", ExpectedKind::String(b"Aa")),
        (
            "\"\\u{1f600}\"",
            ExpectedKind::String(&[0xF0, 0x9F, 0x98, 0x80]),
        ),
    ];

    for (source, kind) in cases {
        assert_scan(source, &single_line_expected(source, kind))?;
    }

    assert_scan(
        "\"\\x41\\z \n\tB\"",
        &[
            token(
                ExpectedKind::String(b"AB"),
                "\"\\x41\\z \n\tB\"",
                KSpan::new(1, 1, 2, 3),
            ),
            eof(KSpan::new(2, 4, 2, 4)),
        ],
    )?;

    Ok(())
}

#[test]
fn invalid_short_string_escapes_return_syntax_errors() -> Result<(), String> {
    assert_syntax_error("'\\q'", KSpan::new(1, 1, 1, 3))?;
    Ok(())
}

#[test]
fn long_bracket_strings_and_comments_follow_level_matching() -> Result<(), String> {
    let string_cases = [
        (
            "[[\nhello\nworld]]",
            ExpectedKind::String(b"hello\nworld"),
            KSpan::new(1, 1, 3, 7),
        ),
        (
            "[=[inner [[ level ]] markers]=]",
            ExpectedKind::String(b"inner [[ level ]] markers"),
            KSpan::new(1, 1, 1, 31),
        ),
    ];

    for (source, kind, span) in string_cases {
        assert_scan(
            source,
            &[
                token(kind, source, span),
                eof(KSpan::new(
                    span.end_line,
                    span.end_column + 1,
                    span.end_line,
                    span.end_column + 1,
                )),
            ],
        )?;
    }

    let comment_source = "--[=[comment\nwith [[nested]] markers\nstill comment]=]\nreturn";
    assert_scan(
        comment_source,
        &[
            token(
                ExpectedKind::Keyword(Keyword::Return),
                "return",
                KSpan::new(4, 1, 4, 6),
            ),
            eof(KSpan::new(4, 7, 4, 7)),
        ],
    )?;

    Ok(())
}
