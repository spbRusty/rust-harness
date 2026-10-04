use anyhow::{Context, Result};
use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub mcp: Vec<McpConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self { endpoint: "http://127.0.0.1:11434".into(), name: "qwen2.5:7b".into() }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceConfig {
    pub path: PathBuf,
}

impl Default for WorkspaceConfig {
    fn default() -> Self { Self { path: PathBuf::from(".") } }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = PathBuf::from("harness.toml");
        if !path.exists() { return Ok(Self { ..Default::default() }); }
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        Ok(toml::from_str(&text).context("parse harness.toml")?)
    }
}
