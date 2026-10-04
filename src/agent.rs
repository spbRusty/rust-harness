use anyhow::{Context, Result};
use serde_json::{json, Value};
use crate::{
    config::Config,
    mcp::McpClient,
    model,
    state::{RunState, RunStatus, VerificationStatus},
    tools,
    verification,
};

const MAX_STEPS: usize = 12;
const MAX_CONTEXT_CHARS: usize = 48_000;
const MAX_VERIFICATION_CYCLES: usize = 3;

fn trim_context(messages: &mut Vec<model::Message>) {
    let mut total = messages
        .iter()
        .map(|m| m.content.as_deref().unwrap_or("").len())
        .sum::<usize>();

    if total <= MAX_CONTEXT_CHARS {
        return;
    }

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

fn verification_status(value: &str) -> VerificationStatus {
    match serde_json::from_str::<Value>(value)
        .ok()
        .and_then(|v| v.get("status").and_then(Value::as_str).map(str::to_owned))
        .as_deref()
    {
        Some("passed") => VerificationStatus::Passed,
        Some("failed") => VerificationStatus::Failed,
        Some("skipped") => VerificationStatus::Skipped,
        Some("error") => VerificationStatus::Error,
        _ => VerificationStatus::Error,
    }
}

fn is_write_like_tool(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "write_file"
        || lower.contains("write")
        || lower.contains("edit")
        || lower.contains("create_file")
        || lower.contains("delete")
        || lower.contains("remove")
        || lower.contains("patch")
}

fn changed_path(tool_name: &str, arguments: &Value) -> Option<String> {
    if !is_write_like_tool(tool_name) {
        return None;
    }

    arguments
        .get("path")
        .or_else(|| arguments.get("file"))
        .or_else(|| arguments.get("file_path"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn print_final(state: &RunState, message: &str) {
    println!("{message}");
    println!(
        "\n[run status: {}; iterations: {}; verification cycles: {}; verification failures: {}]",
        state.status,
        state.iterations,
        state.verification_cycles,
        state.verification_failures
    );
    if !state.changed_files.is_empty() {
        println!("[changed files: {}]", state.changed_files.join(", "));
    }
    if let Some(reason) = &state.stop_reason {
        println!("[stop reason: {reason}]");
    }
}

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

                registry.register(tools::ToolInfo {
                    name: format!("mcp__{}__{}", server.name, name),
                    description: format!(
                        "[MCP server: {}] {}",
                        server.name,
                        tool.get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                    ),
                    input_schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type":"object"})),
                    permission: "execute".into(),
                });
            }
        }

        mcp_clients.push((server.name.clone(), client));
    }

    let system = format!(
        "You are a local coding agent. Workspace: {}. Work iteratively and use tools instead of guessing.          After changing files, the harness automatically runs verification.          A successful verification is required before declaring a file-changing task complete.          If verification fails, inspect the failure, fix the underlying problem, and verify again.          Do not claim completion while verification is failing.",
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

    let mut state = RunState::default();

    for _ in 0..MAX_STEPS {
        state.begin_iteration();

        let assistant = model::chat(config, &messages, &ollama_tools).await?;

        let Some(tool_calls) = &assistant.tool_calls else {
            let answer = assistant.content.unwrap_or_default();

            if state.changed_files.is_empty() || state.verification_successful() {
                state.finish(RunStatus::Done, "completion criteria satisfied");
                print_final(&state, &answer);
                return Ok(());
            }

            if state.verification_cycles >= MAX_VERIFICATION_CYCLES {
                state.finish(
                    RunStatus::Failed,
                    "maximum verification cycles reached without a successful verification",
                );
                print_final(&state, &answer);
                return Ok(());
            }

            messages.push(assistant);
            messages.push(model::Message {
                role: "user".into(),
                content: Some(
                    "The task changed files but verification has not passed. Continue working:                      inspect the verification result, fix the underlying problem, and run the                      required tools. Do not declare the task complete yet."
                        .into(),
                ),
                tool_calls: None,
                tool_name: None,
            });
            trim_context(&mut messages);
            continue;
        };

        if tool_calls.is_empty() {
            state.finish(RunStatus::Done, "model returned no tool calls");
            print_final(&state, &assistant.content.unwrap_or_default());
            return Ok(());
        }

        messages.push(assistant.clone());

        for call in tool_calls {
            let tool_name = &call.function.name;

            if let Some(path) = changed_path(tool_name, &call.function.arguments) {
                state.record_changed_file(path);
            }

            let result = execute_call(
                &workspace,
                config,
                &registry,
                &mut mcp_clients,
                tool_name,
                call.function.arguments.clone(),
            )
            .await;

            let (content, blocked) = match result {
                Ok(value) => (value.to_string(), false),
                Err(error) => {
                    let text = error.to_string();
                    (json!({"error": text}).to_string(), text.contains("permission denied"))
                }
            };

            messages.push(model::Message {
                role: "tool".into(),
                content: Some(content),
                tool_calls: None,
                tool_name: Some(tool_name.clone()),
            });
            trim_context(&mut messages);

            if blocked {
                state.finish(RunStatus::Blocked, "tool permission denied");
                print_final(&state, "Run blocked: tool permission denied.");
                return Ok(());
            }
        }

        if state.verification_pending
            && state.verification_cycles < MAX_VERIFICATION_CYCLES
        {
            let result = verification::verify(&workspace).await;
            let content = match result {
                Ok(value) => value,
                Err(error) => json!({
                    "status": "error",
                    "error": error.to_string()
                }).to_string(),
            };

            state.record_verification(verification_status(&content));

            messages.push(model::Message {
                role: "tool".into(),
                content: Some(content),
                tool_calls: None,
                tool_name: Some("verification".into()),
            });
            trim_context(&mut messages);
        }

        if state.verification_failures >= MAX_VERIFICATION_CYCLES {
            state.finish(
                RunStatus::Failed,
                "maximum verification failures reached",
            );
            print_final(&state, "Run failed: verification keeps failing.");
            return Ok(());
        }
    }

    state.finish(
        RunStatus::MaxIterations,
        format!("maximum of {MAX_STEPS} agent iterations reached"),
    );
    print_final(&state, "Run stopped: maximum iterations reached.");
    Ok(())
}

async fn execute_call(
    workspace: &std::path::Path,
    config: &Config,
    registry: &tools::ToolRegistry,
    mcp_clients: &mut [(String, McpClient)],
    tool_name: &str,
    arguments: Value,
) -> Result<Value> {
    let permission = registry
        .find(tool_name)
        .map(|tool| tool.permission.as_str())
        .unwrap_or("execute");

    if !config.permissions.allows(permission) {
        anyhow::bail!("tool permission denied");
    }

    if tool_name == "run_command" {
        return Ok(json!({
            "content": tools::execute_command(workspace, arguments).await?
        }));
    }

    if let Some(rest) = tool_name.strip_prefix("mcp__") {
        let (server, name) = rest.split_once("__").context("invalid MCP tool name")?;
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
