use std::path::Path;

pub fn path_in(dir: &Path) -> std::path::PathBuf {
    dir.join("mcp-audit.jsonl")
}
