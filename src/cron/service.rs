use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule, CronStore};
use anyhow::Result;
use chrono::{TimeZone, Utc};
use cron::Schedule;
use futures_util::future::BoxFuture;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

pub type CronJobCallback =
    Arc<dyn Fn(CronJob) -> BoxFuture<'static, Result<Option<String>>> + Send + Sync>;

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn compute_next_run(schedule: &CronSchedule, now_ms: i64) -> Option<i64> {
    match schedule.kind.as_str() {
        "at" => schedule.at_ms.filter(|at| *at > now_ms),
        "every" => {
            let interval = schedule.every_ms?;
            if interval <= 0 {
                None
            } else {
                Some(now_ms + interval)
            }
        }
        "cron" => {
            let expr = schedule.expr.as_ref()?;
            let parsed = Schedule::from_str(expr).ok()?;
            let now = Utc.timestamp_millis_opt(now_ms).single()?;
            parsed.after(&now).next().map(|dt| dt.timestamp_millis())
        }
        _ => None,
    }
}

pub struct CronService {
    store_path: std::path::PathBuf,
    on_job: Arc<Mutex<Option<CronJobCallback>>>,
    store: Arc<Mutex<CronStore>>,
    running: Arc<AtomicBool>,
    runner: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl CronService {
    pub fn new(store_path: std::path::PathBuf) -> Self {
        Self {
            store_path,
            on_job: Arc::new(Mutex::new(None)),
            store: Arc::new(Mutex::new(CronStore::default())),
            running: Arc::new(AtomicBool::new(false)),
            runner: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_on_job(&self, callback: CronJobCallback) {
        let mut guard = self.on_job.lock().await;
        *guard = Some(callback);
    }

    pub async fn start(&self) -> Result<()> {
        self.running.store(true, Ordering::Relaxed);
        self.load_store().await?;
        self.recompute_next_runs().await;
        self.save_store().await?;

        let running = self.running.clone();
        let store = self.store.clone();
        let on_job = self.on_job.clone();
        let store_path = self.store_path.clone();
        let runner = tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                let mut due_jobs = Vec::new();
                {
                    let snapshot = store.lock().await;
                    let now = now_ms();
                    for job in snapshot.jobs.iter().filter(|job| {
                        job.enabled
                            && job.state.next_run_at_ms.is_some()
                            && now >= job.state.next_run_at_ms.unwrap_or(i64::MAX)
                    }) {
                        due_jobs.push(job.id.clone());
                    }
                }

                for id in due_jobs {
                    let mut job_to_run = None;
                    {
                        let mut data = store.lock().await;
                        if let Some(job) = data.jobs.iter_mut().find(|j| j.id == id) {
                            job_to_run = Some(job.clone());
                            job.state.last_run_at_ms = Some(now_ms());
                        }
                    }

                    if let Some(job) = job_to_run {
                        let callback = on_job.lock().await.clone();
                        let result = if let Some(callback) = callback {
                            let fut = callback(job.clone());
                            fut.await
                        } else {
                            Ok(None)
                        };
                        let mut data = store.lock().await;
                        if let Some(target) = data.jobs.iter_mut().find(|j| j.id == job.id) {
                            if let Err(err) = &result {
                                target.state.last_status = Some("error".to_string());
                                target.state.last_error = Some(err.to_string());
                            } else {
                                target.state.last_status = Some("ok".to_string());
                                target.state.last_error = None;
                            }
                            target.updated_at_ms = now_ms();

                            if job.schedule.kind == "at" {
                                if job.delete_after_run {
                                    let remove_id = target.id.clone();
                                    data.jobs.retain(|j| j.id != remove_id);
                                } else {
                                    target.enabled = false;
                                    target.state.next_run_at_ms = None;
                                }
                            } else {
                                target.state.next_run_at_ms =
                                    compute_next_run(&target.schedule, now_ms());
                            }
                            let _ = result;
                        }
                    }
                }

                let _ = save_store_static(&store_path, &store).await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });

        let mut runner_slot = self.runner.lock().await;
        *runner_slot = Some(runner);
        Ok(())
    }

    pub async fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.runner.lock().await.take() {
            handle.abort();
        }
    }

