use crate::instruction::Instruction;
use crate::parser::{BinOp, Block, Expr, PrefixExpr, Stmt, TableField, UnOp};

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct UpvalDesc {
    pub name: Option<String>,
    pub in_stack: bool,
    pub idx: u8,
}

#[derive(Debug, Clone)]
pub struct Proto {
    pub instructions: Vec<Instruction>,
    pub constants: Vec<Constant>,
    pub sub_protos: Vec<Proto>,
    pub num_params: u8,
    pub is_vararg: bool,
    pub max_stack_size: u8,
    pub upvalues: Vec<UpvalDesc>,
}

impl Proto {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            constants: Vec::new(),
            sub_protos: Vec::new(),
            num_params: 0,
            is_vararg: false,
            max_stack_size: 2,
            upvalues: Vec::new(),
        }
    }
}

impl Default for Proto {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum CompileError {
    Message(String),
}

#[derive(Debug, Clone)]
pub struct LocalVar {
    pub name: String,
    pub reg: u8,
    pub active: bool,
}

pub struct FuncState {
    pub proto: Proto,
    pub locals: Vec<LocalVar>,
    pub active_registers: u8,
}

impl FuncState {
    pub fn new() -> Self {
        Self {
            proto: Proto::new(),
            locals: Vec::new(),
            active_registers: 0,
        }
    }

    pub fn alloc_reg(&mut self) -> u8 {
        let r = self.active_registers;
        self.active_registers += 1;
        if self.active_registers > self.proto.max_stack_size {
            self.proto.max_stack_size = self.active_registers;
        }
        r
    }

    pub fn free_regs(&mut self, n: u8) {
        self.active_registers -= n;
    }

    pub fn set_active_registers(&mut self, val: u8) {
        self.active_registers = val;
        if self.active_registers > self.proto.max_stack_size {
            self.proto.max_stack_size = self.active_registers;
        }
    }

    pub fn emit(&mut self, inst: Instruction) -> usize {
        let idx = self.proto.instructions.len();
        self.proto.instructions.push(inst);
        idx
    }

    pub fn add_constant(&mut self, val: Constant) -> u16 {
        for (i, c) in self.proto.constants.iter().enumerate() {
            if *c == val {
                return i as u16;
            }
        }
        let idx = self.proto.constants.len();
        self.proto.constants.push(val);
        idx as u16
    }

    pub fn patch_jmp(&mut self, jmp_idx: usize, target_idx: usize) {
        let offset = (target_idx as i32) - (jmp_idx as i32) - 1;
        if let Instruction::Jmp {
            offset: ref mut off,
        } = self.proto.instructions[jmp_idx]
        {
            *off = offset;
        }
    }
}

impl Default for FuncState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Compiler {
    pub states: Vec<FuncState>,
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}

impl Compiler {
    pub fn new() -> Self {
        Self { states: Vec::new() }
    }

    fn active_state(&self) -> &FuncState {
        self.states.last().expect("no active function state")
    }

    fn active_state_mut(&mut self) -> &mut FuncState {
        self.states.last_mut().expect("no active function state")
    }

    fn alloc_reg(&mut self) -> u8 {
        self.active_state_mut().alloc_reg()
    }

    fn free_regs(&mut self, n: u8) {
        self.active_state_mut().free_regs(n);
    }

    fn set_active_registers(&mut self, val: u8) {
        self.active_state_mut().set_active_registers(val);
    }

    fn emit(&mut self, inst: Instruction) -> usize {
        self.active_state_mut().emit(inst)
    }

    fn add_constant(&mut self, val: Constant) -> u16 {
        self.active_state_mut().add_constant(val)
    }

    fn patch_jmp(&mut self, jmp_idx: usize, target_idx: usize) {
        self.active_state_mut().patch_jmp(jmp_idx, target_idx);
    }

    pub fn compile_chunk(block: &Block) -> Result<Proto, CompileError> {
        let mut compiler = Self::new();
        let mut main_state = FuncState::new();
        // The main chunk always has _ENV as upvalue 0
        main_state.proto.upvalues.push(UpvalDesc {
            name: Some("_ENV".to_string()),
            in_stack: true,
            idx: 0,
        });
        compiler.states.push(main_state);

        compiler.compile_block(block)?;
        // Emit implicit return
        compiler.emit(Instruction::Return { start: 0, count: 0 });

        let main_chunk = compiler.states.pop().unwrap().proto;
        Ok(main_chunk)
    }

