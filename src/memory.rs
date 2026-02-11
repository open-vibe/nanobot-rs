use crate::utils::{ensure_dir, today_date};
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

    pub fn remember_fact(&self, fact: &str) -> std::io::Result<bool> {
        let normalized = fact.trim();
        if normalized.is_empty() {
            return Ok(false);
        }

        let mut long_term = self.read_long_term();
        if long_term.is_empty() {
            long_term = "# Long-term Memory\n\n## Important Notes\n".to_string();
        }

        if let Some((key, value)) = parse_keyed_fact(normalized)
            && upsert_keyed_fact(&mut long_term, key, value)?
        {
            self.write_long_term(&long_term)?;
            return Ok(true);
        } else if parse_keyed_fact(normalized).is_some() {
            return Ok(false);
        }

        let normalized_target = normalize_memory_line(normalized);
        if !normalized_target.is_empty()
            && long_term
                .lines()
                .map(normalize_memory_line)
                .any(|line| line == normalized_target)
        {
            return Ok(false);
        }

        if !long_term.contains("## Important Notes") {
            if !long_term.ends_with('\n') {
                long_term.push('\n');
            }
            long_term.push('\n');
            long_term.push_str("## Important Notes\n");
        }

        if !long_term.ends_with('\n') {
            long_term.push('\n');
        }
        long_term.push_str(&format!("- {normalized}\n"));
        self.write_long_term(&long_term)?;
        Ok(true)
    }

    pub fn extract_explicit_memory(message: &str) -> Option<String> {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return None;
        }

        let chinese_prefixes = [
            "记住",
            "请记住",
            "帮我记住",
            "麻烦记住",
            "记一下",
            "请记一下",
        ];
        for prefix in chinese_prefixes {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let normalized = canonicalize_memory_fact(&normalize_memory_request(rest));
                if !normalized.is_empty() {
                    return Some(normalized);
                }
            }
        }

        let lower = trimmed.to_ascii_lowercase();
        let english_prefixes = [
            "remember that",
            "remember",
            "please remember that",
            "please remember",
            "note that",
        ];
        for prefix in english_prefixes {
            if lower.starts_with(prefix) {
                let rest = &trimmed[prefix.len()..];
                let normalized = canonicalize_memory_fact(&normalize_memory_request(rest));
                if !normalized.is_empty() {
                    return Some(normalized);
                }
            }
        }
        None
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

fn normalize_memory_request(input: &str) -> String {
    input
        .trim_start_matches(|c: char| {
            matches!(
                c,
                ' ' | '\t'
                    | '\n'
                    | ','
                    | '，'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | '.'
                    | '。'
                    | '!'
                    | '！'
                    | '?'
                    | '？'
                    | '-'
            )
        })
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

fn normalize_memory_line(line: &str) -> String {
    line.trim()
        .trim_start_matches(|c: char| matches!(c, '-' | '*' | '•'))
        .trim()
        .to_ascii_lowercase()
}

fn parse_keyed_fact(fact: &str) -> Option<(&str, &str)> {
    let (key, value) = if let Some((k, v)) = fact.split_once('：') {
        (k.trim(), v.trim())
    } else if let Some((k, v)) = fact.split_once(':') {
        (k.trim(), v.trim())
    } else {
        return None;
    };
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key, value))
}

fn upsert_keyed_fact(content: &mut String, key: &str, value: &str) -> std::io::Result<bool> {
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;
    let mut changed = false;

    for raw in content.lines() {
        let trimmed = raw.trim();
        if key == "用户姓名"
            && trimmed.starts_with("-")
            && trimmed.contains("我不叫")
            && trimmed.contains("我叫")
        {
            changed = true;
            continue;
        }

        if let Some((existing_key, existing_value)) =
            parse_keyed_fact(trimmed.trim_start_matches('-').trim())
            && existing_key == key
        {
            found = true;
            if existing_value == value {
                lines.push(raw.to_string());
            } else {
                lines.push(format!("- {}：{}", key, value));
                changed = true;
            }
            continue;
        }

        lines.push(raw.to_string());
    }

    if !found {
        if !lines.iter().any(|l| l.trim() == "## Important Notes") {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("## Important Notes".to_string());
        }
        lines.push(format!("- {}：{}", key, value));
        changed = true;
    }

    let mut new_content = lines.join("\n");
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }

    if new_content != *content {
        *content = new_content;
        changed = true;
    }

    Ok(changed)
}

fn canonicalize_memory_fact(fact: &str) -> String {
    if let Some(name) = extract_user_name(fact) {
        return format!("用户姓名：{name}");
    }
    fact.to_string()
}

fn extract_user_name(fact: &str) -> Option<String> {
    if let Some((_, after)) = fact.rsplit_once("我叫")
        && let Some(name) = trim_name(after)
    {
        return Some(name);
    }

    let lower = fact.to_ascii_lowercase();
    if let Some(idx) = lower.rfind("my name is") {
        let after = &fact[idx + "my name is".len()..];
        if let Some(name) = trim_name(after) {
            return Some(name);
        }
    }
    None
}

fn trim_name(value: &str) -> Option<String> {
    let name = value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .split(|c: char| {
            matches!(
                c,
                ',' | '，' | ';' | '；' | '.' | '。' | '!' | '！' | '?' | '？' | '\n'
            )
        })
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryStore;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_workspace() -> PathBuf {
        std::env::temp_dir().join(format!("nanobot-rs-memory-{}", Uuid::new_v4()))
    }

    #[test]
    fn extract_explicit_memory_supports_chinese_and_english() {
        let cn = MemoryStore::extract_explicit_memory("记住,我不叫Leo,我叫习惯安静");
        assert_eq!(cn.as_deref(), Some("用户姓名：习惯安静"));

        let en = MemoryStore::extract_explicit_memory("remember that my name is quiet habit");
        assert_eq!(en.as_deref(), Some("用户姓名：quiet habit"));
    }

    #[test]
    fn remember_fact_appends_and_deduplicates() {
        let workspace = temp_workspace();
        let store = MemoryStore::new(workspace.clone()).expect("create memory store");

        std::fs::write(
            workspace.join("memory").join("MEMORY.md"),
            "# Memory\n- 用户姓名：Leo\n- 项目代号：Nova\n- 我不叫Leo,我叫习惯安静\n",
        )
        .expect("seed memory");

        let first = store
            .remember_fact("用户姓名：习惯安静")
            .expect("write fact");
        assert!(first, "first write should persist");

        let second = store.remember_fact("用户姓名：习惯安静").expect("dedupe");
        assert!(!second, "duplicate fact should not be persisted twice");

        let content = store.read_long_term();
        assert!(
            content.contains("- 用户姓名：习惯安静"),
            "memory file should contain updated name"
        );
        assert_eq!(content.matches("用户姓名：习惯安静").count(), 1);
        assert_eq!(content.matches("用户姓名：Leo").count(), 0);
        assert_eq!(content.matches("我不叫Leo,我叫习惯安静").count(), 0);

        let _ = std::fs::remove_dir_all(workspace);
    }
}
