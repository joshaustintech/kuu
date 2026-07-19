use super::*;

#[derive(Debug, Clone, Default)]
pub struct Resolver {
    next_function_id: usize,
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resolve_chunk(&mut self, chunk: &Chunk) -> KResult<ResolvedChunk> {
        let root = self.resolve_function(
            FunctionKind::Chunk,
            chunk.block.span,
            &chunk.block,
            Vec::new(),
            GlobalPolicy::Writable,
            BTreeMap::new(),
        )?;
        crate::resolve::goto::Resolver::resolve_chunk(chunk)?;
        Ok(ResolvedChunk { root })
    }

    fn resolve_function(
        &mut self,
        kind: FunctionKind,
        span: KSpan,
        block: &Block,
        ancestor_scopes: Vec<ScopeSnapshot>,
        global_policy: GlobalPolicy,
        globals: BTreeMap<String, GlobalBinding>,
    ) -> KResult<ResolvedFunction> {
        let id = self.next_function_id();
        let mut state = FunctionState::new(id, kind, span, ancestor_scopes, global_policy, globals);

        state.push_block();
        self.resolve_block(block, &mut state)?;
        state.pop_block();
        Ok(state.finish())
    }

    fn next_function_id(&mut self) -> usize {
        let id = self.next_function_id;
        self.next_function_id = self.next_function_id.saturating_add(1);
        id
    }

    fn resolve_block(&mut self, block: &Block, state: &mut FunctionState) -> KResult<()> {
        for stmt in &block.statements {
            self.resolve_stmt(stmt, state)?;
        }

        if let Some(return_stmt) = &block.return_stmt {
            self.resolve_return(return_stmt, state)?;
        }
        Ok(())
    }

    fn resolve_return(&mut self, return_stmt: &ReturnStmt, state: &mut FunctionState) -> KResult<()> {
        for value in &return_stmt.values {
            self.resolve_expr(value, state)?;
        }
        Ok(())
    }

