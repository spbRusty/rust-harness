use anyhow::{Context, Result};
use serde_json::{json, Value};
use crate::{config::Config, mcp::McpClient, model, tools};

const MAX_STEPS: usize = 8;

pub async fn run(config: &Config, prompt: &str) -> Result<()> {
    let workspace = config.workspace.path.canonicalize().context("workspace does not exist")?;
    let native = tools::native_tools();
    let mut mcp_clients = Vec::new();
    let mut mcp_tools = Vec::new();

    for server in &config.mcp {
        let mut client = McpClient::spawn(&server.command, &server.args).await?;
        let result = client.list_tools().await?;
        if let Some(items) = result.get("tools").and_then(Value::as_array) {
            for tool in items {
                if let Some(name) = tool.get("name").and_then(Value::as_str) {
                    mcp_tools.push(json!({"type":"mcp","server":server.name,"name":name,"description":tool.get("description").cloned().unwrap_or(Value::Null),"inputSchema":tool.get("inputSchema").cloned().unwrap_or(json!({"type":"object"}))}));
                }
            }
        }
        mcp_clients.push((server.name.clone(), client));
    }

    let tool_text = serde_json::to_string(&json!({"native":native,"mcp":mcp_tools}))?;
    let mut conversation = format!("You are a local coding agent. Workspace: {}\nAvailable tools: {}\n\nUser task: {}\n\nIf you need a tool, respond with ONLY JSON: {{\"tool\":\"name\",\"arguments\":{{...}}}}. For MCP tools use {{\"mcp_server\":\"server\",\"tool\":\"name\",\"arguments\":{{...}}}}. When finished, answer normally.", workspace.display(), tool_text, prompt);

    for _ in 0..MAX_STEPS {
        let answer = model::generate(config, &conversation).await?;
        if let Some(call) = parse_tool_call(&answer) {
            let result = if let Some(server) = call.get("mcp_server").and_then(Value::as_str) {
                let tool_name = call.get("tool").and_then(Value::as_str).context("missing MCP tool")?;
                let args = call.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let client = mcp_clients.iter_mut().find(|(name, _)| name == server).context("MCP server not found")?;
                client.1.call_tool(tool_name, args).await?
            } else {
                let tool_name = call.get("tool").and_then(Value::as_str).context("missing tool")?;
                let args = call.get("arguments").cloned().unwrap_or_else(|| json!({}));
                json!({"content": tools::execute(&workspace, tool_name, args)?})
            };
            conversation.push_str(&format!("\n\nAssistant tool call: {}\nTool result: {}\nContinue the task.", answer, result));
        } else {
            println!("{answer}");
            return Ok(());
        }
    }
    anyhow::bail!("agent reached the maximum of {MAX_STEPS} steps")
}

fn parse_tool_call(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let candidate = if trimmed.starts_with("```") {
        trimmed.trim_matches('`').trim_start_matches("json").trim()
    } else { trimmed };
    serde_json::from_str::<Value>(candidate).ok().filter(|v| v.get("tool").is_some())
}
