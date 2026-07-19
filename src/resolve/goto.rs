use crate::ast::{Block, Expr, FunctionBody, Stmt, TableField, Var, VarKind};
use crate::error::{KError, KResult, KSpan};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Resolver;

#[derive(Debug, Clone, Copy)]
struct LocalLabel {
    span: KSpan,
    stmt_index: usize,
}

#[derive(Debug)]
struct BlockFrame<'a> {
    block: &'a Block,
    known_labels: BTreeMap<String, KSpan>,
    visible_labels: BTreeMap<String, KSpan>,
}

impl Resolver {
    pub fn resolve_chunk(chunk: &crate::ast::Chunk) -> KResult<()> {
        let mut stack = vec![BlockFrame {
            block: &chunk.block,
            known_labels: BTreeMap::new(),
            visible_labels: BTreeMap::new(),
        }];

        while let Some(frame) = stack.pop() {
            Self::resolve_block(frame, &mut stack)?;
        }

        Ok(())
    }

    fn resolve_block<'a>(frame: BlockFrame<'a>, stack: &mut Vec<BlockFrame<'a>>) -> KResult<()> {
        let mut local_labels = BTreeMap::<String, LocalLabel>::new();
        let mut declaration_indices = Vec::<usize>::new();

        for (stmt_index, stmt) in frame.block.statements.iter().enumerate() {
            if let Some((name, span)) = Self::label_from_stmt(stmt) {
                match local_labels.entry(name.clone()) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        if frame.visible_labels.contains_key(&name) {
                            return Err(Self::duplicate_label_error(name, span));
                        }
                        entry.insert(LocalLabel { span, stmt_index });
                    }
                    std::collections::btree_map::Entry::Occupied(_) => {
                        return Err(Self::duplicate_label_error(name, span));
                    }
                }
            }

            if Self::stmt_declares_scope(stmt) {
                declaration_indices.push(stmt_index);
            }
        }

        let mut known_labels = frame.known_labels.clone();
        for (name, label) in &local_labels {
            known_labels.insert(name.clone(), label.span);
        }

        let mut children = Vec::<BlockFrame<'a>>::new();

        for (stmt_index, stmt) in frame.block.statements.iter().enumerate() {
            if let Stmt::Goto { span, name } = stmt {
                Self::validate_goto(
                    *span,
                    name,
                    stmt_index,
                    &local_labels,
                    &frame.known_labels,
                    &declaration_indices,
                    frame.block,
                )?;
            }

            let mut visible_labels = frame.visible_labels.clone();
            for (name, label) in &local_labels {
                if label.stmt_index < stmt_index {
                    visible_labels.insert(name.clone(), label.span);
                }
            }
            Self::collect_stmt_children(stmt, &known_labels, &visible_labels, &mut children)?;
        }

        if let Some(return_stmt) = &frame.block.return_stmt {
            for value in &return_stmt.values {
                Self::collect_expr_children(value, &known_labels, &mut children)?;
            }
        }

        while let Some(child) = children.pop() {
            stack.push(child);
        }

        Ok(())
    }

    fn validate_goto(
        goto_span: KSpan,
        name: &str,
        goto_index: usize,
        local_labels: &BTreeMap<String, LocalLabel>,
        known_labels: &BTreeMap<String, KSpan>,
        declaration_indices: &[usize],
        block: &Block,
    ) -> KResult<()> {
        if let Some(local_label) = local_labels.get(name) {
            let ends_block = block
                .statements
                .iter()
                .skip(local_label.stmt_index.saturating_add(1))
                .all(|stmt| matches!(stmt, Stmt::Empty { .. } | Stmt::Label { .. }));
            if local_label.stmt_index > goto_index
                && Self::crosses_scope(goto_index, local_label.stmt_index, declaration_indices)
                && !(ends_block
                    && Self::only_skips_terminal_plain_locals(
                        goto_index,
                        local_label.stmt_index,
                        block,
                    ))
            {
                return Err(Self::scope_crossing_error(goto_span, name));
            }
            return Ok(());
        }

        if known_labels.contains_key(name) {
            return Ok(());
        }

        Err(Self::unknown_label_error(goto_span, name))
    }

    fn crosses_scope(goto_index: usize, label_index: usize, declaration_indices: &[usize]) -> bool {
        declaration_indices
            .iter()
            .any(|index| *index > goto_index && *index < label_index)
    }

    fn only_skips_terminal_plain_locals(
        goto_index: usize,
        label_index: usize,
        block: &Block,
    ) -> bool {
        block
            .statements
            .iter()
            .enumerate()
            .filter(|(index, _)| *index > goto_index && *index < label_index)
            .filter(|(_, statement)| Self::stmt_declares_scope(statement))
            .all(|(_, statement)| match statement {
                Stmt::LocalDecl {
                    prefix_attribute,
                    names,
                    ..
                } => {
                    prefix_attribute.is_none() && names.iter().all(|name| name.attribute.is_none())
                }
                _ => false,
            })
    }

    fn stmt_declares_scope(stmt: &Stmt) -> bool {
        matches!(
            stmt,
            Stmt::LocalDecl { .. }
                | Stmt::GlobalDecl { .. }
                | Stmt::GlobalAll { .. }
                | Stmt::LocalFunction { .. }
                | Stmt::GlobalFunction { .. }
                | Stmt::NumericFor { .. }
                | Stmt::GenericFor { .. }
        )
    }

    fn label_from_stmt(stmt: &Stmt) -> Option<(String, KSpan)> {
        match stmt {
            Stmt::Label { span, name } => Some((name.clone(), *span)),
            _ => None,
        }
    }

    fn duplicate_label_error(name: String, span: KSpan) -> KError {
        KError::syntax(format!("duplicate visible label '{}'", name), span)
    }

    fn unknown_label_error(span: KSpan, name: &str) -> KError {
        KError::syntax(format!("goto references unknown label '{}'", name), span)
    }

    fn scope_crossing_error(span: KSpan, name: &str) -> KError {
        KError::syntax(
            format!("goto '{}' would enter the scope of a declaration", name),
            span,
        )
    }

    fn collect_stmt_children<'a>(
        stmt: &'a Stmt,
        known_labels: &BTreeMap<String, KSpan>,
        visible_labels: &BTreeMap<String, KSpan>,
        children: &mut Vec<BlockFrame<'a>>,
    ) -> KResult<()> {
        match stmt {
            Stmt::Do { block, .. }
            | Stmt::While { block, .. }
            | Stmt::Repeat { block, .. }
            | Stmt::NumericFor { block, .. }
            | Stmt::GenericFor { block, .. } => {
                children.push(BlockFrame {
                    block,
                    known_labels: known_labels.clone(),
                    visible_labels: visible_labels.clone(),
                });
            }
            Stmt::If {
                branches,
                else_block,
                ..
            } => {
                if let Some(block) = else_block {
                    children.push(BlockFrame {
                        block,
                        known_labels: known_labels.clone(),
                        visible_labels: visible_labels.clone(),
                    });
                }
                for (_, block) in branches.iter().rev() {
                    children.push(BlockFrame {
                        block,
                        known_labels: known_labels.clone(),
                        visible_labels: visible_labels.clone(),
                    });
                }
            }
            Stmt::Function { body, .. }
            | Stmt::LocalFunction { body, .. }
            | Stmt::GlobalFunction { body, .. } => {
                Self::push_function_body(body, children);
            }
            Stmt::Assign {
                targets, values, ..
            } => {
                for target in targets {
                    Self::collect_var_children(target, visible_labels, children)?;
                }
                for value in values {
                    Self::collect_expr_children(value, visible_labels, children)?;
                }
            }
            Stmt::Call { call, .. } => {
                Self::collect_expr_children(&call.prefix, visible_labels, children)?;
                for arg in &call.args {
                    Self::collect_expr_children(arg, visible_labels, children)?;
                }
            }
            Stmt::LocalDecl { values, .. } | Stmt::GlobalDecl { values, .. } => {
                for value in values {
                    Self::collect_expr_children(value, visible_labels, children)?;
                }
            }
            Stmt::GlobalAll { .. }
            | Stmt::Empty { .. }
            | Stmt::Label { .. }
            | Stmt::Break { .. } => {}
            Stmt::Goto { .. } => {}
        }

        Ok(())
    }

    fn collect_var_children<'a>(
        var: &'a Var,
        visible_labels: &BTreeMap<String, KSpan>,
        children: &mut Vec<BlockFrame<'a>>,
    ) -> KResult<()> {
        match &var.kind {
            VarKind::Name(_) => Ok(()),
            VarKind::Field { prefix, .. } => {
                Self::collect_expr_children(prefix, visible_labels, children)
            }
            VarKind::Index { prefix, index } => {
                Self::collect_expr_children(prefix, visible_labels, children)?;
                Self::collect_expr_children(index, visible_labels, children)
            }
        }
    }

    fn collect_expr_children<'a>(
        expr: &'a Expr,
        visible_labels: &BTreeMap<String, KSpan>,
        children: &mut Vec<BlockFrame<'a>>,
    ) -> KResult<()> {
        let mut stack = vec![expr];

        while let Some(expr) = stack.pop() {
            match expr {
                Expr::Function { body, .. } => {
                    Self::push_function_body(body, children);
                }
                Expr::Paren { expr, .. } | Expr::Unary { expr, .. } => {
                    stack.push(expr);
                }
                Expr::Field { prefix, .. } => {
                    stack.push(prefix);
                }
                Expr::Index { prefix, index, .. } => {
                    stack.push(prefix);
                    stack.push(index);
                }
                Expr::Table { constructor, .. } => {
                    for field in constructor.fields.iter().rev() {
                        match field {
                            TableField::Array { value, .. } => stack.push(value),
                            TableField::Named { value, .. } => stack.push(value),
                            TableField::Keyed { key, value, .. } => {
                                stack.push(value);
                                stack.push(key);
                            }
                        }
                    }
                }
                Expr::Binary { left, right, .. } => {
                    stack.push(right);
                    stack.push(left);
                }
                Expr::Call { call, .. } => {
                    for arg in call.args.iter().rev() {
                        stack.push(arg);
                    }
                    stack.push(&call.prefix);
                }
                Expr::Nil { .. }
                | Expr::Bool { .. }
                | Expr::Number { .. }
                | Expr::String { .. }
                | Expr::Vararg { .. }
                | Expr::Name { .. } => {}
            }
        }

        let _ = visible_labels;
        Ok(())
    }

    fn push_function_body<'a>(body: &'a FunctionBody, children: &mut Vec<BlockFrame<'a>>) {
        children.push(BlockFrame {
            block: &body.block,
            known_labels: BTreeMap::new(),
            visible_labels: BTreeMap::new(),
        });
    }
}