    fn resolve_stmt(&mut self, stmt: &Stmt, state: &mut FunctionState) -> KResult<()> {
        match stmt {
            Stmt::Empty { .. } | Stmt::Break { .. } => Ok(()),
            Stmt::Label { span, name } => self.resolve_label(*span, name, state),
            Stmt::Goto { span, name } => {
                let _ = state.add_goto(name.clone(), *span);
                Ok(())
            }
            Stmt::Do { block, .. } => {
                state.push_block();
                self.resolve_block(block, state)?;
                state.pop_block();
                Ok(())
            }
            Stmt::While {
                condition, block, ..
            } => {
                self.resolve_expr(condition, state)?;
                state.push_block();
                self.resolve_block(block, state)?;
                state.pop_block();
                Ok(())
            }
            Stmt::Repeat {
                block, condition, ..
            } => {
                state.push_block();
                self.resolve_block(block, state)?;
                self.resolve_expr(condition, state)?;
                state.pop_block();
                Ok(())
            }
            Stmt::If {
                branches,
                else_block,
                ..
            } => self.resolve_if(branches, else_block.as_ref(), state),
            Stmt::NumericFor {
                name,
                start,
                end,
                step,
                block,
                ..
            } => {
                self.resolve_expr(start, state)?;
                self.resolve_expr(end, state)?;
                if let Some(step) = step {
                    self.resolve_expr(step, state)?;
                }
                state.push_block();
                state.add_local_binding(
                    name.clone(),
                    false,
                    false,
                    block.span,
                    true,
                    DeclarationKind::Local,
                );
                self.resolve_block(block, state)?;
                state.pop_block();
                Ok(())
            }
            Stmt::GenericFor {
                names, iter, block, ..
            } => {
                for expr in iter {
                    self.resolve_expr(expr, state)?;
                }
                state.push_block();
                for name in names {
                    state.add_local_binding(
                        name.clone(),
                        false,
                        false,
                        block.span,
                        true,
                        DeclarationKind::Local,
                    );
                }
                self.resolve_block(block, state)?;
                state.pop_block();
                Ok(())
            }
            Stmt::Function { name, body, .. } => {
                self.resolve_function_name(name, state)?;
                self.resolve_function_body(body, FunctionKind::Function, state)
            }
            Stmt::LocalFunction { span, name, body } => {
                state.add_local_binding(
                    name.clone(),
                    false,
                    false,
                    *span,
                    true,
                    DeclarationKind::Local,
                );
                self.resolve_function_body(body, FunctionKind::LocalFunction, state)
            }
            Stmt::GlobalFunction { span, name, body } => {
                state.add_global_binding(
                    name.clone(),
                    false,
                    Some(*span),
                    true,
                    DeclarationKind::Global,
                );
                if !state.has_global_default {
                    state.global_policy = GlobalPolicy::DeclaredOnly;
                }
                self.resolve_function_body(body, FunctionKind::GlobalFunction, state)
            }
            Stmt::LocalDecl {
                span,
                prefix_attribute,
                names,
                values,
            } => {
                for value in values {
                    self.resolve_expr(value, state)?;
                }

                let prefix_readonly = matches!(prefix_attribute, Some(attr) if attr.name == "const");
                let prefix_close = matches!(prefix_attribute, Some(attr) if attr.name == "close");
                for name in names {
                    let readonly = prefix_readonly
                        || matches!(name.attribute.as_ref(), Some(attr) if attr.name == "const");
                    let close = prefix_close
                        || matches!(name.attribute.as_ref(), Some(attr) if attr.name == "close");
                    state.add_local_binding(
                        name.name.clone(),
                        readonly,
                        close,
                        *span,
                        true,
                        DeclarationKind::Local,
                    );
                }
                Ok(())
            }
            Stmt::GlobalDecl {
                span,
                prefix_attribute,
                names,
                values,
            } => {
                if names.iter().any(|name| name.name == "_ENV") {
                    let reported = names
                        .iter()
                        .find(|name| name.name != "_ENV")
                        .map(|name| name.name.as_str())
                        .unwrap_or("_ENV");
                    return Err(KError::syntax(
                        format!("variable '{reported}' cannot be declared global with _ENV"),
                        *span,
                    ));
                }
                for value in values {
                    self.resolve_expr(value, state)?;
                }

                let prefix_readonly = matches!(prefix_attribute, Some(attr) if attr.name == "const");
                for name in names {
                    let readonly = prefix_readonly
                        || matches!(name.attribute.as_ref(), Some(attr) if attr.name == "const");
                    state.add_global_binding(
                        name.name.clone(),
                        readonly,
                        Some(*span),
                        true,
                        DeclarationKind::Global,
                    );
                }
                if !state.has_global_default {
                    state.global_policy = GlobalPolicy::DeclaredOnly;
                }
                Ok(())
            }
            Stmt::GlobalAll {
                span,
                prefix_attribute,
            } => {
                let readonly = matches!(prefix_attribute, Some(attr) if attr.name == "const");
                let _ = state.add_global_default(readonly, *span);
                state.has_global_default = true;
                state.global_policy = if readonly {
                    GlobalPolicy::Readonly
                } else {
                    GlobalPolicy::Writable
                };
                Ok(())
            }
            Stmt::Assign {
                targets,
                values,
                ..
            } => {
                for target in targets {
                    self.resolve_var(target, state, true)?;
                }
                for value in values {
                    self.resolve_expr(value, state)?;
                }
                Ok(())
            }
            Stmt::Call { call, .. } => self.resolve_call(call, state),
        }
    }

    fn resolve_if(
        &mut self,
        branches: &[(Expr, Block)],
        else_block: Option<&Block>,
        state: &mut FunctionState,
    ) -> KResult<()> {
        for (condition, block) in branches {
            self.resolve_expr(condition, state)?;
            state.push_block();
            self.resolve_block(block, state)?;
            state.pop_block();
        }

        if let Some(block) = else_block {
            state.push_block();
            self.resolve_block(block, state)?;
            state.pop_block();
        }

        Ok(())
    }

