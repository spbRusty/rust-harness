use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Done,
    Failed,
    Blocked,
    MaxIterations,
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::MaxIterations => "max_iterations",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    Passed,
    Failed,
    Skipped,
    Error,
}

#[derive(Debug, Clone)]
pub struct RunState {
    pub status: RunStatus,
    pub iterations: usize,
    pub verification_cycles: usize,
    pub verification_failures: usize,
    pub changed_files: Vec<String>,
    pub last_verification: Option<VerificationStatus>,
    pub verification_pending: bool,
    pub stop_reason: Option<String>,
}

impl Default for RunState {
    fn default() -> Self {
        Self {
            status: RunStatus::Running,
            iterations: 0,
            verification_cycles: 0,
            verification_failures: 0,
            changed_files: Vec::new(),
            last_verification: None,
            verification_pending: false,
            stop_reason: None,
        }
    }
}

impl RunState {
    pub fn begin_iteration(&mut self) {
        self.iterations += 1;
    }

    pub fn record_changed_file(&mut self, path: impl Into<String>) {
        let path = path.into();
        if !self.changed_files.iter().any(|item| item == &path) {
            self.changed_files.push(path);
        }
        self.verification_pending = true;
    }

    pub fn record_verification(&mut self, status: VerificationStatus) {
        self.verification_cycles += 1;
        self.last_verification = Some(status);
        self.verification_pending = false;
        if matches!(status, VerificationStatus::Failed | VerificationStatus::Error) {
            self.verification_failures += 1;
        }
    }

    pub fn finish(&mut self, status: RunStatus, reason: impl Into<String>) {
        self.status = status;
        self.stop_reason = Some(reason.into());
    }

    pub fn verification_successful(&self) -> bool {
        !self.verification_pending
            && matches!(
                self.last_verification,
                Some(VerificationStatus::Passed | VerificationStatus::Skipped)
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_iterations_and_unique_files() {
        let mut state = RunState::default();
        state.begin_iteration();
        state.record_changed_file("src/main.rs");
        state.record_changed_file("src/main.rs");
        state.record_changed_file("src/lib.rs");

        assert_eq!(state.iterations, 1);
        assert_eq!(state.changed_files, vec!["src/main.rs", "src/lib.rs"]);
        assert!(state.verification_pending);
    }

    #[test]
    fn failed_verification_is_not_success() {
        let mut state = RunState::default();
        state.record_changed_file("src/main.rs");
        state.record_verification(VerificationStatus::Failed);

        assert_eq!(state.verification_cycles, 1);
        assert_eq!(state.verification_failures, 1);
        assert!(!state.verification_successful());
    }

    #[test]
    fn passed_verification_is_success() {
        let mut state = RunState::default();
        state.record_changed_file("src/main.rs");
        state.record_verification(VerificationStatus::Passed);

        assert!(state.verification_successful());
        assert!(!state.verification_pending);
    }

    #[test]
    fn new_change_requires_new_verification() {
        let mut state = RunState::default();
        state.record_changed_file("src/main.rs");
        state.record_verification(VerificationStatus::Passed);
        state.record_changed_file("src/lib.rs");

        assert!(!state.verification_successful());
        assert!(state.verification_pending);
    }
}
