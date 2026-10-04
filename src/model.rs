use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;

#[derive(Debug, Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct GenerateResponse { response: String }

pub async fn generate(config: &Config, prompt: &str) -> Result<String> {
    let url = format!("{}/api/generate", config.model.endpoint.trim_end_matches('/'));
    let body = GenerateRequest { model: &config.model.name, prompt, stream: false };
    let response = Client::new().post(url).json(&body).send().await?.error_for_status()?;
    Ok(response.json::<GenerateResponse>().await?.response)
}

pub async fn list_models(config: &Config) -> Result<()> {
    let url = format!("{}/api/tags", config.model.endpoint.trim_end_matches('/'));
    let value: Value = Client::new().get(url).send().await?.error_for_status()?.json().await?;
    if let Some(models) = value.get("models").and_then(Value::as_array) {
        for model in models {
            if let Some(name) = model.get("name").and_then(Value::as_str) { println!("{name}"); }
        }
    } else {
        anyhow::bail!("Ollama returned an unexpected /api/tags response");
    }
    Ok(())
}

pub fn ensure_reachable_error(error: reqwest::Error) -> anyhow::Error {
    anyhow::anyhow!("cannot reach Ollama: {error}. Is `ollama serve` running?")
}
