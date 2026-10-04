use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::Path;

use crate::{config::Config, model, tools};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { Explorer, Executor, Reviewer }

impl Role {
    pub fn parse(value: &str) -> Result<Self> {
        match value { "explorer" => Ok(Self::Explorer), "executor" => Ok(Self::Executor), "reviewer" => Ok(Self::Reviewer), _ => anyhow::bail!("unknown role: {value}; expected explorer, executor or reviewer") }
    }
    fn instructions(self) -> &'static str {
        match self {
            Self::Explorer => "You are the Explorer subagent. Inspect only. Do not modify files or run commands. Return a concise factual map of relevant files, architecture, dependencies and risks. Never invent contents.",
            Self::Executor => "You are the Executor subagent. Implement the requested change. Inspect before editing. Make the smallest coherent change. You may read, search, write and run verification commands. Do not merely describe a solution; perform it. Do not declare success without verification.",
            Self::Reviewer => "You are the Reviewer subagent. Review the workspace against the requested task. Do not modify files. Inspect relevant files and run safe verification commands when useful. Report concrete defects, missing requirements and verification results. Do not rewrite code.",
        }
    }
    fn allowed(self, name: &str) -> bool {
        match self {
            Self::Explorer => matches!(name, "list_dir" | "project_search" | "read_file"),
            Self::Executor => matches!(name, "list_dir" | "project_search" | "read_file" | "write_file" | "run_command"),
            Self::Reviewer => matches!(name, "list_dir" | "project_search" | "read_file" | "run_command"),
        }
    }
}

pub async fn run(config: &Config, role: Role, workspace: &Path, task: &str) -> Result<String> {
    let registry = tools::ToolRegistry::native_filtered(|tool| role.allowed(&tool.name));
    let definitions = registry.ollama_definitions();
    println!("[{role:?}] start: {task}");

    let system = format!("{}\nWorkspace: {}\nFinish with a compact report for the parent agent.", role.instructions(), workspace.display());
    let mut messages = vec![
        model::Message { role: "system".into(), content: Some(system), tool_calls: None, tool_name: None },
        model::Message { role: "user".into(), content: Some(task.into()), tool_calls: None, tool_name: None },
    ];

    for iteration in 1..=6 {
        println!("[{role:?}] model iteration {iteration}/6");
        let assistant = model::chat(config, &messages, &definitions).await?;
        let Some(calls) = &assistant.tool_calls else {
            let answer = assistant.content.clone().unwrap_or_default();
            println!("[{role:?}] finished");
            return Ok(answer);
        };
        messages.push(assistant.clone());
        for call in calls {
            println!("[{role:?}] tool: {}", call.function.name);
            if registry.find(&call.function.name).is_none() {
                messages.push(model::Message { role: "tool".into(), content: Some(json!({"error":"tool not allowed for this role"}).to_string()), tool_calls: None, tool_name: Some(call.function.name.clone()) });
                continue;
            }
            let result = if call.function.name == "run_command" {
                tools::execute_command(workspace, call.function.arguments.clone()).await.map(Value::String)
            } else {
                tools::execute(workspace, &call.function.name, call.function.arguments.clone()).map(Value::String)
            };
            let content = match result {
                Ok(value) => value.to_string(),
                Err(error) => json!({"error": error.to_string()}).to_string(),
            };
            println!("[{role:?}] tool result: {}", if content.len() > 160 { format!("{}...", &content[..160]) } else { content.clone() });
            messages.push(model::Message { role: "tool".into(), content: Some(content), tool_calls: None, tool_name: Some(call.function.name.clone()) });
        }
    }
    anyhow::bail!("{role:?} subagent reached its iteration limit")
}

pub async fn run_named(config: &Config, role: &str, workspace: &Path, task: &str) -> Result<String> {
    run(config, Role::parse(role)?, workspace, task).await.context("delegated subagent failed")
}
