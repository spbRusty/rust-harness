use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}};

pub const MAX_TOOL_OUTPUT: usize = 12_000;

#[derive(Debug, Clone, Default)]
pub struct ToolRegistry { tools: Vec<ToolInfo> }

impl ToolRegistry {
    pub fn native() -> Self { Self { tools: native_tools() } }
    pub fn register(&mut self, tool: ToolInfo) { self.tools.push(tool); }
    pub fn all(&self) -> &[ToolInfo] { &self.tools }
    pub fn ollama_definitions(&self) -> Vec<Value> {
        self.tools.iter().map(|tool| json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":tool.input_schema}})).collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ToolRegistry { tools: Vec<ToolInfo> }

impl ToolRegistry {
    pub fn native() -> Self { Self { tools: native_tools() } }
    pub fn register(&mut self, tool: ToolInfo) { self.tools.push(tool); }
    pub fn all(&self) -> &[ToolInfo] { &self.tools }
    pub fn ollama_definitions(&self) -> Vec<Value> {
        self.tools.iter().map(|tool| json!({
            "type": "function",
            "function": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema
            }
        })).collect()
    }
}

pub fn native_tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo {
            name: "list_dir".into(),
            description: "List files and directories inside the workspace. Path defaults to the workspace root.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string","default":"."}},"required":[]}),
        },
        ToolInfo {
            name: "search_files".into(),
            description: "Search text in project files recursively. Use this before reading many files. Returns matching paths and short lines; skips .git, target and node_modules.".into(),
            input_schema: json!({"type":"object","properties":{
                "query":{"type":"string","description":"Case-insensitive text to search for"},
                "path":{"type":"string","default":"."},
                "max_results":{"type":"integer","minimum":1,"maximum":100,"default":50}
            },"required":["query"]}),
        },
        ToolInfo {
            name: "read_file".into(),
            description: "Read a UTF-8 text file inside the workspace. Read only files relevant to the task.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        },
        ToolInfo {
            name: "write_file".into(),
            description: "Write a UTF-8 text file inside the workspace.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        },
    ]
}

#[derive(Debug, Deserialize)]
struct FileArgs { path: String }
#[derive(Debug, Deserialize)]
struct WriteArgs { path: String, content: String }
#[derive(Debug, Deserialize)]
struct ListArgs { #[serde(default = "default_path")] path: String }
#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default = "default_path")] path: String,
    #[serde(default = "default_max_results")] max_results: usize,
}
fn default_path() -> String { ".".into() }

fn bound_output(mut value: String) -> String {
    if value.len() <= MAX_TOOL_OUTPUT { return value; }
    value.truncate(MAX_TOOL_OUTPUT);
    value.push_str("\n...[tool output truncated]...");
    value
}
fn default_max_results() -> usize { 50 }

pub fn execute(workspace: &Path, name: &str, arguments: Value) -> Result<String> {
    let result = match name {
        "read_file" => {
            let args: FileArgs = serde_json::from_value(arguments)?;
            fs::read_to_string(safe_path(workspace, &args.path)?)?
        }
        "write_file" => {
            let args: WriteArgs = serde_json::from_value(arguments)?;
            let path = safe_path(workspace, &args.path)?;
            if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
            fs::write(path, args.content)?;
            "written".into()
        }
        "list_dir" => {
            let args: ListArgs = serde_json::from_value(arguments)?;
            let mut entries = Vec::new();
            for entry in fs::read_dir(safe_path(workspace, &args.path)?)? {
                entries.push(entry?.file_name().to_string_lossy().to_string());
            }
            entries.sort();
            serde_json::to_string(&entries)?
        }
        "search_files" => {
            let args: SearchArgs = serde_json::from_value(arguments)?;
            search_files(workspace, &args)?
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    };
    Ok(bound_output(result))
}

fn search_files(workspace: &Path, args: &SearchArgs) -> Result<String> {
    let root = safe_path(workspace, &args.path)?;
    let query = args.query.to_lowercase();
    let limit = args.max_results.clamp(1, 100);
    let mut matches = Vec::new();
    search_dir(workspace, &root, &query, limit, &mut matches)?;
    if matches.is_empty() { return Ok("no matches".into()); }
    Ok(matches.join("\n"))
}

fn search_dir(workspace: &Path, dir: &Path, query: &str, limit: usize, matches: &mut Vec<String>) -> Result<()> {
    if matches.len() >= limit { return Ok(()); }
    for entry in fs::read_dir(dir)? {
        if matches.len() >= limit { break; }
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if matches!(name.as_str(), ".git" | "target" | "node_modules") { continue; }
            search_dir(workspace, &path, query, limit, matches)?;
            continue;
        }
        if !is_probably_text(&path) { continue; }
        let Ok(text) = fs::read_to_string(&path) else { continue; };
        for (line_no, line) in text.lines().enumerate() {
            if line.to_lowercase().contains(query) {
                let relative = path.strip_prefix(workspace).unwrap_or(&path).display();
                let line = line.trim();
                let snippet = if line.len() > 300 { &line[..300] } else { line };
                matches.push(format!("{}:{}: {}", relative, line_no + 1, snippet));
                if matches.len() >= limit { break; }
            }
        }
    }
    Ok(())
}

fn is_probably_text(path: &Path) -> bool {
    match path.extension().and_then(|x| x.to_str()) {
        Some(ext) => matches!(ext, "rs"|"toml"|"md"|"txt"|"json"|"jsonc"|"yaml"|"yml"|"js"|"ts"|"jsx"|"tsx"|"html"|"css"|"py"|"lua"|"xml"|"cfg"|"ini"|"sh"|"bat"),
        None => matches!(path.file_name().and_then(|x| x.to_str()), Some("Makefile"|"Dockerfile")),
    }
}

fn bound_output(mut value: String) -> String {
    if value.len() <= MAX_TOOL_OUTPUT { return value; }
    value.truncate(MAX_TOOL_OUTPUT);
    value.push_str("\n...[tool output truncated]...");
    value
}

fn safe_path(workspace: &Path, requested: &str) -> Result<PathBuf> {
    let root = workspace.canonicalize()?;
    let path = root.join(requested);
    let normalized = if path.exists() {
        path.canonicalize()?
    } else {
        let parent = path.parent().unwrap_or(&root).canonicalize()?;
        parent.join(path.file_name().ok_or_else(|| anyhow::anyhow!("invalid path"))?)
    };
    if !normalized.starts_with(&root) { anyhow::bail!("path escapes workspace"); }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_output_is_bounded() {
        let value = bound_output("x".repeat(MAX_TOOL_OUTPUT + 100));
        assert!(value.len() <= MAX_TOOL_OUTPUT + 64);
        assert!(value.contains("[tool output truncated]"));
    }
    #[test]
    fn registry_contains_search() {
        let registry = ToolRegistry::native();
        assert!(registry.all().iter().any(|x| x.name == "search_files"));
    }
}
