use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub path: PathBuf,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct SkillsLoader {
    workspace_skills: PathBuf,
    builtin_skills: PathBuf,
}

impl SkillsLoader {
    pub fn new(workspace: PathBuf, builtin_skills_dir: Option<PathBuf>) -> Self {
        let builtin_skills = builtin_skills_dir
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills"));
        Self {
            workspace_skills: workspace.join("skills"),
            builtin_skills,
        }
    }

    pub fn list_skills(&self, filter_unavailable: bool) -> Vec<SkillInfo> {
        let mut skills = Vec::new();
        let mut seen = BTreeSet::new();

        self.collect_skills_from_dir(&self.workspace_skills, "workspace", &mut seen, &mut skills);
        self.collect_skills_from_dir(&self.builtin_skills, "builtin", &mut seen, &mut skills);

        if filter_unavailable {
            skills
                .into_iter()
                .filter(|skill| {
                    let meta = self.get_skill_metadata(&skill.name).unwrap_or_default();
                    let parsed = parse_nanobot_metadata(
                        meta.get("metadata").map(String::as_str).unwrap_or_default(),
                    );
                    self.check_requirements(&parsed)
                })
                .collect()
        } else {
            skills
        }
    }

    fn collect_skills_from_dir(
        &self,
        dir: &Path,
        source: &str,
        seen: &mut BTreeSet<String>,
        out: &mut Vec<SkillInfo>,
    ) {
        if !dir.exists() || !dir.is_dir() {
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if seen.contains(&name) {
                continue;
            }
            let skill_file = path.join("SKILL.md");
            if !skill_file.exists() {
                continue;
            }
            seen.insert(name.clone());
            out.push(SkillInfo {
                name,
                path: skill_file,
                source: source.to_string(),
            });
        }
    }

    pub fn load_skill(&self, name: &str) -> Option<String> {
        let workspace = self.workspace_skills.join(name).join("SKILL.md");
        if workspace.exists() {
            return std::fs::read_to_string(workspace).ok();
        }
        let builtin = self.builtin_skills.join(name).join("SKILL.md");
        if builtin.exists() {
            return std::fs::read_to_string(builtin).ok();
        }
        None
    }

    pub fn load_skills_for_context(&self, skill_names: &[String]) -> String {
        let mut parts = Vec::new();
        for name in skill_names {
            if let Some(content) = self.load_skill(name) {
                let content = strip_frontmatter(&content);
                parts.push(format!("### Skill: {name}\n\n{content}"));
            }
        }
        parts.join("\n\n---\n\n")
    }

    pub fn build_skills_summary(&self) -> String {
        let skills = self.list_skills(false);
        if skills.is_empty() {
            return String::new();
        }
        let mut lines = vec!["<skills>".to_string()];

        for skill in skills {
            let meta = self.get_skill_metadata(&skill.name).unwrap_or_default();
            let desc = meta
                .get("description")
                .cloned()
                .unwrap_or_else(|| skill.name.clone());
            let skill_meta = parse_nanobot_metadata(
                meta.get("metadata").map(String::as_str).unwrap_or_default(),
            );
            let available = self.check_requirements(&skill_meta);

            lines.push(format!(
                "  <skill available=\"{}\">",
                if available { "true" } else { "false" }
            ));
            lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
            lines.push(format!(
                "    <description>{}</description>",
                escape_xml(&desc)
            ));
            lines.push(format!(
                "    <location>{}</location>",
                escape_xml(&skill.path.display().to_string())
            ));
            if !available {
                let missing = self.get_missing_requirements(&skill_meta);
                if !missing.is_empty() {
                    lines.push(format!("    <requires>{}</requires>", escape_xml(&missing)));
                }
            }
            lines.push("  </skill>".to_string());
        }
        lines.push("</skills>".to_string());
        lines.join("\n")
    }

    pub fn get_always_skills(&self) -> Vec<String> {
        self.list_skills(true)
            .into_iter()
            .filter_map(|skill| {
                let meta = self.get_skill_metadata(&skill.name).unwrap_or_default();
                let parsed = parse_nanobot_metadata(
                    meta.get("metadata").map(String::as_str).unwrap_or_default(),
                );
                let always_in_meta = meta
                    .get("always")
                    .map(|v| v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if always_in_meta
                    || parsed
                        .get("always")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                {
                    Some(skill.name)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn get_skill_metadata(
        &self,
        name: &str,
    ) -> Option<std::collections::HashMap<String, String>> {
        let content = self.load_skill(name)?;
        parse_frontmatter(&content)
    }

    fn check_requirements(&self, meta: &Value) -> bool {
        let requires = meta.get("requires").and_then(Value::as_object);
        let Some(requires) = requires else {
            return true;
        };

        if let Some(bins) = requires.get("bins").and_then(Value::as_array) {
            for bin in bins {
                let Some(bin) = bin.as_str() else {
                    continue;
                };
                if which::which(bin).is_err() {
                    return false;
                }
            }
        }
        if let Some(env_vars) = requires.get("env").and_then(Value::as_array) {
            for env in env_vars {
                let Some(key) = env.as_str() else {
                    continue;
                };
                if std::env::var(key).unwrap_or_default().is_empty() {
                    return false;
                }
            }
        }
        true
    }

    fn get_missing_requirements(&self, meta: &Value) -> String {
        let requires = meta.get("requires").and_then(Value::as_object);
        let Some(requires) = requires else {
            return String::new();
        };
        let mut missing = Vec::new();

        if let Some(bins) = requires.get("bins").and_then(Value::as_array) {
            for bin in bins.iter().filter_map(Value::as_str) {
                if which::which(bin).is_err() {
                    missing.push(format!("CLI: {bin}"));
                }
            }
        }
        if let Some(env_vars) = requires.get("env").and_then(Value::as_array) {
            for env in env_vars.iter().filter_map(Value::as_str) {
                if std::env::var(env).unwrap_or_default().is_empty() {
                    missing.push(format!("ENV: {env}"));
                }
            }
        }
        missing.join(", ")
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn parse_frontmatter(content: &str) -> Option<std::collections::HashMap<String, String>> {
    if !content.starts_with("---") {
        return None;
    }
    let mut lines = content.lines();
    let first = lines.next()?;
    if first.trim() != "---" {
        return None;
    }
    let mut out = std::collections::HashMap::new();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            out.insert(
                key.trim().to_string(),
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    Some(out)
}

fn strip_frontmatter(content: &str) -> String {
    if !content.starts_with("---") {
        return content.to_string();
    }
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return content.to_string();
    }
    let mut in_frontmatter = true;
    let mut out = Vec::new();
    for line in lines {
        if in_frontmatter {
            if line.trim() == "---" {
                in_frontmatter = false;
            }
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

fn parse_nanobot_metadata(raw: &str) -> Value {
    if raw.is_empty() {
        return Value::Object(Default::default());
    }
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| v.get("nanobot").cloned().or(Some(v)))
        .unwrap_or_else(|| Value::Object(Default::default()))
}
