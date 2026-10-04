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


