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
    #[serde(default)]
    pub permissions: PermissionsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self { model: ModelConfig::default(), workspace: WorkspaceConfig::default(), mcp: Vec::new(), permissions: PermissionsConfig::default() }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default = "default_true")]
    pub read: bool,
    #[serde(default = "default_true")]
    pub write: bool,
    #[serde(default)]
    pub execute: bool,
}

impl Default for PermissionsConfig {
    fn default() -> Self { Self { read: true, write: true, execute: false } }
}

impl PermissionsConfig {
    pub fn allows(&self, permission: &str) -> bool {
        match permission {
            "read" => self.read,
            "write" => self.write,
            "execute" => self.execute,
            _ => false,
        }
    }
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
}

impl Default for ModelConfig {
    fn default() -> Self { Self { endpoint: "http://127.0.0.1:11434".into(), name: "qwen2.5:7b".into() } }
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
        if !path.exists() { return Ok(Self::default()); }
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        Ok(toml::from_str(&text).context("parse harness.toml")?)
    }
}
