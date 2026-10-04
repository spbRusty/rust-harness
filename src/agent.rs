use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::{config::Config, mcp::McpClient, model, tools};

const MAX_STEPS: usize = 12;

pub async fn run(config: &Config, prompt: &str) -> Result<()> {
    let workspace = config
        .workspace
        .path
        .canonicalize()
        .context("workspace does not exist")?;

    let mut registry = tools::ToolRegistry::native();
    let mut mcp_clients = Vec::new();

    for server in &config.mcp {
        let mut client = McpClient::spawn(&server.command, &server.args).await?;
        let result = client.list_tools().await?;

        if let Some(items) = result.get("tools").and_then(Value::as_array) {
            for tool in items {
                let Some(name) = tool.get("name").and_then(Value::as_str) else {
                    continue;
                };

                let mut description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                description = format!("[MCP server: {}] {}", server.name, description);

                registry.register(tools::ToolInfo {
                    name: format!("mcp__{}__{}", server.name, name),
                    description,
                    input_schema: tool.get("inputSchema").cloned().unwrap_or_else(|| json!({"type":"object"})),
                });
            }
        }

        mcp_clients.push((server.name.clone(), client));
    }

    let system = format!(
        "You are a local coding agent. Workspace: {}. Work iteratively and use tools instead of guessing. For project inspection, establish structure, locate relevant code, then read the files needed. Do not stop after one tool call if the request requires more evidence. Complete the requested inspection or change before answering. If you modify files, verify the result when practical.",
        workspace.display()
    );

    let ollama_tools = registry.ollama_definitions();

    let mut messages = vec![
        model::Message {
            role: "system".into(),
            content: Some(system),
            tool_calls: None,
            tool_name: None,
        },
        model::Message {
            role: "user".into(),
            content: Some(prompt.into()),
            tool_calls: None,
            tool_name: None,
        },
    ];

    for _ in 0..MAX_STEPS {
        let assistant = model::chat(config, &messages, &ollama_tools).await?;

        if let Some(tool_calls) = &assistant.tool_calls {
            if tool_calls.is_empty() {
                println!("{}", assistant.content.unwrap_or_default());
                return Ok(());
            }

            messages.push(assistant.clone());

            for call in tool_calls {
                let result = execute_call(
                    &workspace,
                    &mut mcp_clients,
                    &call.function.name,
                    call.function.arguments.clone(),
                )
                .await;

                let content = match result {
                    Ok(value) => value.to_string(),
                    Err(error) => json!({"error": error.to_string()}).to_string(),
                };

                messages.push(model::Message {
                    role: "tool".into(),
                    content: Some(content),
                    tool_calls: None,
                    tool_name: Some(call.function.name.clone()),
                });
            }
        } else {
            println!("{}", assistant.content.unwrap_or_default());
            return Ok(());
        }
    }

    anyhow::bail!("agent reached the maximum of {MAX_STEPS} steps")
}

async fn execute_call(
    workspace: &std::path::Path,
    mcp_clients: &mut [(String, McpClient)],
    tool_name: &str,
    arguments: Value,
) -> Result<Value> {
    if let Some(rest) = tool_name.strip_prefix("mcp__") {
        let (server, name) = rest
            .split_once("__")
            .context("invalid MCP tool name")?;

        let client = mcp_clients
            .iter_mut()
            .find(|(configured_name, _)| configured_name == server)
            .context("MCP server not found")?;

        return Ok(client.1.call_tool(name, arguments).await?);
    }

    Ok(json!({
        "content": tools::execute(workspace, tool_name, arguments)?
    }))
}