    fn resolve_function_body(
        &mut self,
        body: &FunctionBody,
        kind: FunctionKind,
        parent: &mut FunctionState,
    ) -> KResult<()> {
        let mut ancestors = parent.ancestor_scopes.clone();
        for snapshot in &mut ancestors {
            snapshot.distance = snapshot.distance.saturating_add(1);
        }
        ancestors.push(parent.current_snapshot());

        let mut child = FunctionState::new(
            self.next_function_id(),
            kind,
            body.span,
            ancestors,
            parent.global_policy,
            parent.globals.clone(),
        );

        child.push_block();
        for parameter in &body.parameters {
            child.add_local_binding(
                parameter.name.clone(),
                false,
                false,
                parameter.span,
                true,
                DeclarationKind::Local,
            );
        }

        if body.is_vararg && let Some(name) = &body.vararg_name {
            child.add_local_binding(
                name.clone(),
                false,
                false,
                body.span,
                true,
                DeclarationKind::Local,
            );
        }

        self.resolve_block(&body.block, &mut child)?;
        child.pop_block();

        for upvalue in &child.upvalues {
            let _ = parent.capture_upvalue(
                &upvalue.name,
                upvalue.readonly,
                upvalue.declaration_span,
                upvalue.source_depth,
            );
        }
        parent.children.push(child.finish());
        Ok(())
    }

    fn resolve_function_name(
        &mut self,
        name: &FunctionName,
        state: &mut FunctionState,
    ) -> KResult<()> {
        if let Some(first) = name.prefix.first() {
            let is_simple_name = name.prefix.len() == 1 && name.method.is_none();
            let _ = self.resolve_name(first, name.span, state, is_simple_name)?;
        }

        Ok(())
    }

    fn resolve_label(
        &mut self,
        span: KSpan,
        name: &str,
        state: &mut FunctionState,
    ) -> KResult<()> {
        if state.visible_labels.contains_key(name) {
            return Err(KError::syntax(
                format!("duplicate visible label '{}'", name),
                span,
            ));
        }
        let _ = state.add_label(name.to_owned(), span);
        Ok(())
    }

    fn resolve_call(&mut self, call: &CallExpr, state: &mut FunctionState) -> KResult<()> {
        self.resolve_expr(&call.prefix, state)?;
        for arg in &call.args {
            self.resolve_expr(arg, state)?;
        }
        Ok(())
    }

    fn resolve_var(
        &mut self,
        var: &Var,
        state: &mut FunctionState,
        is_write: bool,
    ) -> KResult<()> {
        match &var.kind {
            VarKind::Name(name) => {
                let _ = self.resolve_name(name, var.span, state, is_write)?;
            }
            VarKind::Field { prefix, .. } => {
                self.resolve_expr(prefix, state)?;
            }
            VarKind::Index { prefix, index } => {
                self.resolve_expr(prefix, state)?;
                self.resolve_expr(index, state)?;
            }
        }
        Ok(())
    }

    fn resolve_expr(&mut self, expr: &Expr, state: &mut FunctionState) -> KResult<()> {
        match expr {
            Expr::Nil { .. }
            | Expr::Bool { .. }
            | Expr::Number { .. }
            | Expr::String { .. }
            | Expr::Vararg { .. } => Ok(()),
            Expr::Name { span, name } => {
                let _ = self.resolve_name(name, *span, state, false)?;
                Ok(())
            }
            Expr::Paren { expr, .. } => self.resolve_expr(expr, state),
            Expr::Field { prefix, .. } => self.resolve_expr(prefix, state),
            Expr::Index { prefix, index, .. } => {
                self.resolve_expr(prefix, state)?;
                self.resolve_expr(index, state)
            }
            Expr::Table { constructor, .. } => {
                for field in &constructor.fields {
                    self.resolve_table_field(field, state)?;
                }
                Ok(())
            }
            Expr::Function { body, .. } => {
                self.resolve_function_body(body, FunctionKind::Function, state)
            }
            Expr::Unary { expr, .. } => self.resolve_expr(expr, state),
            Expr::Binary { left, right, .. } => {
                self.resolve_expr(left, state)?;
                self.resolve_expr(right, state)
            }
            Expr::Call { call, .. } => self.resolve_call(call, state),
        }
    }

