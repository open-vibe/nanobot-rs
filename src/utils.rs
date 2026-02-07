use chrono::Local;
use std::path::{Path, PathBuf};

pub fn ensure_dir(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(path)?;
    Ok(path.to_path_buf())
}

pub fn get_data_path() -> std::io::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| std::io::Error::other("cannot resolve home directory"))?;
    ensure_dir(&home.join(".nanobot"))
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}

pub fn get_workspace_path(workspace: Option<&str>) -> std::io::Result<PathBuf> {
    let path = match workspace {
        Some(p) => expand_tilde(p),
        None => {
            let home = dirs::home_dir()
                .ok_or_else(|| std::io::Error::other("cannot resolve home directory"))?;
            home.join(".nanobot").join("workspace")
        }
    };
    ensure_dir(&path)
}

pub fn today_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn timestamp() -> String {
    Local::now().to_rfc3339()
}

pub fn safe_filename(name: &str) -> String {
    let mut out = name.to_string();
    for ch in ['<', '>', ':', '"', '/', '\\', '|', '?', '*'] {
        out = out.replace(ch, "_");
    }
    out.trim().to_string()
}

pub fn parse_session_key(key: &str) -> anyhow::Result<(&str, &str)> {
    let (channel, chat_id) = key
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid session key: {key}"))?;
    Ok((channel, chat_id))
}
