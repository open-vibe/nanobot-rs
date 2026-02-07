use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub const DEFAULT_HEARTBEAT_INTERVAL_S: u64 = 30 * 60;
pub const HEARTBEAT_PROMPT: &str = "Read HEARTBEAT.md in your workspace (if it exists).\nFollow any instructions or tasks listed there.\nIf nothing needs attention, reply with just: HEARTBEAT_OK";
pub const HEARTBEAT_OK_TOKEN: &str = "HEARTBEAT_OK";

pub type HeartbeatCallback = Arc<dyn Fn(String) -> BoxFuture<'static, String> + Send + Sync>;

pub fn is_heartbeat_empty(content: Option<&str>) -> bool {
    let Some(content) = content else {
        return true;
    };
    let skip_patterns = ["- [ ]", "* [ ]", "- [x]", "* [x]"];
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("<!--")
            || skip_patterns.contains(&line)
        {
            continue;
        }
        return false;
    }
    true
}

pub struct HeartbeatService {
    workspace: std::path::PathBuf,
    on_heartbeat: Arc<Mutex<Option<HeartbeatCallback>>>,
    interval_s: u64,
    enabled: bool,
    running: Arc<AtomicBool>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl HeartbeatService {
    pub fn new(workspace: std::path::PathBuf, interval_s: u64, enabled: bool) -> Self {
        Self {
            workspace,
            on_heartbeat: Arc::new(Mutex::new(None)),
            interval_s,
            enabled,
            running: Arc::new(AtomicBool::new(false)),
            task: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_on_heartbeat(&self, callback: HeartbeatCallback) {
        let mut guard = self.on_heartbeat.lock().await;
        *guard = Some(callback);
    }

    pub fn heartbeat_file(&self) -> std::path::PathBuf {
        self.workspace.join("HEARTBEAT.md")
    }

    pub async fn start(&self) {
        if !self.enabled {
            return;
        }
        self.running.store(true, Ordering::Relaxed);
        let running = self.running.clone();
        let heartbeat_file = self.heartbeat_file();
        let on_heartbeat = self.on_heartbeat.clone();
        let interval_s = self.interval_s;

        let handle = tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_secs(interval_s)).await;
                if !running.load(Ordering::Relaxed) {
                    break;
                }

                let content = tokio::fs::read_to_string(&heartbeat_file).await.ok();
                if is_heartbeat_empty(content.as_deref()) {
                    continue;
                }

                let callback = on_heartbeat.lock().await.clone();
                if let Some(callback) = callback {
                    let response = callback(HEARTBEAT_PROMPT.to_string()).await;
                    let normalized = response.to_uppercase().replace('_', "");
                    let ok = HEARTBEAT_OK_TOKEN.to_uppercase().replace('_', "");
                    if normalized.contains(&ok) {
                        // no-op
                    }
                }
            }
        });

        let mut slot = self.task.lock().await;
        *slot = Some(handle);
    }

    pub async fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.task.lock().await.take() {
            handle.abort();
        }
    }

    pub async fn trigger_now(&self) -> Option<String> {
        let callback = self.on_heartbeat.lock().await.clone();
        match callback {
            Some(cb) => Some(cb(HEARTBEAT_PROMPT.to_string()).await),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_empty_detection_works() {
        assert!(is_heartbeat_empty(None));
        assert!(is_heartbeat_empty(Some("# Header\n\n- [ ]\n<!-- note -->")));
        assert!(!is_heartbeat_empty(Some("# Header\n- [ ]\nCall mom")));
    }

    #[tokio::test]
    async fn trigger_now_invokes_callback() {
        let service = HeartbeatService::new(std::path::PathBuf::from("."), 60, true);
        service
            .set_on_heartbeat(Arc::new(|prompt| {
                Box::pin(async move { format!("received:{prompt}") })
            }))
            .await;
        let result = service.trigger_now().await;
        assert!(result.is_some());
        let text = result.unwrap_or_default();
        assert!(text.contains("received:"));
        assert!(text.contains("HEARTBEAT_OK"));
    }
}