    fn resolve_table_field(
        &mut self,
        field: &TableField,
        state: &mut FunctionState,
    ) -> KResult<()> {
        match field {
            TableField::Array { value, .. } => self.resolve_expr(value, state),
            TableField::Named { value, .. } => self.resolve_expr(value, state),
            TableField::Keyed { key, value, .. } => {
                self.resolve_expr(key, state)?;
                self.resolve_expr(value, state)
            }
        }
    }

    fn resolve_name(
        &mut self,
        name: &str,
        span: KSpan,
        state: &mut FunctionState,
        is_write: bool,
    ) -> KResult<BindingTarget> {
        if state.global_declared_in_current_block(name) {
            let (binding, _explicit) = state.lookup_global(name, span)?;
            if !matches!(binding, BindingTarget::Global { readonly: false, .. }) && is_write {
                return Err(KError::syntax(
                    format!("attempt to assign to const variable '{name}'"),
                    span,
                ));
            }
            state.record_use(name.to_owned(), span, is_write, binding.clone());
            return Ok(binding);
        }

        if let Some(binding) = state.lookup_local_or_upvalue(name) {
            if is_write && binding.readonly {
                return Err(KError::syntax(
                    format!("attempt to assign to const variable '{}'", name),
                    span,
                ));
            }
            let target = if name == "_ENV"
                && matches!(state.kind, FunctionKind::Chunk)
                && binding.source_depth == 0
                && binding.declaration_span == state.span
            {
                BindingTarget::Upvalue {
                    slot: 0,
                    readonly: false,
                    source_depth: 0,
                    declaration_span: binding.declaration_span,
                }
            } else if binding.source_depth == 0 {
                BindingTarget::Local {
                    slot: binding.slot,
                    readonly: binding.readonly,
                    close: binding.close,
                    declaration_span: binding.declaration_span,
                    block_depth: binding.block_depth,
                }
            } else {
                BindingTarget::Upvalue {
                    slot: binding.slot,
                    readonly: binding.readonly,
                    source_depth: binding.source_depth,
                    declaration_span: binding.declaration_span,
                }
            };
            state.record_use(name.to_owned(), span, is_write, target.clone());
            return Ok(target);
        }

        if matches!(state.kind, FunctionKind::GlobalFunction) && state.globals.contains_key(name)
        {
            let (binding, _explicit) = state.lookup_global(name, span)?;
            state.record_use(name.to_owned(), span, is_write, binding.clone());
            return Ok(binding);
        }

        if let Some((binding, source_depth)) = state.lookup_outer_capture(name) {
            let readonly = binding.readonly;
            if is_write && readonly {
                return Err(KError::syntax(
                    format!("attempt to assign to const variable '{}'", name),
                    span,
                ));
            }
            let captured = state.capture_upvalue(name, readonly, binding.declaration_span, source_depth);
            let target = captured.to_target();
            state.record_use(name.to_owned(), span, is_write, target.clone());
            return Ok(target);
        }

        let (binding, _explicit) = state.lookup_global(name, span)?;
        if !matches!(binding, BindingTarget::Global { readonly: false, .. }) && is_write {
            return Err(KError::syntax(
                format!("attempt to assign to const variable '{}'", name),
                span,
            ));
        }

        state.record_use(name.to_owned(), span, is_write, binding.clone());
        Ok(binding)
    }
}
