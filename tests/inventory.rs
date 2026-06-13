use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetPhase {
    Phase2,
    Phase3,
    Phase4,
    Phase6,
    Phase8,
    Phase10,
    Phase11,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InventoryRow {
    name: String,
    target: TargetPhase,
}

fn classify(name: &str) -> Option<TargetPhase> {
    match name {
        "all.lua" => Some(TargetPhase::Phase11),
        "api.lua" => Some(TargetPhase::Unsupported),
        "attrib.lua" => Some(TargetPhase::Phase10),
        "big.lua" => Some(TargetPhase::Phase8),
        "bitwise.lua" => Some(TargetPhase::Phase10),
        "bwcoercion.lua" => Some(TargetPhase::Phase10),
        "calls.lua" => Some(TargetPhase::Phase3),
        "closure.lua" => Some(TargetPhase::Phase3),
        "code.lua" => Some(TargetPhase::Phase3),
        "constructs.lua" => Some(TargetPhase::Phase3),
        "coroutine.lua" => Some(TargetPhase::Phase6),
        "cstack.lua" => Some(TargetPhase::Phase8),
        "db.lua" => Some(TargetPhase::Phase10),
        "errors.lua" => Some(TargetPhase::Phase2),
        "events.lua" => Some(TargetPhase::Phase4),
        "files.lua" => Some(TargetPhase::Phase10),
        "gc.lua" => Some(TargetPhase::Phase8),
        "gengc.lua" => Some(TargetPhase::Phase8),
        "goto.lua" => Some(TargetPhase::Phase3),
        "heavy.lua" => Some(TargetPhase::Phase8),
        "literals.lua" => Some(TargetPhase::Phase3),
        "locals.lua" => Some(TargetPhase::Phase3),
        "main.lua" => Some(TargetPhase::Phase3),
        "math.lua" => Some(TargetPhase::Phase10),
        "memerr.lua" => Some(TargetPhase::Phase8),
        "nextvar.lua" => Some(TargetPhase::Phase4),
        "pm.lua" => Some(TargetPhase::Phase10),
        "sort.lua" => Some(TargetPhase::Phase10),
        "strings.lua" => Some(TargetPhase::Phase10),
        "tpack.lua" => Some(TargetPhase::Phase10),
        "tracegc.lua" => Some(TargetPhase::Phase8),
        "utf8.lua" => Some(TargetPhase::Phase10),
        "vararg.lua" => Some(TargetPhase::Phase3),
        "verybig.lua" => Some(TargetPhase::Phase8),
        _ => None,
    }
}

fn collect_inventory(root: &Path) -> Result<Vec<InventoryRow>, io::Error> {
    let mut rows = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("lua") {
            continue;
        }

        let name = match path.file_name().and_then(|value| value.to_str()) {
            Some(value) => value,
            None => continue,
        };

        let target = match classify(name) {
            Some(value) => value,
            None => {
                return Err(io::Error::other(format!(
                    "unclassified upstream test: {}",
                    name
                )));
            }
        };

        rows.push(InventoryRow {
            name: name.to_owned(),
            target,
        });
    }

    rows.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(rows)
}

#[test]
fn all_upstream_lua_scripts_are_classified() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new("/Users/josh/lua-5.5.0-tests");
    let rows = collect_inventory(root)?;

    let expected_count = 34;
    assert_eq!(rows.len(), expected_count);
    assert!(
        rows.iter()
            .any(|row| row.name == "api.lua" && row.target == TargetPhase::Unsupported)
    );
    assert!(
        rows.iter()
            .any(|row| row.name == "all.lua" && row.target == TargetPhase::Phase11)
    );
    Ok(())
}
