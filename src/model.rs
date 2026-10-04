use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    tools: &'a [Value],
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: Message,
}

pub async fn chat(config: &Config, messages: &[Message], tools: &[Value]) -> Result<Message> {
    let url = format!("{}/api/chat", config.model.endpoint.trim_end_matches('/'));
    let body = ChatRequest {
        model: &config.model.name,
        messages,
        tools,
        stream: false,
    };

    let response = Client::new()
        .post(url)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;

    Ok(response.json::<ChatResponse>().await?.message)
}

pub async fn list_models(config: &Config) -> Result<()> {
    let url = format!("{}/api/tags", config.model.endpoint.trim_end_matches('/'));
    let value: Value = Client::new()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    if let Some(models) = value.get("models").and_then(Value::as_array) {
        for model in models {
            if let Some(name) = model.get("name").and_then(Value::as_str) {
                println!("{name}");
            }
        }
    } else {
        anyhow::bail!("Ollama returned an unexpected /api/tags response");
    }

    Ok(())
}
