use kuu::lexer::{Lexer, TokenKind};
use std::fs;
use std::path::Path;

fn tokenize_all(source: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut lexer = Lexer::new(source);
    loop {
        let token = lexer.next_token()?;
        if matches!(token.kind, TokenKind::Eof) {
            return Ok(());
        }
    }
}

#[test]
fn local_upstream_scripts_tokenize() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/upstream/lua-5.5.0-tests");
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("lua") {
            continue;
        }
        let source = fs::read(&path)?;
        let source = String::from_utf8_lossy(&source);
        tokenize_all(&source)?;
    }
    Ok(())
}
