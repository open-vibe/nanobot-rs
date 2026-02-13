use crate::utils::ensure_dir;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct MemoryStore {
    pub memory_dir: PathBuf,
    pub memory_file: PathBuf,
    pub history_file: PathBuf,
}

impl MemoryStore {
    pub fn new(workspace: PathBuf) -> std::io::Result<Self> {
        let memory_dir = ensure_dir(&workspace.join("memory"))?;
        let memory_file = memory_dir.join("MEMORY.md");
        let history_file = memory_dir.join("HISTORY.md");
        Ok(Self {
            memory_dir,
            memory_file,
            history_file,
        })
    }

    pub fn read_long_term(&self) -> String {
        std::fs::read_to_string(&self.memory_file).unwrap_or_default()
    }

    pub fn write_long_term(&self, content: &str) -> std::io::Result<()> {
        std::fs::write(&self.memory_file, content)
    }

    pub fn append_history(&self, entry: &str) -> std::io::Result<()> {
        let mut existing = std::fs::read_to_string(&self.history_file).unwrap_or_default();
        existing.push_str(entry.trim_end());
        existing.push_str("\n\n");
        std::fs::write(&self.history_file, existing)
    }

    pub fn get_memory_context(&self) -> String {
        let long_term = self.read_long_term();
        if long_term.is_empty() {
            String::new()
        } else {
            format!("## Long-term Memory\n{}", long_term)
        }
    }
}
