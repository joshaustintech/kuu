use crate::lexer::{LexError, Token};

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Nil,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(Vec<u8>),
    Vararg,
    TableConstructor {
        fields: Vec<TableField>,
    },
    FunctionDef {
        params: Vec<String>,
        is_vararg: bool,
        body: Box<Block>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnOp,
        val: Box<Expr>,
    },
    Prefix(Box<PrefixExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableField {
    KeyVal { key: Expr, val: Expr },
    NameVal { name: String, val: Expr },
    ListVal(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PrefixExpr {
    Identifier(String),
    Index {
        base: Box<PrefixExpr>,
        key: Box<Expr>,
    },
    IndexName {
        base: Box<PrefixExpr>,
        name: String,
    },
    FunctionCall {
        func: Box<PrefixExpr>,
        args: Vec<Expr>,
    },
    MethodCall {
        base: Box<PrefixExpr>,
        method: String,
        args: Vec<Expr>,
    },
    Parens(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    BitAnd,
    BitXor,
    BitOr,
    Shl,
    Shr,
    IDiv,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Len,
    BitNot,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Assign {
        targets: Vec<PrefixExpr>,
        values: Vec<Expr>,
    },
    LocalAssign {
        names: Vec<String>,
        values: Vec<Expr>,
    },
    DoBlock(Block),
    While {
        cond: Expr,
        body: Block,
    },
    Repeat {
        body: Block,
        cond: Expr,
    },
    If {
        cond: Expr,
        then_block: Block,
        elseifs: Vec<(Expr, Block)>,
        else_block: Option<Block>,
    },
    ForNum {
        var: String,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        body: Block,
    },
    ForIn {
        vars: Vec<String>,
        exps: Vec<Expr>,
        body: Block,
    },
    Function {
        name: FuncName,
        params: Vec<String>,
        is_vararg: bool,
        body: Block,
    },
    LocalFunction {
        name: String,
        params: Vec<String>,
        is_vararg: bool,
        body: Block,
    },
    Label(String),
    Break,
    Goto(String),
    Return(Vec<Expr>),
    Expr(PrefixExpr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncName {
    pub parts: Vec<String>,
    pub method: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    Lex(LexError),
    UnexpectedToken(Token),
    UnexpectedEOF,
    ExpectedToken { expected: Token, found: Token },
}

pub struct Parser<'a> {
    lexer: crate::lexer::Lexer<'a>,
    buffer: Vec<crate::lexer::TokenSpan>,
    pos: usize,
}

impl<'a> Parser<'a> {
    pub fn new(lexer: crate::lexer::Lexer<'a>) -> Self {
        Self {
            lexer,
            buffer: Vec::new(),
            pos: 0,
        }
    }

    fn fill_buffer(&mut self, n: usize) -> Result<(), ParseError> {
        while self.buffer.len() <= self.pos + n {
            match self.lexer.next_token().map_err(ParseError::Lex)? {
                Some(span) => self.buffer.push(span),
                None => break,
            }
        }
        Ok(())
    }

    fn peek_token(&mut self) -> Option<Token> {
        self.peek_token_span(0).map(|s| s.token.clone())
    }

    fn peek_token_at(&mut self, offset: usize) -> Option<Token> {
        self.peek_token_span(offset).map(|s| s.token.clone())
    }

    fn peek_token_span(&mut self, offset: usize) -> Option<&crate::lexer::TokenSpan> {
        if self.fill_buffer(offset).is_err() {
            return None;
        }
        if self.pos + offset < self.buffer.len() {
            Some(&self.buffer[self.pos + offset])
        } else {
            None
        }
    }

    fn advance_token(&mut self) -> Option<Token> {
        if self.fill_buffer(0).is_err() {
            return None;
        }
        if self.pos < self.buffer.len() {
            let tok = self.buffer[self.pos].token.clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn expect_token(&mut self, expected: Token) -> Result<(), ParseError> {
        let found = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
        if found == expected {
            Ok(())
        } else {
            Err(ParseError::ExpectedToken { expected, found })
        }
    }

    pub fn parse_chunk(&mut self) -> Result<Block, ParseError> {
        let block = self.parse_block()?;
        if let Some(tok) = self.peek_token() {
            return Err(ParseError::UnexpectedToken(tok));
        }
        Ok(block)
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let mut stmts = Vec::new();
        while let Some(tok) = self.peek_token() {
            if matches!(tok, Token::End | Token::Else | Token::ElseIf | Token::Until) {
                break;
            }
            if tok == Token::Semicolon {
                self.advance_token();
                continue;
            }
            stmts.push(self.parse_stmt()?);
            while self.peek_token() == Some(Token::Semicolon) {
                self.advance_token();
            }
        }
        Ok(Block { stmts })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        let tok = self.peek_token().ok_or(ParseError::UnexpectedEOF)?;
        match tok {
            Token::Local => {
                self.advance_token();
                self.parse_local_stmt()
            }
            Token::Global => {
                self.advance_token();
                self.parse_global_stmt()
            }
            Token::Function => {
                self.advance_token();
                let name = self.parse_funcname()?;
                self.expect_token(Token::LParen)?;
                let (params, is_vararg) = self.parse_parlist()?;
                self.expect_token(Token::RParen)?;
                let body = self.parse_block()?;
                self.expect_token(Token::End)?;
                Ok(Stmt::Function {
                    name,
                    params,
                    is_vararg,
                    body,
                })
            }
            Token::Do => {
                self.advance_token();
                let body = self.parse_block()?;
                self.expect_token(Token::End)?;
                Ok(Stmt::DoBlock(body))
            }
            Token::While => {
                self.advance_token();
                let cond = self.parse_expr()?;
                self.expect_token(Token::Do)?;
                let body = self.parse_block()?;
                self.expect_token(Token::End)?;
                Ok(Stmt::While { cond, body })
            }
            Token::Repeat => {
                self.advance_token();
                let body = self.parse_block()?;
                self.expect_token(Token::Until)?;
                let cond = self.parse_expr()?;
                Ok(Stmt::Repeat { body, cond })
            }
            Token::If => {
                self.advance_token();
                let cond = self.parse_expr()?;
                self.expect_token(Token::Then)?;
                let then_block = self.parse_block()?;
                let mut elseifs = Vec::new();
                while self.peek_token() == Some(Token::ElseIf) {
                    self.advance_token();
                    let cond_ei = self.parse_expr()?;
                    self.expect_token(Token::Then)?;
                    let block_ei = self.parse_block()?;
                    elseifs.push((cond_ei, block_ei));
                }
                let mut else_block = None;
                if self.peek_token() == Some(Token::Else) {
                    self.advance_token();
                    else_block = Some(self.parse_block()?);
                }
                self.expect_token(Token::End)?;
                Ok(Stmt::If {
                    cond,
                    then_block,
                    elseifs,
                    else_block,
                })
            }
            Token::For => {
                self.advance_token();
                let var_tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
                let var = if let Token::Identifier(name) = var_tok {
                    name
                } else {
                    return Err(ParseError::UnexpectedToken(var_tok));
                };

                if self.peek_token() == Some(Token::Assign) {
                    self.advance_token();
                    let start = self.parse_expr()?;
                    self.expect_token(Token::Comma)?;
                    let end = self.parse_expr()?;
                    let mut step = None;
                    if self.peek_token() == Some(Token::Comma) {
                        self.advance_token();
                        step = Some(self.parse_expr()?);
                    }
                    self.expect_token(Token::Do)?;
                    let body = self.parse_block()?;
                    self.expect_token(Token::End)?;
                    Ok(Stmt::ForNum {
                        var,
                        start,
                        end,
                        step,
                        body,
                    })
                } else {
                    let mut vars = vec![var];
                    while self.peek_token() == Some(Token::Comma) {
                        self.advance_token();
                        let v_tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
                        if let Token::Identifier(name) = v_tok {
                            vars.push(name);
                        } else {
                            return Err(ParseError::UnexpectedToken(v_tok));
                        }
                    }
                    self.expect_token(Token::In)?;
                    let exps = self.parse_explist()?;
                    self.expect_token(Token::Do)?;
                    let body = self.parse_block()?;
                    self.expect_token(Token::End)?;
                    Ok(Stmt::ForIn { vars, exps, body })
                }
            }
            Token::DoubleColon => {
                self.advance_token();
                let label_tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
                let label = if let Token::Identifier(name) = label_tok {
                    name
                } else {
                    return Err(ParseError::UnexpectedToken(label_tok));
                };
                self.expect_token(Token::DoubleColon)?;
                Ok(Stmt::Label(label))
            }
            Token::Break => {
                self.advance_token();
                Ok(Stmt::Break)
            }
            Token::Goto => {
                self.advance_token();
                let label_tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
                let label = if let Token::Identifier(name) = label_tok {
                    name
                } else {
                    return Err(ParseError::UnexpectedToken(label_tok));
                };
                Ok(Stmt::Goto(label))
            }
            Token::Return => {
                self.advance_token();
                let mut exps = Vec::new();
                if self.peek_token().is_some_and(|next| {
                    !matches!(
                        next,
                        Token::End | Token::Else | Token::ElseIf | Token::Until | Token::Semicolon
                    )
                }) {
                    exps = self.parse_explist()?;
                }
                if self.peek_token() == Some(Token::Semicolon) {
                    self.advance_token();
                }
                Ok(Stmt::Return(exps))
            }
            _ => {
                let prefix = self.parse_prefix_expr()?;
                if self.peek_token() == Some(Token::Assign)
                    || self.peek_token() == Some(Token::Comma)
                {
                    let mut targets = vec![prefix];
                    while self.peek_token() == Some(Token::Comma) {
                        self.advance_token();
                        let next_prefix = self.parse_prefix_expr()?;
                        targets.push(next_prefix);
                    }
                    self.expect_token(Token::Assign)?;
                    let values = self.parse_explist()?;
                    Ok(Stmt::Assign { targets, values })
                } else {
                    Ok(Stmt::Expr(prefix))
                }
            }
        }
    }

    fn parse_local_stmt(&mut self) -> Result<Stmt, ParseError> {
        let next = self.peek_token().ok_or(ParseError::UnexpectedEOF)?;
        if next == Token::Function {
            self.advance_token();
            let name_tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
            if let Token::Identifier(name) = name_tok {
                self.expect_token(Token::LParen)?;
                let (params, is_vararg) = self.parse_parlist()?;
                self.expect_token(Token::RParen)?;
                let body = self.parse_block()?;
                self.expect_token(Token::End)?;
                return Ok(Stmt::LocalFunction {
                    name,
                    params,
                    is_vararg,
                    body,
                });
            }
            return Err(ParseError::UnexpectedToken(name_tok));
        }

        let names = self.parse_attnamelist()?;
        let mut values = Vec::new();
        if self.peek_token() == Some(Token::Assign) {
            self.advance_token();
            values = self.parse_explist()?;
        }
        Ok(Stmt::LocalAssign { names, values })
    }

    fn parse_global_stmt(&mut self) -> Result<Stmt, ParseError> {
        if self.peek_token() == Some(Token::Function) {
            self.advance_token();
            let name = self.parse_funcname()?;
            self.expect_token(Token::LParen)?;
            let (params, is_vararg) = self.parse_parlist()?;
            self.expect_token(Token::RParen)?;
            let body = self.parse_block()?;
            self.expect_token(Token::End)?;
            return Ok(Stmt::Function {
                name,
                params,
                is_vararg,
                body,
            });
        }

        self.parse_optional_attribute()?;
        if self.peek_token() == Some(Token::Mul) {
            self.advance_token();
            return Ok(empty_stmt());
        }

        let names = self.parse_attnamelist_after_optional_prefix()?;
        if self.peek_token() == Some(Token::Assign) {
            self.advance_token();
            let values = self.parse_explist()?;
            let targets = names.into_iter().map(PrefixExpr::Identifier).collect();
            Ok(Stmt::Assign { targets, values })
        } else {
            Ok(empty_stmt())
        }
    }

    fn parse_attnamelist(&mut self) -> Result<Vec<String>, ParseError> {
        self.parse_optional_attribute()?;
        self.parse_attnamelist_after_optional_prefix()
    }

    fn parse_attnamelist_after_optional_prefix(&mut self) -> Result<Vec<String>, ParseError> {
        let mut names = Vec::new();
        loop {
            let name_tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
            if let Token::Identifier(name) = name_tok {
                names.push(name);
            } else {
                return Err(ParseError::UnexpectedToken(name_tok));
            }
            self.parse_optional_attribute()?;
            if self.peek_token() == Some(Token::Comma) {
                self.advance_token();
            } else {
                break;
            }
        }
        Ok(names)
    }

    fn parse_optional_attribute(&mut self) -> Result<(), ParseError> {
        if self.peek_token() != Some(Token::Lt) {
            return Ok(());
        }

        self.advance_token();
        let attr_tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
        if !matches!(attr_tok, Token::Identifier(_)) {
            return Err(ParseError::UnexpectedToken(attr_tok));
        }
        self.expect_token(Token::Gt)
    }

    fn parse_explist(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut exps = vec![self.parse_expr()?];
        while self.peek_token() == Some(Token::Comma) {
            self.advance_token();
            exps.push(self.parse_expr()?);
        }
        Ok(exps)
    }

    fn parse_funcname(&mut self) -> Result<FuncName, ParseError> {
        let tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
        let mut parts = if let Token::Identifier(name) = tok {
            vec![name]
        } else {
            return Err(ParseError::UnexpectedToken(tok));
        };

        while self.peek_token() == Some(Token::Dot) {
            self.advance_token();
            let tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
            if let Token::Identifier(name) = tok {
                parts.push(name);
            } else {
                return Err(ParseError::UnexpectedToken(tok));
            }
        }

        let mut method = None;
        if self.peek_token() == Some(Token::Colon) {
            self.advance_token();
            let tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
            if let Token::Identifier(name) = tok {
                method = Some(name);
            } else {
                return Err(ParseError::UnexpectedToken(tok));
            }
        }

        Ok(FuncName { parts, method })
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_expr_bp(0)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary_expr()?;

        while let Some(tok) = self.peek_token() {
            if let Some((l_bp, r_bp)) = binop_precedence(&tok) {
                if l_bp < min_bp {
                    break;
                }
                self.advance_token();
                let op = match tok {
                    Token::Add => BinOp::Add,
                    Token::Sub => BinOp::Sub,
                    Token::Mul => BinOp::Mul,
                    Token::Div => BinOp::Div,
                    Token::Mod => BinOp::Mod,
                    Token::Pow => BinOp::Pow,
                    Token::BitAnd => BinOp::BitAnd,
                    Token::BitXor => BinOp::BitXor,
                    Token::BitOr => BinOp::BitOr,
                    Token::Shl => BinOp::Shl,
                    Token::Shr => BinOp::Shr,
                    Token::IDiv => BinOp::IDiv,
                    Token::Concat => BinOp::Concat,
                    Token::Eq => BinOp::Eq,
                    Token::Ne => BinOp::Ne,
                    Token::Lt => BinOp::Lt,
                    Token::Le => BinOp::Le,
                    Token::Gt => BinOp::Gt,
                    Token::Ge => BinOp::Ge,
                    Token::And => BinOp::And,
                    Token::Or => BinOp::Or,
                    _ => unreachable!(),
                };
                let rhs = self.parse_expr_bp(r_bp)?;
                lhs = Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                };
                continue;
            }
            break;
        }
        Ok(lhs)
    }

    fn parse_unary_expr(&mut self) -> Result<Expr, ParseError> {
        if let Some(op) = self.peek_token().and_then(|tok| match_unop(&tok)) {
            self.advance_token();
            let val = self.parse_expr_bp(21)?;
            return Ok(Expr::Unary {
                op,
                val: Box::new(val),
            });
        }
        self.parse_primary_expr()
    }

    fn parse_primary_expr(&mut self) -> Result<Expr, ParseError> {
        let tok = self.peek_token().ok_or(ParseError::UnexpectedEOF)?;
        match tok {
            Token::Nil => {
                self.advance_token();
                Ok(Expr::Nil)
            }
            Token::True => {
                self.advance_token();
                Ok(Expr::Boolean(true))
            }
            Token::False => {
                self.advance_token();
                Ok(Expr::Boolean(false))
            }
            Token::Integer(val) => {
                self.advance_token();
                Ok(Expr::Integer(val))
            }
            Token::Float(val) => {
                self.advance_token();
                Ok(Expr::Float(val))
            }
            Token::String(val) => {
                self.advance_token();
                Ok(Expr::String(val))
            }
            Token::Vararg => {
                self.advance_token();
                Ok(Expr::Vararg)
            }
            Token::LBrace => self.parse_table_constructor(),
            Token::Function => {
                self.advance_token();
                self.parse_function_def()
            }
            Token::Identifier(_) | Token::LParen => {
                let prefix = self.parse_prefix_expr()?;
                Ok(Expr::Prefix(Box::new(prefix)))
            }
            _ => Err(ParseError::UnexpectedToken(tok)),
        }
    }

    fn parse_prefix_expr(&mut self) -> Result<PrefixExpr, ParseError> {
        let tok = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
        let mut base = match tok {
            Token::Identifier(name) => PrefixExpr::Identifier(name),
            Token::LParen => {
                let expr = self.parse_expr()?;
                self.expect_token(Token::RParen)?;
                PrefixExpr::Parens(Box::new(expr))
            }
            _ => return Err(ParseError::UnexpectedToken(tok)),
        };

        loop {
            match self.peek_token() {
                Some(Token::LBracket) => {
                    self.advance_token();
                    let key = self.parse_expr()?;
                    self.expect_token(Token::RBracket)?;
                    base = PrefixExpr::Index {
                        base: Box::new(base),
                        key: Box::new(key),
                    };
                }
                Some(Token::Dot) => {
                    self.advance_token();
                    let next = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
                    if let Token::Identifier(name) = next {
                        base = PrefixExpr::IndexName {
                            base: Box::new(base),
                            name,
                        };
                    } else {
                        return Err(ParseError::UnexpectedToken(next));
                    }
                }
                Some(Token::Colon) => {
                    self.advance_token();
                    let next = self.advance_token().ok_or(ParseError::UnexpectedEOF)?;
                    if let Token::Identifier(method) = next {
                        let args = self.parse_args()?;
                        base = PrefixExpr::MethodCall {
                            base: Box::new(base),
                            method,
                            args,
                        };
                    } else {
                        return Err(ParseError::UnexpectedToken(next));
                    }
                }
                Some(Token::LParen) | Some(Token::LBrace) | Some(Token::String(_)) => {
                    let args = self.parse_args()?;
                    base = PrefixExpr::FunctionCall {
                        func: Box::new(base),
                        args,
                    };
                }
                _ => break,
            }
        }
        Ok(base)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let tok = self.peek_token().ok_or(ParseError::UnexpectedEOF)?;
        match tok {
            Token::LParen => {
                self.advance_token();
                let mut args = Vec::new();
                if self.peek_token() != Some(Token::RParen) {
                    args = self.parse_explist()?;
                }
                self.expect_token(Token::RParen)?;
                Ok(args)
            }
            Token::LBrace => {
                let table = self.parse_table_constructor()?;
                Ok(vec![table])
            }
            Token::String(val) => {
                self.advance_token();
                Ok(vec![Expr::String(val)])
            }
            _ => Err(ParseError::UnexpectedToken(tok)),
        }
    }

    fn parse_table_constructor(&mut self) -> Result<Expr, ParseError> {
        self.expect_token(Token::LBrace)?;
        let mut fields = Vec::new();
        loop {
            if self.peek_token() == Some(Token::RBrace) {
                break;
            }
            fields.push(self.parse_table_field()?);
            if let Some(tok) = self.peek_token() {
                if tok == Token::Comma || tok == Token::Semicolon {
                    self.advance_token();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        self.expect_token(Token::RBrace)?;
        Ok(Expr::TableConstructor { fields })
    }

    fn parse_table_field(&mut self) -> Result<TableField, ParseError> {
        let tok = self.peek_token().ok_or(ParseError::UnexpectedEOF)?;
        match tok {
            Token::LBracket => {
                self.advance_token();
                let key = self.parse_expr()?;
                self.expect_token(Token::RBracket)?;
                self.expect_token(Token::Assign)?;
                let val = self.parse_expr()?;
                Ok(TableField::KeyVal { key, val })
            }
            Token::Identifier(name) => {
                if self.peek_token_at(1) == Some(Token::Assign) {
                    self.advance_token();
                    self.advance_token();
                    let val = self.parse_expr()?;
                    Ok(TableField::NameVal { name, val })
                } else {
                    let expr = self.parse_expr()?;
                    Ok(TableField::ListVal(expr))
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                Ok(TableField::ListVal(expr))
            }
        }
    }

    fn parse_function_def(&mut self) -> Result<Expr, ParseError> {
        self.expect_token(Token::LParen)?;
        let (params, is_vararg) = self.parse_parlist()?;
        self.expect_token(Token::RParen)?;
        let body = self.parse_block()?;
        self.expect_token(Token::End)?;
        Ok(Expr::FunctionDef {
            params,
            is_vararg,
            body: Box::new(body),
        })
    }

    fn parse_parlist(&mut self) -> Result<(Vec<String>, bool), ParseError> {
        let mut params = Vec::new();
        let mut is_vararg = false;
        while let Some(tok) = self.peek_token() {
            match tok {
                Token::RParen => break,
                Token::Vararg => {
                    self.advance_token();
                    is_vararg = true;
                    if let Some(Token::Identifier(_)) = self.peek_token() {
                        self.advance_token();
                    }
                    break;
                }
                Token::Identifier(name) => {
                    self.advance_token();
                    params.push(name);
                    if self.peek_token() == Some(Token::Comma) {
                        self.advance_token();
                    } else {
                        break;
                    }
                }
                _ => return Err(ParseError::UnexpectedToken(tok)),
            }
        }
        Ok((params, is_vararg))
    }
}

fn empty_stmt() -> Stmt {
    Stmt::DoBlock(Block { stmts: Vec::new() })
}

fn binop_precedence(token: &Token) -> Option<(u8, u8)> {
    match token {
        Token::Or => Some((1, 2)),
        Token::And => Some((3, 4)),
        Token::Lt | Token::Gt | Token::Le | Token::Ge | Token::Ne | Token::Eq => Some((5, 6)),
        Token::BitOr => Some((7, 8)),
        Token::BitXor => Some((9, 10)),
        Token::BitAnd => Some((11, 12)),
        Token::Shl | Token::Shr => Some((13, 14)),
        Token::Concat => Some((16, 15)),
        Token::Add | Token::Sub => Some((17, 18)),
        Token::Mul | Token::Div | Token::IDiv | Token::Mod => Some((19, 20)),
        Token::Pow => Some((22, 21)),
        _ => None,
    }
}

fn match_unop(tok: &Token) -> Option<UnOp> {
    match tok {
        Token::Sub => Some(UnOp::Neg),
        Token::Not => Some(UnOp::Not),
        Token::Len => Some(UnOp::Len),
        Token::BitXor => Some(UnOp::BitNot),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    #[test]
    fn test_parser_basic_expressions() {
        let lex = Lexer::new(b"return 10 + 20 * 30");
        let mut parser = Parser::new(lex);
        let block = parser.parse_chunk().unwrap();

        // return 10 + (20 * 30)
        assert_eq!(
            block.stmts[0],
            Stmt::Return(vec![Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(Expr::Integer(10)),
                rhs: Box::new(Expr::Binary {
                    op: BinOp::Mul,
                    lhs: Box::new(Expr::Integer(20)),
                    rhs: Box::new(Expr::Integer(30)),
                })
            }])
        );
    }

    #[test]
    fn test_parser_statements() {
        let lex = Lexer::new(b"local x = 42\nif x == 42 then\n  print(x)\nend");
        let mut parser = Parser::new(lex);
        let block = parser.parse_chunk().unwrap();

        assert_eq!(block.stmts.len(), 2);
        assert_eq!(
            block.stmts[0],
            Stmt::LocalAssign {
                names: vec!["x".to_string()],
                values: vec![Expr::Integer(42)],
            }
        );
    }
}
