use kuu::parser::Parser;
use std::fs;
use std::path::Path;

#[test]
fn all_upstream_lua_fixtures_parse() -> Result<(), String> {
    let root = Path::new("/Users/josh/lua-5.5.0-tests");
    let mut entries = Vec::new();

    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("lua") {
            continue;
        }
        entries.push(path);
    }

    entries.sort();

    for path in entries {
        let source_bytes = fs::read(&path)
            .map_err(|error| format!("failed to read {}: {}", path.display(), error))?;
        let source = String::from_utf8_lossy(&source_bytes).into_owned();
        let mut parser = Parser::new(&source)
            .map_err(|error| format!("failed to init parser for {}: {}", path.display(), error))?;
        parser
            .parse_chunk()
            .map_err(|error| format!("failed to parse {}: {}", path.display(), error))?;
    }

    Ok(())
}
