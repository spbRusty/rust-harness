use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpClient {
    pub async fn spawn(command: &str, args: &[String]) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("start MCP server: {command}"))?;
        let stdin = child.stdin.take().context("MCP stdin unavailable")?;
        let stdout = child.stdout.take().context("MCP stdout unavailable")?;
        let mut client = Self { child, stdin, stdout: BufReader::new(stdout), next_id: 1 };
        client.request("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "rust-harness", "version": env!("CARGO_PKG_VERSION")}
        })).await?;
        client.notify("notifications/initialized", json!({})).await?;
        Ok(client)
    }

    pub async fn list_tools(&mut self) -> Result<Value> {
        self.request("tools/list", json!({})).await
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        self.request("tools/call", json!({"name": name, "arguments": arguments})).await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        self.send(message).await?;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).await?;
            if line.is_empty() { anyhow::bail!("MCP server closed stdout"); }
            let value: Value = serde_json::from_str(line.trim()).context("invalid MCP JSON-RPC message")?;
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = value.get("error") { anyhow::bail!("MCP error: {error}"); }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params})).await
    }

    async fn send(&mut self, value: Value) -> Result<()> {
        let text = serde_json::to_string(&value)?;
        self.stdin.write_all(text.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

impl Drop for McpClient {
    fn drop(&mut self) { let _ = self.child.start_kill(); }
}
