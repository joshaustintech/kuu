use crate::error::KSpan;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub block: Block,
}

impl Chunk {
    pub fn snapshot(&self) -> String {
        let mut out = String::new();
        self.fmt_snapshot(0, &mut out);
        out
    }

    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(indent, out, "Chunk");
        self.block.fmt_snapshot(indent + 2, out);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub span: KSpan,
    pub statements: Vec<Stmt>,
    pub return_stmt: Option<ReturnStmt>,
}

impl Block {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(indent, out, &format!("Block span={}", self.span));
        for stmt in &self.statements {
            stmt.fmt_snapshot(indent + 2, out);
        }
        if let Some(return_stmt) = &self.return_stmt {
            return_stmt.fmt_snapshot(indent + 2, out);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnStmt {
    pub span: KSpan,
    pub values: Vec<Expr>,
}

impl ReturnStmt {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(indent, out, &format!("Return span={}", self.span));
        for value in &self.values {
            value.fmt_snapshot(indent + 2, out);
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Empty {
        span: KSpan,
    },
    Assign {
        span: KSpan,
        targets: Vec<Var>,
        values: Vec<Expr>,
    },
    Call {
        span: KSpan,
        call: CallExpr,
    },
    Label {
        span: KSpan,
        name: String,
    },
    Break {
        span: KSpan,
    },
    Goto {
        span: KSpan,
        name: String,
    },
    Do {
        span: KSpan,
        block: Block,
    },
    While {
        span: KSpan,
        condition: Expr,
        block: Block,
    },
    Repeat {
        span: KSpan,
        block: Block,
        condition: Expr,
    },
    If {
        span: KSpan,
        branches: Vec<(Expr, Block)>,
        else_block: Option<Block>,
    },
    NumericFor {
        span: KSpan,
        name: String,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        block: Block,
    },
    GenericFor {
        span: KSpan,
        names: Vec<String>,
        iter: Vec<Expr>,
        block: Block,
    },
    Function {
        span: KSpan,
        name: FunctionName,
        body: FunctionBody,
    },
    LocalFunction {
        span: KSpan,
        name: String,
        body: FunctionBody,
    },
    GlobalFunction {
        span: KSpan,
        name: String,
        body: FunctionBody,
    },
    LocalDecl {
        span: KSpan,
        prefix_attribute: Option<Attribute>,
        names: Vec<AttributedName>,
        values: Vec<Expr>,
    },
    GlobalDecl {
        span: KSpan,
        prefix_attribute: Option<Attribute>,
        names: Vec<AttributedName>,
        values: Vec<Expr>,
    },
    GlobalAll {
        span: KSpan,
        prefix_attribute: Option<Attribute>,
    },
}

impl Stmt {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        match self {
            Self::Empty { span } => indent_line(indent, out, &format!("Empty span={}", span)),
            Self::Assign {
                span,
                targets,
                values,
            } => {
                indent_line(indent, out, &format!("Assign span={}", span));
                for target in targets {
                    target.fmt_snapshot(indent + 2, out);
                }
                for value in values {
                    value.fmt_snapshot(indent + 2, out);
                }
            }
            Self::Call { span, call } => {
                indent_line(indent, out, &format!("CallStmt span={}", span));
                call.fmt_snapshot(indent + 2, out);
            }
            Self::Label { span, name } => {
                indent_line(indent, out, &format!("Label span={} name={}", span, name))
            }
            Self::Break { span } => indent_line(indent, out, &format!("Break span={}", span)),
            Self::Goto { span, name } => {
                indent_line(indent, out, &format!("Goto span={} name={}", span, name))
            }
            Self::Do { span, block } => {
                indent_line(indent, out, &format!("Do span={}", span));
                block.fmt_snapshot(indent + 2, out);
            }
            Self::While {
                span,
                condition,
                block,
            } => {
                indent_line(indent, out, &format!("While span={}", span));
                condition.fmt_snapshot(indent + 2, out);
                block.fmt_snapshot(indent + 2, out);
            }
            Self::Repeat {
                span,
                block,
                condition,
            } => {
                indent_line(indent, out, &format!("Repeat span={}", span));
                block.fmt_snapshot(indent + 2, out);
                condition.fmt_snapshot(indent + 2, out);
            }
            Self::If {
                span,
                branches,
                else_block,
            } => {
                indent_line(indent, out, &format!("If span={}", span));
                for (condition, block) in branches {
                    condition.fmt_snapshot(indent + 2, out);
                    block.fmt_snapshot(indent + 2, out);
                }
                if let Some(block) = else_block {
                    indent_line(indent + 2, out, "Else");
                    block.fmt_snapshot(indent + 4, out);
                }
            }
            Self::NumericFor {
                span,
                name,
                start,
                end,
                step,
                block,
            } => {
                indent_line(
                    indent,
                    out,
                    &format!("NumericFor span={} name={}", span, name),
                );
                start.fmt_snapshot(indent + 2, out);
                end.fmt_snapshot(indent + 2, out);
                if let Some(step) = step {
                    step.fmt_snapshot(indent + 2, out);
                }
                block.fmt_snapshot(indent + 2, out);
            }
            Self::GenericFor {
                span,
                names,
                iter,
                block,
            } => {
                indent_line(indent, out, &format!("GenericFor span={}", span));
                for name in names {
                    indent_line(indent + 2, out, &format!("Name {}", name));
                }
                for expr in iter {
                    expr.fmt_snapshot(indent + 2, out);
                }
                block.fmt_snapshot(indent + 2, out);
            }
            Self::Function { span, name, body } => {
                indent_line(indent, out, &format!("Function span={}", span));
                name.fmt_snapshot(indent + 2, out);
                body.fmt_snapshot(indent + 2, out);
            }
            Self::LocalFunction { span, name, body } => {
                indent_line(
                    indent,
                    out,
                    &format!("LocalFunction span={} name={}", span, name),
                );
                body.fmt_snapshot(indent + 2, out);
            }
            Self::GlobalFunction { span, name, body } => {
                indent_line(
                    indent,
                    out,
                    &format!("GlobalFunction span={} name={}", span, name),
                );
                body.fmt_snapshot(indent + 2, out);
            }
            Self::LocalDecl {
                span,
                prefix_attribute,
                names,
                values,
            } => {
                indent_line(indent, out, &format!("LocalDecl span={}", span));
                if let Some(attribute) = prefix_attribute {
                    attribute.fmt_snapshot(indent + 2, out);
                }
                for name in names {
                    name.fmt_snapshot(indent + 2, out);
                }
                for value in values {
                    value.fmt_snapshot(indent + 2, out);
                }
            }
            Self::GlobalDecl {
                span,
                prefix_attribute,
                names,
                values,
            } => {
                indent_line(indent, out, &format!("GlobalDecl span={}", span));
                if let Some(attribute) = prefix_attribute {
                    attribute.fmt_snapshot(indent + 2, out);
                }
                for name in names {
                    name.fmt_snapshot(indent + 2, out);
                }
                for value in values {
                    value.fmt_snapshot(indent + 2, out);
                }
            }
            Self::GlobalAll {
                span,
                prefix_attribute,
            } => {
                indent_line(indent, out, &format!("GlobalAll span={}", span));
                if let Some(attribute) = prefix_attribute {
                    attribute.fmt_snapshot(indent + 2, out);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionName {
    pub span: KSpan,
    pub prefix: Vec<String>,
    pub method: Option<String>,
}

impl FunctionName {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(
            indent,
            out,
            &format!(
                "FunctionName span={} prefix={:?} method={:?}",
                self.span, self.prefix, self.method
            ),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionBody {
    pub span: KSpan,
    pub parameters: Vec<Param>,
    pub is_vararg: bool,
    pub vararg_name: Option<String>,
    pub block: Block,
}

impl FunctionBody {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(
            indent,
            out,
            &format!(
                "FunctionBody span={} is_vararg={} vararg_name={:?}",
                self.span, self.is_vararg, self.vararg_name
            ),
        );
        for parameter in &self.parameters {
            parameter.fmt_snapshot(indent + 2, out);
        }
        self.block.fmt_snapshot(indent + 2, out);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub span: KSpan,
    pub name: String,
}

impl Param {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(
            indent,
            out,
            &format!("Param span={} name={}", self.span, self.name),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributedName {
    pub span: KSpan,
    pub name: String,
    pub attribute: Option<Attribute>,
}

impl AttributedName {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(
            indent,
            out,
            &format!("AttributedName span={} name={}", self.span, self.name),
        );
        if let Some(attribute) = &self.attribute {
            attribute.fmt_snapshot(indent + 2, out);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub span: KSpan,
    pub name: String,
}

impl Attribute {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(
            indent,
            out,
            &format!("Attribute span={} name={}", self.span, self.name),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableConstructor {
    pub span: KSpan,
    pub fields: Vec<TableField>,
}

impl TableConstructor {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(indent, out, &format!("TableConstructor span={}", self.span));
        for field in &self.fields {
            field.fmt_snapshot(indent + 2, out);
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableField {
    Array {
        span: KSpan,
        value: Expr,
    },
    Named {
        span: KSpan,
        name: String,
        value: Expr,
    },
    Keyed {
        span: KSpan,
        key: Expr,
        value: Expr,
    },
}

impl TableField {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        match self {
            Self::Array { span, value } => {
                indent_line(indent, out, &format!("ArrayField span={}", span));
                value.fmt_snapshot(indent + 2, out);
            }
            Self::Named { span, name, value } => {
                indent_line(
                    indent,
                    out,
                    &format!("NamedField span={} name={}", span, name),
                );
                value.fmt_snapshot(indent + 2, out);
            }
            Self::Keyed { span, key, value } => {
                indent_line(indent, out, &format!("KeyedField span={}", span));
                key.fmt_snapshot(indent + 2, out);
                value.fmt_snapshot(indent + 2, out);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallExpr {
    pub span: KSpan,
    pub prefix: Box<Expr>,
    pub method: Option<String>,
    pub args: Vec<Expr>,
}

impl CallExpr {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        indent_line(
            indent,
            out,
            &format!("CallExpr span={} method={:?}", self.span, self.method),
        );
        self.prefix.fmt_snapshot(indent + 2, out);
        for arg in &self.args {
            arg.fmt_snapshot(indent + 2, out);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Var {
    pub span: KSpan,
    pub kind: VarKind,
}

impl Var {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        match &self.kind {
            VarKind::Name(name) => {
                indent_line(
                    indent,
                    out,
                    &format!("VarName span={} name={}", self.span, name),
                );
            }
            VarKind::Field { prefix, name } => {
                indent_line(
                    indent,
                    out,
                    &format!("VarField span={} name={}", self.span, name),
                );
                prefix.fmt_snapshot(indent + 2, out);
            }
            VarKind::Index { prefix, index } => {
                indent_line(indent, out, &format!("VarIndex span={}", self.span));
                prefix.fmt_snapshot(indent + 2, out);
                index.fmt_snapshot(indent + 2, out);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarKind {
    Name(String),
    Field { prefix: Box<Expr>, name: String },
    Index { prefix: Box<Expr>, index: Box<Expr> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Nil {
        span: KSpan,
    },
    Bool {
        span: KSpan,
        value: bool,
    },
    Number {
        span: KSpan,
        lexeme: String,
    },
    String {
        span: KSpan,
        bytes: Vec<u8>,
    },
    Vararg {
        span: KSpan,
        name: Option<String>,
    },
    Name {
        span: KSpan,
        name: String,
    },
    Paren {
        span: KSpan,
        expr: Box<Expr>,
    },
    Field {
        span: KSpan,
        prefix: Box<Expr>,
        name: String,
    },
    Index {
        span: KSpan,
        prefix: Box<Expr>,
        index: Box<Expr>,
    },
    Table {
        span: KSpan,
        constructor: TableConstructor,
    },
    Function {
        span: KSpan,
        body: FunctionBody,
    },
    Unary {
        span: KSpan,
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        span: KSpan,
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        span: KSpan,
        call: CallExpr,
    },
}

impl Expr {
    fn fmt_snapshot(&self, indent: usize, out: &mut String) {
        match self {
            Self::Nil { span } => indent_line(indent, out, &format!("Nil span={}", span)),
            Self::Bool { span, value } => {
                indent_line(indent, out, &format!("Bool span={} value={}", span, value))
            }
            Self::Number { span, lexeme } => indent_line(
                indent,
                out,
                &format!("Number span={} lexeme={}", span, lexeme),
            ),
            Self::String { span, bytes } => indent_line(
                indent,
                out,
                &format!("String span={} bytes={:?}", span, bytes),
            ),
            Self::Vararg { span, name } => indent_line(
                indent,
                out,
                &format!("Vararg span={} name={:?}", span, name),
            ),
            Self::Name { span, name } => {
                indent_line(indent, out, &format!("Name span={} name={}", span, name))
            }
            Self::Paren { span, expr } => {
                indent_line(indent, out, &format!("Paren span={}", span));
                expr.fmt_snapshot(indent + 2, out);
            }
            Self::Field { span, prefix, name } => {
                indent_line(indent, out, &format!("Field span={} name={}", span, name));
                prefix.fmt_snapshot(indent + 2, out);
            }
            Self::Index {
                span,
                prefix,
                index,
            } => {
                indent_line(indent, out, &format!("Index span={}", span));
                prefix.fmt_snapshot(indent + 2, out);
                index.fmt_snapshot(indent + 2, out);
            }
            Self::Table { span, constructor } => {
                indent_line(indent, out, &format!("Table span={}", span));
                constructor.fmt_snapshot(indent + 2, out);
            }
            Self::Function { span, body } => {
                indent_line(indent, out, &format!("FunctionExpr span={}", span));
                body.fmt_snapshot(indent + 2, out);
            }
            Self::Unary { span, op, expr } => {
                indent_line(indent, out, &format!("Unary span={} op={:?}", span, op));
                expr.fmt_snapshot(indent + 2, out);
            }
            Self::Binary {
                span,
                op,
                left,
                right,
            } => {
                indent_line(indent, out, &format!("Binary span={} op={:?}", span, op));
                left.fmt_snapshot(indent + 2, out);
                right.fmt_snapshot(indent + 2, out);
            }
            Self::Call { span, call } => {
                indent_line(indent, out, &format!("Call span={}", span));
                call.fmt_snapshot(indent + 2, out);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Minus,
    Not,
    Len,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Less,
    Greater,
    LessEq,
    GreaterEq,
    EqEq,
    NotEq,
    BitOr,
    BitXor,
    BitAnd,
    ShiftLeft,
    ShiftRight,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
}

fn indent_line(indent: usize, out: &mut String, line: &str) {
    for _ in 0..indent {
        out.push(' ');
    }
    out.push_str(line);
    out.push('\n');
}

impl fmt::Display for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.snapshot())
    }
}
