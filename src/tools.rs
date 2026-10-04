use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}};
use std::process::Stdio;
use tokio::{process::Command, time::{timeout, Duration}};

pub const MAX_TOOL_OUTPUT: usize = 12_000;
pub const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 30;
pub const MAX_COMMAND_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo { pub name: String, pub description: String, pub input_schema: Value, pub permission: String }

#[derive(Debug, Clone, Default)]
pub struct ToolRegistry { tools: Vec<ToolInfo> }
impl ToolRegistry {
    pub fn native() -> Self { Self { tools: native_tools() } }
    pub fn native_filtered<F>(mut predicate: F) -> Self where F: FnMut(&ToolInfo) -> bool { Self { tools: native_tools().into_iter().filter(|t| predicate(t)).collect() } }
    pub fn register(&mut self, tool: ToolInfo) { self.tools.push(tool); }
    pub fn all(&self) -> &[ToolInfo] { &self.tools }
    pub fn find(&self, name: &str) -> Option<&ToolInfo> { self.tools.iter().find(|tool| tool.name == name) }
    pub fn ollama_definitions(&self) -> Vec<Value> { self.tools.iter().map(|tool| json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":tool.input_schema}})).collect() }
}

pub fn native_tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo { name: "list_dir".into(), description: "List files and directories inside the workspace. Path defaults to the workspace root.".into(), input_schema: json!({"type":"object","properties":{"path":{"type":"string","default":"."}},"required":[]}), permission: "read".into() },
        ToolInfo { name: "project_search".into(), description: "Search text recursively in project source files. Skips .git, target and node_modules.".into(), input_schema: json!({"type":"object","properties":{"query":{"type":"string"},"path":{"type":"string","default":"."},"max_results":{"type":"integer","minimum":1,"maximum":100,"default":50}},"required":["query"]}), permission: "read".into() },
        ToolInfo { name: "read_file".into(), description: "Read a UTF-8 text file inside the workspace.".into(), input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}), permission: "read".into() },
        ToolInfo { name: "write_file".into(), description: "Write a UTF-8 text file inside the workspace.".into(), input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}), permission: "write".into() },
        ToolInfo { name: "run_command".into(), description: "Run a program in the workspace with explicit arguments. Use for tests, checks and builds. Do not use a shell command string.".into(), input_schema: json!({"type":"object","properties":{"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"},"default":[]},"timeout_secs":{"type":"integer","minimum":1,"maximum":120,"default":30}},"required":["program"]}), permission: "execute".into() },
        ToolInfo { name: "task".into(), description: "Delegate a bounded subtask. role must be explorer, executor or reviewer. Explorer inspects only; executor edits and verifies; reviewer inspects and verifies without edits. Use for focused work that benefits from isolated context.".into(), input_schema: json!({"type":"object","properties":{"role":{"type":"string","enum":["explorer","executor","reviewer"]},"task":{"type":"string"}},"required":["role","task"]}), permission: "delegate".into() },
    ]
}

#[derive(Debug, Deserialize)] struct FileArgs { path: String }
#[derive(Debug, Deserialize)] struct WriteArgs { path: String, content: String }
#[derive(Debug, Deserialize)] struct ListArgs { #[serde(default = "default_path")] path: String }
#[derive(Debug, Deserialize)] struct SearchArgs { query: String, #[serde(default = "default_path")] path: String, #[serde(default = "default_max_results")] max_results: usize }
#[derive(Debug, Deserialize)] struct CommandArgs { program: String, #[serde(default)] args: Vec<String>, #[serde(default = "default_command_timeout")] timeout_secs: u64 }
fn default_path() -> String { ".".into() }
fn default_max_results() -> usize { 50 }
fn default_command_timeout() -> u64 { DEFAULT_COMMAND_TIMEOUT_SECS }

pub fn execute(workspace: &Path, name: &str, arguments: Value) -> Result<String> {
    let result = match name {
        "read_file" => { let args: FileArgs = serde_json::from_value(arguments)?; fs::read_to_string(safe_path(workspace, &args.path)?)? }
        "write_file" => { let args: WriteArgs = serde_json::from_value(arguments)?; let path = safe_path(workspace, &args.path)?; if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; } fs::write(path, args.content)?; "written".into() }
        "list_dir" => { let args: ListArgs = serde_json::from_value(arguments)?; let mut entries = Vec::new(); for entry in fs::read_dir(safe_path(workspace, &args.path)?)? { entries.push(entry?.file_name().to_string_lossy().to_string()); } entries.sort(); serde_json::to_string(&entries)? }
        "project_search" => { let args: SearchArgs = serde_json::from_value(arguments)?; project_search(workspace, &args)? }
        "run_command" | "task" => anyhow::bail!("{name} must be executed asynchronously"),
        _ => anyhow::bail!("unknown tool: {name}"),
    };
    Ok(bound_output(result))
}