    fn compile_block(&mut self, block: &Block) -> Result<(), CompileError> {
        let prev_locals = self.active_state().locals.len();
        let prev_regs = self.active_state().active_registers;

        for stmt in &block.stmts {
            self.compile_stmt(stmt)?;
        }

        // Pop locals defined in this block
        self.active_state_mut().locals.truncate(prev_locals);
        self.set_active_registers(prev_regs);
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        match stmt {
            Stmt::LocalAssign { names, values } => {
                let start_reg = self.active_state().active_registers;
                for name in names {
                    let reg = self.alloc_reg();
                    self.active_state_mut().locals.push(LocalVar {
                        name: name.clone(),
                        reg,
                        active: true,
                    });
                }
                self.compile_expr_list_adjusted(values, start_reg, names.len() as u8)?;
            }
            Stmt::LocalFunction {
                name,
                params,
                is_vararg,
                body,
            } => {
                let reg = self.alloc_reg();
                // Declare local name first so the function can reference itself recursively
                self.active_state_mut().locals.push(LocalVar {
                    name: name.clone(),
                    reg,
                    active: true,
                });

                let expr = Expr::FunctionDef {
                    params: params.clone(),
                    is_vararg: *is_vararg,
                    body: Box::new(body.clone()),
                };
                self.compile_expr(&expr, reg)?;
            }
            Stmt::Function {
                name,
                params,
                is_vararg,
                body,
            } => {
                let start_reg = self.active_state().active_registers;

                let mut actual_params = params.clone();
                if name.method.is_some() {
                    actual_params.insert(0, "self".to_string());
                }

                let reg = self.alloc_reg();
                let expr = Expr::FunctionDef {
                    params: actual_params,
                    is_vararg: *is_vararg,
                    body: Box::new(body.clone()),
                };
                self.compile_expr(&expr, reg)?;

                if name.parts.len() == 1 && name.method.is_none() {
                    let var_name = &name.parts[0];
                    match self.resolve_var(var_name) {
                        VarLocation::Local(dst_reg) => {
                            self.emit(Instruction::Move {
                                dst: dst_reg,
                                src: reg,
                            });
                        }
                        VarLocation::Upvalue(idx) => {
                            self.emit(Instruction::SetUpval {
                                upval_idx: idx,
                                src: reg,
                            });
                        }
                        VarLocation::Global(const_idx) => {
                            self.emit(Instruction::SetTabUp {
                                upval_idx: 0,
                                key_const: const_idx,
                                src: reg,
                            });
                        }
                    }
                } else {
                    let mut current_reg = self.alloc_reg();
                    let first_part = &name.parts[0];
                    match self.resolve_var(first_part) {
                        VarLocation::Local(local_reg) => {
                            self.emit(Instruction::Move {
                                dst: current_reg,
                                src: local_reg,
                            });
                        }
                        VarLocation::Upvalue(idx) => {
                            self.emit(Instruction::GetUpval {
                                dst: current_reg,
                                upval_idx: idx,
                            });
                        }
                        VarLocation::Global(const_idx) => {
                            self.emit(Instruction::GetTabUp {
                                dst: current_reg,
                                upval_idx: 0,
                                key_const: const_idx,
                            });
                        }
                    }

                    let (traverse_parts, last_key) = if let Some(ref m) = name.method {
                        (&name.parts[1..], m.clone())
                    } else {
                        let len = name.parts.len();
                        (&name.parts[1..len - 1], name.parts[len - 1].clone())
                    };

                    for part in traverse_parts {
                        let next_reg = self.alloc_reg();
                        let key_reg = self.alloc_reg();
                        let const_idx =
                            self.add_constant(Constant::String(part.as_bytes().to_vec()));
                        self.emit(Instruction::LoadK {
                            dst: key_reg,
                            const_idx,
                        });
                        self.emit(Instruction::GetTable {
                            dst: next_reg,
                            tbl: current_reg,
                            key: key_reg,
                        });
                        self.set_active_registers(next_reg + 1);
                        current_reg = next_reg;
                    }

                    let key_reg = self.alloc_reg();
                    let const_idx =
                        self.add_constant(Constant::String(last_key.as_bytes().to_vec()));
                    self.emit(Instruction::LoadK {
                        dst: key_reg,
                        const_idx,
                    });
                    self.emit(Instruction::SetTable {
                        tbl: current_reg,
                        key: key_reg,
                        val: reg,
                    });
                }
                self.set_active_registers(start_reg);
            }
            Stmt::Assign { targets, values } => {
                let val_start_reg = self.active_state().active_registers;
                self.compile_expr_list_adjusted(values, val_start_reg, targets.len() as u8)?;

                for (i, target) in targets.iter().enumerate() {
                    let val_reg = val_start_reg + i as u8;

                    match target {
                        PrefixExpr::Identifier(name) => match self.resolve_var(name) {
                            VarLocation::Local(reg) => {
                                self.emit(Instruction::Move {
                                    dst: reg,
                                    src: val_reg,
                                });
                            }
                            VarLocation::Upvalue(idx) => {
                                self.emit(Instruction::SetUpval {
                                    upval_idx: idx,
                                    src: val_reg,
                                });
                            }
                            VarLocation::Global(const_idx) => {
                                self.emit(Instruction::SetTabUp {
                                    upval_idx: 0,
                                    key_const: const_idx,
                                    src: val_reg,
                                });
                            }
                        },
                        PrefixExpr::Index { base, key } => {
                            let base_reg = self.alloc_reg();
                            self.compile_prefix_expr(base, base_reg)?;

                            let key_reg = self.alloc_reg();
                            self.compile_expr(key, key_reg)?;

                            self.emit(Instruction::SetTable {
                                tbl: base_reg,
                                key: key_reg,
                                val: val_reg,
                            });
                            self.free_regs(2);
                        }
                        PrefixExpr::IndexName { base, name } => {
                            let base_reg = self.alloc_reg();
                            self.compile_prefix_expr(base, base_reg)?;

                            let key_reg = self.alloc_reg();
                            let const_idx =
                                self.add_constant(Constant::String(name.clone().into_bytes()));
                            self.emit(Instruction::LoadK {
                                dst: key_reg,
                                const_idx,
                            });

                            self.emit(Instruction::SetTable {
                                tbl: base_reg,
                                key: key_reg,
                                val: val_reg,
                            });
                            self.free_regs(2);
                        }
                        _ => {
                            return Err(CompileError::Message("invalid assign target".to_string()))
                        }
                    }
                }
                self.set_active_registers(val_start_reg);
            }
            Stmt::Return(exps) => {
                if exps.is_empty() {
                    self.emit(Instruction::Return { start: 0, count: 0 });
                } else {
                    let start_reg = self.active_state().active_registers;
                    self.compile_expr_list_adjusted(exps, start_reg, exps.len() as u8)?;
                    self.emit(Instruction::Return {
                        start: start_reg,
                        count: exps.len() as u8,
                    });
                    self.set_active_registers(start_reg);
                }
            }
            Stmt::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                let cond_reg = self.active_state().active_registers;
                self.set_active_registers(cond_reg + 1);
                self.compile_expr(cond, cond_reg)?;

                self.emit(Instruction::Test {
                    reg: cond_reg,
                    cond: false,
                });
                let jmp_to_next = self.emit(Instruction::Jmp { offset: 0 });
                self.set_active_registers(cond_reg);

                self.compile_block(then_block)?;

                let mut end_jmps = Vec::new();
                if !elseifs.is_empty() || else_block.is_some() {
                    end_jmps.push(self.emit(Instruction::Jmp { offset: 0 }));
                }

                self.patch_jmp(jmp_to_next, self.active_state().proto.instructions.len());

                for (ei_cond, ei_block) in elseifs {
                    self.set_active_registers(cond_reg + 1);
                    self.compile_expr(ei_cond, cond_reg)?;
                    self.emit(Instruction::Test {
                        reg: cond_reg,
                        cond: false,
                    });
                    let ei_jmp = self.emit(Instruction::Jmp { offset: 0 });
                    self.set_active_registers(cond_reg);

                    self.compile_block(ei_block)?;
                    end_jmps.push(self.emit(Instruction::Jmp { offset: 0 }));
                    self.patch_jmp(ei_jmp, self.active_state().proto.instructions.len());
                }

                if let Some(eb) = else_block {
                    self.compile_block(eb)?;
                }

                for jmp in end_jmps {
                    self.patch_jmp(jmp, self.active_state().proto.instructions.len());
                }
            }
            Stmt::While { cond, body } => {
                let start_idx = self.active_state().proto.instructions.len();
                let cond_reg = self.active_state().active_registers;
                self.set_active_registers(cond_reg + 1);
                self.compile_expr(cond, cond_reg)?;

                self.emit(Instruction::Test {
                    reg: cond_reg,
                    cond: false,
                });
                let exit_jmp = self.emit(Instruction::Jmp { offset: 0 });
                self.set_active_registers(cond_reg);

                self.compile_block(body)?;
                let back_jmp = self.emit(Instruction::Jmp { offset: 0 });
                self.patch_jmp(back_jmp, start_idx);

                self.patch_jmp(exit_jmp, self.active_state().proto.instructions.len());
            }
            Stmt::Repeat { body, cond } => {
                let start_idx = self.active_state().proto.instructions.len();
                self.compile_block(body)?;

                let cond_reg = self.active_state().active_registers;
                self.set_active_registers(cond_reg + 1);
                self.compile_expr(cond, cond_reg)?;

                self.emit(Instruction::Test {
                    reg: cond_reg,
                    cond: false,
                });
                let back_jmp = self.emit(Instruction::Jmp { offset: 0 });
                self.patch_jmp(back_jmp, start_idx);
                self.set_active_registers(cond_reg);
            }
            Stmt::ForNum {
                var,
                start,
                end,
                step,
                body,
            } => {
                let start_reg = self.active_state().active_registers;

                let init_reg = self.alloc_reg();
                self.compile_expr(start, init_reg)?;

                let limit_reg = self.alloc_reg();
                self.compile_expr(end, limit_reg)?;

                let step_reg = self.alloc_reg();
                if let Some(step_expr) = step {
                    self.compile_expr(step_expr, step_reg)?;
                } else {
                    let const_idx = self.add_constant(Constant::Integer(1));
                    self.emit(Instruction::LoadK {
                        dst: step_reg,
                        const_idx,
                    });
                }

                let user_reg = self.alloc_reg();
                self.emit(Instruction::LoadNil {
                    dst: user_reg,
                    count: 1,
                });

                self.active_state_mut().locals.push(LocalVar {
                    name: var.clone(),
                    reg: user_reg,
                    active: true,
                });

                let prep_ip = self.emit(Instruction::ForPrep {
                    reg: init_reg,
                    offset: 0,
                });
                let body_start_ip = self.active_state().proto.instructions.len();

                self.compile_block(body)?;

                if let Some(local) = self.active_state_mut().locals.last_mut() {
                    local.active = false;
                }

                let loop_ip = self.active_state().proto.instructions.len();
                let loop_offset = (body_start_ip as i32) - (loop_ip as i32) - 1;
                self.emit(Instruction::ForLoop {
                    reg: init_reg,
                    offset: loop_offset,
                });

                let prep_offset = (loop_ip as i32) - (prep_ip as i32) - 1;
                if let Instruction::ForPrep {
                    offset: ref mut off,
                    ..
                } = self.active_state_mut().proto.instructions[prep_ip]
                {
                    *off = prep_offset;
                }

                self.set_active_registers(start_reg);
            }
            Stmt::ForIn { vars, exps, body } => {
                let start_reg = self.active_state().active_registers;

                let f_reg = self.alloc_reg();
                let s_reg = self.alloc_reg();
                let var_reg = self.alloc_reg();

                self.compile_expr_list_adjusted(exps, f_reg, 3)?;

                let mut user_regs = Vec::new();
                for var in vars {
                    let reg = self.alloc_reg();
                    self.emit(Instruction::LoadNil { dst: reg, count: 1 });
                    user_regs.push(reg);
                    self.active_state_mut().locals.push(LocalVar {
                        name: var.clone(),
                        reg,
                        active: true,
                    });
                }

                let loop_start_ip = self.active_state().proto.instructions.len();

                let temp_f = self.alloc_reg();
                let temp_s = self.alloc_reg();
                let temp_var = self.alloc_reg();

                self.emit(Instruction::Move {
                    dst: temp_f,
                    src: f_reg,
                });
                self.emit(Instruction::Move {
                    dst: temp_s,
                    src: s_reg,
                });
                self.emit(Instruction::Move {
                    dst: temp_var,
                    src: var_reg,
                });

                self.emit(Instruction::Call {
                    func: temp_f,
                    num_args: 2,
                    num_results: vars.len() as u8,
                });

                self.emit(Instruction::Move {
                    dst: var_reg,
                    src: temp_f,
                });

                self.emit(Instruction::Test {
                    reg: var_reg,
                    cond: false,
                });
                let exit_jmp = self.emit(Instruction::Jmp { offset: 0 });

                for (i, &user_reg) in user_regs.iter().enumerate() {
                    self.emit(Instruction::Move {
                        dst: user_reg,
                        src: temp_f + i as u8,
                    });
                }

                self.compile_block(body)?;

                let back_offset = (loop_start_ip as i32)
                    - (self.active_state().proto.instructions.len() as i32)
                    - 1;
                self.emit(Instruction::Jmp {
                    offset: back_offset,
                });

                let exit_ip = self.active_state().proto.instructions.len();
                let exit_offset = (exit_ip as i32) - (exit_jmp as i32) - 1;
                if let Instruction::Jmp {
                    offset: ref mut off,
                } = self.active_state_mut().proto.instructions[exit_jmp]
                {
                    *off = exit_offset;
                }

                for _ in vars {
                    if let Some(local) = self.active_state_mut().locals.last_mut() {
                        local.active = false;
                    }
                }

                self.set_active_registers(start_reg);
            }
            Stmt::Expr(prefix) => {
                let reg = self.active_state().active_registers;
                self.compile_prefix_expr_with_results(prefix, reg, 0)?;
            }
            Stmt::DoBlock(block) => {
                self.compile_block(block)?;
            }
            _ => {
                return Err(CompileError::Message(format!(
                    "statement compiling not fully implemented yet: {:?}",
                    stmt
                )))
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr, dest_reg: u8) -> Result<(), CompileError> {
        match expr {
            Expr::Nil => {
                self.emit(Instruction::LoadNil {
                    dst: dest_reg,
                    count: 1,
                });
            }
            Expr::Boolean(val) => {
                self.emit(Instruction::LoadBool {
                    dst: dest_reg,
                    val: *val,
                    skip_next: false,
                });
            }
            Expr::Integer(val) => {
                let const_idx = self.add_constant(Constant::Integer(*val));
                self.emit(Instruction::LoadK {
                    dst: dest_reg,
                    const_idx,
                });
            }
            Expr::Float(val) => {
                let const_idx = self.add_constant(Constant::Number(*val));
                self.emit(Instruction::LoadK {
                    dst: dest_reg,
                    const_idx,
                });
            }
            Expr::String(val) => {
                let const_idx = self.add_constant(Constant::String(val.clone()));
                self.emit(Instruction::LoadK {
                    dst: dest_reg,
                    const_idx,
                });
            }
            Expr::FunctionDef {
                params,
                is_vararg,
                body,
            } => {
                let mut sub_state = FuncState::new();
                sub_state.proto.num_params = params.len() as u8;
                sub_state.proto.is_vararg = *is_vararg;

                // Push _ENV as upvalue 0 automatically
                sub_state.proto.upvalues.push(UpvalDesc {
                    name: Some("_ENV".to_string()),
                    in_stack: false,
                    idx: 0,
                });

                for param in params {
                    let reg = sub_state.alloc_reg();
                    sub_state.locals.push(LocalVar {
                        name: param.clone(),
                        reg,
                        active: true,
                    });
                }

                self.states.push(sub_state);
                self.compile_block(body)?;
                self.emit(Instruction::Return { start: 0, count: 0 });

                let popped = self.states.pop().unwrap();
                let sub_proto = popped.proto;

                let proto_idx = self.active_state_mut().proto.sub_protos.len() as u16;
                self.active_state_mut().proto.sub_protos.push(sub_proto);

                self.emit(Instruction::Closure {
                    dst: dest_reg,
                    proto_idx,
                });
            }
            Expr::Binary { op, lhs, rhs } => {
                if *op == BinOp::And || *op == BinOp::Or {
                    self.compile_expr(lhs, dest_reg)?;
                    let cond = *op == BinOp::Or;
                    self.emit(Instruction::Test {
                        reg: dest_reg,
                        cond,
                    });
                    let jmp_idx = self.emit(Instruction::Jmp { offset: 0 });
                    self.compile_expr(rhs, dest_reg)?;
                    let end_idx = self.active_state().proto.instructions.len();
                    self.patch_jmp(jmp_idx, end_idx);
                    return Ok(());
                }

                let left_reg = self.alloc_reg();
                self.compile_expr(lhs, left_reg)?;

                let right_reg = self.alloc_reg();
                self.compile_expr(rhs, right_reg)?;

                let inst = match op {
                    BinOp::Add => Instruction::Add {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::Sub => Instruction::Sub {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::Mul => Instruction::Mul {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::Div => Instruction::Div {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::Mod => Instruction::Mod {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::Pow => Instruction::Pow {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::IDiv => Instruction::IDiv {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::BitAnd => Instruction::BAnd {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::BitOr => Instruction::BOr {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::BitXor => Instruction::BXor {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::Shl => Instruction::Shl {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::Shr => Instruction::Shr {
                        dst: dest_reg,
                        lhs: left_reg,
                        rhs: right_reg,
                    },
                    BinOp::Eq => {
                        self.emit(Instruction::Eq {
                            lhs: left_reg,
                            rhs: right_reg,
                            eq: true,
                        });
                        self.emit(Instruction::Jmp { offset: 1 });
                        self.emit(Instruction::LoadBool {
                            dst: dest_reg,
                            val: false,
                            skip_next: true,
                        });
                        Instruction::LoadBool {
                            dst: dest_reg,
                            val: true,
                            skip_next: false,
                        }
                    }
                    BinOp::Ne => {
                        self.emit(Instruction::Eq {
                            lhs: left_reg,
                            rhs: right_reg,
                            eq: false,
                        });
                        self.emit(Instruction::Jmp { offset: 1 });
                        self.emit(Instruction::LoadBool {
                            dst: dest_reg,
                            val: false,
                            skip_next: true,
                        });
                        Instruction::LoadBool {
                            dst: dest_reg,
                            val: true,
                            skip_next: false,
                        }
                    }
                    BinOp::Lt => {
                        self.emit(Instruction::Lt {
                            lhs: left_reg,
                            rhs: right_reg,
                            eq: true,
                        });
                        self.emit(Instruction::Jmp { offset: 1 });
                        self.emit(Instruction::LoadBool {
                            dst: dest_reg,
                            val: false,
                            skip_next: true,
                        });
                        Instruction::LoadBool {
                            dst: dest_reg,
                            val: true,
                            skip_next: false,
                        }
                    }
                    BinOp::Le => {
                        self.emit(Instruction::Le {
                            lhs: left_reg,
                            rhs: right_reg,
                            eq: true,
                        });
                        self.emit(Instruction::Jmp { offset: 1 });
                        self.emit(Instruction::LoadBool {
                            dst: dest_reg,
                            val: false,
                            skip_next: true,
                        });
                        Instruction::LoadBool {
                            dst: dest_reg,
                            val: true,
                            skip_next: false,
                        }
                    }
                    BinOp::Gt => {
                        self.emit(Instruction::Lt {
                            lhs: right_reg,
                            rhs: left_reg,
                            eq: true,
                        });
                        self.emit(Instruction::Jmp { offset: 1 });
                        self.emit(Instruction::LoadBool {
                            dst: dest_reg,
                            val: false,
                            skip_next: true,
                        });
                        Instruction::LoadBool {
                            dst: dest_reg,
                            val: true,
                            skip_next: false,
                        }
                    }
                    BinOp::Ge => {
                        self.emit(Instruction::Le {
                            lhs: right_reg,
                            rhs: left_reg,
                            eq: true,
                        });
                        self.emit(Instruction::Jmp { offset: 1 });
                        self.emit(Instruction::LoadBool {
                            dst: dest_reg,
                            val: false,
                            skip_next: true,
                        });
                        Instruction::LoadBool {
                            dst: dest_reg,
                            val: true,
                            skip_next: false,
                        }
                    }
                    BinOp::Concat => Instruction::Concat {
                        dst: dest_reg,
                        start: left_reg,
                        count: 2,
                    },
                    _ => {
                        return Err(CompileError::Message(
                            "binary operator compiling not fully implemented".to_string(),
                        ))
                    }
                };

                self.emit(inst);
                self.free_regs(2);
            }
            Expr::Unary { op, val } => {
                let src_reg = self.alloc_reg();
                self.compile_expr(val, src_reg)?;

                let inst = match op {
                    UnOp::Neg => Instruction::UNeg {
                        dst: dest_reg,
                        src: src_reg,
                    },
                    UnOp::Not => Instruction::UNot {
                        dst: dest_reg,
                        src: src_reg,
                    },
                    UnOp::Len => Instruction::ULen {
                        dst: dest_reg,
                        src: src_reg,
                    },
                    UnOp::BitNot => Instruction::UBNot {
                        dst: dest_reg,
                        src: src_reg,
                    },
                };

                self.emit(inst);
                self.free_regs(1);
            }
            Expr::Prefix(prefix) => {
                self.compile_prefix_expr(prefix, dest_reg)?;
            }
            Expr::TableConstructor { fields } => {
                let array_size = fields
                    .iter()
                    .filter(|field| matches!(field, TableField::ListVal(_)))
                    .count() as u16;
                let hash_size = fields.len() as u16 - array_size;
                self.emit(Instruction::NewTable {
                    dst: dest_reg,
                    array_size,
                    hash_size,
                });
                let mut arr_count: i64 = 1;
                for field in fields {
                    match field {
                        TableField::ListVal(val_expr) => {
                            let key_reg = self.alloc_reg();
                            let const_idx = self.add_constant(Constant::Integer(arr_count));
                            self.emit(Instruction::LoadK {
                                dst: key_reg,
                                const_idx,
                            });

                            let val_reg = self.alloc_reg();
                            self.compile_expr(val_expr, val_reg)?;

                            self.emit(Instruction::SetTable {
                                tbl: dest_reg,
                                key: key_reg,
                                val: val_reg,
                            });
                            arr_count += 1;
                            self.free_regs(2);
                        }
                        TableField::NameVal { name, val } => {
                            let key_reg = self.alloc_reg();
                            let const_idx =
                                self.add_constant(Constant::String(name.clone().into_bytes()));
                            self.emit(Instruction::LoadK {
                                dst: key_reg,
                                const_idx,
                            });

                            let val_reg = self.alloc_reg();
                            self.compile_expr(val, val_reg)?;

                            self.emit(Instruction::SetTable {
                                tbl: dest_reg,
                                key: key_reg,
                                val: val_reg,
                            });
                            self.free_regs(2);
                        }
                        TableField::KeyVal { key, val } => {
                            let key_reg = self.alloc_reg();
                            self.compile_expr(key, key_reg)?;

                            let val_reg = self.alloc_reg();
                            self.compile_expr(val, val_reg)?;

                            self.emit(Instruction::SetTable {
                                tbl: dest_reg,
                                key: key_reg,
                                val: val_reg,
                            });
                            self.free_regs(2);
                        }
                    }
                }
            }
            Expr::Vararg => {
                self.emit(Instruction::Vararg {
                    dst: dest_reg,
                    count: 1,
                });
            }
        }
        Ok(())
    }

    fn compile_expr_with_results(
        &mut self,
        expr: &Expr,
        dest_reg: u8,
        num_results: u8,
    ) -> Result<(), CompileError> {
        self.ensure_stack_for_results(dest_reg, num_results);
        match expr {
            Expr::Prefix(prefix) => {
                self.compile_prefix_expr_with_results(prefix, dest_reg, num_results)?;
            }
            Expr::Vararg => {
                self.emit(Instruction::Vararg {
                    dst: dest_reg,
                    count: num_results,
                });
            }
            _ => {
                if num_results > 0 {
                    self.compile_expr(expr, dest_reg)?;
                    if num_results > 1 {
                        self.emit(Instruction::LoadNil {
                            dst: dest_reg + 1,
                            count: num_results - 1,
                        });
                    }
                } else {
                    let temp_reg = self.active_state().active_registers;
                    self.set_active_registers(temp_reg + 1);
                    self.compile_expr(expr, temp_reg)?;
                    self.set_active_registers(temp_reg);
                }
            }
        }
        Ok(())
    }

    fn compile_expr_list_adjusted(
        &mut self,
        values: &[Expr],
        start_reg: u8,
        wanted: u8,
    ) -> Result<(), CompileError> {
        if wanted == 0 {
            for value in values {
                self.compile_expr_with_results(value, start_reg, 0)?;
            }
            return Ok(());
        }

        if values.is_empty() {
            self.ensure_stack_for_results(start_reg, wanted);
            self.emit(Instruction::LoadNil {
                dst: start_reg,
                count: wanted,
            });
            return Ok(());
        }

        let fixed_count = values.len().saturating_sub(1).min(wanted as usize);
        for (i, value) in values.iter().take(fixed_count).enumerate() {
            self.compile_expr_with_results(value, start_reg + i as u8, 1)?;
        }

        if values.len() <= wanted as usize {
            let last_index = values.len() - 1;
            let last_reg = start_reg + last_index as u8;
            let remaining = wanted - last_index as u8;
            self.compile_expr_with_results(&values[last_index], last_reg, remaining)?;
        } else {
            for value in &values[fixed_count..] {
                self.compile_expr_with_results(value, start_reg + fixed_count as u8, 0)?;
            }
        }
        Ok(())
    }

    fn ensure_stack_for_results(&mut self, start_reg: u8, count: u8) {
        if count > 0 {
            self.set_active_registers(start_reg + count);
        }
    }

    fn compile_prefix_expr(
        &mut self,
        prefix: &PrefixExpr,
        dest_reg: u8,
    ) -> Result<(), CompileError> {
        match prefix {
            PrefixExpr::Identifier(name) => match self.resolve_var(name) {
                VarLocation::Local(reg) => {
                    self.emit(Instruction::Move {
                        dst: dest_reg,
                        src: reg,
                    });
                }
                VarLocation::Upvalue(idx) => {
                    self.emit(Instruction::GetUpval {
                        dst: dest_reg,
                        upval_idx: idx,
                    });
                }
                VarLocation::Global(const_idx) => {
                    self.emit(Instruction::GetTabUp {
                        dst: dest_reg,
                        upval_idx: 0,
                        key_const: const_idx,
                    });
                }
            },
            PrefixExpr::FunctionCall { func, args } => {
                let func_reg = self.alloc_reg();
                self.compile_prefix_expr(func, func_reg)?;
                self.compile_expr_list_adjusted(args, func_reg + 1, args.len() as u8)?;

                self.emit(Instruction::Call {
                    func: func_reg,
                    num_args: args.len() as u8,
                    num_results: 1,
                });

                self.emit(Instruction::Move {
                    dst: dest_reg,
                    src: func_reg,
                });
                self.set_active_registers(func_reg); // reclaim stack space
            }
            PrefixExpr::Index { base, key } => {
                let tbl_reg = self.alloc_reg();
                self.compile_prefix_expr(base, tbl_reg)?;

                let key_reg = self.alloc_reg();
                self.compile_expr(key, key_reg)?;

                self.emit(Instruction::GetTable {
                    dst: dest_reg,
                    tbl: tbl_reg,
                    key: key_reg,
                });

                self.set_active_registers(tbl_reg);
            }
            PrefixExpr::IndexName { base, name } => {
                let tbl_reg = self.alloc_reg();
                self.compile_prefix_expr(base, tbl_reg)?;

                let key_reg = self.alloc_reg();
                let const_idx = self.add_constant(Constant::String(name.as_bytes().to_vec()));
                self.emit(Instruction::LoadK {
                    dst: key_reg,
                    const_idx,
                });

                self.emit(Instruction::GetTable {
                    dst: dest_reg,
                    tbl: tbl_reg,
                    key: key_reg,
                });

                self.set_active_registers(tbl_reg);
            }
            PrefixExpr::MethodCall { base, method, args } => {
                let tbl_reg = self.alloc_reg();
                self.compile_prefix_expr(base, tbl_reg)?;

                let func_reg = self.alloc_reg();

                let key_reg = self.alloc_reg();
                let const_idx = self.add_constant(Constant::String(method.as_bytes().to_vec()));
                self.emit(Instruction::LoadK {
                    dst: key_reg,
                    const_idx,
                });

                self.emit(Instruction::GetTable {
                    dst: func_reg,
                    tbl: tbl_reg,
                    key: key_reg,
                });

                self.set_active_registers(func_reg + 1);

                let self_arg_reg = self.alloc_reg();
                self.emit(Instruction::Move {
                    dst: self_arg_reg,
                    src: tbl_reg,
                });
                self.compile_expr_list_adjusted(args, self_arg_reg + 1, args.len() as u8)?;

                self.emit(Instruction::Call {
                    func: func_reg,
                    num_args: (args.len() + 1) as u8,
                    num_results: 1,
                });

                self.emit(Instruction::Move {
                    dst: dest_reg,
                    src: func_reg,
                });
                self.set_active_registers(tbl_reg);
            }
            PrefixExpr::Parens(expr) => {
                self.compile_expr(expr, dest_reg)?;
            }
        }
        Ok(())
    }

    fn compile_prefix_expr_with_results(
        &mut self,
        prefix: &PrefixExpr,
        dest_reg: u8,
        num_results: u8,
    ) -> Result<(), CompileError> {
        match prefix {
            PrefixExpr::FunctionCall { func, args } => {
                let func_reg = self.alloc_reg();
                self.compile_prefix_expr(func, func_reg)?;
                self.compile_expr_list_adjusted(args, func_reg + 1, args.len() as u8)?;

                self.emit(Instruction::Call {
                    func: func_reg,
                    num_args: args.len() as u8,
                    num_results,
                });

                for i in 0..num_results {
                    self.emit(Instruction::Move {
                        dst: dest_reg + i,
                        src: func_reg + i,
                    });
                }
                self.set_active_registers(func_reg);
            }
            PrefixExpr::MethodCall { base, method, args } => {
                let tbl_reg = self.alloc_reg();
                self.compile_prefix_expr(base, tbl_reg)?;

                let func_reg = self.alloc_reg();

                let key_reg = self.alloc_reg();
                let const_idx = self.add_constant(Constant::String(method.as_bytes().to_vec()));
                self.emit(Instruction::LoadK {
                    dst: key_reg,
                    const_idx,
                });

                self.emit(Instruction::GetTable {
                    dst: func_reg,
                    tbl: tbl_reg,
                    key: key_reg,
                });

                self.set_active_registers(func_reg + 1);

                let self_arg_reg = self.alloc_reg();
                self.emit(Instruction::Move {
                    dst: self_arg_reg,
                    src: tbl_reg,
                });
                self.compile_expr_list_adjusted(args, self_arg_reg + 1, args.len() as u8)?;

                self.emit(Instruction::Call {
                    func: func_reg,
                    num_args: (args.len() + 1) as u8,
                    num_results,
                });

                for i in 0..num_results {
                    self.emit(Instruction::Move {
                        dst: dest_reg + i,
                        src: func_reg + i,
                    });
                }
                self.set_active_registers(tbl_reg);
            }
            _ => {
                if num_results > 0 {
                    self.compile_prefix_expr(prefix, dest_reg)?;
                    if num_results > 1 {
                        self.emit(Instruction::LoadNil {
                            dst: dest_reg + 1,
                            count: num_results - 1,
                        });
                    }
                } else {
                    self.compile_prefix_expr(prefix, dest_reg)?;
                }
            }
        }
        Ok(())
    }

    fn resolve_var(&mut self, name: &str) -> VarLocation {
        let active_idx = self.states.len() - 1;

        // 1. Search locally in the active function state
        for local in self.states[active_idx].locals.iter().rev() {
            if local.active && local.name == name {
                return VarLocation::Local(local.reg);
            }
        }

        // 2. Search in parent function states
        if active_idx > 0 {
            for p in (0..active_idx).rev() {
                let mut found_reg = None;
                for local in self.states[p].locals.iter().rev() {
                    if local.active && local.name == name {
                        found_reg = Some(local.reg);
                        break;
                    }
                }

                if let Some(reg) = found_reg {
                    let mut current_idx = reg;
                    let mut in_stack = true;

                    for state_idx in (p + 1)..=active_idx {
                        let mut upval_pos = None;
                        for (i, up) in self.states[state_idx].proto.upvalues.iter().enumerate() {
                            if up.in_stack == in_stack && up.idx == current_idx {
                                upval_pos = Some(i as u8);
                                break;
                            }
                        }

                        let next_upval_idx = match upval_pos {
                            Some(idx) => idx,
                            None => {
                                let idx = self.states[state_idx].proto.upvalues.len() as u8;
                                self.states[state_idx].proto.upvalues.push(UpvalDesc {
                                    name: Some(name.to_string()),
                                    in_stack,
                                    idx: current_idx,
                                });
                                idx
                            }
                        };
                        current_idx = next_upval_idx;
                        in_stack = false;
                    }

                    return VarLocation::Upvalue(current_idx);
                }
            }
        }

        // 3. Fallback to global
        let const_idx = self.add_constant(Constant::String(name.as_bytes().to_vec()));
        VarLocation::Global(const_idx)
    }
}

pub enum VarLocation {
    Local(u8),
    Upvalue(u8),
    Global(u16),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn compile_source(source: &[u8]) -> Proto {
        let lex = Lexer::new(source);
        let mut parser = Parser::new(lex);
        let block = parser.parse_chunk().unwrap();
        Compiler::compile_chunk(&block).unwrap()
    }

    #[test]
    fn test_compiler_basic_arithmetic() {
        let proto = compile_source(b"local x = 10 + 20\nreturn x");

        assert!(!proto.instructions.is_empty());
        assert!(proto.constants.len() >= 2);
    }

    #[test]
    fn test_table_constructor_lowers_without_set_list() {
        let proto = compile_source(b"local k = 'x'\nlocal t = {10, [k] = 20, y = 30}\nreturn t[1]");

        assert!(proto
            .instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::NewTable { .. })));
        assert!(proto
            .instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::SetTable { .. })));
        assert!(!proto
            .instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::SetList { .. })));
    }

    #[test]
    fn test_local_assign_requests_fixed_call_results() {
        let proto = compile_source(
            b"local function vals()\n  return 1, 2, 3\nend\nlocal a, b, c, d = vals()\nreturn a",
        );

        assert!(proto
            .instructions
            .iter()
            .any(|inst| { matches!(inst, Instruction::Call { num_results: 4, .. }) }));
    }

    #[test]
    fn test_generic_for_requests_iterator_triple() {
        let proto = compile_source(b"local t = {}\nfor k, v in pairs(t) do\nend");

        assert!(proto
            .instructions
            .iter()
            .any(|inst| { matches!(inst, Instruction::Call { num_results: 3, .. }) }));
    }

    #[test]
    fn test_call_statement_requests_zero_results() {
        let proto = compile_source(b"print(1)");

        assert!(proto
            .instructions
            .iter()
            .any(|inst| { matches!(inst, Instruction::Call { num_results: 0, .. }) }));
    }
}
