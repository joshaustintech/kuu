use crate::ast::{
    BinaryOp, Block, CallExpr, Chunk, Expr, FunctionBody, FunctionName, Stmt, TableField,
    UnaryOp as AstUnaryOp, Var, VarKind,
};
use crate::error::{KError, KResult, KSpan};
use crate::instruction::{
    ArithmeticOp, CompareOp, ConstantIndex, Instruction, JumpOffset, PrototypeIndex, Register,
    UnaryOpKind,
};
use crate::proto::{Constant, Proto, UpvalueDescriptor};
use crate::resolve::{
    BindingTarget, DeclarationKind, EnvironmentTarget, ResolvedFunction, Resolver as ScopeResolver,
};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct Compiler {
    resolver: ScopeResolver,
}

impl Compiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compile_chunk(&mut self, chunk: &Chunk) -> KResult<Proto> {
        let resolved = self.resolver.resolve_chunk(chunk)?;
        let mut compiler =
            FunctionCompiler::new(&resolved.root, None, chunk.block.span, true, 0, false)?;
        compiler.compile_block(&chunk.block, true)?;
        compiler.finish(Some(b"chunk".to_vec()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ConstantKey {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(u64),
    String(Vec<u8>),
}

impl From<&Constant> for ConstantKey {
    fn from(value: &Constant) -> Self {
        match value {
            Constant::Nil => Self::Nil,
            Constant::Boolean(value) => Self::Boolean(*value),
            Constant::Integer(value) => Self::Integer(*value),
            Constant::Number(value) => Self::Number(value.to_bits()),
            Constant::String(bytes) => Self::String(bytes.clone()),
        }
    }
}

#[derive(Debug, Clone)]
struct BlockFrame {
    min_close_slot: Option<u16>,
}

#[derive(Debug, Clone)]
struct LoopFrame {
    break_sites: Vec<usize>,
}

#[derive(Debug, Clone)]
enum Target {
    Local(Register),
    Upvalue(u16),
    Global {
        name: String,
        environment: EnvironmentTarget,
    },
    Table {
        table: Register,
        key: Register,
    },
}

#[derive(Debug)]
struct FunctionCompiler<'a> {
    resolved: &'a ResolvedFunction,
    parent: Option<&'a ResolvedFunction>,
    child_index: usize,
    instructions: Vec<Instruction>,
    nested: Vec<Proto>,
    constants: Vec<Constant>,
    constant_lookup: BTreeMap<ConstantKey, u32>,
    labels: BTreeMap<String, Vec<(KSpan, usize)>>,
    goto_patches: Vec<(usize, String, KSpan)>,
    block_stack: Vec<BlockFrame>,
    loop_stack: Vec<LoopFrame>,
    next_temp: u16,
    stack_size: u16,
    implicit_return: bool,
    parameters: u16,
    vararg: bool,
    span: KSpan,
}

impl<'a> FunctionCompiler<'a> {
    fn new(
        resolved: &'a ResolvedFunction,
        parent: Option<&'a ResolvedFunction>,
        span: KSpan,
        implicit_return: bool,
        parameters: u16,
        vararg: bool,
    ) -> KResult<Self> {
        let local_slots = resolved
            .declarations
            .iter()
            .filter(|record| matches!(record.kind, DeclarationKind::Local))
            .map(|record| record.slot)
            .max()
            .map(|slot| slot.saturating_add(1))
            .unwrap_or(0);
        let next_temp = u16::try_from(local_slots)
            .map_err(|_| KError::bytecode("local slot count exceeds u16"))?;

        Ok(Self {
            resolved,
            parent,
            child_index: 0,
            instructions: Vec::new(),
            nested: Vec::new(),
            constants: Vec::new(),
            constant_lookup: BTreeMap::new(),
            labels: BTreeMap::new(),
            goto_patches: Vec::new(),
            block_stack: Vec::new(),
            loop_stack: Vec::new(),
            next_temp,
            stack_size: next_temp,
            implicit_return,
            parameters,
            vararg,
            span,
        })
    }

    fn finish(mut self, name: Option<Vec<u8>>) -> KResult<Proto> {
        if self.implicit_return && !self.ends_with_return() {
            self.emit_close_for_active_scopes()?;
            self.instructions.push(Instruction::Return {
                first: Register::new(0),
                count: 0,
            });
        }

        self.patch_gotos()?;
        self.stack_size = self.stack_size.max(self.next_temp);

        let upvalues = self.build_upvalues()?;

        Ok(Proto {
            name,
            parameters: self.parameters,
            is_vararg: self.vararg,
            stack_size: self.stack_size,
            upvalues,
            constants: self.constants,
            instructions: self.instructions,
            nested: self.nested,
        })
    }

    fn build_upvalues(&self) -> KResult<Vec<UpvalueDescriptor>> {
        let mut descriptors = Vec::with_capacity(self.resolved.upvalues.len());
        for binding in &self.resolved.upvalues {
            if binding.name == "_ENV" && self.parent.is_none() {
                descriptors.push(UpvalueDescriptor {
                    instack: false,
                    index: 0,
                });
                continue;
            }

            let Some(parent) = self.parent else {
                return Err(KError::bytecode(format!(
                    "missing parent function for upvalue '{}'",
                    binding.name
                )));
            };

            if let Some(decl) = parent.declarations.iter().find(|record| {
                record.name == binding.name
                    && record.span == binding.declaration_span
                    && matches!(record.kind, DeclarationKind::Local)
            }) {
                descriptors.push(UpvalueDescriptor {
                    instack: true,
                    index: u16::try_from(decl.slot)
                        .map_err(|_| KError::bytecode("upvalue slot exceeds u16"))?,
                });
                continue;
            }

            if let Some(upvalue) = parent
                .upvalues
                .iter()
                .find(|upvalue| upvalue.name == binding.name)
            {
                descriptors.push(UpvalueDescriptor {
                    instack: false,
                    index: u16::try_from(upvalue.slot)
                        .map_err(|_| KError::bytecode("upvalue slot exceeds u16"))?,
                });
                continue;
            }

            return Err(KError::bytecode(format!(
                "unable to resolve upvalue '{}'",
                binding.name
            )));
        }

        Ok(descriptors)
    }

    fn compile_block(&mut self, block: &Block, implicit_return: bool) -> KResult<()> {
        self.push_block();
        for stmt in &block.statements {
            self.compile_stmt(stmt)?;
        }

        if let Some(return_stmt) = &block.return_stmt {
            self.compile_return(return_stmt)?;
            self.pop_block_silent();
        } else {
            if implicit_return {
                self.emit_close_for_active_scopes()?;
                self.instructions.push(Instruction::Return {
                    first: Register::new(0),
                    count: 0,
                });
            }
            self.pop_block();
        }

        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> KResult<()> {
        match stmt {
            Stmt::Empty { .. } => Ok(()),
            Stmt::Break { .. } => self.compile_break(),
            Stmt::Goto { span, name } => self.compile_goto(name, *span),
            Stmt::Label { span, name } => {
                self.labels
                    .entry(name.clone())
                    .or_default()
                    .push((*span, self.instructions.len()));
                Ok(())
            }
            Stmt::Do { block, .. } => self.compile_block(block, false),
            Stmt::While {
                condition, block, ..
            } => self.compile_while(condition, block),
            Stmt::Repeat {
                block, condition, ..
            } => self.compile_repeat(block, condition),
            Stmt::If {
                branches,
                else_block,
                ..
            } => self.compile_if(branches, else_block.as_ref()),
            Stmt::NumericFor {
                name,
                start,
                end,
                step,
                block,
                ..
            } => self.compile_numeric_for(name, start, end, step.as_ref(), block),
            Stmt::GenericFor {
                names, iter, block, ..
            } => self.compile_generic_for(names, iter, block),
            Stmt::Function { name, body, .. } => self.compile_named_function(name, body),
            Stmt::LocalFunction { span, name, body } => {
                let slot = self.find_decl_slot(name, *span)?;
                let proto = self.compile_nested_function(body, Some(name.as_bytes().to_vec()))?;
                self.instructions.push(Instruction::Closure {
                    dst: Register::new(slot),
                    proto: PrototypeIndex::new(proto),
                });
                Ok(())
            }
            Stmt::GlobalFunction { name, body, .. } => {
                let proto = self.compile_nested_function(body, Some(name.as_bytes().to_vec()))?;
                let temp = self.alloc_temp()?;
                self.instructions.push(Instruction::Closure {
                    dst: temp,
                    proto: PrototypeIndex::new(proto),
                });
                self.emit_set_global(name, temp)
            }
            Stmt::LocalDecl {
                span,
                prefix_attribute,
                names,
                values,
            } => self.compile_local_decl(*span, prefix_attribute.as_ref(), names, values),
            Stmt::GlobalDecl {
                span,
                names,
                values,
                ..
            } => self.compile_global_decl(*span, names, values),
            Stmt::GlobalAll { .. } => Ok(()),
            Stmt::Assign {
                targets, values, ..
            } => self.compile_assignment(targets, values),
            Stmt::Call { call, .. } => {
                let scratch = self.alloc_temp()?;
                let _ = self.compile_call_into(scratch, call, 0, false)?;
                Ok(())
            }
        }
    }

    fn compile_return(&mut self, return_stmt: &crate::ast::ReturnStmt) -> KResult<()> {
        if return_stmt.values.is_empty() {
            self.emit_close_for_active_scopes()?;
            self.instructions.push(Instruction::Return {
                first: Register::new(0),
                count: 0,
            });
            return Ok(());
        }

        if return_stmt.values.len() == 1
            && matches!(return_stmt.values.first(), Some(Expr::Vararg { .. }))
        {
            let value = return_stmt
                .values
                .first()
                .ok_or_else(|| KError::bytecode("missing return value"))?;
            let start = Register::new(self.next_temp);
            self.compile_expr_into(start, value, usize::MAX)?;
            self.emit_close_for_active_scopes()?;
            self.instructions.push(Instruction::Return {
                first: start,
                count: u16::MAX,
            });
            return Ok(());
        }

        if return_stmt.values.len() == 1
            && matches!(return_stmt.values.first(), Some(Expr::Call { .. }))
            && let Some(Expr::Call { call, .. }) = return_stmt.values.first()
        {
            self.emit_close_for_active_scopes()?;
            let _ = self.compile_call_into(Register::new(self.next_temp), call, 0, true)?;
            return Ok(());
        }

        let start = Register::new(self.next_temp);
        if let Some((last, prefix)) = return_stmt.values.split_last()
            && matches!(last, Expr::Call { .. } | Expr::Vararg { .. })
        {
            let written = self.compile_value_list_into(start, prefix, Some(prefix.len()), false)?;
            let trailing = Register::new(
                start
                    .index()
                    .checked_add(
                        u16::try_from(written)
                            .map_err(|_| KError::bytecode("register overflow"))?,
                    )
                    .ok_or_else(|| KError::bytecode("register overflow"))?,
            );
            self.compile_expr_into(trailing, last, usize::MAX)?;
            self.emit_close_for_active_scopes()?;
            self.instructions.push(Instruction::Return {
                first: start,
                count: u16::MAX,
            });
            return Ok(());
        }
        let written = self.compile_value_list_into(
            start,
            &return_stmt.values,
            Some(return_stmt.values.len()),
            true,
        )?;
        self.emit_close_for_active_scopes()?;
        self.instructions.push(Instruction::Return {
            first: start,
            count: u16::try_from(written)
                .map_err(|_| KError::bytecode("return count exceeds u16"))?,
        });
        Ok(())
    }

    fn compile_break(&mut self) -> KResult<()> {
        self.emit_close_for_active_scopes()?;
        let jump = self.emit_jump_placeholder();
        let Some(loop_frame) = self.loop_stack.last_mut() else {
            return Err(KError::syntax("break outside loop", self.span));
        };
        loop_frame.break_sites.push(jump);
        Ok(())
    }

    fn compile_goto(&mut self, name: &str, span: KSpan) -> KResult<()> {
        if let Some(slot) = self.goto_close_slot(name, span) {
            self.instructions.push(Instruction::Close {
                from: Register::new(slot),
            });
        }
        let jump = self.emit_jump_placeholder();
        self.goto_patches.push((jump, name.to_owned(), span));
        Ok(())
    }

    fn goto_close_slot(&self, name: &str, span: KSpan) -> Option<u16> {
        let source = self
            .resolved
            .gotos
            .iter()
            .find(|record| record.name == name && record.span == span)?;
        let target = self
            .resolved
            .labels
            .iter()
            .filter(|label| label.name == name)
            .filter(|label| source.scope_path.starts_with(&label.scope_path))
            .filter(|label| label.active_decls == source.active_decls)
            .min_by_key(|label| {
                let future = label.span.start_line > span.start_line
                    || (label.span.start_line == span.start_line
                        && label.span.start_column > span.start_column);
                (!future, label.span.start_line, label.span.start_column)
            })
            .or_else(|| {
                self.resolved
                    .labels
                    .iter()
                    .filter(|label| {
                        label.name == name
                            && source.scope_path.starts_with(&label.scope_path)
                            && (label.span.start_line > span.start_line
                                || (label.span.start_line == span.start_line
                                    && label.span.start_column > span.start_column))
                    })
                    .min_by_key(|label| (label.span.start_line, label.span.start_column))
            })
            .or_else(|| {
                self.resolved
                    .labels
                    .iter()
                    .filter(|label| {
                        label.name == name
                            && source.scope_path.starts_with(&label.scope_path)
                            && (label.span.start_line < span.start_line
                                || (label.span.start_line == span.start_line
                                    && label.span.start_column <= span.start_column))
                    })
                    .max_by_key(|label| (label.span.start_line, label.span.start_column))
            })?;
        self.resolved
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration.kind, DeclarationKind::Local))
            .filter(|declaration| {
                source.active_decls.contains(&declaration.id)
                    && !target.active_decls.contains(&declaration.id)
            })
            .filter_map(|declaration| u16::try_from(declaration.slot).ok())
            .min()
    }

    fn compile_if(
        &mut self,
        branches: &[(Expr, Block)],
        else_block: Option<&Block>,
    ) -> KResult<()> {
        let mut end_jumps = Vec::new();
        for (index, (condition, block)) in branches.iter().enumerate() {
            let cond_reg = self.compile_expr_result(condition)?;
            let branch_jump = self.emit_conditional_jump(false, cond_reg);
            self.compile_block(block, false)?;
            if index + 1 != branches.len() || else_block.is_some() {
                end_jumps.push(self.emit_jump_placeholder());
            }
            self.patch_jump(branch_jump, self.instructions.len())?;
        }

        if let Some(block) = else_block {
            self.compile_block(block, false)?;
        }

        let end = self.instructions.len();
        for jump in end_jumps {
            self.patch_jump(jump, end)?;
        }
        Ok(())
    }

    fn compile_while(&mut self, condition: &Expr, block: &Block) -> KResult<()> {
        let loop_start = self.instructions.len();
        let cond_reg = self.compile_expr_result(condition)?;
        let exit_jump = self.emit_conditional_jump(false, cond_reg);
        self.loop_stack.push(LoopFrame {
            break_sites: Vec::new(),
        });
        self.compile_block(block, false)?;
        self.emit_jump(loop_start)?;
        let loop_end = self.instructions.len();
        self.patch_jump(exit_jump, loop_end)?;
        self.patch_loop_breaks(loop_end)?;
        let _ = self.loop_stack.pop();
        Ok(())
    }

    fn compile_repeat(&mut self, block: &Block, condition: &Expr) -> KResult<()> {
        let loop_start = self.instructions.len();
        self.loop_stack.push(LoopFrame {
            break_sites: Vec::new(),
        });
        self.compile_block(block, false)?;
        let cond_reg = self.compile_expr_result(condition)?;
        let exit_jump = self.emit_conditional_jump(false, cond_reg);
        self.patch_jump(exit_jump, loop_start)?;
        self.patch_loop_breaks(self.instructions.len())?;
        let _ = self.loop_stack.pop();
        Ok(())
    }

    fn compile_numeric_for(
        &mut self,
        name: &str,
        start: &Expr,
        end: &Expr,
        step: Option<&Expr>,
        block: &Block,
    ) -> KResult<()> {
        let loop_slot = self.find_loop_slot(name, block.span)?;
        let loop_reg = Register::new(loop_slot);
        let start_reg = self.compile_expr_result(start)?;
        let end_reg = self.compile_expr_result(end)?;
        let step_reg = if let Some(step) = step {
            self.compile_expr_result(step)?
        } else {
            self.load_integer_temp(1)?
        };

        self.emit_move(loop_reg, start_reg)?;
        self.loop_stack.push(LoopFrame {
            break_sites: Vec::new(),
        });
        let loop_start = self.instructions.len();
        let zero_reg = self.load_integer_temp(0)?;
        let negative_reg = self.alloc_temp()?;
        self.instructions.push(Instruction::Compare {
            op: CompareOp::Less,
            dst: negative_reg,
            left: step_reg,
            right: zero_reg,
        });
        let negative_jump = self.emit_conditional_jump(true, negative_reg);

        let positive_cond = self.alloc_temp()?;
        self.instructions.push(Instruction::Compare {
            op: CompareOp::LessEq,
            dst: positive_cond,
            left: loop_reg,
            right: end_reg,
        });
        let exit_positive = self.emit_conditional_jump(false, positive_cond);
        let skip_negative = self.emit_jump_placeholder();

        let negative_branch = self.instructions.len();
        self.patch_jump(negative_jump, negative_branch)?;
        let negative_cond = self.alloc_temp()?;
        self.instructions.push(Instruction::Compare {
            op: CompareOp::GreaterEq,
            dst: negative_cond,
            left: loop_reg,
            right: end_reg,
        });
        let exit_negative = self.emit_conditional_jump(false, negative_cond);

        let body_start = self.instructions.len();
        self.patch_jump(skip_negative, body_start)?;
        self.compile_block(block, false)?;

        let next_value = self.alloc_temp()?;
        self.instructions.push(Instruction::Arithmetic {
            op: ArithmeticOp::Add,
            dst: next_value,
            left: loop_reg,
            right: step_reg,
        });
        self.emit_move(loop_reg, next_value)?;
        self.emit_jump(loop_start)?;

        let loop_end = self.instructions.len();
        self.patch_jump(exit_positive, loop_end)?;
        self.patch_jump(exit_negative, loop_end)?;
        self.patch_loop_breaks(loop_end)?;
        let _ = self.loop_stack.pop();
        Ok(())
    }

    fn compile_generic_for(
        &mut self,
        names: &[String],
        iter: &[Expr],
        block: &Block,
    ) -> KResult<()> {
        if names.is_empty() {
            return Err(KError::bytecode("generic for requires names"));
        }

        if iter.is_empty() {
            return Err(KError::bytecode("generic for missing iterator"));
        }
        let iterator_reg = self.alloc_temp()?;
        let state_reg = self.alloc_temp()?;
        let control_reg = self.alloc_temp()?;
        let _ = self.compile_value_list_into(iterator_reg, iter, Some(3), true)?;

        let first_name = names
            .first()
            .ok_or_else(|| KError::bytecode("generic for requires names"))?;
        let loop_slot = self.find_decl_slot(first_name, block.span)?;
        self.emit(Instruction::LoadNil {
            dst: Register::new(loop_slot),
        });
        self.loop_stack.push(LoopFrame {
            break_sites: Vec::new(),
        });
        let call_base = self.alloc_temp()?;
        let call_slots = usize::max(3, names.len());
        for _ in 1..call_slots {
            let _ = self.alloc_temp()?;
        }
        let call_state = Register::new(
            call_base
                .index()
                .checked_add(1)
                .ok_or_else(|| KError::bytecode("register overflow"))?,
        );
        let call_control = Register::new(
            call_base
                .index()
                .checked_add(2)
                .ok_or_else(|| KError::bytecode("register overflow"))?,
        );
        let loop_start = self.instructions.len();
        self.emit_move(call_base, iterator_reg)?;
        self.emit_move(call_state, state_reg)?;
        self.emit_move(call_control, control_reg)?;
        let call_results = u16::try_from(names.len())
            .map_err(|_| KError::bytecode("generic for arity exceeds u16"))?;
        self.instructions.push(Instruction::Call {
            function: call_base,
            args: 2,
            results: call_results,
        });
        let nil_reg = self.load_nil_temp()?;
        let done_reg = self.alloc_temp()?;
        self.instructions.push(Instruction::Compare {
            op: CompareOp::Eq,
            dst: done_reg,
            left: call_base,
            right: nil_reg,
        });
        let exit = self.emit_conditional_jump(true, done_reg);

        for (index, name) in names.iter().enumerate() {
            let slot = self.find_decl_slot(name, block.span)?;
            let source = Register::new(
                call_base
                    .index()
                    .checked_add(
                        u16::try_from(index).map_err(|_| KError::bytecode("register overflow"))?,
                    )
                    .ok_or_else(|| KError::bytecode("register overflow"))?,
            );
            self.emit_move(Register::new(slot), source)?;
        }

        self.compile_block(block, false)?;
        self.emit_move(control_reg, call_base)?;
        self.emit_jump(loop_start)?;

        let loop_end = self.instructions.len();
        self.patch_jump(exit, loop_end)?;
        self.patch_loop_breaks(loop_end)?;
        let _ = self.loop_stack.pop();
        Ok(())
    }

    fn compile_named_function(&mut self, name: &FunctionName, body: &FunctionBody) -> KResult<()> {
        let proto =
            self.compile_nested_function(body, Some(self.render_function_name(name).into_bytes()))?;
        if name.prefix.len() == 1 && name.method.is_none() {
            let first_prefix = name
                .prefix
                .first()
                .ok_or_else(|| KError::bytecode("missing function name"))?;
            let target = self.find_name_target(first_prefix, name.span, true)?;
            let closure = self.alloc_temp()?;
            self.instructions.push(Instruction::Closure {
                dst: closure,
                proto: PrototypeIndex::new(proto),
            });
            return self.store_name_target(target, first_prefix, closure);
        }

        let prefix = if name.method.is_some() {
            self.compile_name_prefix(&name.prefix, name.span)?
        } else if name.prefix.len() > 1 {
            let (_, prefix_names) = name
                .prefix
                .split_last()
                .ok_or_else(|| KError::bytecode("missing function name"))?;
            self.compile_name_prefix(prefix_names, name.span)?
        } else {
            let first_prefix = name
                .prefix
                .first()
                .ok_or_else(|| KError::bytecode("missing function name"))?;
            self.compile_name_reference(first_prefix, name.span)?
        };

        let field_name = if let Some(method) = &name.method {
            method.clone()
        } else {
            name.prefix
                .last()
                .cloned()
                .ok_or_else(|| KError::bytecode("missing function name"))?
        };

        let closure = self.alloc_temp()?;
        self.instructions.push(Instruction::Closure {
            dst: closure,
            proto: PrototypeIndex::new(proto),
        });
        let key = self.load_string_temp(field_name.as_bytes().to_vec())?;
        self.instructions.push(Instruction::SetTable {
            table: prefix,
            key,
            value: closure,
        });
        Ok(())
    }

    fn compile_local_decl(
        &mut self,
        span: KSpan,
        prefix_attribute: Option<&crate::ast::Attribute>,
        names: &[crate::ast::AttributedName],
        values: &[Expr],
    ) -> KResult<()> {
        let target_count = names.len();
        let start = Register::new(self.next_temp);
        let written = self.compile_value_list_into(start, values, Some(target_count), false)?;
        let prefix_close = matches!(prefix_attribute, Some(attr) if attr.name == "close");
        for (index, name) in names.iter().enumerate() {
            let slot = self.find_decl_slot(&name.name, span)?;
            if prefix_close || matches!(name.attribute.as_ref(), Some(attr) if attr.name == "close")
            {
                self.record_close_slot(usize::from(slot));
            }
            let source = Register::new(
                start
                    .index()
                    .checked_add(
                        u16::try_from(index).map_err(|_| KError::bytecode("register overflow"))?,
                    )
                    .ok_or_else(|| KError::bytecode("register overflow"))?,
            );
            let produced = usize::from(source.index().saturating_sub(start.index()));
            if produced < written {
                self.emit_move(Register::new(slot), source)?;
            } else {
                self.instructions.push(Instruction::LoadNil {
                    dst: Register::new(slot),
                });
            }
        }
        Ok(())
    }

    fn compile_global_decl(
        &mut self,
        span: KSpan,
        names: &[crate::ast::AttributedName],
        values: &[Expr],
    ) -> KResult<()> {
        if values.is_empty() {
            return Ok(());
        }
        for name in names {
            let BindingTarget::Global { environment, .. } =
                self.find_name_target(&name.name, span, true)?
            else {
                return Err(KError::bytecode(
                    "global declaration is not a global binding",
                ));
            };
            self.emit_check_environment(environment, &name.name)?;
        }
        let start = Register::new(self.next_temp);
        let written = self.compile_value_list_into(start, values, Some(names.len()), false)?;
        for (index, name) in names.iter().enumerate() {
            let source = Register::new(
                start
                    .index()
                    .checked_add(
                        u16::try_from(index).map_err(|_| KError::bytecode("register overflow"))?,
                    )
                    .ok_or_else(|| KError::bytecode("register overflow"))?,
            );
            let produced = usize::from(source.index().saturating_sub(start.index()));
            if produced < written {
                let target = self.find_name_target(&name.name, span, true)?;
                self.store_name_target(target, &name.name, source)?;
            } else {
                let nil = self.load_nil_temp()?;
                let target = self.find_name_target(&name.name, span, true)?;
                self.store_name_target(target, &name.name, nil)?;
            }
        }
        Ok(())
    }

    fn compile_assignment(&mut self, targets: &[Var], values: &[Expr]) -> KResult<()> {
        let target_specs = self.compile_targets(targets)?;
        let start = Register::new(self.next_temp);
        let written = self.compile_value_list_into(start, values, Some(targets.len()), false)?;
        for (index, target) in target_specs.iter().enumerate() {
            let source = Register::new(
                start
                    .index()
                    .checked_add(
                        u16::try_from(index).map_err(|_| KError::bytecode("register overflow"))?,
                    )
                    .ok_or_else(|| KError::bytecode("register overflow"))?,
            );
            let produced = usize::from(source.index().saturating_sub(start.index()));
            if produced < written {
                self.store_target(target, source)?;
            } else {
                let nil = self.load_nil_temp()?;
                self.store_target(target, nil)?;
            }
        }
        Ok(())
    }

    fn compile_targets(&mut self, targets: &[Var]) -> KResult<Vec<Target>> {
        let mut result = Vec::with_capacity(targets.len());
        for target in targets {
            match &target.kind {
                VarKind::Name(name) => {
                    let binding = self.find_name_target(name, target.span, true)?;
                    let compiled = match binding {
                        BindingTarget::Local { slot, .. } => {
                            Target::Local(Register::new(slot as u16))
                        }
                        BindingTarget::Upvalue { slot, .. } => Target::Upvalue(
                            u16::try_from(slot)
                                .map_err(|_| KError::bytecode("upvalue slot exceeds u16"))?,
                        ),
                        BindingTarget::Global { environment, .. } => Target::Global {
                            name: name.clone(),
                            environment,
                        },
                    };
                    result.push(compiled);
                }
                VarKind::Field { prefix, name } => {
                    let table = self.compile_expr_result(prefix)?;
                    let key = self.load_string_temp(name.as_bytes().to_vec())?;
                    result.push(Target::Table { table, key });
                }
                VarKind::Index { prefix, index } => {
                    let table = self.compile_expr_result(prefix)?;
                    let key = self.compile_expr_result(index)?;
                    result.push(Target::Table { table, key });
                }
            }
        }
        Ok(result)
    }

    fn compile_expr_result(&mut self, expr: &Expr) -> KResult<Register> {
        let dst = self.alloc_temp()?;
        self.compile_expr_into(dst, expr, 1)?;
        Ok(dst)
    }

    fn compile_expr_into(
        &mut self,
        dst: Register,
        expr: &Expr,
        desired_results: usize,
    ) -> KResult<usize> {
        let reserve_count = if desired_results == usize::MAX {
            1
        } else {
            desired_results.max(1)
        };
        self.reserve_temp_space(dst, reserve_count)?;
        match expr {
            Expr::Nil { .. } => {
                self.instructions.push(Instruction::LoadNil { dst });
                Ok(1)
            }
            Expr::Bool { value, .. } => {
                self.instructions
                    .push(Instruction::LoadBool { dst, value: *value });
                Ok(1)
            }
            Expr::Number { lexeme, .. } => {
                self.compile_number_into(dst, lexeme)?;
                Ok(1)
            }
            Expr::String { bytes, .. } => {
                let index = self.string_constant_index(bytes.clone())?;
                self.instructions.push(Instruction::LoadConstant {
                    dst,
                    constant: ConstantIndex::new(index),
                });
                Ok(1)
            }
            Expr::Vararg { .. } => {
                if desired_results == usize::MAX {
                    self.instructions
                        .push(Instruction::Vararg { dst, count: None });
                    return Ok(usize::MAX);
                }
                self.instructions.push(Instruction::Vararg {
                    dst,
                    count: if desired_results > 1 {
                        Some(
                            u16::try_from(desired_results)
                                .map_err(|_| KError::bytecode("vararg count exceeds u16"))?,
                        )
                    } else {
                        Some(1)
                    },
                });
                Ok(desired_results.max(1))
            }
            Expr::Name { span, name } => {
                let target = self.find_name_target(name, *span, false)?;
                self.compile_name_target_into(dst, target, name)?;
                Ok(1)
            }
            Expr::Paren { expr, .. } => self.compile_expr_into(dst, expr, desired_results),
            Expr::Field { prefix, name, .. } => {
                let table = self.compile_expr_result(prefix)?;
                let key = self.load_string_temp(name.as_bytes().to_vec())?;
                self.instructions
                    .push(Instruction::GetTable { dst, table, key });
                Ok(1)
            }
            Expr::Index { prefix, index, .. } => {
                let table = self.compile_expr_result(prefix)?;
                let key = self.compile_expr_result(index)?;
                self.instructions
                    .push(Instruction::GetTable { dst, table, key });
                Ok(1)
            }
            Expr::Table { constructor, .. } => {
                self.instructions.push(Instruction::NewTable { dst });
                let mut array_index = 1i64;
                for (field_index, field) in constructor.fields.iter().enumerate() {
                    match field {
                        TableField::Array { value, .. } => {
                            let expands = field_index + 1 == constructor.fields.len()
                                && matches!(value, Expr::Call { .. } | Expr::Vararg { .. });
                            if expands {
                                let values = self.alloc_temp()?;
                                self.compile_expr_into(values, value, usize::MAX)?;
                                self.instructions.push(Instruction::SetTableRange {
                                    table: dst,
                                    start: array_index,
                                    values,
                                    count: None,
                                });
                                continue;
                            }
                            let key = self.compile_expr_result(&Expr::Number {
                                span: constructor.span,
                                lexeme: array_index.to_string(),
                            })?;
                            array_index = array_index.saturating_add(1);
                            let value_reg = self.compile_expr_result(value)?;
                            self.instructions.push(Instruction::SetTable {
                                table: dst,
                                key,
                                value: value_reg,
                            });
                        }
                        TableField::Named { name, value, .. } => {
                            let key = self.load_string_temp(name.as_bytes().to_vec())?;
                            let value_reg = self.compile_expr_result(value)?;
                            self.instructions.push(Instruction::SetTable {
                                table: dst,
                                key,
                                value: value_reg,
                            });
                        }
                        TableField::Keyed { key, value, .. } => {
                            let key_reg = self.compile_expr_result(key)?;
                            let value_reg = self.compile_expr_result(value)?;
                            self.instructions.push(Instruction::SetTable {
                                table: dst,
                                key: key_reg,
                                value: value_reg,
                            });
                        }
                    }
                }
                Ok(1)
            }
            Expr::Function { body, .. } => {
                let proto = self.compile_nested_function(body, None)?;
                self.instructions.push(Instruction::Closure {
                    dst,
                    proto: PrototypeIndex::new(proto),
                });
                Ok(1)
            }
            Expr::Unary { op, expr, .. } => self.compile_unary_into(dst, *op, expr),
            Expr::Binary {
                op, left, right, ..
            } => self.compile_binary_into(dst, *op, left, right),
            Expr::Call { call, .. } => self.compile_call_into(dst, call, desired_results, false),
        }
    }

    fn compile_value_list_into(
        &mut self,
        start: Register,
        values: &[Expr],
        desired_total: Option<usize>,
        allow_multi_last: bool,
    ) -> KResult<usize> {
        let mut written = 0usize;
        for (index, value) in values.iter().enumerate() {
            let remaining = desired_total
                .map(|total| total.saturating_sub(written))
                .unwrap_or(1);
            let target = Register::new(
                start
                    .index()
                    .checked_add(
                        u16::try_from(written)
                            .map_err(|_| KError::bytecode("register overflow"))?,
                    )
                    .ok_or_else(|| KError::bytecode("register overflow"))?,
            );
            let count = if index + 1 == values.len() {
                match value {
                    Expr::Call { .. } if allow_multi_last && desired_total.is_none() => usize::MAX,
                    Expr::Vararg { .. } if allow_multi_last && desired_total.is_none() => {
                        usize::MAX
                    }
                    Expr::Call { .. } if remaining > 1 => remaining,
                    Expr::Vararg { .. } if remaining > 1 => remaining,
                    _ => 1,
                }
            } else {
                1
            };

            let produced = self.compile_expr_into(target, value, count)?;
            if produced == usize::MAX {
                return Ok(usize::MAX);
            }
            written = written.saturating_add(produced);
        }

        if let Some(total) = desired_total {
            while written < total {
                let target = Register::new(
                    start
                        .index()
                        .checked_add(
                            u16::try_from(written)
                                .map_err(|_| KError::bytecode("register overflow"))?,
                        )
                        .ok_or_else(|| KError::bytecode("register overflow"))?,
                );
                self.instructions.push(Instruction::LoadNil { dst: target });
                written = written.saturating_add(1);
            }
            self.reserve_temp_space(start, total)?;
        }

        Ok(written)
    }

    fn compile_unary_into(&mut self, dst: Register, op: AstUnaryOp, expr: &Expr) -> KResult<usize> {
        match op {
            AstUnaryOp::Minus => {
                let value = self.compile_expr_result(expr)?;
                self.instructions.push(Instruction::Unary {
                    op: UnaryOpKind::Minus,
                    dst,
                    src: value,
                });
                Ok(1)
            }
            AstUnaryOp::Not => {
                let value = self.compile_expr_result(expr)?;
                self.compile_not_into(dst, value)?;
                Ok(1)
            }
            AstUnaryOp::Len => {
                let value = self.compile_expr_result(expr)?;
                self.instructions.push(Instruction::Unary {
                    op: UnaryOpKind::Len,
                    dst,
                    src: value,
                });
                Ok(1)
            }
            AstUnaryOp::BitNot => {
                let value = self.compile_expr_result(expr)?;
                self.instructions.push(Instruction::Unary {
                    op: UnaryOpKind::BitNot,
                    dst,
                    src: value,
                });
                Ok(1)
            }
        }
    }

    fn compile_binary_into(
        &mut self,
        dst: Register,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> KResult<usize> {
        match op {
            BinaryOp::Or => {
                let left_reg = self.compile_expr_result(left)?;
                self.emit_move(dst, left_reg)?;
                let jump = self.emit_conditional_jump(true, left_reg);
                let right_reg = self.compile_expr_result(right)?;
                self.emit_move(dst, right_reg)?;
                self.patch_jump(jump, self.instructions.len())?;
                Ok(1)
            }
            BinaryOp::And => {
                let left_reg = self.compile_expr_result(left)?;
                self.emit_move(dst, left_reg)?;
                let jump = self.emit_conditional_jump(false, left_reg);
                let right_reg = self.compile_expr_result(right)?;
                self.emit_move(dst, right_reg)?;
                self.patch_jump(jump, self.instructions.len())?;
                Ok(1)
            }
            BinaryOp::Concat => {
                self.reserve_temp_space(dst, 2)?;
                let left_reg = dst;
                let right_reg = Register::new(
                    dst.index()
                        .checked_add(1)
                        .ok_or_else(|| KError::bytecode("register overflow"))?,
                );
                self.compile_expr_into(left_reg, left, 1)?;
                self.compile_expr_into(right_reg, right, 1)?;
                self.instructions.push(Instruction::Concat {
                    dst,
                    first: left_reg,
                    last: right_reg,
                });
                Ok(1)
            }
            BinaryOp::Less
            | BinaryOp::Greater
            | BinaryOp::LessEq
            | BinaryOp::GreaterEq
            | BinaryOp::EqEq
            | BinaryOp::NotEq => {
                let left_reg = self.compile_expr_result(left)?;
                let right_reg = self.compile_expr_result(right)?;
                let compare = match op {
                    BinaryOp::Less => CompareOp::Less,
                    BinaryOp::Greater => CompareOp::Greater,
                    BinaryOp::LessEq => CompareOp::LessEq,
                    BinaryOp::GreaterEq => CompareOp::GreaterEq,
                    BinaryOp::EqEq => CompareOp::Eq,
                    BinaryOp::NotEq => CompareOp::NotEq,
                    _ => return Err(KError::bytecode("invalid compare operator")),
                };
                self.instructions.push(Instruction::Compare {
                    op: compare,
                    dst,
                    left: left_reg,
                    right: right_reg,
                });
                Ok(1)
            }
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::FloorDiv
            | BinaryOp::Mod
            | BinaryOp::Pow => {
                let left_reg = self.compile_expr_result(left)?;
                let right_reg = self.compile_expr_result(right)?;
                let arithmetic = match op {
                    BinaryOp::Add => ArithmeticOp::Add,
                    BinaryOp::Sub => ArithmeticOp::Sub,
                    BinaryOp::Mul => ArithmeticOp::Mul,
                    BinaryOp::Div => ArithmeticOp::Div,
                    BinaryOp::FloorDiv => ArithmeticOp::FloorDiv,
                    BinaryOp::Mod => ArithmeticOp::Mod,
                    BinaryOp::Pow => ArithmeticOp::Pow,
                    _ => return Err(KError::bytecode("invalid arithmetic operator")),
                };
                self.instructions.push(Instruction::Arithmetic {
                    op: arithmetic,
                    dst,
                    left: left_reg,
                    right: right_reg,
                });
                Ok(1)
            }
            BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitAnd
            | BinaryOp::ShiftLeft
            | BinaryOp::ShiftRight => {
                let left_reg = self.compile_expr_result(left)?;
                let right_reg = self.compile_expr_result(right)?;
                let arithmetic = match op {
                    BinaryOp::BitOr => ArithmeticOp::BitOr,
                    BinaryOp::BitXor => ArithmeticOp::BitXor,
                    BinaryOp::BitAnd => ArithmeticOp::BitAnd,
                    BinaryOp::ShiftLeft => ArithmeticOp::ShiftLeft,
                    BinaryOp::ShiftRight => ArithmeticOp::ShiftRight,
                    _ => return Err(KError::bytecode("invalid bitwise operator")),
                };
                self.instructions.push(Instruction::Arithmetic {
                    op: arithmetic,
                    dst,
                    left: left_reg,
                    right: right_reg,
                });
                Ok(1)
            }
        }
    }

    fn compile_not_into(&mut self, dst: Register, value: Register) -> KResult<()> {
        let jump_if_true = self.emit_conditional_jump(true, value);
        self.instructions
            .push(Instruction::LoadBool { dst, value: true });
        let end = self.emit_jump_placeholder();
        self.patch_jump(jump_if_true, self.instructions.len())?;
        self.instructions
            .push(Instruction::LoadBool { dst, value: false });
        self.patch_jump(end, self.instructions.len())?;
        Ok(())
    }

    fn compile_call_into(
        &mut self,
        dst: Register,
        call: &CallExpr,
        desired_results: usize,
        tailcall: bool,
    ) -> KResult<usize> {
        let layout_floor = if call.method.is_some() {
            dst.index()
                .checked_add(16)
                .ok_or_else(|| KError::bytecode("register overflow"))?
        } else {
            dst.index()
                .checked_add(12)
                .ok_or_else(|| KError::bytecode("register overflow"))?
        };
        self.reserve_temp_floor(layout_floor)?;
        if tailcall {
            if let Some(method) = &call.method {
                let receiver = Register::new(
                    dst.index()
                        .checked_add(1)
                        .ok_or_else(|| KError::bytecode("register overflow"))?,
                );
                let key = Register::new(
                    dst.index()
                        .checked_add(2)
                        .ok_or_else(|| KError::bytecode("register overflow"))?,
                );
                let prefix = self.compile_expr_result(&call.prefix)?;
                self.emit_move(receiver, prefix)?;
                let key_index = self.string_constant_index(method.as_bytes().to_vec())?;
                self.instructions.push(Instruction::LoadConstant {
                    dst: key,
                    constant: ConstantIndex::new(key_index),
                });
                self.instructions.push(Instruction::GetTable {
                    dst,
                    table: receiver,
                    key,
                });
                let arg_count = self.compile_call_args(key, &call.args, true)?;
                self.instructions.push(Instruction::TailCall {
                    function: dst,
                    args: if arg_count == usize::MAX {
                        u16::MAX
                    } else {
                        u16::try_from(arg_count + 1)
                            .map_err(|_| KError::bytecode("tail call arity exceeds u16"))?
                    },
                });
                return Ok(0);
            }
            let prefix = self.compile_expr_result(&call.prefix)?;
            self.emit_move(dst, prefix)?;
            let arg_count = self.compile_call_args(
                Register::new(
                    dst.index()
                        .checked_add(1)
                        .ok_or_else(|| KError::bytecode("register overflow"))?,
                ),
                &call.args,
                true,
            )?;
            self.instructions.push(Instruction::TailCall {
                function: dst,
                args: if arg_count == usize::MAX {
                    u16::MAX
                } else {
                    u16::try_from(arg_count)
                        .map_err(|_| KError::bytecode("tail call arity exceeds u16"))?
                },
            });
            return Ok(0);
        }

        if let Some(method) = &call.method {
            let receiver = Register::new(
                dst.index()
                    .checked_add(1)
                    .ok_or_else(|| KError::bytecode("register overflow"))?,
            );
            let key = Register::new(
                dst.index()
                    .checked_add(2)
                    .ok_or_else(|| KError::bytecode("register overflow"))?,
            );
            let prefix = self.compile_expr_result(&call.prefix)?;
            self.emit_move(receiver, prefix)?;
            let key_index = self.string_constant_index(method.as_bytes().to_vec())?;
            self.instructions.push(Instruction::LoadConstant {
                dst: key,
                constant: ConstantIndex::new(key_index),
            });
            self.instructions.push(Instruction::GetTable {
                dst,
                table: receiver,
                key,
            });
            let arg_count = self.compile_call_args(key, &call.args, true)?;
            self.instructions.push(Instruction::Call {
                function: dst,
                args: if arg_count == usize::MAX {
                    u16::MAX
                } else {
                    u16::try_from(arg_count + 1)
                        .map_err(|_| KError::bytecode("call arity exceeds u16"))?
                },
                results: if desired_results == usize::MAX {
                    u16::MAX
                } else {
                    u16::try_from(desired_results)
                        .map_err(|_| KError::bytecode("result count exceeds u16"))?
                },
            });
            Ok(if desired_results == usize::MAX {
                usize::MAX
            } else {
                desired_results.max(1)
            })
        } else {
            let prefix = self.compile_expr_result(&call.prefix)?;
            self.emit_move(dst, prefix)?;
            let arg_count = self.compile_call_args(
                Register::new(
                    dst.index()
                        .checked_add(1)
                        .ok_or_else(|| KError::bytecode("register overflow"))?,
                ),
                &call.args,
                true,
            )?;
            self.instructions.push(Instruction::Call {
                function: dst,
                args: if arg_count == usize::MAX {
                    u16::MAX
                } else {
                    u16::try_from(arg_count)
                        .map_err(|_| KError::bytecode("call arity exceeds u16"))?
                },
                results: if desired_results == usize::MAX {
                    u16::MAX
                } else {
                    u16::try_from(desired_results)
                        .map_err(|_| KError::bytecode("result count exceeds u16"))?
                },
            });
            Ok(if desired_results == usize::MAX {
                usize::MAX
            } else {
                desired_results.max(1)
            })
        }
    }

    fn compile_call_args(
        &mut self,
        start: Register,
        args: &[Expr],
        allow_multi_last: bool,
    ) -> KResult<usize> {
        self.compile_value_list_into(start, args, None, allow_multi_last)
    }

    fn compile_name_target_into(
        &mut self,
        dst: Register,
        target: BindingTarget,
        name: &str,
    ) -> KResult<()> {
        match target {
            BindingTarget::Local { slot, .. } => {
                let source = Register::new(
                    u16::try_from(slot).map_err(|_| KError::bytecode("local slot exceeds u16"))?,
                );
                self.emit_move(dst, source)
            }
            BindingTarget::Upvalue { slot, .. } => {
                let slot = u16::try_from(slot)
                    .map_err(|_| KError::bytecode("upvalue slot exceeds u16"))?;
                self.instructions
                    .push(Instruction::GetUpvalue { dst, upvalue: slot });
                Ok(())
            }
            BindingTarget::Global { environment, .. } => {
                self.emit_get_environment(environment, name, dst)
            }
        }
    }

    fn compile_nested_function(
        &mut self,
        body: &FunctionBody,
        name: Option<Vec<u8>>,
    ) -> KResult<u32> {
        let child = self.take_child()?;
        let mut compiler = FunctionCompiler::new(
            child,
            Some(self.resolved),
            body.span,
            false,
            u16::try_from(body.parameters.len())
                .map_err(|_| KError::bytecode("parameter count exceeds u16"))?,
            body.is_vararg,
        )?;
        compiler.compile_block(&body.block, true)?;
        let proto = compiler.finish(name)?;
        let index = u32::try_from(self.nested.len())
            .map_err(|_| KError::bytecode("nested prototype count exceeds u32"))?;
        self.nested.push(proto);
        Ok(index)
    }

    fn take_child(&mut self) -> KResult<&'a ResolvedFunction> {
        let child = self
            .resolved
            .children
            .get(self.child_index)
            .ok_or_else(|| KError::bytecode("missing nested function"))?;
        self.child_index = self.child_index.saturating_add(1);
        Ok(child)
    }

    fn find_decl_slot(&self, name: &str, span: KSpan) -> KResult<u16> {
        self.resolved
            .declarations
            .iter()
            .find(|record| record.name == name && record.span == span)
            .map(|record| {
                u16::try_from(record.slot)
                    .map_err(|_| KError::bytecode("declaration slot exceeds u16"))
            })
            .transpose()?
            .ok_or_else(|| KError::bytecode(format!("missing declaration slot for '{name}'")))
    }

    fn find_loop_slot(&self, name: &str, span: KSpan) -> KResult<u16> {
        self.resolved
            .declarations
            .iter()
            .find(|record| {
                record.name == name
                    && record.span == span
                    && matches!(record.kind, DeclarationKind::Local)
            })
            .map(|record| {
                u16::try_from(record.slot)
                    .map_err(|_| KError::bytecode("loop variable slot exceeds u16"))
            })
            .transpose()?
            .ok_or_else(|| KError::bytecode(format!("missing loop variable slot for '{name}'")))
    }

    fn find_name_target(&self, name: &str, span: KSpan, is_write: bool) -> KResult<BindingTarget> {
        self.resolved
            .uses
            .iter()
            .find(|entry| entry.name == name && entry.span == span && entry.is_write == is_write)
            .map(|entry| entry.binding.clone())
            .ok_or_else(|| KError::bytecode(format!("missing binding for '{name}'")))
    }

    fn store_target(&mut self, target: &Target, value: Register) -> KResult<()> {
        match target {
            Target::Local(dst) => self.emit_move(*dst, value),
            Target::Upvalue(slot) => {
                self.instructions.push(Instruction::SetUpvalue {
                    src: value,
                    upvalue: *slot,
                });
                Ok(())
            }
            Target::Global { name, environment } => {
                self.emit_set_environment(environment.clone(), name, value)
            }
            Target::Table { table, key } => {
                self.instructions.push(Instruction::SetTable {
                    table: *table,
                    key: *key,
                    value,
                });
                Ok(())
            }
        }
    }

    fn emit_get_global(&mut self, name: &str, dst: Register) -> KResult<()> {
        let index = self.string_constant_index(name.as_bytes().to_vec())?;
        self.instructions.push(Instruction::GetGlobal {
            dst,
            name: ConstantIndex::new(index),
        });
        Ok(())
    }

    fn emit_set_global(&mut self, name: &str, src: Register) -> KResult<()> {
        let index = self.string_constant_index(name.as_bytes().to_vec())?;
        self.instructions.push(Instruction::SetGlobal {
            src,
            name: ConstantIndex::new(index),
        });
        Ok(())
    }

    fn emit_get_environment(
        &mut self,
        environment: EnvironmentTarget,
        name: &str,
        dst: Register,
    ) -> KResult<()> {
        let table = self.alloc_temp()?;
        self.load_environment(environment, table)?;
        let key = self.load_string_temp(name.as_bytes().to_vec())?;
        self.instructions
            .push(Instruction::GetTable { dst, table, key });
        Ok(())
    }

    fn emit_set_environment(
        &mut self,
        environment: EnvironmentTarget,
        name: &str,
        value: Register,
    ) -> KResult<()> {
        let table = self.alloc_temp()?;
        self.load_environment(environment, table)?;
        let key = self.load_string_temp(name.as_bytes().to_vec())?;
        self.instructions
            .push(Instruction::SetTable { table, key, value });
        Ok(())
    }

    fn emit_check_environment(
        &mut self,
        environment: EnvironmentTarget,
        name: &str,
    ) -> KResult<()> {
        let table = self.alloc_temp()?;
        self.load_environment(environment, table)?;
        let key = self.load_string_temp(name.as_bytes().to_vec())?;
        self.instructions
            .push(Instruction::CheckGlobal { table, key });
        Ok(())
    }

    fn load_environment(&mut self, environment: EnvironmentTarget, dst: Register) -> KResult<()> {
        match environment {
            EnvironmentTarget::Local { slot } => self.emit_move(
                dst,
                Register::new(
                    u16::try_from(slot)
                        .map_err(|_| KError::bytecode("_ENV local slot exceeds u16"))?,
                ),
            ),
            EnvironmentTarget::Upvalue { slot } => {
                self.instructions.push(Instruction::GetUpvalue {
                    dst,
                    upvalue: u16::try_from(slot)
                        .map_err(|_| KError::bytecode("_ENV upvalue slot exceeds u16"))?,
                });
                Ok(())
            }
        }
    }

    fn string_constant_index(&mut self, bytes: Vec<u8>) -> KResult<u32> {
        let constant = Constant::String(bytes);
        let key = ConstantKey::from(&constant);
        if let Some(index) = self.constant_lookup.get(&key) {
            return Ok(*index);
        }

        let index = u32::try_from(self.constants.len())
            .map_err(|_| KError::bytecode("constant pool exceeds u32"))?;
        self.constants.push(constant);
        self.constant_lookup.insert(key, index);
        Ok(index)
    }

    fn compile_number_into(&mut self, dst: Register, lexeme: &str) -> KResult<()> {
        if let Some(value) = parse_integer_literal(lexeme) {
            self.instructions
                .push(Instruction::LoadInteger { dst, value });
            return Ok(());
        }

        let value = parse_number_literal(lexeme)
            .ok_or_else(|| KError::bytecode(format!("invalid numeric literal {lexeme}")))?;
        self.instructions
            .push(Instruction::LoadNumber { dst, value });
        Ok(())
    }

    fn load_integer_temp(&mut self, value: i64) -> KResult<Register> {
        let dst = self.alloc_temp()?;
        self.instructions
            .push(Instruction::LoadInteger { dst, value });
        Ok(dst)
    }

    fn alloc_temp(&mut self) -> KResult<Register> {
        let reg = self.next_temp;
        self.next_temp = self
            .next_temp
            .checked_add(1)
            .ok_or_else(|| KError::bytecode("register overflow"))?;
        self.stack_size = self.stack_size.max(self.next_temp);
        Ok(Register::new(reg))
    }

    fn reserve_temp_floor(&mut self, floor: u16) -> KResult<()> {
        if self.next_temp < floor {
            self.next_temp = floor;
            self.stack_size = self.stack_size.max(self.next_temp);
        }
        Ok(())
    }

    fn reserve_temp_space(&mut self, start: Register, count: usize) -> KResult<()> {
        let count =
            u16::try_from(count).map_err(|_| KError::bytecode("register span exceeds u16"))?;
        let end = start
            .index()
            .checked_add(count)
            .ok_or_else(|| KError::bytecode("register overflow"))?;
        self.reserve_temp_floor(end)
    }

    fn emit_move(&mut self, dst: Register, src: Register) -> KResult<()> {
        if dst != src {
            self.instructions.push(Instruction::Move { dst, src });
        }
        Ok(())
    }

    fn emit_jump_placeholder(&mut self) -> usize {
        let index = self.instructions.len();
        self.instructions.push(Instruction::Jump {
            offset: JumpOffset::from_i32(0),
        });
        index
    }

    fn emit_jump(&mut self, target: usize) -> KResult<()> {
        let index = self.emit_jump_placeholder();
        self.patch_jump(index, target)
    }

    fn emit_conditional_jump(&mut self, jump_if_true: bool, cond: Register) -> usize {
        let index = self.instructions.len();
        let instruction = if jump_if_true {
            Instruction::JumpIfTrue {
                cond,
                offset: JumpOffset::from_i32(0),
            }
        } else {
            Instruction::JumpIfFalse {
                cond,
                offset: JumpOffset::from_i32(0),
            }
        };
        self.instructions.push(instruction);
        index
    }

    fn patch_jump(&mut self, index: usize, target: usize) -> KResult<()> {
        let offset = i64::try_from(target)
            .and_then(|target| i64::try_from(index).map(|index| target - index - 1))
            .map_err(|_| KError::bytecode("jump offset overflow"))?;
        let jump = JumpOffset::new(offset)?;
        let Some(instruction) = self.instructions.get_mut(index) else {
            return Err(KError::bytecode("invalid jump patch index"));
        };
        match instruction {
            Instruction::Jump { offset }
            | Instruction::ForPrep { offset, .. }
            | Instruction::ForLoop { offset, .. }
            | Instruction::JumpIfTrue { offset, .. }
            | Instruction::JumpIfFalse { offset, .. } => {
                *offset = jump;
                Ok(())
            }
            _ => Err(KError::bytecode("patch target is not a jump")),
        }
    }

    fn patch_gotos(&mut self) -> KResult<()> {
        let patches = self.goto_patches.clone();
        for (index, name, goto_span) in patches {
            let goto_record = self
                .resolved
                .gotos
                .iter()
                .find(|record| record.name == name && record.span == goto_span);
            let goto_active_decls = goto_record.map(|record| &record.active_decls);
            let goto_scope_path = goto_record.map(|record| &record.scope_path);
            let labels = self
                .labels
                .get(&name)
                .ok_or_else(|| KError::bytecode(format!("unresolved label '{name}'")))?;
            let target = self
                .resolved
                .labels
                .iter()
                .filter(|label| label.name == name)
                .filter(|label| {
                    goto_scope_path.is_some_and(|path| path.starts_with(&label.scope_path))
                })
                .filter(|label| {
                    goto_active_decls.is_some_and(|active| label.active_decls == *active)
                })
                .filter_map(|label| {
                    labels
                        .iter()
                        .find(|(span, _)| *span == label.span)
                        .map(|(_, target)| (label.span, *target))
                })
                .min_by_key(|(span, _)| {
                    let future = span.start_line > goto_span.start_line
                        || (span.start_line == goto_span.start_line
                            && span.start_column > goto_span.start_column);
                    (!future, span.start_line, span.start_column)
                })
                .map(|(_, target)| target)
                .or_else(|| {
                    self.resolved
                        .labels
                        .iter()
                        .filter(|label| {
                            label.name == name
                                && goto_scope_path
                                    .is_some_and(|path| path.starts_with(&label.scope_path))
                                && (label.span.start_line > goto_span.start_line
                                    || (label.span.start_line == goto_span.start_line
                                        && label.span.start_column > goto_span.start_column))
                        })
                        .min_by_key(|label| (label.span.start_line, label.span.start_column))
                        .and_then(|label| {
                            labels
                                .iter()
                                .find(|(span, _)| *span == label.span)
                                .map(|(_, target)| *target)
                        })
                })
                .or_else(|| {
                    self.resolved
                        .labels
                        .iter()
                        .filter(|label| {
                            label.name == name
                                && goto_scope_path
                                    .is_some_and(|path| path.starts_with(&label.scope_path))
                                && (label.span.start_line < goto_span.start_line
                                    || (label.span.start_line == goto_span.start_line
                                        && label.span.start_column <= goto_span.start_column))
                        })
                        .max_by_key(|label| (label.span.start_line, label.span.start_column))
                        .and_then(|label| {
                            labels
                                .iter()
                                .find(|(span, _)| *span == label.span)
                                .map(|(_, target)| *target)
                        })
                })
                .ok_or_else(|| KError::bytecode(format!("unresolved label '{name}'")))?;
            self.patch_jump(index, target)?;
        }
        Ok(())
    }

    fn patch_loop_breaks(&mut self, target: usize) -> KResult<()> {
        let break_sites = if let Some(frame) = self.loop_stack.last_mut() {
            frame.break_sites.drain(..).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for index in break_sites {
            self.patch_jump(index, target)?;
        }
        Ok(())
    }

    fn current_close_slot(&self) -> Option<u16> {
        self.block_stack
            .iter()
            .filter_map(|frame| frame.min_close_slot)
            .min()
    }

    fn record_close_slot(&mut self, slot: usize) {
        let Some(slot) = u16::try_from(slot).ok() else {
            return;
        };
        if let Some(frame) = self.block_stack.last_mut() {
            frame.min_close_slot = Some(match frame.min_close_slot {
                Some(previous) => previous.min(slot),
                None => slot,
            });
        }
    }

    fn emit_close_for_active_scopes(&mut self) -> KResult<()> {
        if let Some(slot) = self.current_close_slot() {
            self.instructions.push(Instruction::Close {
                from: Register::new(slot),
            });
        }
        Ok(())
    }

    fn push_block(&mut self) {
        self.block_stack.push(BlockFrame {
            min_close_slot: None,
        });
    }

    fn pop_block(&mut self) {
        if let Some(frame) = self.block_stack.pop()
            && let Some(slot) = frame.min_close_slot
        {
            self.instructions.push(Instruction::Close {
                from: Register::new(slot),
            });
        }
    }

    fn pop_block_silent(&mut self) {
        let _ = self.block_stack.pop();
    }

    fn compile_name_prefix(&mut self, names: &[String], span: KSpan) -> KResult<Register> {
        let first = names
            .first()
            .ok_or_else(|| KError::bytecode("missing prefix name"))?;
        let mut current = self.compile_name_reference(first, span)?;
        for name in names.iter().skip(1) {
            let key = self.load_string_temp(name.as_bytes().to_vec())?;
            let next = self.alloc_temp()?;
            self.instructions.push(Instruction::GetTable {
                dst: next,
                table: current,
                key,
            });
            current = next;
        }
        Ok(current)
    }

    fn store_name_target(
        &mut self,
        target: BindingTarget,
        name: &str,
        value: Register,
    ) -> KResult<()> {
        match target {
            BindingTarget::Local { slot, .. } => {
                let slot =
                    u16::try_from(slot).map_err(|_| KError::bytecode("local slot exceeds u16"))?;
                self.emit_move(Register::new(slot), value)
            }
            BindingTarget::Upvalue { slot, .. } => {
                let slot = u16::try_from(slot)
                    .map_err(|_| KError::bytecode("upvalue slot exceeds u16"))?;
                self.instructions.push(Instruction::SetUpvalue {
                    src: value,
                    upvalue: slot,
                });
                Ok(())
            }
            BindingTarget::Global { environment, .. } => {
                self.emit_set_environment(environment, name, value)
            }
        }
    }

    fn compile_name_reference(&mut self, name: &str, span: KSpan) -> KResult<Register> {
        if let Ok(target) = self.find_name_target(name, span, false) {
            let dst = self.alloc_temp()?;
            self.compile_name_target_into(dst, target, name)?;
            return Ok(dst);
        }

        let dst = self.alloc_temp()?;
        self.emit_get_global(name, dst)?;
        Ok(dst)
    }

    fn load_nil_temp(&mut self) -> KResult<Register> {
        let dst = self.alloc_temp()?;
        self.instructions.push(Instruction::LoadNil { dst });
        Ok(dst)
    }

    fn load_string_temp(&mut self, bytes: Vec<u8>) -> KResult<Register> {
        let dst = self.alloc_temp()?;
        let index = self.string_constant_index(bytes)?;
        self.instructions.push(Instruction::LoadConstant {
            dst,
            constant: ConstantIndex::new(index),
        });
        Ok(dst)
    }

    fn emit(&mut self, instruction: Instruction) {
        self.instructions.push(instruction);
    }

    fn render_function_name(&self, name: &FunctionName) -> String {
        let mut out = name.prefix.join(".");
        if let Some(method) = &name.method {
            if !out.is_empty() {
                out.push(':');
            }
            out.push_str(method);
        }
        out
    }

    fn ends_with_return(&self) -> bool {
        matches!(
            self.instructions.last(),
            Some(Instruction::Return { .. } | Instruction::TailCall { .. })
        )
    }
}

fn parse_integer_literal(lexeme: &str) -> Option<i64> {
    if let Some(digits) = lexeme
        .strip_prefix("0x")
        .or_else(|| lexeme.strip_prefix("0X"))
    {
        if digits.contains(['.', 'p', 'P']) {
            return None;
        }
        let value = u64::from_str_radix(digits, 16).ok()?;
        return Some(i64::from_ne_bytes(value.to_ne_bytes()));
    }
    lexeme.parse::<i64>().ok()
}

fn parse_number_literal(lexeme: &str) -> Option<f64> {
    if !lexeme.starts_with("0x") && !lexeme.starts_with("0X") {
        return lexeme.parse::<f64>().ok();
    }
    let digits = lexeme.get(2..)?;
    let (mantissa, exponent) = digits
        .split_once(['p', 'P'])
        .map_or((digits, 0), |(mantissa, exponent)| {
            (mantissa, exponent.parse::<i32>().unwrap_or(0))
        });
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let whole = if whole.is_empty() {
        0.0
    } else {
        u64::from_str_radix(whole, 16).ok()? as f64
    };
    let fraction = fraction
        .chars()
        .enumerate()
        .try_fold(0.0, |value, (index, digit)| {
            let digit = digit.to_digit(16)? as f64;
            Some(value + digit * 16.0_f64.powi(-i32::try_from(index + 1).ok()?))
        })?;
    Some((whole + fraction) * 2.0_f64.powi(exponent))
}