pub async fn execute_command(workspace: &Path, arguments: Value) -> Result<String> {
    let args: CommandArgs = serde_json::from_value(arguments)?; let timeout_secs = args.timeout_secs.clamp(1, MAX_COMMAND_TIMEOUT_SECS);
    let output = timeout(Duration::from_secs(timeout_secs), Command::new(&args.program).args(&args.args).current_dir(workspace).stdin(Stdio::null()).output()).await??;
    let mut result = format!("exit_code: {:?}\n", output.status.code());
    if !output.stdout.is_empty() { result.push_str("stdout:\n"); result.push_str(&String::from_utf8_lossy(&output.stdout)); }
    if !output.stderr.is_empty() { result.push_str("\nstderr:\n"); result.push_str(&String::from_utf8_lossy(&output.stderr)); }
    Ok(bound_output(result))
}

fn project_search(workspace: &Path, args: &SearchArgs) -> Result<String> { let root = safe_path(workspace, &args.path)?; let query = args.query.to_lowercase(); let limit = args.max_results.clamp(1, 100); let mut results = Vec::new(); search_dir(workspace, &root, &query, limit, &mut results)?; if results.is_empty() { return Ok("no matches".into()); } Ok(results.join("\n")) }
fn search_dir(workspace: &Path, dir: &Path, query: &str, limit: usize, results: &mut Vec<String>) -> Result<()> { if results.len() >= limit { return Ok(()); } for entry in fs::read_dir(dir)? { if results.len() >= limit { break; } let entry = entry?; let path = entry.path(); let name = entry.file_name().to_string_lossy().to_string(); if entry.file_type()?.is_dir() { if matches!(name.as_str(), ".git" | "target" | "node_modules") { continue; } search_dir(workspace, &path, query, limit, results)?; continue; } if !is_text_file(&path) { continue; } let Ok(text) = fs::read_to_string(&path) else { continue; }; for (line_no, line) in text.lines().enumerate() { if line.to_lowercase().contains(query) { let relative = path.strip_prefix(workspace).unwrap_or(&path).display(); let snippet = if line.len() > 300 { &line[..300] } else { line }; results.push(format!("{}:{}: {}", relative, line_no + 1, snippet.trim())); if results.len() >= limit { break; } } } } Ok(()) }
fn is_text_file(path: &Path) -> bool { matches!(path.extension().and_then(|x| x.to_str()), Some("rs"|"toml"|"md"|"txt"|"json"|"jsonc"|"yaml"|"yml"|"js"|"ts"|"jsx"|"tsx"|"html"|"css"|"py"|"lua"|"xml"|"cfg"|"ini"|"sh"|"bat")) }
fn bound_output(mut value: String) -> String { if value.len() <= MAX_TOOL_OUTPUT { return value; } value.truncate(MAX_TOOL_OUTPUT); value.push_str("\n...[tool output truncated]..."); value }
fn safe_path(workspace: &Path, requested: &str) -> Result<PathBuf> { let root = workspace.canonicalize()?; let path = root.join(requested); let normalized = if path.exists() { path.canonicalize()? } else { let parent = path.parent().unwrap_or(&root).canonicalize()?; parent.join(path.file_name().ok_or_else(|| anyhow::anyhow!("invalid path"))?) }; if !normalized.starts_with(&root) { anyhow::bail!("path escapes workspace"); } Ok(normalized) }

#[cfg(test)]
mod tests { use super::*; #[test] fn tool_output_is_bounded() { let value = bound_output("x".repeat(MAX_TOOL_OUTPUT + 100)); assert!(value.len() <= MAX_TOOL_OUTPUT + 64); assert!(value.contains("[tool output truncated]")); } #[test] fn registry_contains_task() { let registry = ToolRegistry::native(); assert!(registry.all().iter().any(|x| x.name == "task")); } }
