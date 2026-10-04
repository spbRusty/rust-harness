use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: usize,
    pub description: String,
    pub verification: String,
}

impl Plan {
    pub fn from_model_json(value: &str) -> Result<Self> {
        Ok(serde_json::from_str(value)?)
    }

    pub fn next_unfinished(&self, completed: &[usize]) -> Option<&PlanStep> {
        self.steps.iter().find(|step| !completed.contains(&step.id))
    }
}
