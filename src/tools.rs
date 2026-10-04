use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub fn native_tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo { name: "read_file".into(), description: "Read a UTF-8 text file inside the workspace.".into(), input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}) },
        ToolInfo { name: "write_file".into(), description: "Write a UTF-8 text file inside the workspace.".into(), input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}) },
        ToolInfo { name: "list_dir".into(), description: "List files and directories inside the workspace.".into(), input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":[]}) },
    ]
}

#[derive(Debug, Deserialize)]
struct FileArgs { path: String }
#[derive(Debug, Deserialize)]
struct WriteArgs { path: String, content: String }

pub fn execute(workspace: &Path, name: &str, arguments: Value) -> Result<String> {
    match name {
        "read_file" => {
            let args: FileArgs = serde_json::from_value(arguments)?;
            Ok(fs::read_to_string(safe_path(workspace, &args.path)?)?)
        }
        "write_file" => {
            let args: WriteArgs = serde_json::from_value(arguments)?;
            let path = safe_path(workspace, &args.path)?;
            if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
            fs::write(path, args.content)?;
            Ok("written".into())
        }
        "list_dir" => {
            let args: FileArgs = serde_json::from_value(arguments)?;
            let mut entries = Vec::new();
            for entry in fs::read_dir(safe_path(workspace, &args.path)?)? {
                let entry = entry?;
                entries.push(entry.file_name().to_string_lossy().to_string());
            }
            entries.sort();
            Ok(serde_json::to_string(&entries)?)
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn safe_path(workspace: &Path, requested: &str) -> Result<std::path::PathBuf> {
    let root = workspace.canonicalize()?;
    let path = root.join(requested);
    let normalized = if path.exists() { path.canonicalize()? } else {
        let parent = path.parent().unwrap_or(&root).canonicalize()?;
        parent.join(path.file_name().ok_or_else(|| anyhow::anyhow!("invalid path"))?)
    };
    if !normalized.starts_with(&root) { anyhow::bail!("path escapes workspace"); }
    Ok(normalized)
}
