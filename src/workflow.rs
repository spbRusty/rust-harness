use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{config::Config, model, planner::Plan, project_context::ProjectContext};

const MAX_PLAN_STEPS: usize = 8;
const MAX_REPAIRS: usize = 2;
const MAX_FEEDBACK_ITEMS: usize = 8;
const MAX_FEEDBACK_CHARS: usize = 12_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub goal: String,
    pub plan: Option<Plan>,
    pub completed_steps: Vec<usize>,
    pub status: String,
    pub changed_files: Vec<String>,
    pub feedback: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ReviewResult {
    status: String,
    #[serde(default)]
    findings: Vec<String>,
}

pub async fn build_plan(config: &Config, workspace: &Path, goal: &str) -> Result<Plan> {
    println!("[PLAN] inspecting project...");
    let context = ProjectContext::discover(workspace)?;
    let prompt = format!(
        "Ты Planner. Не меняй файлы.\n{}\n\nЦель: {}\n\nВерни ТОЛЬКО JSON: {{\"goal\":\"...\",\"steps\":[{{\"id\":1,\"description\":\"...\",\"verification\":\"...\"}}]}}. Максимум {} коротких последовательных шагов.",
        context.prompt(), goal, MAX_PLAN_STEPS
    );
    println!("[PLAN] asking model...");
    let messages = vec![
        model::Message { role: "system".into(), content: Some("Ты Planner coding harness.".into()), tool_calls: None, tool_name: None },
        model::Message { role: "user".into(), content: Some(prompt), tool_calls: None, tool_name: None },
    ];
    let response = model::chat(config, &messages, &[]).await?;
    let text = response.content.unwrap_or_default();
    let json = extract_json(&text)?;
    let plan = Plan::from_model_json(&json)?;
    println!("[PLAN] {} step(s)", plan.steps.len());
    for step in &plan.steps {
        println!("[PLAN] {}: {}", step.id, step.description);
    }
    Ok(plan)
}

fn extract_json(text: &str) -> Result<String> {
    let start = text.find('{').ok_or_else(|| anyhow::anyhow!("planner did not return JSON"))?;
    let end = text.rfind('}').ok_or_else(|| anyhow::anyhow!("planner returned incomplete JSON"))?;
    Ok(text[start..=end].to_string())
}

fn parse_review(text: &str) -> Result<ReviewResult> {
    let json = extract_json(text)?;
    let review: ReviewResult = serde_json::from_str(&json)?;
    if !matches!(review.status.to_ascii_lowercase().as_str(), "pass" | "fail") {
        anyhow::bail!("reviewer returned invalid status: {}", review.status);
    }
    Ok(review)
}

fn compact_feedback(feedback: &[String]) -> String {
    let start = feedback.len().saturating_sub(MAX_FEEDBACK_ITEMS);
    let mut value = feedback[start..].join("\n");
    if value.len() > MAX_FEEDBACK_CHARS {
        value.truncate(MAX_FEEDBACK_CHARS);
        value.push_str("\n...[feedback truncated]...");
    }
    value
}

fn review_prompt(step_id: usize, description: &str, verification: &str) -> String {
    format!(
        "Ты Reviewer. Не меняй файлы. Независимо проверь текущий workspace против шага {}.\nШаг: {}. Требование: {}.\nВерни ТОЛЬКО JSON вида {{\"status\":\"pass|fail\",\"findings\":[\"...\"]}}.\nstatus=pass только если требование реально выполнено; иначе fail.",
        step_id, description, verification
    )
}

fn changed_files(before: &str, after: &str) -> Vec<String> {
    let mut result = Vec::new();
    let before_value: serde_json::Value = serde_json::from_str(before).unwrap_or_default();
    let after_value: serde_json::Value = serde_json::from_str(after).unwrap_or_default();
    let _ = (before_value, after_value);
    if before != after {
        result.push("workspace changed (see git diff)".into());
    }
    result
}

