use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{config::Config, model, planner::Plan, project_context::ProjectContext};

const MAX_PLAN_STEPS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub goal: String,
    pub plan: Option<Plan>,
    pub completed_steps: Vec<usize>,
    pub status: String,
    pub feedback: Vec<String>,
}

pub async fn build_plan(config: &Config, workspace: &Path, goal: &str) -> Result<Plan> {
    let context = ProjectContext::discover(workspace)?;
    let prompt = format!(
        "Ты Planner. Не меняй файлы.\n{}\n\nЦель: {}\n\n" +
        "Верни ТОЛЬКО JSON: {{\"goal\":\"...\",\"steps\":[{{\"id\":1,\"description\":\"...\",\"verification\":\"...\"}}]}}. " +
        "Максимум {} коротких последовательных шагов.",
        context.prompt(), goal, MAX_PLAN_STEPS
    );
    let messages = vec![
        model::Message { role: "system".into(), content: Some("Ты Planner coding harness.".into()), tool_calls: None, tool_name: None },
        model::Message { role: "user".into(), content: Some(prompt), tool_calls: None, tool_name: None },
    ];
    let response = model::chat(config, &messages, &[]).await?;
    let text = response.content.unwrap_or_default();
    let json = extract_json(&text)?;
    Plan::from_model_json(&json)
}

fn extract_json(text: &str) -> Result<String> {
    let start = text.find('{').ok_or_else(|| anyhow::anyhow!("planner did not return JSON"))?;
    let end = text.rfind('}').ok_or_else(|| anyhow::anyhow!("planner returned incomplete JSON"))?;
    Ok(text[start..=end].to_string())
}

pub async fn run(config: &Config, workspace: &Path, goal: &str) -> Result<WorkflowResult> {
    let plan = build_plan(config, workspace, goal).await?;
    if plan.steps.is_empty() {
        return Ok(WorkflowResult { goal: goal.into(), plan: Some(plan), completed_steps: vec![], status: "failed".into(), feedback: vec!["Planner returned an empty plan".into()] });
    }

    let mut completed = Vec::new();
    let mut feedback = Vec::new();
    for step in &plan.steps {
        let executor_prompt = format!(
            "Ты Executor. Выполни ТОЛЬКО этот шаг.\nЦель: {}\nШаг {}: {}\nПроверка: {}\n{}",
            goal, step.id, step.description, step.verification,
            if feedback.is_empty() { String::new() } else { format!("Предыдущая обратная связь: {}", feedback.join("\n")) }
        );
        let result = crate::orchestration::run_named(config, "executor", workspace, &executor_prompt).await?;
        feedback.push(format!("Executor step {}: {}", step.id, result));

        let review_prompt = format!(
            "Ты Reviewer. Не меняй файлы. Проверь выполнение шага {}: {}. Ожидаемая проверка: {}. Ответь кратко: PASS или FAIL и причины.",
            step.id, step.description, step.verification
        );
        let review = crate::orchestration::run_named(config, "reviewer", workspace, &review_prompt).await?;
        feedback.push(format!("Reviewer step {}: {}", step.id, review));

        if review.trim_start().to_ascii_uppercase().contains("PASS") {
            completed.push(step.id);
            continue;
        }

        const MAX_REPAIRS: usize = 2;
        let mut repaired = false;
        for attempt in 1..=MAX_REPAIRS {
            let repair_prompt = format!(
                "Ты Executor. Исправь только текущий шаг после замечаний Reviewer.                  Не начинай задачу заново. Проверь результат после исправления.                 \nЦель: {}\nШаг {}: {}\nТребование проверки: {}\nЗамечания Reviewer: {}\nПопытка исправления: {}",
                goal, step.id, step.description, step.verification, review, attempt
            );
            let repair = crate::orchestration::run_named(config, "executor", workspace, &repair_prompt).await?;
            feedback.push(format!("Executor repair {} step {}: {}", attempt, step.id, repair));

            let repair_review_prompt = format!(
                "Ты Reviewer. Не меняй файлы. Повторно проверь шаг {} после исправления.                  Требование: {}. Предыдущие замечания: {}. Ответь в первой строке строго PASS или FAIL, затем причины.",
                step.id, step.verification, review
            );
            let repair_review = crate::orchestration::run_named(config, "reviewer", workspace, &repair_review_prompt).await?;
            feedback.push(format!("Reviewer repair {} step {}: {}", attempt, step.id, repair_review));

            if repair_review.trim_start().to_ascii_uppercase().starts_with("PASS") {
                completed.push(step.id);
                repaired = true;
                break;
            }
        }

        if !repaired {
            return Ok(WorkflowResult {
                goal: goal.into(),
                plan: Some(plan),
                completed_steps: completed,
                status: "failed".into(),
                feedback,
            });
        }
    }

    Ok(WorkflowResult { goal: goal.into(), plan: Some(plan), completed_steps: completed, status: "done".into(), feedback })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_json() { assert_eq!(extract_json("answer {\"goal\":\"x\"}").unwrap(), "{\"goal\":\"x\"}"); }
}
