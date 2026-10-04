use anyhow::{Context, Result};
use serde_json::{json, Value};
use crate::{config::Config, mcp::McpClient, model, tools, verification};

const MAX_STEPS: usize = 12;
const MAX_CONTEXT_CHARS: usize = 48_000;
const MAX_VERIFICATION_CYCLES: usize = 3;

fn trim_context(messages: &mut Vec<model::Message>) {
    let mut total = messages.iter().map(|m| m.content.as_deref().unwrap_or("").len()).sum::<usize>();
    if total <= MAX_CONTEXT_CHARS { return; }
    let mut i = 2;
    while total > MAX_CONTEXT_CHARS && i < messages.len() {
        if messages[i].role == "tool" {
            if let Some(content) = messages[i].content.take() {
                total -= content.len();
                messages[i].content = Some("[previous tool result pruned]".into());
            }
        }
        i += 1;
    }
}

pub async fn run(config: &Config, prompt: &str) -> Result<()> {
    let workspace = config.workspace.path.canonicalize().context("workspace does not exist")?;
    let mut registry = tools::ToolRegistry::native();
    let mut mcp_clients = Vec::new();

    for server in &config.mcp {
        let mut client = McpClient::spawn(&server.command, &server.args).await?;
        let result = client.list_tools().await?;
        if let Some(items) = result.get("tools").and_then(Value::as_array) {
            for tool in items {
                let Some(name) = tool.get("name").and_then(Value::as_str) else { continue; };
                registry.register(tools::ToolInfo {
                    name: format!("mcp__{}__{}", server.name, name),
                    description: format!("[MCP server: {}] {}", server.name, tool.get("description").and_then(Value::as_str).unwrap_or("")),
                    input_schema: tool.get("inputSchema").cloned().unwrap_or_else(|| json!({"type":"object"})),
                    permission: "execute".into(),
                });
            }
        }
        mcp_clients.push((server.name.clone(), client));
    }

    let system = format!(
        "You are a local coding agent. Workspace: {}. Work iteratively and use tools instead of guessing. After changing files, the harness automatically runs verification. Treat a failed verification as actionable feedback and fix the underlying problem before declaring the task complete.",
        workspace.display()
    );
    let ollama_tools = registry.ollama_definitions();
    let mut messages = vec![
        model::Message { role: "system".into(), content: Some(system), tool_calls: None, tool_name: None },
        model::Message { role: "user".into(), content: Some(prompt.into()), tool_calls: None, tool_name: None },
    ];
    let mut verification_cycles = 0;

    for _ in 0..MAX_STEPS {
        let assistant = model::chat(config, &messages, &ollama_tools).await?;
        if let Some(tool_calls) = &assistant.tool_calls {
            if tool_calls.is_empty() {
                println!("{}", assistant.content.unwrap_or_default());
                return Ok(());
            }

            messages.push(assistant.clone());
            let mut changed_files = false;

            for call in tool_calls {
                if call.function.name == "write_file" {
                    changed_files = true;
                }

                let result = execute_call(
                    &workspace,
                    config,
                    &registry,
                    &mut mcp_clients,
                    &call.function.name,
                    call.function.arguments.clone(),
                ).await;

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
                trim_context(&mut messages);
            }

            if changed_files && verification_cycles < MAX_VERIFICATION_CYCLES {
                verification_cycles += 1;
                let result = verification::verify(&workspace).await;
                let content = match result {
                    Ok(value) => value,
                    Err(error) => json!({"status":"error","error":error.to_string()}).to_string(),
                };
                messages.push(model::Message {
                    role: "tool".into(),
                    content: Some(content),
                    tool_calls: None,
                    tool_name: Some("verification".into()),
                });
                trim_context(&mut messages);
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
    config: &Config,
    registry: &tools::ToolRegistry,
    mcp_clients: &mut [(String, McpClient)],
    tool_name: &str,
    arguments: Value,
) -> Result<Value> {
    let permission = registry.find(tool_name).map(|tool| tool.permission.as_str()).unwrap_or("execute");
    if !config.permissions.allows(permission) {
        anyhow::bail!("tool permission denied");
    }

    if tool_name == "run_command" {
        return Ok(json!({"content": tools::execute_command(workspace, arguments).await?}));
    }

    if let Some(rest) = tool_name.strip_prefix("mcp__") {
        let (server, name) = rest.split_once("__").context("invalid MCP tool name")?;
        let client = mcp_clients
            .iter_mut()
            .find(|(configured_name, _)| configured_name == server)
            .context("MCP server not found")?;
        return Ok(client.1.call_tool(name, arguments).await?);
    }

    Ok(json!({"content": tools::execute(workspace, tool_name, arguments)?}))
}