    async fn recompute_next_runs(&self) {
        let mut store = self.store.lock().await;
        let now = now_ms();
        for job in &mut store.jobs {
            if job.enabled {
                job.state.next_run_at_ms = compute_next_run(&job.schedule, now);
            }
        }
    }

    async fn load_store(&self) -> Result<()> {
        if !self.store_path.exists() {
            *self.store.lock().await = CronStore::default();
            return Ok(());
        }

        let raw = tokio::fs::read_to_string(&self.store_path).await?;
        let store: CronStore = serde_json::from_str(&raw).unwrap_or_default();
        *self.store.lock().await = store;
        Ok(())
    }

    async fn save_store(&self) -> Result<()> {
        save_store_static(&self.store_path, &self.store).await
    }

    pub async fn list_jobs(&self, include_disabled: bool) -> Vec<CronJob> {
        let store = self.store.lock().await;
        let mut jobs = if include_disabled {
            store.jobs.clone()
        } else {
            store.jobs.iter().filter(|j| j.enabled).cloned().collect()
        };
        jobs.sort_by_key(|j| j.state.next_run_at_ms.unwrap_or(i64::MAX));
        jobs
    }

    pub async fn add_job(
        &self,
        name: String,
        schedule: CronSchedule,
        message: String,
        deliver: bool,
        channel: Option<String>,
        to: Option<String>,
        delete_after_run: bool,
    ) -> Result<CronJob> {
        let now = now_ms();
        let job = CronJob {
            id: Uuid::new_v4().simple().to_string()[..8].to_string(),
            name,
            enabled: true,
            schedule: schedule.clone(),
            payload: CronPayload {
                kind: "agent_turn".to_string(),
                message,
                deliver,
                channel,
                to,
            },
            state: CronJobState {
                next_run_at_ms: compute_next_run(&schedule, now),
                ..Default::default()
            },
            created_at_ms: now,
            updated_at_ms: now,
            delete_after_run,
        };

        {
            let mut store = self.store.lock().await;
            store.jobs.push(job.clone());
        }
        self.save_store().await?;
        self.recompute_next_runs().await;
        self.save_store().await?;
        Ok(job)
    }

    pub async fn remove_job(&self, job_id: &str) -> Result<bool> {
        let mut store = self.store.lock().await;
        let before = store.jobs.len();
        store.jobs.retain(|j| j.id != job_id);
        let removed = store.jobs.len() < before;
        drop(store);
        if removed {
            self.save_store().await?;
        }
        Ok(removed)
    }

    pub async fn enable_job(&self, job_id: &str, enabled: bool) -> Result<Option<CronJob>> {
        let mut store = self.store.lock().await;
        if let Some(job) = store.jobs.iter_mut().find(|j| j.id == job_id) {
            job.enabled = enabled;
            job.updated_at_ms = now_ms();
            if enabled {
                job.state.next_run_at_ms = compute_next_run(&job.schedule, now_ms());
            } else {
                job.state.next_run_at_ms = None;
            }
            let out = job.clone();
            drop(store);
            self.save_store().await?;
            return Ok(Some(out));
        }
        Ok(None)
    }

    pub async fn run_job(&self, job_id: &str, force: bool) -> Result<bool> {
        let job_opt = {
            let store = self.store.lock().await;
            store.jobs.iter().find(|j| j.id == job_id).cloned()
        };
        let Some(job) = job_opt else {
            return Ok(false);
        };
        if !force && !job.enabled {
            return Ok(false);
        }

        let callback = self.on_job.lock().await.clone();
        let result = if let Some(callback) = callback {
            callback(job.clone()).await
        } else {
            Ok(None)
        };
        let mut store = self.store.lock().await;
        if let Some(target) = store.jobs.iter_mut().find(|j| j.id == job_id) {
            if let Err(err) = &result {
                target.state.last_status = Some("error".to_string());
                target.state.last_error = Some(err.to_string());
            } else {
                target.state.last_status = Some("ok".to_string());
                target.state.last_error = None;
            }
            target.state.last_run_at_ms = Some(now_ms());
            target.updated_at_ms = now_ms();
            target.state.next_run_at_ms = compute_next_run(&target.schedule, now_ms());
            if target.schedule.kind == "at" && target.delete_after_run {
                let remove_id = target.id.clone();
                store.jobs.retain(|j| j.id != remove_id);
            }
        }
        drop(store);
        self.save_store().await?;
        Ok(true)
    }

