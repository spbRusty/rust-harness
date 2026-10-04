#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Explorer,
    Planner,
    Executor,
    Reviewer,
}

impl Role {
    pub fn system_prompt(self) -> &'static str {
        match self {
            Self::Explorer => "Исследуй проект только чтением. Не меняй файлы.",
            Self::Planner => "Составь короткий проверяемый план. Не меняй файлы.",
            Self::Executor => "Выполняй только текущий шаг плана. После изменений нужна проверка.",
            Self::Reviewer => "Проверь результат независимо. Не исправляй файлы.",
        }
    }
}
