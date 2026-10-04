use anyhow::Result;
use std::{fs, path::{Path, PathBuf}};

const MAX_FILE_CHARS: usize = 12_000;

#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub instructions: Vec<(PathBuf, String)>,
    pub manifests: Vec<PathBuf>,
}

impl ProjectContext {
    pub fn discover(root: &Path) -> Result<Self> {
        let mut instructions = Vec::new();
        for name in ["AGENTS.md", "README.md", "CLAUDE.md", ".cursorrules"] {
            let path = root.join(name);
            if path.is_file() {
                let mut text = fs::read_to_string(&path)?;
                if text.len() > MAX_FILE_CHARS { text.truncate(MAX_FILE_CHARS); }
                instructions.push((path, text));
            }
        }
        let mut manifests = Vec::new();
        for name in ["Cargo.toml", "package.json", "pyproject.toml", "go.mod", "composer.json"] {
            let path = root.join(name);
            if path.is_file() { manifests.push(path); }
        }
        Ok(Self { root: root.to_path_buf(), instructions, manifests })
    }

    pub fn prompt(&self) -> String {
        let mut out = format!("Project root: {}\n", self.root.display());
        if !self.manifests.is_empty() {
            out.push_str("Manifests: ");
            out.push_str(&self.manifests.iter().map(|p| p.file_name().unwrap().to_string_lossy()).collect::<Vec<_>>().join(", "));
            out.push('\n');
        }
        for (path, text) in &self.instructions {
            out.push_str(&format!("\n--- {} ---\n{}\n", path.display(), text));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discovers_project_context() {
        let dir = std::env::temp_dir().join(format!("rust-harness-context-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("README.md"), "test project").unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
        let ctx = ProjectContext::discover(&dir).unwrap();
        assert_eq!(ctx.instructions.len(), 1);
        assert_eq!(ctx.manifests.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }
}
