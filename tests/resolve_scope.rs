use kuu::parser::Parser;
use kuu::resolve::{
    BindingTarget, DeclarationKind, FunctionKind, NameUseRecord, ResolvedChunk, ResolvedFunction,
    Resolver,
};

fn resolve(source: &str) -> Result<ResolvedChunk, String> {
    let mut parser = Parser::new(source).map_err(|error| error.to_string())?;
    let chunk = parser.parse_chunk().map_err(|error| error.to_string())?;
    Resolver::new()
        .resolve_chunk(&chunk)
        .map_err(|error| error.to_string())
}

fn expect_err(source: &str) -> Result<String, String> {
    let mut parser = Parser::new(source).map_err(|error| error.to_string())?;
    let chunk = parser.parse_chunk().map_err(|error| error.to_string())?;
    match Resolver::new().resolve_chunk(&chunk) {
        Ok(_) => Err("expected resolver error".to_owned()),
        Err(error) => Ok(error.to_string()),
    }
}

fn child(function: &ResolvedFunction, index: usize) -> Result<&ResolvedFunction, String> {
    function
        .children
        .get(index)
        .ok_or_else(|| format!("missing child {index}"))
}

fn use_binding<'a>(uses: &'a [NameUseRecord], name: &str) -> Result<&'a NameUseRecord, String> {
    uses.iter()
        .find(|entry| entry.name == name)
        .ok_or_else(|| format!("missing use for {name}"))
}

#[test]
fn locals_shadow_and_inner_functions_capture_the_latest_binding() -> Result<(), String> {
    let chunk = resolve(
        "local x = 1\n\
         do\n\
           local x = x\n\
           local function inner()\n\
             return x\n\
           end\n\
         end\n",
    )?;

    assert_eq!(chunk.root.kind, FunctionKind::Chunk);
    assert_eq!(chunk.root.declarations.len(), 3);
    assert_eq!(chunk.root.declarations[0].name, "x");
    assert_eq!(chunk.root.declarations[0].kind, DeclarationKind::Local);
    assert_eq!(chunk.root.declarations[1].name, "x");
    assert_eq!(chunk.root.declarations[2].name, "inner");
    assert_eq!(chunk.root.uses.len(), 1);

    let initializer = use_binding(&chunk.root.uses, "x")?;
    match &initializer.binding {
        BindingTarget::Local { slot, .. } => assert_eq!(*slot, 0),
        other => return Err(format!("expected local binding, got {other:?}")),
    }

    let inner = child(&chunk.root, 0)?;
    assert_eq!(inner.kind, FunctionKind::LocalFunction);
    assert_eq!(inner.upvalues.len(), 1);
    assert_eq!(inner.upvalues[0].name, "x");
    assert_eq!(inner.upvalues[0].source_depth, 1);

    let inner_use = use_binding(&inner.uses, "x")?;
    match &inner_use.binding {
        BindingTarget::Upvalue { source_depth, .. } => assert_eq!(*source_depth, 1),
        other => return Err(format!("expected upvalue binding, got {other:?}")),
    }

    Ok(())
}

#[test]
fn nested_functions_capture_env_for_globals_and_shadow_local_env() -> Result<(), String> {
    let chunk = resolve(
        "local _ENV = {print = print}\n\
         local function outer()\n\
           return function()\n\
             return print\n\
           end\n\
         end\n",
    )?;

    let outer = child(&chunk.root, 0)?;
    let inner = child(outer, 0)?;

    assert!(inner.upvalues.iter().any(|upvalue| upvalue.name == "_ENV"));
    assert_eq!(
        inner
            .upvalues
            .iter()
            .find(|upvalue| upvalue.name == "_ENV")
            .map(|upvalue| upvalue.source_depth),
        Some(1)
    );

    let print_use = use_binding(&inner.uses, "print")?;
    match &print_use.binding {
        BindingTarget::Global { explicit, .. } => assert!(!explicit),
        other => return Err(format!("expected global binding, got {other:?}")),
    }

    Ok(())
}

#[test]
fn global_declarations_control_free_names_and_reads_only_globals() -> Result<(), String> {
    let error = expect_err("global none\nX = 1\n")?;
    assert!(error.contains("not declared"));
    assert!(error.contains("X"));

    let error = expect_err("global<const> *\nX = 1\n")?;
    assert!(error.contains("const variable"));
    assert!(error.contains("X"));

    Ok(())
}

#[test]
fn environment_cannot_be_declared_global() -> Result<(), String> {
    let error = expect_err("global _ENV, value; value = 1\n")?;
    assert!(error.contains("variable 'value'"));
    Ok(())
}

#[test]
fn function_statements_write_through_the_resolver() -> Result<(), String> {
    let chunk = resolve("global X\nfunction X() end\n")?;

    assert_eq!(chunk.root.declarations.len(), 1);
    assert_eq!(chunk.root.uses.len(), 1);

    let write_use = use_binding(&chunk.root.uses, "X")?;
    assert!(write_use.is_write);
    match &write_use.binding {
        BindingTarget::Global {
            explicit, readonly, ..
        } => {
            assert!(*explicit);
            assert!(!*readonly);
        }
        other => return Err(format!("expected global binding, got {other:?}")),
    }

    Ok(())
}

#[test]
fn const_locals_reject_function_redefinitions() -> Result<(), String> {
    let error = expect_err("local foo<const> = 10\nfunction foo() end\n")?;
    assert!(error.contains("const variable"));
    assert!(error.contains("foo"));
    Ok(())
}

#[test]
fn multi_level_captures_have_stable_source_depth() -> Result<(), String> {
    let chunk = resolve(
        "local x = 1\n\
         local function middle()\n\
           return function()\n\
             return x\n\
           end\n\
         end\n",
    )?;

    let middle = child(&chunk.root, 0)?;
    let inner = child(middle, 0)?;

    let captured = inner
        .upvalues
        .iter()
        .find(|upvalue| upvalue.name == "x")
        .ok_or_else(|| "missing upvalue x".to_owned())?;
    assert_eq!(captured.source_depth, 2);
    Ok(())
}

#[test]
fn parent_functions_propagate_child_upvalues_to_grandchildren() -> Result<(), String> {
    let chunk = resolve(
        "local format = string.format\n\
         local function outer()\n\
           local function middle()\n\
             return function(m)\n\
               return format(\"%.1f\", m)\n\
             end\n\
           end\n\
         end\n",
    )?;

    let outer = child(&chunk.root, 0)?;
    let middle = child(outer, 0)?;
    let inner = child(middle, 0)?;

    assert!(
        middle
            .upvalues
            .iter()
            .any(|upvalue| upvalue.name == "format")
    );
    assert!(
        inner
            .upvalues
            .iter()
            .any(|upvalue| upvalue.name == "format")
    );
    Ok(())
}

#[test]
fn forward_goto_to_terminal_label_may_skip_local() -> Result<(), String> {
    resolve("do goto done; local value = 23; ::done::; end")?;
    Ok(())
}

#[test]
fn nested_label_may_shadow_later_outer_label() -> Result<(), String> {
    resolve(
        "local function f(a)\n\
         if a == 4 then\n\
           goto l1\n\
           ::l1:: return 5\n\
         end\n\
         ::l1:: return 1\n\
         end",
    )?;
    Ok(())
}
