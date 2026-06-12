#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
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

    // Operators and Punctuation
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Len,
    BitAnd,
    BitXor,
    BitOr,
    Shl,
    Shr,
    IDiv,
    Eq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
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
    Concat,
    Vararg,

    // Literals & Identifiers
    Identifier(String),
    Integer(i64),
    Float(f64),
    String(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TokenSpan {
    pub token: Token,
    pub start: SourceLocation,
    pub end: SourceLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexError {
    UnterminatedString,
    UnterminatedLongString,
    InvalidEscapeSequence,
    InvalidNumber,
    UnexpectedChar,
}

pub struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.peek_byte(0)
    }

    fn peek_byte(&self, offset: usize) -> Option<u8> {
        if self.pos + offset < self.input.len() {
            Some(self.input[self.pos + offset])
        } else {
            None
        }
    }

    fn advance(&mut self) -> Option<u8> {
        if self.pos < self.input.len() {
            let b = self.input[self.pos];
            self.pos += 1;
            if b == b'\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
            Some(b)
        } else {
            None
        }
    }

    fn advance_by(&mut self, n: usize) {
        for _ in 0..n {
            self.advance();
        }
    }

    fn current_location(&self) -> SourceLocation {
        SourceLocation {
            line: self.line,
            column: self.column,
        }
    }

    pub fn next_token(&mut self) -> Result<Option<TokenSpan>, LexError> {
        self.skip_whitespace_and_comments()?;

        if self.pos >= self.input.len() {
            return Ok(None);
        }

        let start = self.current_location();

        // Check for long string bracket
        if let Some(level) = self.match_long_bracket() {
            let content = self.scan_long_string(level)?;
            let end = self.current_location();
            return Ok(Some(TokenSpan {
                token: Token::String(content),
                start,
                end,
            }));
        }

        let first = self.peek().unwrap();

        // 1. Quoted Strings
        if first == b'"' || first == b'\'' {
            let content = self.scan_quoted_string(first)?;
            let end = self.current_location();
            return Ok(Some(TokenSpan {
                token: Token::String(content),
                start,
                end,
            }));
        }

        // 2. Numbers starting with dot (e.g. .5)
        if first == b'.' && self.peek_byte(1).is_some_and(|b| b.is_ascii_digit()) {
            let token = self.scan_number()?;
            let end = self.current_location();
            return Ok(Some(TokenSpan { token, start, end }));
        }

        // 3. Numbers starting with digit
        if first.is_ascii_digit() {
            let token = self.scan_number()?;
            let end = self.current_location();
            return Ok(Some(TokenSpan { token, start, end }));
        }

        // 4. Identifiers and Keywords
        if first.is_ascii_alphabetic() || first == b'_' {
            let ident_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_alphanumeric() || b == b'_' {
                    self.advance();
                } else {
                    break;
                }
            }
            let raw = &self.input[ident_start..self.pos];
            let ident = std::str::from_utf8(raw).unwrap().to_string();
            let token = match ident.as_str() {
                "and" => Token::And,
                "break" => Token::Break,
                "do" => Token::Do,
                "else" => Token::Else,
                "elseif" => Token::ElseIf,
                "end" => Token::End,
                "false" => Token::False,
                "for" => Token::For,
                "function" => Token::Function,
                "global" => Token::Global,
                "goto" => Token::Goto,
                "if" => Token::If,
                "in" => Token::In,
                "local" => Token::Local,
                "nil" => Token::Nil,
                "not" => Token::Not,
                "or" => Token::Or,
                "repeat" => Token::Repeat,
                "return" => Token::Return,
                "then" => Token::Then,
                "true" => Token::True,
                "until" => Token::Until,
                "while" => Token::While,
                _ => Token::Identifier(ident),
            };
            let end = self.current_location();
            return Ok(Some(TokenSpan { token, start, end }));
        }

        // 5. Operators and Punctuations
        self.advance(); // consume first byte
        let token = match first {
            b'+' => Token::Add,
            b'-' => Token::Sub,
            b'*' => Token::Mul,
            b'%' => Token::Mod,
            b'^' => Token::Pow,
            b'#' => Token::Len,
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'{' => Token::LBrace,
            b'}' => Token::RBrace,
            b']' => Token::RBracket,
            b';' => Token::Semicolon,
            b',' => Token::Comma,
            b'&' => Token::BitAnd,
            b'|' => Token::BitOr,

            b'/' => {
                if self.peek() == Some(b'/') {
                    self.advance();
                    Token::IDiv
                } else {
                    Token::Div
                }
            }

            b'~' => {
                if self.peek() == Some(b'=') {
                    self.advance();
                    Token::Ne
                } else {
                    Token::BitXor
                }
            }

            b'=' => {
                if self.peek() == Some(b'=') {
                    self.advance();
                    Token::Eq
                } else {
                    Token::Assign
                }
            }

            b'<' => {
                if self.peek() == Some(b'<') {
                    self.advance();
                    Token::Shl
                } else if self.peek() == Some(b'=') {
                    self.advance();
                    Token::Le
                } else {
                    Token::Lt
                }
            }

            b'>' => {
                if self.peek() == Some(b'>') {
                    self.advance();
                    Token::Shr
                } else if self.peek() == Some(b'=') {
                    self.advance();
                    Token::Ge
                } else {
                    Token::Gt
                }
            }

            b':' => {
                if self.peek() == Some(b':') {
                    self.advance();
                    Token::DoubleColon
                } else {
                    Token::Colon
                }
            }

            b'[' => Token::LBracket,

            b'.' => {
                if self.peek() == Some(b'.') {
                    self.advance();
                    if self.peek() == Some(b'.') {
                        self.advance();
                        Token::Vararg
                    } else {
                        Token::Concat
                    }
                } else {
                    Token::Dot
                }
            }

            _ => return Err(LexError::UnexpectedChar),
        };

        let end = self.current_location();
        Ok(Some(TokenSpan { token, start, end }))
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<(), LexError> {
        loop {
            if self.pos >= self.input.len() {
                break;
            }
            let b = self.input[self.pos];
            if b == b'#' && self.pos == 0 && self.line == 1 && self.column == 1 {
                while let Some(next) = self.peek() {
                    if next == b'\n' || next == b'\r' {
                        break;
                    }
                    self.advance();
                }
            } else if b.is_ascii_whitespace() {
                self.advance();
            } else if b == b'-' && self.peek_byte(1) == Some(b'-') {
                self.advance(); // consume '-'
                self.advance(); // consume '-'
                if let Some(level) = self.match_long_bracket() {
                    self.scan_long_string(level)?;
                } else {
                    while let Some(next) = self.peek() {
                        if next == b'\n' || next == b'\r' {
                            break;
                        }
                        self.advance();
                    }
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn match_long_bracket(&self) -> Option<usize> {
        if self.peek_byte(0) != Some(b'[') {
            return None;
        }
        let mut n = 0;
        while self.peek_byte(n + 1) == Some(b'=') {
            n += 1;
        }
        if self.peek_byte(n + 1) == Some(b'[') {
            Some(n)
        } else {
            None
        }
    }

    fn scan_long_string(&mut self, level: usize) -> Result<Vec<u8>, LexError> {
        self.advance_by(2 + level); // consume opening bracket

        // Discard first newline directly following the bracket
        if self.peek() == Some(b'\n') {
            self.advance();
        } else if self.peek() == Some(b'\r') {
            self.advance();
            if self.peek() == Some(b'\n') {
                self.advance();
            }
        }

        let mut content = Vec::new();
        loop {
            let b = match self.advance() {
                Some(b) => b,
                None => return Err(LexError::UnterminatedLongString),
            };

            if b == b']' {
                let mut matches = true;
                for i in 0..level {
                    if self.peek_byte(i) != Some(b'=') {
                        matches = false;
                        break;
                    }
                }
                if matches && self.peek_byte(level) == Some(b']') {
                    self.advance_by(level + 1);
                    break;
                }
            }
            content.push(b);
        }
        Ok(content)
    }

    fn scan_quoted_string(&mut self, quote: u8) -> Result<Vec<u8>, LexError> {
        self.advance(); // consume quote
        let mut content = Vec::new();
        loop {
            let b = match self.advance() {
                Some(b) => b,
                None => return Err(LexError::UnterminatedString),
            };

            if b == quote {
                break;
            }

            if b == b'\\' {
                let esc = match self.advance() {
                    Some(esc) => esc,
                    None => return Err(LexError::UnterminatedString),
                };

                match esc {
                    b'a' => content.push(7),
                    b'b' => content.push(8),
                    b'f' => content.push(12),
                    b'n' => content.push(10),
                    b'r' => content.push(13),
                    b't' => content.push(9),
                    b'v' => content.push(11),
                    b'\\' => content.push(b'\\'),
                    b'"' => content.push(b'"'),
                    b'\'' => content.push(b'\''),
                    b'\n' => content.push(b'\n'),
                    b'\r' => {
                        if self.peek() == Some(b'\n') {
                            self.advance();
                        }
                        content.push(b'\n');
                    }
                    b'z' => {
                        while let Some(next) = self.peek() {
                            if next.is_ascii_whitespace() {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    b'x' => {
                        let h1 = match self.advance() {
                            Some(h) if h.is_ascii_hexdigit() => h,
                            _ => return Err(LexError::InvalidEscapeSequence),
                        };
                        let h2 = match self.advance() {
                            Some(h) if h.is_ascii_hexdigit() => h,
                            _ => return Err(LexError::InvalidEscapeSequence),
                        };
                        let byte = u8::from_str_radix(std::str::from_utf8(&[h1, h2]).unwrap(), 16)
                            .unwrap();
                        content.push(byte);
                    }
                    b'u' => {
                        if self.advance() != Some(b'{') {
                            return Err(LexError::InvalidEscapeSequence);
                        }
                        let mut hex_digits = Vec::new();
                        loop {
                            match self.advance() {
                                Some(b'}') => break,
                                Some(h) if h.is_ascii_hexdigit() => hex_digits.push(h),
                                _ => return Err(LexError::InvalidEscapeSequence),
                            }
                        }
                        if hex_digits.is_empty() || hex_digits.len() > 8 {
                            return Err(LexError::InvalidEscapeSequence);
                        }
                        let hex_str = std::str::from_utf8(&hex_digits).unwrap();
                        let codepoint = u32::from_str_radix(hex_str, 16)
                            .map_err(|_| LexError::InvalidEscapeSequence)?;
                        encode_lua_utf8_escape(codepoint, &mut content)?;
                    }
                    b'0'..=b'9' => {
                        let mut digits = vec![esc];
                        if self.peek().is_some_and(|b| b.is_ascii_digit()) {
                            digits.push(self.advance().unwrap());
                            if self.peek().is_some_and(|b| b.is_ascii_digit()) {
                                digits.push(self.advance().unwrap());
                            }
                        }
                        let decimal_str = std::str::from_utf8(&digits).unwrap();
                        let val = decimal_str.parse::<u32>().unwrap();
                        if val > 255 {
                            return Err(LexError::InvalidEscapeSequence);
                        }
                        content.push(val as u8);
                    }
                    _ => return Err(LexError::InvalidEscapeSequence),
                }
            } else {
                content.push(b);
            }
        }
        Ok(content)
    }

    fn scan_number(&mut self) -> Result<Token, LexError> {
        let start_pos = self.pos;
        let is_hex = if self.peek_byte(0) == Some(b'0')
            && (self.peek_byte(1) == Some(b'x') || self.peek_byte(1) == Some(b'X'))
        {
            self.advance();
            self.advance();
            true
        } else {
            false
        };

        let mut has_dot = false;
        let mut has_exponent = false;

        if is_hex {
            while let Some(b) = self.peek() {
                match b {
                    b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' => {
                        self.advance();
                    }
                    b'.' => {
                        if has_dot {
                            break;
                        }
                        if self.peek_byte(1) == Some(b'.') {
                            break;
                        }
                        has_dot = true;
                        self.advance();
                    }
                    b'p' | b'P' => {
                        if has_exponent {
                            break;
                        }
                        has_exponent = true;
                        self.advance();
                        if self.peek().is_some_and(|next| next == b'+' || next == b'-') {
                            self.advance();
                        }
                    }
                    _ => break,
                }
            }
        } else {
            while let Some(b) = self.peek() {
                match b {
                    b'0'..=b'9' => {
                        self.advance();
                    }
                    b'.' => {
                        if has_dot {
                            break;
                        }
                        if self.peek_byte(1) == Some(b'.') {
                            break;
                        }
                        has_dot = true;
                        self.advance();
                    }
                    b'e' | b'E' => {
                        if has_exponent {
                            break;
                        }
                        has_exponent = true;
                        self.advance();
                        if self.peek().is_some_and(|next| next == b'+' || next == b'-') {
                            self.advance();
                        }
                    }
                    _ => break,
                }
            }
        }

        let raw = &self.input[start_pos..self.pos];
        let num_str = std::str::from_utf8(raw).map_err(|_| LexError::InvalidNumber)?;

        if is_hex {
            if has_dot || has_exponent {
                let float_val = parse_hex_float(num_str)?;
                Ok(Token::Float(float_val))
            } else {
                let hex_digits = &num_str[2..];
                match u64::from_str_radix(hex_digits, 16) {
                    Ok(val) => Ok(Token::Integer(val as i64)),
                    Err(_) => {
                        let val = parse_hex_float(num_str)?;
                        Ok(Token::Float(val))
                    }
                }
            }
        } else if has_dot || has_exponent {
            let val = num_str
                .parse::<f64>()
                .map_err(|_| LexError::InvalidNumber)?;
            Ok(Token::Float(val))
        } else {
            match num_str.parse::<i64>() {
                Ok(val) => Ok(Token::Integer(val)),
                Err(_) => {
                    let val = num_str
                        .parse::<f64>()
                        .map_err(|_| LexError::InvalidNumber)?;
                    Ok(Token::Float(val))
                }
            }
        }
    }
}

fn encode_lua_utf8_escape(codepoint: u32, out: &mut Vec<u8>) -> Result<(), LexError> {
    match codepoint {
        0x0000..=0x007f => out.push(codepoint as u8),
        0x0080..=0x07ff => {
            out.push((0xc0 | (codepoint >> 6)) as u8);
            out.push((0x80 | (codepoint & 0x3f)) as u8);
        }
        0x0800..=0xffff => {
            out.push((0xe0 | (codepoint >> 12)) as u8);
            out.push((0x80 | ((codepoint >> 6) & 0x3f)) as u8);
            out.push((0x80 | (codepoint & 0x3f)) as u8);
        }
        0x1_0000..=0x1f_ffff => {
            out.push((0xf0 | (codepoint >> 18)) as u8);
            out.push((0x80 | ((codepoint >> 12) & 0x3f)) as u8);
            out.push((0x80 | ((codepoint >> 6) & 0x3f)) as u8);
            out.push((0x80 | (codepoint & 0x3f)) as u8);
        }
        0x20_0000..=0x3ff_ffff => {
            out.push((0xf8 | (codepoint >> 24)) as u8);
            out.push((0x80 | ((codepoint >> 18) & 0x3f)) as u8);
            out.push((0x80 | ((codepoint >> 12) & 0x3f)) as u8);
            out.push((0x80 | ((codepoint >> 6) & 0x3f)) as u8);
            out.push((0x80 | (codepoint & 0x3f)) as u8);
        }
        0x400_0000..=0x7fff_ffff => {
            out.push((0xfc | (codepoint >> 30)) as u8);
            out.push((0x80 | ((codepoint >> 24) & 0x3f)) as u8);
            out.push((0x80 | ((codepoint >> 18) & 0x3f)) as u8);
            out.push((0x80 | ((codepoint >> 12) & 0x3f)) as u8);
            out.push((0x80 | ((codepoint >> 6) & 0x3f)) as u8);
            out.push((0x80 | (codepoint & 0x3f)) as u8);
        }
        _ => return Err(LexError::InvalidEscapeSequence),
    }
    Ok(())
}

fn parse_hex_float(s: &str) -> Result<f64, LexError> {
    let s = if s.starts_with("0x") || s.starts_with("0X") {
        &s[2..]
    } else {
        s
    };

    let (significand, exponent) = if let Some(p_idx) = s.find(['p', 'P']) {
        let significand = &s[..p_idx];
        let exponent_str = &s[p_idx + 1..];
        let exponent = exponent_str
            .parse::<i32>()
            .map_err(|_| LexError::InvalidNumber)?;
        (significand, exponent)
    } else {
        (s, 0)
    };

    let (int_part, frac_part) = if let Some(dot_idx) = significand.find('.') {
        (&significand[..dot_idx], &significand[dot_idx + 1..])
    } else {
        (significand, "")
    };

    if int_part.is_empty() && frac_part.is_empty() {
        return Err(LexError::InvalidNumber);
    }

    let mut int_val = 0.0;
    for c in int_part.chars() {
        let digit_val = c.to_digit(16).ok_or(LexError::InvalidNumber)? as f64;
        int_val = int_val * 16.0 + digit_val;
    }

    let mut frac_val = 0.0;
    let mut base = 1.0 / 16.0;
    for c in frac_part.chars() {
        let digit_val = c.to_digit(16).ok_or(LexError::InvalidNumber)? as f64;
        frac_val += digit_val * base;
        base /= 16.0;
    }

    let total_significand = int_val + frac_val;
    Ok(total_significand * (exponent as f64).exp2())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_basic() {
        let mut lex = Lexer::new(b"local x = 123  -- comment\n  local y = 4.56");

        let t1 = lex.next_token().unwrap().unwrap();
        assert_eq!(t1.token, Token::Local);

        let t2 = lex.next_token().unwrap().unwrap();
        assert_eq!(t2.token, Token::Identifier("x".to_string()));

        let t3 = lex.next_token().unwrap().unwrap();
        assert_eq!(t3.token, Token::Assign);

        let t4 = lex.next_token().unwrap().unwrap();
        assert_eq!(t4.token, Token::Integer(123));

        let t5 = lex.next_token().unwrap().unwrap();
        assert_eq!(t5.token, Token::Local);

        let t6 = lex.next_token().unwrap().unwrap();
        assert_eq!(t6.token, Token::Identifier("y".to_string()));

        let t7 = lex.next_token().unwrap().unwrap();
        assert_eq!(t7.token, Token::Assign);

        let t8 = lex.next_token().unwrap().unwrap();
        assert_eq!(t8.token, Token::Float(4.56));

        assert!(lex.next_token().unwrap().is_none());
    }

    #[test]
    fn test_lexer_strings() {
        let mut lex = Lexer::new(b"\"hello\\nworld\" 'foo' [=[bar]=]");

        let t1 = lex.next_token().unwrap().unwrap();
        assert_eq!(t1.token, Token::String(b"hello\nworld".to_vec()));

        let t2 = lex.next_token().unwrap().unwrap();
        assert_eq!(t2.token, Token::String(b"foo".to_vec()));

        let t3 = lex.next_token().unwrap().unwrap();
        assert_eq!(t3.token, Token::String(b"bar".to_vec()));
    }

    #[test]
    fn test_lexer_hex_float() {
        let mut lex = Lexer::new(b"0x1.5p-2 0x1p3 0x1.8");

        let t1 = lex.next_token().unwrap().unwrap();
        assert_eq!(t1.token, Token::Float(0.328125));

        let t2 = lex.next_token().unwrap().unwrap();
        assert_eq!(t2.token, Token::Float(8.0));

        let t3 = lex.next_token().unwrap().unwrap();
        assert_eq!(t3.token, Token::Float(1.5));
    }
}
