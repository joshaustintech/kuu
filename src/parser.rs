use crate::ast::{
    Attribute, AttributedName, BinaryOp, Block, CallExpr, Chunk, Expr, FunctionBody, FunctionName,
    Param, ReturnStmt, Stmt, TableConstructor, TableField, UnaryOp, Var, VarKind,
};
use crate::error::{KError, KResult, KSpan};
use crate::lexer::{Keyword, Lexer, Token, TokenKind};
use std::mem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Assoc {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockStop {
    Eof,
    End,
    ElseIfOrElseOrEnd,
    Until,
}

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token,
    next: Token,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str) -> KResult<Self> {
        let mut lexer = Lexer::new(source);
        let current = lexer.next_token()?;
        let next = lexer.next_token()?;
        Ok(Self {
            lexer,
            current,
            next,
        })
    }

    fn advance(&mut self) -> KResult<()> {
        self.current = mem::replace(&mut self.next, self.lexer.next_token()?);
        Ok(())
    }

    pub fn parse_chunk(&mut self) -> KResult<Chunk> {
        let block = self.parse_block(BlockStop::Eof)?;
        if !self.is_eof() {
            return Err(self.error("unexpected trailing input"));
        }
        Ok(Chunk { block })
    }

    fn parse_block(&mut self, stop: BlockStop) -> KResult<Block> {
        let start = self.current.span;
        let mut statements = Vec::new();
        let mut return_stmt = None;
        let mut last_span = start;

        while !self.is_block_stop(stop) {
            if self.at_punct(TokenKind::Semicolon) {
                let span = self.current.span;
                self.advance()?;
                statements.push(Stmt::Empty { span });
                last_span = span;
                continue;
            }

            if self.at_keyword(Keyword::Return) {
                let stmt = self.parse_return_statement()?;
                last_span = stmt.span;
                return_stmt = Some(stmt);
                break;
            }

            let stmt = self.parse_statement()?;
            last_span = self.stmt_span(&stmt);
            statements.push(stmt);
        }

        Ok(Block {
            span: self.merge_span(start, last_span),
            statements,
            return_stmt,
        })
    }

    fn parse_statement(&mut self) -> KResult<Stmt> {
        if self.at_keyword(Keyword::Break) {
            let span = self.current.span;
            self.advance()?;
            return Ok(Stmt::Break { span });
        }

        if self.at_keyword(Keyword::Goto) {
            let start = self.current.span;
            self.advance()?;
            let (name, name_span) = self.consume_name()?;
            return Ok(Stmt::Goto {
                span: self.merge_span(start, name_span),
                name,
            });
        }

        if self.at_keyword(Keyword::Do) {
            let start = self.current.span;
            self.advance()?;
            let block = self.parse_block(BlockStop::End)?;
            let end = self.consume_keyword(Keyword::End)?;
            return Ok(Stmt::Do {
                span: self.merge_span(start, end.span),
                block,
            });
        }

        if self.at_keyword(Keyword::While) {
            let start = self.current.span;
            self.advance()?;
            let condition = self.parse_expression(0)?;
            self.consume_keyword(Keyword::Do)?;
            let block = self.parse_block(BlockStop::End)?;
            let end = self.consume_keyword(Keyword::End)?;
            return Ok(Stmt::While {
                span: self.merge_span(start, end.span),
                condition,
                block,
            });
        }

        if self.at_keyword(Keyword::Repeat) {
            let start = self.current.span;
            self.advance()?;
            let block = self.parse_block(BlockStop::Until)?;
            self.consume_keyword(Keyword::Until)?;
            let condition = self.parse_expression(0)?;
            let span = self.merge_span(start, self.expr_span(&condition));
            return Ok(Stmt::Repeat {
                span,
                block,
                condition,
            });
        }

        if self.at_keyword(Keyword::If) {
            return self.parse_if_statement();
        }

        if self.at_keyword(Keyword::For) {
            return self.parse_for_statement();
        }

        if self.at_keyword(Keyword::Function) {
            return self.parse_function_statement();
        }

        if self.at_keyword(Keyword::Local) {
            return self.parse_local_statement();
        }

        if self.at_keyword(Keyword::Global) {
            if matches!(self.next.kind, TokenKind::Assign) {
                let name_span = self.current.span;
                self.advance()?;
                self.consume_punct(TokenKind::Assign)?;
                let values = self.parse_expression_list()?;
                let end = self.expr_span(
                    values
                        .last()
                        .ok_or_else(|| self.error("missing global assignment value"))?,
                );
                return Ok(Stmt::Assign {
                    span: self.merge_span(name_span, end),
                    targets: vec![Var {
                        span: name_span,
                        kind: VarKind::Name("global".to_owned()),
                    }],
                    values,
                });
            }
            return self.parse_global_statement();
        }

        if self.at_punct(TokenKind::DoubleColon) {
            let start = self.current.span;
            self.advance()?;
            let (name, _) = self.consume_name()?;
            let end = self.consume_punct(TokenKind::DoubleColon)?;
            return Ok(Stmt::Label {
                span: self.merge_span(start, end.span),
                name,
            });
        }

        self.parse_assignment_or_call_statement()
    }

    fn is_eof(&self) -> bool {
        matches!(self.current_kind(), TokenKind::Eof)
    }

    fn current_kind(&self) -> &TokenKind {
        &self.current.kind
    }

    fn next_kind(&self) -> &TokenKind {
        &self.next.kind
    }

    fn is_block_stop(&self, stop: BlockStop) -> bool {
        match stop {
            BlockStop::Eof => self.is_eof(),
            BlockStop::End => self.at_keyword(Keyword::End),
            BlockStop::ElseIfOrElseOrEnd => {
                self.at_keyword(Keyword::End)
                    || self.at_keyword(Keyword::Else)
                    || self.at_keyword(Keyword::ElseIf)
                    || self.is_eof()
            }
            BlockStop::Until => self.at_keyword(Keyword::Until) || self.is_eof(),
        }
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.current_kind(), TokenKind::Keyword(current) if *current == keyword)
    }

    fn at_punct(&self, kind: TokenKind) -> bool {
        self.current.kind == kind
    }

    fn error(&self, message: impl Into<String>) -> KError {
        KError::syntax(message, self.current.span)
    }

    fn merge_span(&self, start: KSpan, end: KSpan) -> KSpan {
        KSpan::new(
            start.start_line,
            start.start_column,
            end.end_line,
            end.end_column,
        )
    }

    fn stmt_span(&self, stmt: &Stmt) -> KSpan {
        match stmt {
            Stmt::Empty { span }
            | Stmt::Break { span }
            | Stmt::Goto { span, .. }
            | Stmt::Label { span, .. }
            | Stmt::Do { span, .. }
            | Stmt::While { span, .. }
            | Stmt::Repeat { span, .. }
            | Stmt::If { span, .. }
            | Stmt::NumericFor { span, .. }
            | Stmt::GenericFor { span, .. }
            | Stmt::Function { span, .. }
            | Stmt::LocalFunction { span, .. }
            | Stmt::GlobalFunction { span, .. }
            | Stmt::LocalDecl { span, .. }
            | Stmt::GlobalDecl { span, .. }
            | Stmt::GlobalAll { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::Call { span, .. } => *span,
        }
    }

    fn expr_span(&self, expr: &Expr) -> KSpan {
        match expr {
            Expr::Nil { span }
            | Expr::Bool { span, .. }
            | Expr::Number { span, .. }
            | Expr::String { span, .. }
            | Expr::Vararg { span, .. }
            | Expr::Name { span, .. }
            | Expr::Paren { span, .. }
            | Expr::Field { span, .. }
            | Expr::Index { span, .. }
            | Expr::Table { span, .. }
            | Expr::Function { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Call { span, .. } => *span,
        }
    }

    fn consume_keyword(&mut self, keyword: Keyword) -> KResult<Token> {
        if !self.at_keyword(keyword) {
            return Err(self.error(format!("expected {:?}", keyword)));
        }
        let token = self.current.clone();
        self.advance()?;
        Ok(token)
    }

    fn consume_punct(&mut self, kind: TokenKind) -> KResult<Token> {
        if !self.at_punct(kind.clone()) {
            return Err(self.error(format!("expected {:?}", kind)));
        }
        let token = self.current.clone();
        self.advance()?;
        Ok(token)
    }

    fn consume_name(&mut self) -> KResult<(String, KSpan)> {
        self.consume_name_like("expected name")
    }

    fn consume_attribute_name(&mut self) -> KResult<(String, KSpan)> {
        self.consume_name_like("expected attribute name")
    }

    fn consume_name_like(&mut self, error: &'static str) -> KResult<(String, KSpan)> {
        match &self.current.kind {
            TokenKind::Name(name) => {
                let span = self.current.span;
                let name = name.clone();
                self.advance()?;
                Ok((name, span))
            }
            TokenKind::Keyword(Keyword::Const) => {
                let span = self.current.span;
                self.advance()?;
                Ok(("const".to_owned(), span))
            }
            TokenKind::Keyword(Keyword::Close) => {
                let span = self.current.span;
                self.advance()?;
                Ok(("close".to_owned(), span))
            }
            TokenKind::Keyword(Keyword::Global) => {
                let span = self.current.span;
                self.advance()?;
                Ok(("global".to_owned(), span))
            }
            _ => Err(self.error(error)),
        }
    }

    fn parse_return_statement(&mut self) -> KResult<ReturnStmt> {
        let start = self.current.span;
        self.consume_keyword(Keyword::Return)?;
        let mut values = Vec::new();
        if self.can_start_expression() {
            values = self.parse_expression_list()?;
        }
        if self.at_punct(TokenKind::Semicolon) {
            self.advance()?;
        }
        let end = if values.is_empty() {
            start
        } else {
            self.expr_span(
                values
                    .last()
                    .ok_or_else(|| self.error("missing return value"))?,
            )
        };
        if !(self.is_block_stop(BlockStop::Eof)
            || self.is_block_stop(BlockStop::End)
            || self.is_block_stop(BlockStop::ElseIfOrElseOrEnd)
            || self.is_block_stop(BlockStop::Until)
            || self.at_punct(TokenKind::Semicolon))
        {
            return Err(self.error("return must be the last statement in a block"));
        }
        Ok(ReturnStmt {
            span: self.merge_span(start, end),
            values,
        })
    }

    fn parse_if_statement(&mut self) -> KResult<Stmt> {
        let start = self.current.span;
        self.consume_keyword(Keyword::If)?;
        let condition = self.parse_expression(0)?;
        self.consume_keyword(Keyword::Then)?;
        let mut branches = Vec::new();
        let first_block = self.parse_block(BlockStop::ElseIfOrElseOrEnd)?;
        branches.push((condition, first_block));

        while self.at_keyword(Keyword::ElseIf) {
            self.advance()?;
            let condition = self.parse_expression(0)?;
            self.consume_keyword(Keyword::Then)?;
            let block = self.parse_block(BlockStop::ElseIfOrElseOrEnd)?;
            branches.push((condition, block));
        }

        let else_block = if self.at_keyword(Keyword::Else) {
            self.advance()?;
            Some(self.parse_block(BlockStop::End)?)
        } else {
            None
        };

        let end = self.consume_keyword(Keyword::End)?;
        Ok(Stmt::If {
            span: self.merge_span(start, end.span),
            branches,
            else_block,
        })
    }

    fn parse_for_statement(&mut self) -> KResult<Stmt> {
        let start = self.current.span;
        self.consume_keyword(Keyword::For)?;
        let (name, _) = self.consume_name()?;

        if self.at_punct(TokenKind::Assign) {
            self.advance()?;
            let start_expr = self.parse_expression(0)?;
            self.consume_punct(TokenKind::Comma)?;
            let end_expr = self.parse_expression(0)?;
            let step_expr = if self.at_punct(TokenKind::Comma) {
                self.advance()?;
                Some(self.parse_expression(0)?)
            } else {
                None
            };
            self.consume_keyword(Keyword::Do)?;
            let block = self.parse_block(BlockStop::End)?;
            let end = self.consume_keyword(Keyword::End)?;
            return Ok(Stmt::NumericFor {
                span: self.merge_span(start, end.span),
                name,
                start: start_expr,
                end: end_expr,
                step: step_expr,
                block,
            });
        }

        let mut names = vec![name];
        while self.at_punct(TokenKind::Comma) {
            self.advance()?;
            let (next_name, _) = self.consume_name()?;
            names.push(next_name);
        }
        self.consume_keyword(Keyword::In)?;
        let iter = self.parse_expression_list()?;
        self.consume_keyword(Keyword::Do)?;
        let block = self.parse_block(BlockStop::End)?;
        let end = self.consume_keyword(Keyword::End)?;
        Ok(Stmt::GenericFor {
            span: self.merge_span(start, end.span),
            names,
            iter,
            block,
        })
    }

    fn parse_function_statement(&mut self) -> KResult<Stmt> {
        let start = self.current.span;
        self.consume_keyword(Keyword::Function)?;
        let name = self.parse_function_name()?;
        let mut body = self.parse_function_body()?;
        if name.method.is_some() {
            body.parameters.insert(
                0,
                Param {
                    span: name.span,
                    name: "self".to_owned(),
                },
            );
        }
        Ok(Stmt::Function {
            span: self.merge_span(start, body.span),
            name,
            body,
        })
    }

    fn parse_local_statement(&mut self) -> KResult<Stmt> {
        let start = self.current.span;
        self.consume_keyword(Keyword::Local)?;
        if self.at_keyword(Keyword::Function) {
            self.advance()?;
            let (name, _) = self.consume_name()?;
            let body = self.parse_function_body()?;
            return Ok(Stmt::LocalFunction {
                span: self.merge_span(start, body.span),
                name,
                body,
            });
        }

        let prefix_attribute = if self.at_punct(TokenKind::Less) {
            Some(self.parse_attribute()?)
        } else {
            None
        };
        let (prefix_attribute, names) =
            self.parse_attributed_name_list_from(prefix_attribute, true)?;
        let values = if self.at_punct(TokenKind::Assign) {
            self.advance()?;
            self.parse_expression_list()?
        } else {
            Vec::new()
        };
        Ok(Stmt::LocalDecl {
            span: self.merge_span(
                start,
                if values.is_empty() {
                    self.attributed_names_end_span(&names)
                } else {
                    self.expr_span(
                        values
                            .last()
                            .ok_or_else(|| self.error("missing local value"))?,
                    )
                },
            ),
            prefix_attribute,
            names,
            values,
        })
    }

    fn parse_global_statement(&mut self) -> KResult<Stmt> {
        let start = self.current.span;
        self.consume_keyword(Keyword::Global)?;
        if self.at_keyword(Keyword::Function) {
            self.advance()?;
            let (name, _) = self.consume_name()?;
            let body = self.parse_function_body()?;
            return Ok(Stmt::GlobalFunction {
                span: self.merge_span(start, body.span),
                name,
                body,
            });
        }

        let prefix_attribute = if self.at_punct(TokenKind::Less) {
            Some(self.parse_attribute()?)
        } else {
            None
        };

        if self.at_punct(TokenKind::Star) {
            let star_span = self.current.span;
            self.advance()?;
            if matches!(prefix_attribute.as_ref(), Some(attr) if attr.name == "close") {
                return Err(self.error("attribute <close> cannot be used on global variables"));
            }
            return Ok(Stmt::GlobalAll {
                span: self.merge_span(start, star_span),
                prefix_attribute,
            });
        }

        let (prefix_attribute, names) =
            self.parse_attributed_name_list_from(prefix_attribute, false)?;
        let values = if self.at_punct(TokenKind::Assign) {
            self.advance()?;
            self.parse_expression_list()?
        } else {
            Vec::new()
        };
        Ok(Stmt::GlobalDecl {
            span: self.merge_span(
                start,
                if values.is_empty() {
                    self.attributed_names_end_span(&names)
                } else {
                    self.expr_span(
                        values
                            .last()
                            .ok_or_else(|| self.error("missing global value"))?,
                    )
                },
            ),
            prefix_attribute,
            names,
            values,
        })
    }

    fn parse_assignment_or_call_statement(&mut self) -> KResult<Stmt> {
        let expr = self.parse_expression(0)?;
        if self.at_punct(TokenKind::Assign) || self.at_punct(TokenKind::Comma) {
            let first_var = self.expr_to_var(expr)?;
            let mut targets = vec![first_var];
            while self.at_punct(TokenKind::Comma) {
                self.advance()?;
                let target = self.parse_expression(0)?;
                targets.push(self.expr_to_var(target)?);
            }
            self.consume_punct(TokenKind::Assign)?;
            let values = self.parse_expression_list()?;
            let span = self.merge_span(
                self.stmt_span_from_var_list(&targets),
                self.expr_span(
                    values
                        .last()
                        .ok_or_else(|| self.error("missing assignment value"))?,
                ),
            );
            return Ok(Stmt::Assign {
                span,
                targets,
                values,
            });
        }

        if let Expr::Call { span, call } = expr {
            return Ok(Stmt::Call { span, call });
        }

        Err(self.error("expected assignment or function call"))
    }

    fn parse_attribute(&mut self) -> KResult<Attribute> {
        let start = self.current.span;
        self.consume_punct(TokenKind::Less)?;
        let (name, _) = self.consume_attribute_name()?;
        let end = self.consume_punct(TokenKind::Greater)?;
        match name.as_str() {
            "const" | "close" => Ok(Attribute {
                span: self.merge_span(start, end.span),
                name,
            }),
            _ => Err(self.error(format!("unknown attribute '{}'", name))),
        }
    }

    fn parse_attributed_name_list_from(
        &mut self,
        prefix_attribute: Option<Attribute>,
        allow_close: bool,
    ) -> KResult<(Option<Attribute>, Vec<AttributedName>)> {
        let mut names = Vec::new();
        let mut close_count = 0usize;
        let mut prefix_close = false;

        if let Some(attribute) = &prefix_attribute
            && attribute.name == "close"
        {
            prefix_close = true;
        }

        loop {
            let (name, name_span) = self.consume_name()?;
            let attribute = if self.at_punct(TokenKind::Less) {
                Some(self.parse_attribute()?)
            } else {
                None
            };
            if let Some(attr) = &attribute
                && attr.name == "close"
            {
                if !allow_close {
                    return Err(self.error("attribute <close> cannot be used on global variables"));
                }
                close_count += 1;
            }
            let span = if let Some(attr) = &attribute {
                self.merge_span(name_span, attr.span)
            } else {
                name_span
            };
            names.push(AttributedName {
                span,
                name,
                attribute,
            });
            if !self.at_punct(TokenKind::Comma) {
                break;
            }
            self.advance()?;
        }

        if allow_close && prefix_close {
            close_count += names.len();
        } else if !allow_close && prefix_close {
            return Err(self.error("attribute <close> cannot be used on global variables"));
        }

        if allow_close && close_count > 1 {
            return Err(self.error("multiple to-be-closed variables in declaration"));
        }

        Ok((prefix_attribute, names))
    }

    fn parse_function_name(&mut self) -> KResult<FunctionName> {
        let (first, start_span) = self.consume_name()?;
        let mut prefix = vec![first];
        let mut end_span = start_span;

        while self.at_punct(TokenKind::Dot) {
            self.advance()?;
            let (name, span) = self.consume_name()?;
            end_span = span;
            prefix.push(name);
        }

        let method = if self.at_punct(TokenKind::Colon) {
            self.advance()?;
            let (name, span) = self.consume_name()?;
            end_span = span;
            Some(name)
        } else {
            None
        };

        Ok(FunctionName {
            span: self.merge_span(start_span, end_span),
            prefix,
            method,
        })
    }

    fn parse_function_body(&mut self) -> KResult<FunctionBody> {
        let start = self.current.span;
        self.consume_punct(TokenKind::LParen)?;
        let mut parameters = Vec::new();
        let mut is_vararg = false;
        let mut vararg_name = None;

        if !self.at_punct(TokenKind::RParen) {
            if self.at_punct(TokenKind::DotDotDot) {
                self.advance()?;
                is_vararg = true;
                if matches!(self.current_kind(), TokenKind::Name(_)) {
                    let (name, _) = self.consume_name()?;
                    vararg_name = Some(name);
                }
            } else {
                loop {
                    let (name, span) = self.consume_name()?;
                    parameters.push(Param { span, name });
                    if !self.at_punct(TokenKind::Comma) {
                        break;
                    }
                    self.advance()?;
                    if self.at_punct(TokenKind::DotDotDot) {
                        self.advance()?;
                        is_vararg = true;
                        if matches!(self.current_kind(), TokenKind::Name(_)) {
                            let (name, _) = self.consume_name()?;
                            vararg_name = Some(name);
                        }
                        break;
                    }
                }
            }
        }

        self.consume_punct(TokenKind::RParen)?;
        let block = self.parse_block(BlockStop::End)?;
        let end = self.consume_keyword(Keyword::End)?;
        Ok(FunctionBody {
            span: self.merge_span(start, end.span),
            parameters,
            is_vararg,
            vararg_name,
            block,
        })
    }

    fn attributed_names_end_span(&self, names: &[AttributedName]) -> KSpan {
        names
            .last()
            .map(|name| {
                name.attribute
                    .as_ref()
                    .map_or(name.span, |attribute| attribute.span)
            })
            .unwrap_or(self.current.span)
    }

    fn stmt_span_from_var_list(&self, targets: &[Var]) -> KSpan {
        match (targets.first(), targets.last()) {
            (Some(first), Some(last)) => self.merge_span(self.var_span(first), self.var_span(last)),
            _ => self.current.span,
        }
    }

    fn var_span(&self, var: &Var) -> KSpan {
        match var.kind {
            VarKind::Name(_) | VarKind::Field { .. } | VarKind::Index { .. } => var.span,
        }
    }

    fn parse_expression_list(&mut self) -> KResult<Vec<Expr>> {
        let mut values = Vec::new();
        values.push(self.parse_expression(0)?);
        while self.at_punct(TokenKind::Comma) {
            self.advance()?;
            values.push(self.parse_expression(0)?);
        }
        Ok(values)
    }

    fn can_start_expression(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Name(_)
                | TokenKind::Integer(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::Keyword(Keyword::Nil)
                | TokenKind::Keyword(Keyword::False)
                | TokenKind::Keyword(Keyword::True)
                | TokenKind::Keyword(Keyword::Const)
                | TokenKind::Keyword(Keyword::Close)
                | TokenKind::Keyword(Keyword::Global)
                | TokenKind::Keyword(Keyword::Function)
                | TokenKind::DotDotDot
                | TokenKind::LParen
                | TokenKind::LBrace
                | TokenKind::Minus
                | TokenKind::Hash
                | TokenKind::Tilde
                | TokenKind::Keyword(Keyword::Not)
        )
    }

    fn expr_to_var(&self, expr: Expr) -> KResult<Var> {
        match expr {
            Expr::Name { span, name } => Ok(Var {
                span,
                kind: VarKind::Name(name),
            }),
            Expr::Field { span, prefix, name } => Ok(Var {
                span,
                kind: VarKind::Field { prefix, name },
            }),
            Expr::Index {
                span,
                prefix,
                index,
            } => Ok(Var {
                span,
                kind: VarKind::Index { prefix, index },
            }),
            _ => Err(self.error("expected a variable")),
        }
    }

    fn parse_expression(&mut self, min_bp: u8) -> KResult<Expr> {
        let mut left = if self.is_unary_op() {
            self.parse_unary_expression()?
        } else {
            self.parse_primary_expression()?
        };

        while let Some((op, precedence, assoc)) = self.current_binary_op() {
            if precedence < min_bp {
                break;
            }
            self.advance()?;
            let rhs_min = match assoc {
                Assoc::Left => precedence + 1,
                Assoc::Right => precedence,
            };
            let right = self.parse_expression(rhs_min)?;
            let span = self.merge_span(self.expr_span(&left), self.expr_span(&right));
            left = Expr::Binary {
                span,
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn is_unary_op(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Minus
                | TokenKind::Hash
                | TokenKind::Tilde
                | TokenKind::Keyword(Keyword::Not)
        )
    }

    fn parse_unary_expression(&mut self) -> KResult<Expr> {
        let token = self.current.clone();
        let op = match token.kind {
            TokenKind::Minus => UnaryOp::Minus,
            TokenKind::Hash => UnaryOp::Len,
            TokenKind::Tilde => UnaryOp::BitNot,
            TokenKind::Keyword(Keyword::Not) => UnaryOp::Not,
            _ => return Err(self.error("expected unary operator")),
        };
        self.advance()?;
        let expr = self.parse_expression(11)?;
        let span = self.merge_span(token.span, self.expr_span(&expr));
        Ok(Expr::Unary {
            span,
            op,
            expr: Box::new(expr),
        })
    }

    fn current_binary_op(&self) -> Option<(BinaryOp, u8, Assoc)> {
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Or) => Some((BinaryOp::Or, 1, Assoc::Left)),
            TokenKind::Keyword(Keyword::And) => Some((BinaryOp::And, 2, Assoc::Left)),
            TokenKind::Less => Some((BinaryOp::Less, 3, Assoc::Left)),
            TokenKind::Greater => Some((BinaryOp::Greater, 3, Assoc::Left)),
            TokenKind::LessEq => Some((BinaryOp::LessEq, 3, Assoc::Left)),
            TokenKind::GreaterEq => Some((BinaryOp::GreaterEq, 3, Assoc::Left)),
            TokenKind::EqEq => Some((BinaryOp::EqEq, 3, Assoc::Left)),
            TokenKind::NotEq => Some((BinaryOp::NotEq, 3, Assoc::Left)),
            TokenKind::Pipe => Some((BinaryOp::BitOr, 4, Assoc::Left)),
            TokenKind::Tilde => Some((BinaryOp::BitXor, 5, Assoc::Left)),
            TokenKind::Ampersand => Some((BinaryOp::BitAnd, 6, Assoc::Left)),
            TokenKind::ShiftLeft => Some((BinaryOp::ShiftLeft, 7, Assoc::Left)),
            TokenKind::ShiftRight => Some((BinaryOp::ShiftRight, 7, Assoc::Left)),
            TokenKind::DotDot => Some((BinaryOp::Concat, 8, Assoc::Right)),
            TokenKind::Plus => Some((BinaryOp::Add, 9, Assoc::Left)),
            TokenKind::Minus => Some((BinaryOp::Sub, 9, Assoc::Left)),
            TokenKind::Star => Some((BinaryOp::Mul, 10, Assoc::Left)),
            TokenKind::Slash => Some((BinaryOp::Div, 10, Assoc::Left)),
            TokenKind::DoubleSlash => Some((BinaryOp::FloorDiv, 10, Assoc::Left)),
            TokenKind::Percent => Some((BinaryOp::Mod, 10, Assoc::Left)),
            TokenKind::Caret => Some((BinaryOp::Pow, 12, Assoc::Right)),
            _ => None,
        }
    }

    fn parse_primary_expression(&mut self) -> KResult<Expr> {
        let expr = match self.current_kind() {
            TokenKind::Keyword(Keyword::Nil) => {
                let span = self.current.span;
                self.advance()?;
                Expr::Nil { span }
            }
            TokenKind::Keyword(Keyword::False) => {
                let span = self.current.span;
                self.advance()?;
                Expr::Bool { span, value: false }
            }
            TokenKind::Keyword(Keyword::True) => {
                let span = self.current.span;
                self.advance()?;
                Expr::Bool { span, value: true }
            }
            TokenKind::Integer(text) | TokenKind::Float(text) => {
                let span = self.current.span;
                let lexeme = text.clone();
                self.advance()?;
                Expr::Number { span, lexeme }
            }
            TokenKind::String(bytes) => {
                let span = self.current.span;
                let bytes = bytes.clone();
                self.advance()?;
                Expr::String { span, bytes }
            }
            TokenKind::DotDotDot => {
                let span = self.current.span;
                self.advance()?;
                let name = if matches!(self.current_kind(), TokenKind::Name(_)) {
                    let (name, _) = self.consume_name()?;
                    Some(name)
                } else {
                    None
                };
                Expr::Vararg { span, name }
            }
            TokenKind::Keyword(Keyword::Function) => {
                let start = self.current.span;
                self.advance()?;
                let body = self.parse_function_body()?;
                Expr::Function {
                    span: self.merge_span(start, body.span),
                    body,
                }
            }
            TokenKind::LBrace => self.parse_table_constructor()?,
            TokenKind::Name(_)
            | TokenKind::Keyword(Keyword::Const)
            | TokenKind::Keyword(Keyword::Close)
            | TokenKind::Keyword(Keyword::Global) => self.parse_prefix_expression()?,
            TokenKind::LParen => self.parse_prefix_expression()?,
            _ => return Err(self.error("expected expression")),
        };

        Ok(expr)
    }

    fn parse_prefix_expression(&mut self) -> KResult<Expr> {
        let mut expr = match self.current_kind() {
            TokenKind::Name(_)
            | TokenKind::Keyword(Keyword::Const)
            | TokenKind::Keyword(Keyword::Close)
            | TokenKind::Keyword(Keyword::Global) => {
                let (name, span) = self.consume_name()?;
                Expr::Name { span, name }
            }
            TokenKind::LParen => {
                let start = self.current.span;
                self.advance()?;
                let inner = self.parse_expression(0)?;
                let end = self.consume_punct(TokenKind::RParen)?;
                Expr::Paren {
                    span: self.merge_span(start, end.span),
                    expr: Box::new(inner),
                }
            }
            _ => return Err(self.error("expected prefix expression")),
        };

        loop {
            if self.at_punct(TokenKind::Dot) {
                let start = self.expr_span(&expr);
                self.advance()?;
                let (name, span) = self.consume_name()?;
                expr = Expr::Field {
                    span: self.merge_span(start, span),
                    prefix: Box::new(expr),
                    name,
                };
                continue;
            }

            if self.at_punct(TokenKind::LBracket) {
                let start = self.expr_span(&expr);
                self.advance()?;
                let index = self.parse_expression(0)?;
                let end = self.consume_punct(TokenKind::RBracket)?;
                expr = Expr::Index {
                    span: self.merge_span(start, end.span),
                    prefix: Box::new(expr),
                    index: Box::new(index),
                };
                continue;
            }

            if self.at_punct(TokenKind::Colon) {
                let start = self.expr_span(&expr);
                self.advance()?;
                let (method, _) = self.consume_name()?;
                let (args, end_span) = self.parse_call_args()?;
                expr = Expr::Call {
                    span: self.merge_span(start, end_span),
                    call: CallExpr {
                        span: self.merge_span(start, end_span),
                        prefix: Box::new(expr),
                        method: Some(method),
                        args,
                    },
                };
                continue;
            }

            if matches!(
                self.current_kind(),
                TokenKind::LParen | TokenKind::LBrace | TokenKind::String(_)
            ) {
                let start = self.expr_span(&expr);
                let (args, end_span) = self.parse_call_args()?;
                expr = Expr::Call {
                    span: self.merge_span(start, end_span),
                    call: CallExpr {
                        span: self.merge_span(start, end_span),
                        prefix: Box::new(expr),
                        method: None,
                        args,
                    },
                };
                continue;
            }

            break;
        }

        Ok(expr)
    }

    fn parse_call_args(&mut self) -> KResult<(Vec<Expr>, KSpan)> {
        match &self.current.kind {
            TokenKind::LParen => {
                let start = self.current.span;
                self.advance()?;
                let mut args = Vec::new();
                if !self.at_punct(TokenKind::RParen) {
                    args = self.parse_expression_list()?;
                }
                let end = self.consume_punct(TokenKind::RParen)?;
                Ok((args, self.merge_span(start, end.span)))
            }
            TokenKind::LBrace => {
                let table = self.parse_table_constructor()?;
                let span = self.expr_span(&table);
                Ok((vec![table], span))
            }
            TokenKind::String(_) => {
                let expr = self.parse_primary_expression()?;
                let span = self.expr_span(&expr);
                Ok((vec![expr], span))
            }
            _ => Err(self.error("expected call arguments")),
        }
    }

    fn parse_table_constructor(&mut self) -> KResult<Expr> {
        let start = self.current.span;
        self.consume_punct(TokenKind::LBrace)?;
        let mut fields = Vec::new();

        while !self.at_punct(TokenKind::RBrace) {
            let field = if self.at_punct(TokenKind::LBracket) {
                let field_start = self.current.span;
                self.advance()?;
                let key = self.parse_expression(0)?;
                self.consume_punct(TokenKind::RBracket)?;
                self.consume_punct(TokenKind::Assign)?;
                let value = self.parse_expression(0)?;
                let span = self.merge_span(field_start, self.expr_span(&value));
                TableField::Keyed { span, key, value }
            } else if matches!(
                self.current_kind(),
                TokenKind::Name(_)
                    | TokenKind::Keyword(Keyword::Const)
                    | TokenKind::Keyword(Keyword::Close)
            ) && matches!(self.next_kind(), TokenKind::Assign)
            {
                let (name, name_span) = self.consume_name()?;
                self.consume_punct(TokenKind::Assign)?;
                let value = self.parse_expression(0)?;
                let span = self.merge_span(name_span, self.expr_span(&value));
                TableField::Named { span, name, value }
            } else {
                let value = self.parse_expression(0)?;
                let span = self.expr_span(&value);
                TableField::Array { span, value }
            };
            fields.push(field);
            if self.at_punct(TokenKind::Comma) || self.at_punct(TokenKind::Semicolon) {
                self.advance()?;
            } else {
                break;
            }
        }

        let end = self.consume_punct(TokenKind::RBrace)?;
        Ok(Expr::Table {
            span: self.merge_span(start, end.span),
            constructor: TableConstructor {
                span: self.merge_span(start, end.span),
                fields,
            },
        })
    }
}
