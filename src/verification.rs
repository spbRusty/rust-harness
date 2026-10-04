use anyhow::Result;
use serde_json::json;
use std::{fs, path::Path};
use std::process::Stdio;
use tokio::{process::Command, time::{timeout, Duration}};

const MAX_OUTPUT: usize = 12_000;
const TIMEOUT_SECS: u64 = 60;

pub async fn verify(workspace: &Path) -> Result<String> {
    if workspace.join("Cargo.toml").exists() {
        return run(workspace, "cargo", &["check"]).await;
    }

    if workspace.join("package.json").exists() {
        return verify_node(workspace).await;
    }

    Ok(json!({
        "status": "skipped",
        "reason": "no supported project manifest found"
    }).to_string())
}

async fn verify_node(workspace: &Path) -> Result<String> {
    let package = fs::read_to_string(workspace.join("package.json"))?;
    let value: serde_json::Value = serde_json::from_str(&package)?;
    let scripts = value.get("scripts").and_then(|v| v.as_object());

    let command = if scripts.and_then(|s| s.get("test")).is_some() {
        ("npm", vec!["test".to_string()])
    } else if scripts.and_then(|s| s.get("build")).is_some() {
        ("npm", vec!["run".to_string(), "build".to_string()])
    } else {
        return Ok(json!({
            "status": "skipped",
            "reason": "package.json has no test or build script"
        }).to_string());
    };

    let args: Vec<&str> = command.1.iter().map(String::as_str).collect();
    run(workspace, command.0, &args).await
}

async fn run(workspace: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = timeout(
        Duration::from_secs(TIMEOUT_SECS),
        Command::new(program)
            .args(args)
            .current_dir(workspace)
            .stdin(Stdio::null())
            .output(),
    ).await??;

    let status = if output.status.success() { "passed" } else { "failed" };
    let mut text = format!("status: {status}\nexit_code: {:?}\n", output.status.code());

    if !output.stdout.is_empty() {
        text.push_str("stdout:\n");
        text.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        text.push_str("\nstderr:\n");
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }

    if text.len() > MAX_OUTPUT {
        text.truncate(MAX_OUTPUT);
        text.push_str("\n...[verification output truncated]...");
    }

    Ok(text)
}