pub async fn run(config: &Config, workspace: &Path, goal: &str) -> Result<WorkflowResult> {
    println!("[TASK] goal: {goal}");
    let before = git_status(workspace).unwrap_or_default();
    let plan = build_plan(config, workspace, goal).await?;
    if plan.steps.is_empty() {
        return Ok(WorkflowResult { goal: goal.into(), plan: Some(plan), completed_steps: vec![], status: "failed".into(), changed_files: vec![], feedback: vec!["Planner returned an empty plan".into()] });
    }

    let mut completed = Vec::new();
    let mut feedback = Vec::new();

    for step in &plan.steps {
        println!("[EXECUTOR] step {}/{}: {}", step.id, plan.steps.len(), step.description);
        let context = compact_feedback(&feedback);
        let executor_prompt = format!(
            "Ты Executor. Выполни ТОЛЬКО этот шаг.\nЦель: {}\nШаг {}: {}\nПроверка: {}\n{}",
            goal, step.id, step.description, step.verification,
            if context.is_empty() { String::new() } else { format!("Предыдущая обратная связь:\n{}", context) }
        );

        let result = crate::orchestration::run_named(config, "executor", workspace, &executor_prompt).await?;
        feedback.push(format!("Executor step {}: {}", step.id, result));

        println!("[REVIEWER] checking step {}", step.id);
        let review = crate::orchestration::run_named(config, "reviewer", workspace, &review_prompt(step.id, &step.description, &step.verification)).await?;
        feedback.push(format!("Reviewer step {}: {}", step.id, review));

        let parsed_review = match parse_review(&review) {
            Ok(value) => value,
            Err(error) => {
                feedback.push(format!("Reviewer parse error step {}: {}", step.id, error));
                return Ok(WorkflowResult { goal: goal.into(), plan: Some(plan), completed_steps: completed, status: "failed".into(), changed_files: changed_files(&before, &git_status(workspace).unwrap_or_default()), feedback });
            }
        };

        if parsed_review.status.eq_ignore_ascii_case("pass") {
            println!("[REVIEWER] step {} PASS", step.id);
            completed.push(step.id);
            continue;
        }

        println!("[REVIEWER] step {} FAIL: {}", step.id, parsed_review.findings.join("; "));
        let mut repaired = false;
        let mut findings = parsed_review.findings.join("\n");

        for attempt in 1..=MAX_REPAIRS {
            println!("[REPAIR] step {} attempt {}/{}", step.id, attempt, MAX_REPAIRS);
            let repair_prompt = format!(
                "Ты Executor. Исправь ТОЛЬКО текущий шаг после замечаний Reviewer.\nНе начинай задачу заново. После исправления проверь результат.\nЦель: {}\nШаг {}: {}\nТребование проверки: {}\nЗамечания Reviewer:\n{}\nПопытка исправления: {}",
                goal, step.id, step.description, step.verification, findings, attempt
            );
            let repair = crate::orchestration::run_named(config, "executor", workspace, &repair_prompt).await?;
            feedback.push(format!("Executor repair {} step {}: {}", attempt, step.id, repair));

            println!("[REVIEWER] rechecking step {} after repair", step.id);
            let repair_review = crate::orchestration::run_named(config, "reviewer", workspace, &review_prompt(step.id, &step.description, &step.verification)).await?;
            feedback.push(format!("Reviewer repair {} step {}: {}", attempt, step.id, repair_review));

            let parsed = match parse_review(&repair_review) {
                Ok(value) => value,
                Err(error) => {
                    findings = error.to_string();
                    feedback.push(format!("Reviewer parse error repair {} step {}: {}", attempt, step.id, error));
                    continue;
                }
            };

            if parsed.status.eq_ignore_ascii_case("pass") {
                println!("[REVIEWER] step {} PASS after repair", step.id);
                completed.push(step.id);
                repaired = true;
                break;
            }

            println!("[REVIEWER] step {} still FAIL: {}", step.id, parsed.findings.join("; "));
            findings = parsed.findings.join("\n");
        }

        if !repaired {
            println!("[TASK] FAILED at step {}", step.id);
            return Ok(WorkflowResult { goal: goal.into(), plan: Some(plan), completed_steps: completed, status: "failed".into(), changed_files: changed_files(&before, &git_status(workspace).unwrap_or_default()), feedback });
        }
    }

    let changed = changed_files(&before, &git_status(workspace).unwrap_or_default());
    println!("[TASK] DONE; changed: {}", if changed.is_empty() { "none" } else { "yes" });
    Ok(WorkflowResult { goal: goal.into(), plan: Some(plan), completed_steps: completed, status: "done".into(), changed_files: changed, feedback })
}

fn git_status(workspace: &Path) -> Option<String> {
    let output = std::process::Command::new("git").arg("-C").arg(workspace).arg("status").arg("--short").output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json() {
        assert_eq!(extract_json(r#"answer {"goal":"x"}"#).unwrap(), r#"{"goal":"x"}"#);
    }

    #[test]
    fn parses_structured_review() {
        let review = parse_review(r#"{"status":"pass","findings":[]}"#).unwrap();
        assert_eq!(review.status, "pass");
        assert!(review.findings.is_empty());
    }

    #[test]
    fn rejects_invalid_review_status() {
        assert!(parse_review(r#"{"status":"maybe","findings":[]}"#).is_err());
    }

    #[test]
    fn compacts_feedback() {
        let feedback = vec!["a".to_string(); MAX_FEEDBACK_ITEMS + 2];
        let compacted = compact_feedback(&feedback);
        assert_eq!(compacted.lines().count(), MAX_FEEDBACK_ITEMS);
    }
}
