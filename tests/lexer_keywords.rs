mod lexer_support;

use kuu::error::KSpan;
use kuu::lexer::Keyword;

use lexer_support::{ExpectedKind, ExpectedToken, Punctuator, assert_scan, eof, token};

fn single_line_expected(source: &'static str, kind: ExpectedKind) -> Vec<ExpectedToken> {
    let end_column = source.chars().count();
    vec![
        token(kind, source, KSpan::new(1, 1, 1, end_column)),
        eof(KSpan::new(1, end_column + 1, 1, end_column + 1)),
    ]
}

#[test]
fn keyword_tokens_match_manual_section_3_1() -> Result<(), String> {
    let cases = [
        ("and", Keyword::And),
        ("break", Keyword::Break),
        ("do", Keyword::Do),
        ("else", Keyword::Else),
        ("elseif", Keyword::ElseIf),
        ("end", Keyword::End),
        ("false", Keyword::False),
        ("for", Keyword::For),
        ("function", Keyword::Function),
        ("global", Keyword::Global),
        ("goto", Keyword::Goto),
        ("if", Keyword::If),
        ("in", Keyword::In),
        ("local", Keyword::Local),
        ("nil", Keyword::Nil),
        ("not", Keyword::Not),
        ("or", Keyword::Or),
        ("repeat", Keyword::Repeat),
        ("return", Keyword::Return),
        ("then", Keyword::Then),
        ("true", Keyword::True),
        ("until", Keyword::Until),
        ("while", Keyword::While),
        ("const", Keyword::Const),
        ("close", Keyword::Close),
    ];

    for (source, keyword) in cases {
        assert_scan(
            source,
            &single_line_expected(source, ExpectedKind::Keyword(keyword)),
        )?;
    }

    Ok(())
}

#[test]
fn names_are_case_sensitive_and_do_not_capture_keywords() -> Result<(), String> {
    let cases = ["foo", "_VERSION", "And", "AND", "foo123", "_x"];

    for source in cases {
        assert_scan(source, &single_line_expected(source, ExpectedKind::Name))?;
    }

    Ok(())
}

#[test]
fn punctuators_and_operators_match_manual_section_3_1() -> Result<(), String> {
    let cases = [
        ("+", Punctuator::Plus),
        ("-", Punctuator::Minus),
        ("*", Punctuator::Star),
        ("/", Punctuator::Slash),
        ("%", Punctuator::Percent),
        ("^", Punctuator::Caret),
        ("#", Punctuator::Hash),
        ("&", Punctuator::Ampersand),
        ("~", Punctuator::Tilde),
        ("|", Punctuator::Pipe),
        ("<<", Punctuator::ShiftLeft),
        (">>", Punctuator::ShiftRight),
        ("//", Punctuator::DoubleSlash),
        ("==", Punctuator::EqEq),
        ("~=", Punctuator::NotEq),
        ("<=", Punctuator::LessEq),
        (">=", Punctuator::GreaterEq),
        ("<", Punctuator::Less),
        (">", Punctuator::Greater),
        ("=", Punctuator::Assign),
        ("(", Punctuator::LParen),
        (")", Punctuator::RParen),
        ("{", Punctuator::LBrace),
        ("}", Punctuator::RBrace),
        ("[", Punctuator::LBracket),
        ("]", Punctuator::RBracket),
        ("::", Punctuator::DoubleColon),
        (";", Punctuator::Semicolon),
        (":", Punctuator::Colon),
        (",", Punctuator::Comma),
        (".", Punctuator::Dot),
        ("..", Punctuator::DotDot),
        ("...", Punctuator::DotDotDot),
    ];

    for (source, punctuator) in cases {
        assert_scan(
            source,
            &single_line_expected(source, ExpectedKind::Punctuator(punctuator)),
        )?;
    }

    Ok(())
}

#[test]
fn empty_input_produces_eof() -> Result<(), String> {
    assert_scan("", &[eof(KSpan::new(1, 1, 1, 1))])?;
    Ok(())
}