    pub async fn status(&self) -> serde_json::Value {
        let store = self.store.lock().await;
        let next_wake = store
            .jobs
            .iter()
            .filter(|j| j.enabled)
            .filter_map(|j| j.state.next_run_at_ms)
            .min();
        serde_json::json!({
            "enabled": self.running.load(Ordering::Relaxed),
            "jobs": store.jobs.len(),
            "next_wake_at_ms": next_wake,
        })
    }
}

async fn save_store_static(path: &std::path::Path, store: &Arc<Mutex<CronStore>>) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let text = {
        let data = store.lock().await;
        serde_json::to_string_pretty(&*data)?
    };
    tokio::fs::write(path, text).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn temp_store_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nanobot-rs-cron-{}.json", Uuid::new_v4()))
    }

    #[test]
    fn compute_next_run_for_every_and_at() {
        let now = now_ms();
        let every = CronSchedule {
            kind: "every".to_string(),
            every_ms: Some(5_000),
            ..Default::default()
        };
        let at = CronSchedule {
            kind: "at".to_string(),
            at_ms: Some(now - 1),
            ..Default::default()
        };

        assert_eq!(compute_next_run(&every, now), Some(now + 5_000));
        assert_eq!(compute_next_run(&at, now), None);
    }

    #[tokio::test]
    async fn cron_service_add_run_and_remove_job() -> Result<()> {
        let store_path = temp_store_path();
        let service = CronService::new(store_path.clone());
        service
            .set_on_job(Arc::new(|_| Box::pin(async { Ok(Some("ok".to_string())) })))
            .await;
        service.start().await?;

        let schedule = CronSchedule {
            kind: "every".to_string(),
            every_ms: Some(10_000),
            ..Default::default()
        };
        let job = service
            .add_job(
                "test".to_string(),
                schedule,
                "ping".to_string(),
                false,
                None,
                None,
                false,
            )
            .await?;
        assert!(!job.id.is_empty());

        let listed = service.list_jobs(true).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "test");

        let disabled = service.enable_job(&job.id, false).await?;
        assert!(disabled.is_some());
        assert!(!service.run_job(&job.id, false).await?);
        assert!(service.run_job(&job.id, true).await?);

        assert!(service.remove_job(&job.id).await?);
        assert!(service.list_jobs(true).await.is_empty());

        service.stop().await;
        let _ = std::fs::remove_file(store_path);
        Ok(())
    }

    #[tokio::test]
    async fn callback_error_sets_last_error() -> Result<()> {
        let store_path = temp_store_path();
        let service = CronService::new(store_path.clone());
        service
            .set_on_job(Arc::new(|_| {
                Box::pin(async { Err(anyhow::anyhow!("callback failed")) })
            }))
            .await;
        service.start().await?;

        let schedule = CronSchedule {
            kind: "every".to_string(),
            every_ms: Some(10_000),
            ..Default::default()
        };
        let job = service
            .add_job(
                "failing".to_string(),
                schedule,
                "ping".to_string(),
                false,
                None,
                None,
                false,
            )
            .await?;

        assert!(service.run_job(&job.id, true).await?);
        let jobs = service.list_jobs(true).await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state.last_status.as_deref(), Some("error"));
        assert!(
            jobs[0]
                .state
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("callback failed")
        );

        service.stop().await;
        let _ = std::fs::remove_file(store_path);
        Ok(())
    }

    #[tokio::test]
    async fn at_job_with_delete_after_run_is_removed() -> Result<()> {
        let store_path = temp_store_path();
        let service = CronService::new(store_path.clone());
        service.start().await?;

        let schedule = CronSchedule {
            kind: "at".to_string(),
            at_ms: Some(now_ms() + 60_000),
            ..Default::default()
        };
        let job = service
            .add_job(
                "oneshot".to_string(),
                schedule,
                "ping".to_string(),
                false,
                None,
                None,
                true,
            )
            .await?;

        assert!(service.run_job(&job.id, true).await?);
        assert!(service.list_jobs(true).await.is_empty());

        service.stop().await;
        let _ = std::fs::remove_file(store_path);
        Ok(())
    }
}
