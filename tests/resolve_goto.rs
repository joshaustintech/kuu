use kuu::parser::Parser;
use kuu::resolve::goto::Resolver;

fn resolve(source: &str) -> Result<(), String> {
    let mut parser = Parser::new(source).map_err(|error| error.to_string())?;
    let chunk = parser.parse_chunk().map_err(|error| error.to_string())?;
    Resolver::resolve_chunk(&chunk).map_err(|error| error.to_string())
}

#[test]
fn allows_forward_and_backward_jumps_in_the_same_block() -> Result<(), String> {
    resolve(
        "goto later\n::early::\ngoto early\n::later::\n",
    )?;
    Ok(())
}

#[test]
fn rejects_duplicate_labels_visible_from_nested_blocks() -> Result<(), String> {
    let error = match resolve("::loop::\ndo\n  ::loop::\nend\n") {
        Ok(()) => return Err("expected duplicate label error".to_string()),
        Err(error) => error,
    };
    assert!(
        error.contains("duplicate visible label"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn rejects_goto_that_enters_global_scope() -> Result<(), String> {
    let error = match resolve("goto done\nglobal *\n::done::\n") {
        Ok(()) => return Err("expected global scope error".to_string()),
        Err(error) => error,
    };
    assert!(
        error.contains("enter the scope"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn rejects_goto_that_enters_to_be_closed_scope() -> Result<(), String> {
    let error = match resolve("goto done\nlocal x<close> = 1\n::done::\n") {
        Ok(()) => return Err("expected to-be-closed scope error".to_string()),
        Err(error) => error,
    };
    assert!(
        error.contains("enter the scope"),
        "unexpected error: {error}"
    );
    Ok(())
}
