use crate::utils::{ensure_dir, today_date};
use chrono::{Duration, Local};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct MemoryStore {
    pub workspace: PathBuf,
    pub memory_dir: PathBuf,
    pub memory_file: PathBuf,
}

impl MemoryStore {
    pub fn new(workspace: PathBuf) -> std::io::Result<Self> {
        let memory_dir = ensure_dir(&workspace.join("memory"))?;
        let memory_file = memory_dir.join("MEMORY.md");
        Ok(Self {
            workspace,
            memory_dir,
            memory_file,
        })
    }

    pub fn get_today_file(&self) -> PathBuf {
        self.memory_dir.join(format!("{}.md", today_date()))
    }

    pub fn read_today(&self) -> String {
        let path = self.get_today_file();
        std::fs::read_to_string(path).unwrap_or_default()
    }

    pub fn append_today(&self, content: &str) -> std::io::Result<()> {
        let path = self.get_today_file();
        if path.exists() {
            let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
            if !existing.is_empty() {
                existing.push('\n');
            }
            existing.push_str(content);
            std::fs::write(path, existing)?;
        } else {
            let body = format!("# {}\n\n{}", today_date(), content);
            std::fs::write(path, body)?;
        }
        Ok(())
    }

    pub fn read_long_term(&self) -> String {
        std::fs::read_to_string(&self.memory_file).unwrap_or_default()
    }

    pub fn write_long_term(&self, content: &str) -> std::io::Result<()> {
        std::fs::write(&self.memory_file, content)
    }

    pub fn get_recent_memories(&self, days: usize) -> String {
        let mut memories = Vec::new();
        let today = Local::now().date_naive();

        for i in 0..days {
            let date = today - Duration::days(i as i64);
            let date_str = date.format("%Y-%m-%d").to_string();
            let file_path = self.memory_dir.join(format!("{date_str}.md"));
            if file_path.exists()
                && let Ok(content) = std::fs::read_to_string(file_path)
            {
                memories.push(content);
            }
        }

        memories.join("\n\n---\n\n")
    }

    pub fn list_memory_files(&self) -> Vec<PathBuf> {
        if !self.memory_dir.exists() {
            return Vec::new();
        }
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                {
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default();
                    if stem.len() == 10 {
                        files.push(path);
                    }
                }
            }
        }
        files.sort_by(|a, b| b.cmp(a));
        files
    }

    pub fn get_memory_context(&self) -> String {
        let mut parts = Vec::new();
        let long_term = self.read_long_term();
        if !long_term.is_empty() {
            parts.push(format!("## Long-term Memory\n{}", long_term));
        }
        let today = self.read_today();
        if !today.is_empty() {
            parts.push(format!("## Today's Notes\n{}", today));
        }
        parts.join("\n\n")
    }
}
