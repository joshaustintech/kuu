# Lexer Edge Cases

This file tracks lexer cases that are easy to miss in a hand-written Lua 5.5 scanner.
Each item is paired with a regression test in `tests/lexer_edge_cases.rs`.

## Covered Cases

- First-line `#` prolog comments, including shebang lines, are skipped.
- CRLF line endings count as a single newline for spans.
- Short comments stop at the newline and do not consume the next token.
- Long-bracket strings report an error when the closing delimiter is missing.
- Long-bracket comments report an error when the closing delimiter is missing.
- Decimal escapes that overflow a byte are rejected.
- Hex escapes require exactly two hex digits.
- Empty `\u{}` escapes are rejected.
- `1..2` and `1...2` split into the correct numeric and punctuator tokens.

## Notes

- The lexer API currently tokenizes from `&str`, so the test suite uses lossy decoding only for upstream corpus loading where needed.
- The edge-case tests intentionally pin span behavior for the tricky cases above so future lexer changes stay observable.
