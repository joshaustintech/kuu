#[allow(unused_imports)]
use crate::ast::{
    Attribute, AttributedName, Block, CallExpr, Chunk, Expr, FunctionBody, FunctionName, Param,
    ReturnStmt, Stmt, TableConstructor, TableField, UnaryOp, Var, VarKind,
};
use crate::error::{KError, KResult, KSpan};
use std::collections::{BTreeMap, BTreeSet};

#[path = "resolve/scope.rs"]
mod scope;

pub use scope::Resolver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    Chunk,
    Function,
    LocalFunction,
    GlobalFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalPolicy {
    Writable,
    Readonly,
    DeclaredOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Local,
    Global,
    GlobalDefault,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedChunk {
    pub root: ResolvedFunction,
}

impl ResolvedChunk {
    pub fn root(&self) -> &ResolvedFunction {
        &self.root
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFunction {
    pub id: usize,
    pub kind: FunctionKind,
    pub span: KSpan,
    pub declarations: Vec<DeclarationRecord>,
    pub uses: Vec<NameUseRecord>,
    pub labels: Vec<LabelRecord>,
    pub gotos: Vec<GotoRecord>,
    pub upvalues: Vec<UpvalueBinding>,
    pub children: Vec<ResolvedFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationRecord {
    pub name: String,
    pub kind: DeclarationKind,
    pub slot: usize,
    pub readonly: bool,
    pub close: bool,
    pub explicit: bool,
    pub span: KSpan,
    pub block_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameUseRecord {
    pub name: String,
    pub is_write: bool,
    pub span: KSpan,
    pub binding: BindingTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRecord {
    pub name: String,
    pub span: KSpan,
    pub active_decls: BTreeSet<usize>,
    pub block_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GotoRecord {
    pub name: String,
    pub span: KSpan,
    pub active_decls: BTreeSet<usize>,
    pub block_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpvalueBinding {
    pub name: String,
    pub slot: usize,
    pub readonly: bool,
    pub source_depth: usize,
    pub declaration_span: KSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingTarget {
    Local {
        slot: usize,
        readonly: bool,
        close: bool,
        declaration_span: KSpan,
        block_depth: usize,
    },
    Global {
        slot: usize,
        readonly: bool,
        explicit: bool,
        declaration_span: Option<KSpan>,
        block_depth: usize,
        environment: EnvironmentTarget,
    },
    Upvalue {
        slot: usize,
        readonly: bool,
        source_depth: usize,
        declaration_span: KSpan,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentTarget {
    Local { slot: usize },
    Upvalue { slot: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) name: String,
    pub(crate) slot: usize,
    pub(crate) readonly: bool,
    pub(crate) close: bool,
    pub(crate) declaration_span: KSpan,
    pub(crate) block_depth: usize,
    pub(crate) source_depth: usize,
}

impl Binding {
    pub(crate) fn to_target(&self) -> BindingTarget {
        if self.source_depth == 0 {
            BindingTarget::Local {
                slot: self.slot,
                readonly: self.readonly,
                close: self.close,
                declaration_span: self.declaration_span,
                block_depth: self.block_depth,
            }
        } else {
            BindingTarget::Upvalue {
                slot: self.slot,
                readonly: self.readonly,
                source_depth: self.source_depth,
                declaration_span: self.declaration_span,
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GlobalBinding {
    pub(crate) name: String,
    pub(crate) slot: usize,
    pub(crate) readonly: bool,
    pub(crate) declaration_span: Option<KSpan>,
    pub(crate) block_depth: usize,
}

impl GlobalBinding {
    pub(crate) fn to_target(
        &self,
        explicit: bool,
        environment: EnvironmentTarget,
    ) -> BindingTarget {
        BindingTarget::Global {
            slot: self.slot,
            readonly: self.readonly,
            explicit,
            declaration_span: self.declaration_span,
            block_depth: self.block_depth,
            environment,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopeSnapshot {
    pub(crate) distance: usize,
    pub(crate) bindings: BTreeMap<String, Binding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockFrame {
    pub(crate) previous_policy: GlobalPolicy,
    pub(crate) previous_global_default: bool,
    pub(crate) local_undos: Vec<(String, Option<Binding>)>,
    pub(crate) global_undos: Vec<(String, Option<GlobalBinding>)>,
    pub(crate) label_undos: Vec<(String, Option<LabelRecord>)>,
    pub(crate) decl_ids: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FunctionState {
    pub(crate) id: usize,
    pub(crate) kind: FunctionKind,
    pub(crate) span: KSpan,
    pub(crate) declarations: Vec<DeclarationRecord>,
    pub(crate) uses: Vec<NameUseRecord>,
    pub(crate) labels: Vec<LabelRecord>,
    pub(crate) gotos: Vec<GotoRecord>,
    pub(crate) upvalues: Vec<UpvalueBinding>,
    pub(crate) children: Vec<ResolvedFunction>,
    pub(crate) visible_bindings: BTreeMap<String, Binding>,
    pub(crate) globals: BTreeMap<String, GlobalBinding>,
    pub(crate) visible_labels: BTreeMap<String, LabelRecord>,
    pub(crate) implicit_globals: BTreeMap<String, usize>,
    pub(crate) global_policy: GlobalPolicy,
    pub(crate) has_global_default: bool,
    pub(crate) ancestor_scopes: Vec<ScopeSnapshot>,
    pub(crate) active_decls: BTreeSet<usize>,
    pub(crate) block_depth: usize,
    pub(crate) next_local_slot: usize,
    pub(crate) next_global_slot: usize,
    pub(crate) next_upvalue_slot: usize,
    pub(crate) next_decl_id: usize,
    pub(crate) block_frames: Vec<BlockFrame>,
}

impl FunctionState {
    pub(crate) fn new(
        id: usize,
        kind: FunctionKind,
        span: KSpan,
        ancestor_scopes: Vec<ScopeSnapshot>,
        global_policy: GlobalPolicy,
        globals: BTreeMap<String, GlobalBinding>,
    ) -> Self {
        let mut visible_bindings = BTreeMap::new();
        let mut upvalues = Vec::new();

        if matches!(kind, FunctionKind::Chunk) {
            let env_binding = Binding {
                name: "_ENV".to_owned(),
                slot: 0,
                readonly: false,
                close: false,
                declaration_span: span,
                block_depth: 0,
                source_depth: 0,
            };
            visible_bindings.insert("_ENV".to_owned(), env_binding.clone());
            upvalues.push(UpvalueBinding {
                name: "_ENV".to_owned(),
                slot: 0,
                readonly: false,
                source_depth: 0,
                declaration_span: span,
            });
        }

        Self {
            id,
            kind,
            span,
            declarations: Vec::new(),
            uses: Vec::new(),
            labels: Vec::new(),
            gotos: Vec::new(),
            upvalues,
            children: Vec::new(),
            visible_bindings,
            globals,
            visible_labels: BTreeMap::new(),
            implicit_globals: BTreeMap::new(),
            global_policy,
            has_global_default: false,
            ancestor_scopes,
            active_decls: BTreeSet::new(),
            block_depth: 0,
            next_local_slot: 0,
            next_global_slot: 0,
            next_upvalue_slot: 0,
            next_decl_id: 0,
            block_frames: Vec::new(),
        }
    }

    pub(crate) fn finish(self) -> ResolvedFunction {
        ResolvedFunction {
            id: self.id,
            kind: self.kind,
            span: self.span,
            declarations: self.declarations,
            uses: self.uses,
            labels: self.labels,
            gotos: self.gotos,
            upvalues: self.upvalues,
            children: self.children,
        }
    }

    pub(crate) fn push_block(&mut self) {
        self.block_frames.push(BlockFrame {
            previous_policy: self.global_policy,
            previous_global_default: self.has_global_default,
            local_undos: Vec::new(),
            global_undos: Vec::new(),
            label_undos: Vec::new(),
            decl_ids: Vec::new(),
        });
        self.block_depth += 1;
    }

    pub(crate) fn pop_block(&mut self) {
        let Some(frame) = self.block_frames.pop() else {
            return;
        };
        for (name, previous) in frame.local_undos.into_iter().rev() {
            match previous {
                Some(binding) => {
                    self.visible_bindings.insert(name, binding);
                }
                None => {
                    self.visible_bindings.remove(&name);
                }
            }
        }
        for (name, previous) in frame.global_undos.into_iter().rev() {
            match previous {
                Some(binding) => {
                    self.globals.insert(name, binding);
                }
                None => {
                    self.globals.remove(&name);
                }
            }
        }
        for (name, previous) in frame.label_undos.into_iter().rev() {
            match previous {
                Some(binding) => {
                    self.visible_labels.insert(name, binding);
                }
                None => {
                    self.visible_labels.remove(&name);
                }
            }
        }
        for decl_id in frame.decl_ids {
            self.active_decls.remove(&decl_id);
        }

        self.global_policy = frame.previous_policy;
        self.has_global_default = frame.previous_global_default;
        self.block_depth = self.block_depth.saturating_sub(1);
    }

    pub(crate) fn current_snapshot(&self) -> ScopeSnapshot {
        ScopeSnapshot {
            distance: 1,
            bindings: self.visible_bindings.clone(),
        }
    }

    pub(crate) fn next_decl_id(&mut self) -> usize {
        let id = self.next_decl_id;
        self.next_decl_id = self.next_decl_id.saturating_add(1);
        id
    }

    pub(crate) fn add_local_binding(
        &mut self,
        name: String,
        readonly: bool,
        close: bool,
        span: KSpan,
        explicit: bool,
        kind: DeclarationKind,
    ) -> Binding {
        let slot = self.next_local_slot;
        self.next_local_slot = self.next_local_slot.saturating_add(1);
        let decl_id = self.next_decl_id();
        let binding = Binding {
            name: name.clone(),
            slot,
            readonly,
            close,
            declaration_span: span,
            block_depth: self.block_depth,
            source_depth: 0,
        };
        let previous = self.visible_bindings.insert(name.clone(), binding.clone());
        self.active_decls.insert(decl_id);
        self.declarations.push(DeclarationRecord {
            name: name.clone(),
            kind,
            slot,
            readonly,
            close,
            explicit,
            span,
            block_depth: self.block_depth,
        });
        if let Some(frame) = self.block_frames.last_mut() {
            frame.local_undos.push((name, previous));
            frame.decl_ids.push(decl_id);
        }
        binding
    }

    pub(crate) fn add_global_binding(
        &mut self,
        name: String,
        readonly: bool,
        span: Option<KSpan>,
        explicit: bool,
        kind: DeclarationKind,
    ) -> GlobalBinding {
        let slot = self.next_global_slot;
        self.next_global_slot = self.next_global_slot.saturating_add(1);
        let decl_id = self.next_decl_id();
        let binding = GlobalBinding {
            name: name.clone(),
            slot,
            readonly,
            declaration_span: span,
            block_depth: self.block_depth,
        };
        let previous = self.globals.insert(name.clone(), binding.clone());
        self.active_decls.insert(decl_id);
        self.declarations.push(DeclarationRecord {
            name: name.clone(),
            kind,
            slot,
            readonly,
            close: false,
            explicit,
            span: span.unwrap_or(self.span),
            block_depth: self.block_depth,
        });
        if let Some(frame) = self.block_frames.last_mut() {
            frame.global_undos.push((name, previous));
            frame.decl_ids.push(decl_id);
        }
        binding
    }

    pub(crate) fn add_global_default(
        &mut self,
        readonly: bool,
        span: KSpan,
    ) -> DeclarationRecord {
        let slot = self.next_global_slot;
        self.next_global_slot = self.next_global_slot.saturating_add(1);
        let decl_id = self.next_decl_id();
        self.active_decls.insert(decl_id);
        let record = DeclarationRecord {
            name: "*".to_owned(),
            kind: DeclarationKind::GlobalDefault,
            slot,
            readonly,
            close: false,
            explicit: false,
            span,
            block_depth: self.block_depth,
        };
        self.declarations.push(record.clone());
        if let Some(frame) = self.block_frames.last_mut() {
            frame.decl_ids.push(decl_id);
        }
        record
    }

    pub(crate) fn add_label(&mut self, name: String, span: KSpan) -> LabelRecord {
        let record = LabelRecord {
            name: name.clone(),
            span,
            active_decls: self.active_decls.clone(),
            block_depth: self.block_depth,
        };
        let previous = self.visible_labels.insert(name.clone(), record.clone());
        self.labels.push(record.clone());
        if let Some(frame) = self.block_frames.last_mut() {
            frame.label_undos.push((name, previous));
        }
        record
    }

    pub(crate) fn add_goto(&mut self, name: String, span: KSpan) -> GotoRecord {
        let record = GotoRecord {
            name,
            span,
            active_decls: self.active_decls.clone(),
            block_depth: self.block_depth,
        };
        self.gotos.push(record.clone());
        record
    }

    pub(crate) fn record_use(&mut self, name: String, span: KSpan, is_write: bool, binding: BindingTarget) {
        self.uses.push(NameUseRecord {
            name,
            is_write,
            span,
            binding,
        });
    }

    pub(crate) fn capture_upvalue(
        &mut self,
        name: &str,
        readonly: bool,
        declaration_span: KSpan,
        source_depth: usize,
    ) -> Binding {
        if let Some(binding) = self.visible_bindings.get(name) {
            return binding.clone();
        }

        let slot = self.next_upvalue_slot;
        self.next_upvalue_slot = self.next_upvalue_slot.saturating_add(1);
        let binding = Binding {
            name: name.to_owned(),
            slot,
            readonly,
            close: false,
            declaration_span,
            block_depth: self.block_depth,
            source_depth,
        };
        self.visible_bindings.insert(name.to_owned(), binding.clone());
        self.upvalues.push(UpvalueBinding {
            name: name.to_owned(),
            slot,
            readonly,
            source_depth,
            declaration_span,
        });
        binding
    }

    pub(crate) fn capture_env(&mut self, declaration_span: KSpan, source_depth: usize) -> Binding {
        self.capture_upvalue("_ENV", false, declaration_span, source_depth)
    }

    pub(crate) fn lookup_local_or_upvalue(&self, name: &str) -> Option<Binding> {
        self.visible_bindings.get(name).cloned()
    }

    pub(crate) fn lookup_outer_capture(&self, name: &str) -> Option<(Binding, usize)> {
        for snapshot in self.ancestor_scopes.iter().rev() {
            if let Some(binding) = snapshot.bindings.get(name) {
                let source_depth = snapshot.distance.saturating_add(binding.source_depth);
                return Some((binding.clone(), source_depth));
            }
        }
        None
    }

    pub(crate) fn lookup_global(&mut self, name: &str, span: KSpan) -> KResult<(BindingTarget, bool)> {
        if !self.visible_bindings.contains_key("_ENV") {
            let _ = self.ensure_env_capture();
        }
        let environment = self.environment_target()?;
        if let Some(binding) = self.globals.get(name) {
            return Ok((binding.to_target(true, environment), true));
        }

        match self.global_policy {
            GlobalPolicy::DeclaredOnly => {
                Err(KError::syntax(format!("variable '{}' is not declared", name), span))
            }
            GlobalPolicy::Writable | GlobalPolicy::Readonly => {
                let readonly = matches!(self.global_policy, GlobalPolicy::Readonly);
                let slot = if let Some(slot) = self.implicit_globals.get(name) {
                    *slot
                } else {
                    let slot = self.next_global_slot;
                    self.next_global_slot = self.next_global_slot.saturating_add(1);
                    self.implicit_globals.insert(name.to_owned(), slot);
                    slot
                };
                Ok((
                    BindingTarget::Global {
                        slot,
                        readonly,
                        explicit: false,
                        declaration_span: None,
                        block_depth: self.block_depth,
                        environment,
                    },
                    false,
                ))
            }
        }
    }

    fn environment_target(&self) -> KResult<EnvironmentTarget> {
        let binding = self.visible_bindings.get("_ENV").ok_or_else(|| {
            KError::bytecode("missing _ENV binding while resolving a global")
        })?;
        if matches!(self.kind, FunctionKind::Chunk)
            && binding.source_depth == 0
            && binding.declaration_span == self.span
        {
            return Ok(EnvironmentTarget::Upvalue { slot: 0 });
        }
        if binding.source_depth == 0 {
            Ok(EnvironmentTarget::Local { slot: binding.slot })
        } else {
            Ok(EnvironmentTarget::Upvalue { slot: binding.slot })
        }
    }

    pub(crate) fn ensure_env_capture(&mut self) -> Binding {
        if let Some(binding) = self.visible_bindings.get("_ENV") {
            return binding.clone();
        }
        let source_depth = self
            .ancestor_scopes
            .last()
            .map(|snapshot| snapshot.distance)
            .unwrap_or(0);
        self.capture_env(self.span, source_depth)
    }
}

impl From<FunctionState> for ResolvedFunction {
    fn from(value: FunctionState) -> Self {
        value.finish()
    }
}
