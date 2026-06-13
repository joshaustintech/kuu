use crate::error::{KError, KResult, KSpan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: KSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Eof,
    Name(String),
    Keyword(Keyword),
    Integer(String),
    Float(String),
    String(Vec<u8>),
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
pub enum Keyword {
    And,
    Break,
    Do,
    Else,
    ElseIf,
    End,
    False,
    For,
    Function,
    Global,
    Goto,
    If,
    In,
    Local,
    Nil,
    Not,
    Or,
    Repeat,
    Return,
    Then,
    True,
    Until,
    While,
    Const,
    Close,
}

#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    source: &'a str,
    index: usize,
    line: usize,
    column: usize,
    last_line: usize,
    last_column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            index: 0,
            line: 1,
            column: 1,
            last_line: 1,
            last_column: 1,
        }
    }

    pub fn next_token(&mut self) -> KResult<Token> {
        self.skip_trivia()?;

        let start_index = self.index;
        let start_line = self.line;
        let start_column = self.column;

        let Some(ch) = self.peek_char(0) else {
            return Ok(Token {
                kind: TokenKind::Eof,
                lexeme: String::new(),
                span: KSpan::new(start_line, start_column, start_line, start_column),
            });
        };

        if ch == '\'' || ch == '"' {
            return self.scan_short_string(ch, start_index, start_line, start_column);
        }

        if ch == '.' {
            if self.peek_char(1).is_some_and(|next| next.is_ascii_digit()) {
                return self.scan_number(start_index, start_line, start_column);
            }
            return self.scan_punctuation(start_index, start_line, start_column);
        }

        if ch.is_ascii_digit() {
            return self.scan_number(start_index, start_line, start_column);
        }

        if is_name_start(ch) {
            return Ok(self.scan_name_or_keyword(start_index, start_line, start_column));
        }

        self.scan_punctuation(start_index, start_line, start_column)
    }

    fn scan_name_or_keyword(
        &mut self,
        start_index: usize,
        start_line: usize,
        start_column: usize,
    ) -> Token {
        let _ = self.consume_char();
        while self.peek_char(0).is_some_and(is_name_continue) {
            let _ = self.consume_char();
        }

        let lexeme = self.slice(start_index, self.index);
        let kind = match lexeme.as_str() {
            "and" => TokenKind::Keyword(Keyword::And),
            "break" => TokenKind::Keyword(Keyword::Break),
            "do" => TokenKind::Keyword(Keyword::Do),
            "else" => TokenKind::Keyword(Keyword::Else),
            "elseif" => TokenKind::Keyword(Keyword::ElseIf),
            "end" => TokenKind::Keyword(Keyword::End),
            "false" => TokenKind::Keyword(Keyword::False),
            "for" => TokenKind::Keyword(Keyword::For),
            "function" => TokenKind::Keyword(Keyword::Function),
            "global" => TokenKind::Keyword(Keyword::Global),
            "goto" => TokenKind::Keyword(Keyword::Goto),
            "if" => TokenKind::Keyword(Keyword::If),
            "in" => TokenKind::Keyword(Keyword::In),
            "local" => TokenKind::Keyword(Keyword::Local),
            "nil" => TokenKind::Keyword(Keyword::Nil),
            "not" => TokenKind::Keyword(Keyword::Not),
            "or" => TokenKind::Keyword(Keyword::Or),
            "repeat" => TokenKind::Keyword(Keyword::Repeat),
            "return" => TokenKind::Keyword(Keyword::Return),
            "then" => TokenKind::Keyword(Keyword::Then),
            "true" => TokenKind::Keyword(Keyword::True),
            "until" => TokenKind::Keyword(Keyword::Until),
            "while" => TokenKind::Keyword(Keyword::While),
            "const" => TokenKind::Keyword(Keyword::Const),
            "close" => TokenKind::Keyword(Keyword::Close),
            _ => TokenKind::Name(lexeme.clone()),
        };

        self.finish_token(kind, start_index, start_line, start_column)
    }

    fn scan_number(
        &mut self,
        start_index: usize,
        start_line: usize,
        start_column: usize,
    ) -> KResult<Token> {
        let mut is_hex = false;
        let mut saw_dot = false;
        let mut saw_exp = false;
        let mut saw_digits = false;

        if self.peek_char(0) == Some('0') && matches!(self.peek_char(1), Some('x' | 'X')) {
            is_hex = true;
            let _ = self.consume_char();
            let _ = self.consume_char();
            saw_digits |= self.consume_digits(16);
            if self.peek_char(0) == Some('.') && self.peek_char(1) != Some('.') {
                saw_dot = true;
                let _ = self.consume_char();
                saw_digits |= self.consume_digits(16);
            }
            if matches!(self.peek_char(0), Some('p' | 'P')) {
                saw_exp = true;
                let _ = self.consume_char();
                if matches!(self.peek_char(0), Some('+' | '-')) {
                    let _ = self.consume_char();
                }
                if !self.consume_digits(10) {
                    return Err(self.syntax_error("malformed numeral", start_line, start_column));
                }
            }
            if !saw_digits || self.has_attached_trailing_junk() {
                return Err(self.syntax_error("malformed numeral", start_line, start_column));
            }
        } else if self.peek_char(0) == Some('.') {
            saw_dot = true;
            let _ = self.consume_char();
            if !self.consume_digits(10) {
                return Err(self.syntax_error("malformed numeral", start_line, start_column));
            }
            if matches!(self.peek_char(0), Some('e' | 'E')) {
                saw_exp = true;
                let _ = self.consume_char();
                if matches!(self.peek_char(0), Some('+' | '-')) {
                    let _ = self.consume_char();
                }
                if !self.consume_digits(10) {
                    return Err(self.syntax_error("malformed numeral", start_line, start_column));
                }
            }
            if self.has_attached_trailing_junk() {
                return Err(self.syntax_error("malformed numeral", start_line, start_column));
            }
        } else {
            saw_digits = self.consume_digits(10);
            if !saw_digits {
                return Err(self.syntax_error("malformed numeral", start_line, start_column));
            }
            if self.peek_char(0) == Some('.') && self.peek_char(1) != Some('.') {
                saw_dot = true;
                let _ = self.consume_char();
                let _ = self.consume_digits(10);
            }
            if matches!(self.peek_char(0), Some('e' | 'E')) {
                saw_exp = true;
                let _ = self.consume_char();
                if matches!(self.peek_char(0), Some('+' | '-')) {
                    let _ = self.consume_char();
                }
                if !self.consume_digits(10) {
                    return Err(self.syntax_error("malformed numeral", start_line, start_column));
                }
            }
            if self.has_attached_trailing_junk() {
                return Err(self.syntax_error("malformed numeral", start_line, start_column));
            }
        }

        let lexeme = self.slice(start_index, self.index);
        let kind = if is_hex {
            if saw_dot || saw_exp {
                TokenKind::Float(lexeme.clone())
            } else {
                TokenKind::Integer(lexeme.clone())
            }
        } else if saw_dot || saw_exp {
            TokenKind::Float(lexeme.clone())
        } else {
            TokenKind::Integer(lexeme.clone())
        };

        Ok(self.finish_token(kind, start_index, start_line, start_column))
    }

    fn scan_short_string(
        &mut self,
        quote: char,
        start_index: usize,
        start_line: usize,
        start_column: usize,
    ) -> KResult<Token> {
        let _ = self.consume_char();
        let mut value = Vec::new();

        loop {
            let Some(ch) = self.peek_char(0) else {
                return Err(self.syntax_error("unterminated string", start_line, start_column));
            };

            if ch == quote {
                let _ = self.consume_char();
                return Ok(self.finish_token(
                    TokenKind::String(value),
                    start_index,
                    start_line,
                    start_column,
                ));
            }

            if is_line_break(ch) {
                return Err(self.syntax_error("unterminated string", start_line, start_column));
            }

            if ch != '\\' {
                self.push_char(&mut value, ch);
                let _ = self.consume_char();
                continue;
            }

            let _ = self.consume_char();
            let Some(esc) = self.peek_char(0) else {
                return Err(self.syntax_error(
                    "unterminated string escape",
                    start_line,
                    start_column,
                ));
            };

            match esc {
                'a' => {
                    value.push(0x07);
                    let _ = self.consume_char();
                }
                'b' => {
                    value.push(0x08);
                    let _ = self.consume_char();
                }
                'f' => {
                    value.push(0x0c);
                    let _ = self.consume_char();
                }
                'n' => {
                    value.push(b'\n');
                    let _ = self.consume_char();
                }
                'r' => {
                    value.push(b'\r');
                    let _ = self.consume_char();
                }
                't' => {
                    value.push(b'\t');
                    let _ = self.consume_char();
                }
                'v' => {
                    value.push(0x0b);
                    let _ = self.consume_char();
                }
                '\\' => {
                    value.push(b'\\');
                    let _ = self.consume_char();
                }
                '"' => {
                    value.push(b'"');
                    let _ = self.consume_char();
                }
                '\'' => {
                    value.push(b'\'');
                    let _ = self.consume_char();
                }
                '\n' | '\r' => {
                    self.consume_newline();
                    value.push(b'\n');
                }
                'z' => {
                    let _ = self.consume_char();
                    self.skip_whitespace_only();
                }
                'x' => {
                    let _ = self.consume_char();
                    let hi = self.read_hex_digit(start_line, start_column)?;
                    let lo = self.read_hex_digit(start_line, start_column)?;
                    value.push((hi << 4) | lo);
                }
                'u' => {
                    let _ = self.consume_char();
                    if self.peek_char(0) != Some('{') {
                        return Err(self.syntax_error(
                            "invalid unicode escape",
                            start_line,
                            start_column,
                        ));
                    }
                    let _ = self.consume_char();
                    let codepoint = self.read_unicode_escape(start_line, start_column)?;
                    let Some(encoded) = encode_lua_utf8(codepoint) else {
                        return Err(self.syntax_error(
                            "invalid unicode escape",
                            start_line,
                            start_column,
                        ));
                    };
                    value.extend_from_slice(&encoded);
                }
                c if c.is_ascii_digit() => {
                    let byte = self.read_decimal_escape(start_line, start_column)?;
                    value.push(byte);
                }
                _ => {
                    return Err(self.syntax_error(
                        "invalid escape sequence",
                        start_line,
                        start_column,
                    ));
                }
            }
        }
    }

    fn scan_punctuation(
        &mut self,
        start_index: usize,
        start_line: usize,
        start_column: usize,
    ) -> KResult<Token> {
        let kind = match self.peek_char(0) {
            Some('+') => {
                let _ = self.consume_char();
                TokenKind::Plus
            }
            Some('-') => {
                let _ = self.consume_char();
                TokenKind::Minus
            }
            Some('*') => {
                let _ = self.consume_char();
                TokenKind::Star
            }
            Some('/') => {
                let _ = self.consume_char();
                if self.peek_char(0) == Some('/') {
                    let _ = self.consume_char();
                    TokenKind::DoubleSlash
                } else {
                    TokenKind::Slash
                }
            }
            Some('%') => {
                let _ = self.consume_char();
                TokenKind::Percent
            }
            Some('^') => {
                let _ = self.consume_char();
                TokenKind::Caret
            }
            Some('#') => {
                let _ = self.consume_char();
                TokenKind::Hash
            }
            Some('&') => {
                let _ = self.consume_char();
                TokenKind::Ampersand
            }
            Some('~') => {
                let _ = self.consume_char();
                if self.peek_char(0) == Some('=') {
                    let _ = self.consume_char();
                    TokenKind::NotEq
                } else {
                    TokenKind::Tilde
                }
            }
            Some('|') => {
                let _ = self.consume_char();
                TokenKind::Pipe
            }
            Some('<') => {
                let _ = self.consume_char();
                if self.peek_char(0) == Some('<') {
                    let _ = self.consume_char();
                    TokenKind::ShiftLeft
                } else if self.peek_char(0) == Some('=') {
                    let _ = self.consume_char();
                    TokenKind::LessEq
                } else {
                    TokenKind::Less
                }
            }
            Some('>') => {
                let _ = self.consume_char();
                if self.peek_char(0) == Some('>') {
                    let _ = self.consume_char();
                    TokenKind::ShiftRight
                } else if self.peek_char(0) == Some('=') {
                    let _ = self.consume_char();
                    TokenKind::GreaterEq
                } else {
                    TokenKind::Greater
                }
            }
            Some('=') => {
                let _ = self.consume_char();
                if self.peek_char(0) == Some('=') {
                    let _ = self.consume_char();
                    TokenKind::EqEq
                } else {
                    TokenKind::Assign
                }
            }
            Some('(') => {
                let _ = self.consume_char();
                TokenKind::LParen
            }
            Some(')') => {
                let _ = self.consume_char();
                TokenKind::RParen
            }
            Some('{') => {
                let _ = self.consume_char();
                TokenKind::LBrace
            }
            Some('}') => {
                let _ = self.consume_char();
                TokenKind::RBrace
            }
            Some('[') => {
                if let Some((level, opener_len)) = self.long_bracket_level_at_current() {
                    self.consume_char_n(opener_len);
                    let value = self.scan_long_bracket_content(level)?;
                    return Ok(self.finish_token(
                        TokenKind::String(value),
                        start_index,
                        start_line,
                        start_column,
                    ));
                }
                let _ = self.consume_char();
                TokenKind::LBracket
            }
            Some(']') => {
                let _ = self.consume_char();
                TokenKind::RBracket
            }
            Some(':') => {
                let _ = self.consume_char();
                if self.peek_char(0) == Some(':') {
                    let _ = self.consume_char();
                    TokenKind::DoubleColon
                } else {
                    TokenKind::Colon
                }
            }
            Some(';') => {
                let _ = self.consume_char();
                TokenKind::Semicolon
            }
            Some(',') => {
                let _ = self.consume_char();
                TokenKind::Comma
            }
            Some('.') => {
                let _ = self.consume_char();
                if self.peek_char(0) == Some('.') {
                    let _ = self.consume_char();
                    if self.peek_char(0) == Some('.') {
                        let _ = self.consume_char();
                        TokenKind::DotDotDot
                    } else {
                        TokenKind::DotDot
                    }
                } else {
                    TokenKind::Dot
                }
            }
            _ => {
                return Err(self.syntax_error("unexpected character", start_line, start_column));
            }
        };

        Ok(self.finish_token(kind, start_index, start_line, start_column))
    }

    fn skip_trivia(&mut self) -> KResult<()> {
        loop {
            if self.index == 0
                && self.line == 1
                && self.column == 1
                && self.peek_char(0) == Some('#')
                && self.peek_char(1) == Some('!')
            {
                self.skip_short_comment();
            }
            match self.peek_char(0) {
                Some(ch) if ch.is_whitespace() => {
                    self.skip_whitespace_only();
                }
                Some('-') if self.peek_char(1) == Some('-') => {
                    let _ = self.consume_char();
                    let _ = self.consume_char();
                    if let Some((level, opener_len)) = self.long_bracket_level_at_current() {
                        self.consume_char_n(opener_len);
                        let _ = self.scan_long_bracket_content(level)?;
                    } else {
                        self.skip_short_comment();
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn skip_short_comment(&mut self) {
        while let Some(ch) = self.peek_char(0) {
            if is_line_break(ch) {
                break;
            }
            let _ = self.consume_char();
        }
    }

    fn scan_long_bracket_content(&mut self, level: usize) -> KResult<Vec<u8>> {
        let mut value = Vec::new();

        if matches!(self.peek_char(0), Some('\n' | '\r')) {
            self.consume_newline();
        }

        loop {
            let Some(ch) = self.peek_char(0) else {
                return Err(self.syntax_error(
                    "unterminated long string or comment",
                    self.line,
                    self.column,
                ));
            };

            if ch == ']' && self.long_bracket_close_at_current(level) {
                self.consume_char_n(level + 2);
                return Ok(value);
            }

            if is_line_break(ch) {
                self.consume_newline();
                value.push(b'\n');
                continue;
            }

            self.push_char(&mut value, ch);
            let _ = self.consume_char();
        }
    }

    fn long_bracket_level_at_current(&self) -> Option<(usize, usize)> {
        let mut chars = self.source[self.index..].chars();
        match chars.next()? {
            '[' => {}
            _ => return None,
        }
        let mut consumed = 1usize;
        let mut level = 0usize;
        loop {
            match chars.clone().next() {
                Some('=') => {
                    let _ = chars.next();
                    consumed += 1;
                    level += 1;
                }
                Some('[') => {
                    let _ = chars.next();
                    consumed += 1;
                    return Some((level, consumed));
                }
                _ => return None,
            }
        }
    }

    fn long_bracket_close_at_current(&self, level: usize) -> bool {
        let mut chars = self.source[self.index..].chars();
        if chars.next() != Some(']') {
            return false;
        }
        for _ in 0..level {
            if chars.next() != Some('=') {
                return false;
            }
        }
        chars.next() == Some(']')
    }

    fn consume_char_n(&mut self, count: usize) {
        for _ in 0..count {
            let _ = self.consume_char();
        }
    }

    fn consume_char(&mut self) -> Option<char> {
        let ch = self.peek_char(0)?;
        self.last_line = self.line;
        self.last_column = self.column;
        self.index += ch.len_utf8();
        self.column += 1;
        Some(ch)
    }

    fn consume_newline(&mut self) {
        let Some(first) = self.peek_char(0) else {
            return;
        };
        self.last_line = self.line;
        self.last_column = self.column;
        self.index += first.len_utf8();
        if let Some(second) = self.peek_char(0)
            && is_matching_newline_pair(first, second)
        {
            self.index += second.len_utf8();
        }
        self.line += 1;
        self.column = 1;
    }

    fn skip_whitespace_only(&mut self) {
        while let Some(ch) = self.peek_char(0) {
            if !ch.is_whitespace() {
                break;
            }
            if is_line_break(ch) {
                self.consume_newline();
            } else {
                let _ = self.consume_char();
            }
        }
    }

    fn consume_digits(&mut self, radix: u32) -> bool {
        let mut consumed = false;
        while let Some(ch) = self.peek_char(0) {
            let valid = match radix {
                10 => ch.is_ascii_digit(),
                16 => ch.is_ascii_hexdigit(),
                _ => false,
            };
            if !valid {
                break;
            }
            consumed = true;
            let _ = self.consume_char();
        }
        consumed
    }

    fn has_attached_trailing_junk(&self) -> bool {
        match self.peek_char(0) {
            Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' => true,
            Some('.') if self.peek_char(1) != Some('.') => true,
            _ => false,
        }
    }

    fn read_hex_digit(&mut self, start_line: usize, start_column: usize) -> KResult<u8> {
        let Some(ch) = self.peek_char(0) else {
            return Err(self.syntax_error("invalid hex escape", start_line, start_column));
        };
        let Some(value) = ch.to_digit(16) else {
            return Err(self.syntax_error("invalid hex escape", start_line, start_column));
        };
        let _ = self.consume_char();
        Ok(value as u8)
    }

    fn read_decimal_escape(&mut self, start_line: usize, start_column: usize) -> KResult<u8> {
        let mut value = 0u16;
        let mut count = 0u8;
        while let Some(ch) = self.peek_char(0) {
            if !ch.is_ascii_digit() || count == 3 {
                break;
            }
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(u16::from(ch as u8 - b'0')))
                .ok_or_else(|| {
                    self.syntax_error("invalid decimal escape", start_line, start_column)
                })?;
            let _ = self.consume_char();
            count += 1;
        }
        if value > u16::from(u8::MAX) {
            return Err(self.syntax_error("invalid decimal escape", start_line, start_column));
        }
        Ok(value as u8)
    }

    fn read_unicode_escape(&mut self, start_line: usize, start_column: usize) -> KResult<u32> {
        let mut value = 0u32;
        let mut seen_digit = false;
        loop {
            let Some(ch) = self.peek_char(0) else {
                return Err(self.syntax_error(
                    "unterminated unicode escape",
                    start_line,
                    start_column,
                ));
            };
            if ch == '}' {
                let _ = self.consume_char();
                return if seen_digit {
                    Ok(value)
                } else {
                    Err(self.syntax_error("invalid unicode escape", start_line, start_column))
                };
            }
            let Some(digit) = ch.to_digit(16) else {
                return Err(self.syntax_error("invalid unicode escape", start_line, start_column));
            };
            seen_digit = true;
            value = value
                .checked_mul(16)
                .and_then(|v| v.checked_add(digit))
                .ok_or_else(|| {
                    self.syntax_error("invalid unicode escape", start_line, start_column)
                })?;
            let _ = self.consume_char();
        }
    }

    fn push_char(&self, bytes: &mut Vec<u8>, ch: char) {
        let mut buf = [0u8; 4];
        bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }

    fn peek_char(&self, lookahead: usize) -> Option<char> {
        self.source[self.index..].chars().nth(lookahead)
    }

    fn slice(&self, start: usize, end: usize) -> String {
        self.source[start..end].to_owned()
    }

    fn finish_token(
        &self,
        kind: TokenKind,
        start_index: usize,
        start_line: usize,
        start_column: usize,
    ) -> Token {
        Token {
            kind,
            lexeme: self.slice(start_index, self.index),
            span: KSpan::new(start_line, start_column, self.last_line, self.last_column),
        }
    }

    fn syntax_error(
        &self,
        message: &'static str,
        start_line: usize,
        start_column: usize,
    ) -> KError {
        KError::syntax(
            message,
            KSpan::new(start_line, start_column, self.line, self.column),
        )
    }
}

fn is_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_name_continue(ch: char) -> bool {
    is_name_start(ch) || ch.is_ascii_digit()
}

fn is_line_break(ch: char) -> bool {
    matches!(ch, '\n' | '\r')
}

fn is_matching_newline_pair(first: char, second: char) -> bool {
    matches!((first, second), ('\n', '\r') | ('\r', '\n'))
}

fn encode_lua_utf8(codepoint: u32) -> Option<Vec<u8>> {
    if codepoint >= 0x8000_0000 {
        return None;
    }

    let bytes = if codepoint <= 0x7f {
        vec![codepoint as u8]
    } else if codepoint <= 0x7ff {
        vec![
            0b1100_0000 | ((codepoint >> 6) as u8),
            0b1000_0000 | ((codepoint & 0b0011_1111) as u8),
        ]
    } else if codepoint <= 0xffff {
        vec![
            0b1110_0000 | ((codepoint >> 12) as u8),
            0b1000_0000 | (((codepoint >> 6) & 0b0011_1111) as u8),
            0b1000_0000 | ((codepoint & 0b0011_1111) as u8),
        ]
    } else if codepoint <= 0x1f_ffff {
        vec![
            0b1111_0000 | ((codepoint >> 18) as u8),
            0b1000_0000 | (((codepoint >> 12) & 0b0011_1111) as u8),
            0b1000_0000 | (((codepoint >> 6) & 0b0011_1111) as u8),
            0b1000_0000 | ((codepoint & 0b0011_1111) as u8),
        ]
    } else if codepoint <= 0x3ff_ffff {
        vec![
            0b1111_1000 | ((codepoint >> 24) as u8),
            0b1000_0000 | (((codepoint >> 18) & 0b0011_1111) as u8),
            0b1000_0000 | (((codepoint >> 12) & 0b0011_1111) as u8),
            0b1000_0000 | (((codepoint >> 6) & 0b0011_1111) as u8),
            0b1000_0000 | ((codepoint & 0b0011_1111) as u8),
        ]
    } else {
        vec![
            0b1111_1100 | ((codepoint >> 30) as u8),
            0b1000_0000 | (((codepoint >> 24) & 0b0011_1111) as u8),
            0b1000_0000 | (((codepoint >> 18) & 0b0011_1111) as u8),
            0b1000_0000 | (((codepoint >> 12) & 0b0011_1111) as u8),
            0b1000_0000 | (((codepoint >> 6) & 0b0011_1111) as u8),
            0b1000_0000 | ((codepoint & 0b0011_1111) as u8),
        ]
    };

    Some(bytes)
}
