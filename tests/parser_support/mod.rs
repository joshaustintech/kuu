use kuu::ast::Chunk;
use kuu::parser::Parser;

#[allow(dead_code)]
pub fn parse_snapshot(source: &str) -> Result<String, String> {
    let mut parser = Parser::new(source).map_err(|error| error.to_string())?;
    let chunk = parser.parse_chunk().map_err(|error| error.to_string())?;
    Ok(chunk.snapshot())
}

#[allow(dead_code)]
pub fn assert_snapshot(source: &str, expected: &str) -> Result<(), String> {
    let actual = parse_snapshot(source)?;
    if actual != expected {
        return Err(format!(
            "snapshot mismatch for {:?}\nexpected:\n{}\nactual:\n{}",
            source, expected, actual
        ));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn assert_parse_error_contains(source: &str, needle: &str) -> Result<(), String> {
    let mut parser = Parser::new(source).map_err(|error| error.to_string())?;
    match parser.parse_chunk() {
        Ok(chunk) => Err(format!(
            "expected parse error for {:?}, got {:?}",
            source, chunk
        )),
        Err(error) => {
            let text = error.to_string();
            if text.contains(needle) {
                Ok(())
            } else {
                Err(format!(
                    "expected error containing {:?} for {:?}, got {}",
                    needle, source, text
                ))
            }
        }
    }
}

#[allow(dead_code)]
pub fn assert_chunk_parses(source: &str) -> Result<Chunk, String> {
    let mut parser = Parser::new(source).map_err(|error| error.to_string())?;
    parser.parse_chunk().map_err(|error| error.to_string())
}
