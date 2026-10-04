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

    let native = tools::native_tools();
    let mut mcp_clients = Vec::new();
    let mut ollama_tools = Vec::new();

    for tool in native {
        ollama_tools.push(json!({
            "type": "function",
            "function": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema
            }
        }));
    }

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

                ollama_tools.push(json!({
                    "type": "function",
                    "function": {
                        "name": format!("mcp__{}__{}", server.name, name),
                        "description": description,
                        "parameters": tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({"type":"object"}))
                    }
                }));
            }
        }

        mcp_clients.push((server.name.clone(), client));
    }

    let system = format!(
        "You are a local coding agent. Workspace: {}.          Use the available tools when you need information or need to modify files.          Do not guess file contents or project structure.          If the user asks you to inspect a project, actually read the relevant files before answering.",
        workspace.display()
    );

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
