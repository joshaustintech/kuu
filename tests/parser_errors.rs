mod parser_support;

use parser_support::assert_parse_error_contains;

#[test]
fn unknown_attributes_are_rejected() -> Result<(), String> {
    assert_parse_error_contains("local x <XXX> = 10", "unknown attribute")
}

#[test]
fn multiple_close_attributes_are_rejected() -> Result<(), String> {
    assert_parse_error_contains("local <close> a, b", "multiple")
}

#[test]
fn duplicate_postfix_close_attributes_are_rejected() -> Result<(), String> {
    assert_parse_error_contains("local a<close>, b<close>", "multiple")
}

#[test]
fn invalid_for_syntax_is_rejected() -> Result<(), String> {
    assert_parse_error_contains("for x do end", "expected")
}

#[test]
fn invalid_method_statement_is_rejected() -> Result<(), String> {
    assert_parse_error_contains("x:call", "expected call arguments")
}

#[test]
fn incomplete_expression_is_rejected() -> Result<(), String> {
    assert_parse_error_contains("a.", "expected name")
}

#[test]
fn return_must_end_its_block() -> Result<(), String> {
    assert_parse_error_contains("do return 1; break end", "last statement")
}

#[test]
fn vararg_cannot_be_followed_by_more_parameters() -> Result<(), String> {
    assert_parse_error_contains("function f(a, ...rest, b) end", "expected")
}

#[test]
fn close_attributes_are_rejected_on_globals() -> Result<(), String> {
    assert_parse_error_contains("global<close> *", "global variables")
}
