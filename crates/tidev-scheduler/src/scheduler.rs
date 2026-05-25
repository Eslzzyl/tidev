use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use tokio::process::Command;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{self, Duration};
use uuid::Uuid;

use crate::delivery::DeliveryBus;
use crate::schedule::next_run_for_schedule;
use crate::store::CronStore;
use crate::types::{CronDeliveryMessage, CronJob, JobResult, JobType, Schedule};

use tidev_engine::agent::runtime::{AgentLoopConfig, AgentRuntime};
use tidev_engine::config::ActiveModel;
use tidev_session::session::{Message, MessageRole};
use tidev_types::prompts::SessionMode;

const SHELL_JOB_TIMEOUT_SECS: u64 = 120;
const MIN_POLL_SECONDS: u64 = 5;

/// Configuration for the scheduler run loop.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub poll_secs: u64,
    pub max_tasks: usize,
    pub max_concurrent: usize,
    pub max_run_history: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            poll_secs: 15,
            max_tasks: 10,
            max_concurrent: 3,
            max_run_history: 100,
        }
    }
}

/// The main task scheduler.
///
/// Polls the database for due jobs, executes them (shell or agent), persists
/// results, and optionally sends delivery messages through the [`DeliveryBus`].
pub struct Scheduler {
    store: Arc<CronStore>,
    config: SchedulerConfig,
    delivery_bus: Option<DeliveryBus>,
    /// Cloned for each agent job execution.
    agent_runtime: Option<AgentRuntime>,
    active_model: ActiveModel,
    workspace_root: std::path::PathBuf,
}

impl Scheduler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: CronStore,
        config: SchedulerConfig,
        delivery_bus: Option<DeliveryBus>,
        agent_runtime: Option<AgentRuntime>,
        active_model: ActiveModel,
        workspace_root: std::path::PathBuf,
    ) -> Self {
        Self {
            store: Arc::new(store),
            config,
            delivery_bus,
            agent_runtime,
            active_model,
            workspace_root,
        }
    }

    /// Run the scheduler loop forever.
    ///
    /// This function should be spawned as a local task:
    /// ```ignore
    /// tokio::task::spawn_local(async move { scheduler.run().await });
    /// ```
    pub async fn run(&self) -> Result<()> {
        let poll_secs = self.config.poll_secs.max(MIN_POLL_SECONDS);
        let mut interval = time::interval(Duration::from_secs(poll_secs));
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        log::info!(
            "Scheduler started: poll={poll_secs}s, max_tasks={}, max_concurrent={}",
            self.config.max_tasks,
            self.config.max_concurrent,
        );

        loop {
            interval.tick().await;

            let now = Utc::now();
            let due_jobs = match self.store.due_jobs(now) {
                Ok(jobs) => jobs,
                Err(e) => {
                    log::error!("Scheduler: failed to query due jobs: {e}");
                    continue;
                }
            };

            if due_jobs.is_empty() {
                continue;
            }

            log::info!(
                "Scheduler: {} job(s) due, processing up to {} concurrently",
                due_jobs.len(),
                self.config.max_concurrent,
            );

            self.process_jobs(due_jobs).await;
        }
    }

    async fn process_jobs(&self, jobs: Vec<CronJob>) {
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent));
        let mut handles = Vec::new();

        for job in jobs {
            let permit = match Arc::clone(&semaphore).acquire_owned().await {
                Ok(p) => p,
                Err(_) => break, // Semaphore closed
            };

            let store = Arc::clone(&self.store);
            let delivery_bus = self.delivery_bus.clone();
            let agent_runtime = self.agent_runtime.clone();
            let active_model = self.active_model.clone();
            let workspace_root = self.workspace_root.clone();

            let handle = tokio::task::spawn_local(async move {
                execute_and_deliver(
                    job,
                    store,
                    delivery_bus,
                    agent_runtime,
                    active_model,
                    workspace_root,
                    permit,
                )
                .await;
            });
            handles.push(handle);
        }

        // Wait for all spawned jobs to complete.
        for handle in handles {
            let _ = handle.await;
        }
    }
}

/// Execute a single job, persist the result, and optionally deliver it.
async fn execute_and_deliver(
    job: CronJob,
    store: Arc<CronStore>,
    delivery_bus: Option<DeliveryBus>,
    agent_runtime: Option<AgentRuntime>,
    active_model: ActiveModel,
    workspace_root: std::path::PathBuf,
    _permit: OwnedSemaphorePermit,
) {
    let job_name = job.name.as_deref().unwrap_or(&job.id);
    log::info!("Executing cron job '{job_name}' (type={:?})", job.job_type);

    let result = match job.job_type {
        JobType::Shell => execute_shell_job(&job).await,
        JobType::Agent => match &agent_runtime {
            Some(rt) => execute_agent_job(&job, rt, &active_model, &workspace_root).await,
            None => {
                log::error!("Cannot execute agent job '{job_name}': no AgentRuntime configured");
                JobResult {
                    success: false,
                    output: "AgentRuntime not available".to_string(),
                    started_at: Utc::now(),
                    finished_at: Utc::now(),
                    duration_ms: 0,
                }
            }
        },
    };

    // Persist the run.
    let run = match store.record_run(&job, &result) {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to record run for job '{job_name}': {e}");
            return;
        }
    };

    // Reschedule or disable.
    let next = match &job.schedule {
        Schedule::At { .. } => {
            // One-shot job: disable after execution.
            let _ = store.reschedule_after_run(&job, None);
            None
        }
        _ => match next_run_for_schedule(&job.schedule, Utc::now()) {
            Ok(next) => {
                let _ = store.reschedule_after_run(&job, Some(next));
                Some(next)
            }
            Err(e) => {
                log::warn!("Failed to compute next run for job '{job_name}': {e}");
                let _ = store.reschedule_after_run(&job, None);
                None
            }
        },
    };

    log::info!(
        "Cron job '{job_name}' completed: success={}, next_run={:?}",
        result.success,
        next,
    );

    // Deliver if configured.
    if let Some(ref bus) = delivery_bus
        && job.delivery.mode == "announce"
    {
        let msg = CronDeliveryMessage {
            job_id: job.id.clone(),
            job_name: job_name.to_string(),
            output: result.output.clone(),
            delivery: job.delivery.clone(),
            success: result.success,
            executed_at: Utc::now(),
        };
        bus.send(msg);

        // Mark the run as delivered in the database.
        let _ = store.mark_delivered(&run.id);
    }
}

/// Execute a shell command with a timeout.
async fn execute_shell_job(job: &CronJob) -> JobResult {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    let output = tokio::time::timeout(
        Duration::from_secs(SHELL_JOB_TIMEOUT_SECS),
        Command::new("sh").arg("-c").arg(&job.command).output(),
    )
    .await;

    let finished_at = Utc::now();
    let duration_ms = start_instant.elapsed().as_millis() as i64;

    match output {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else {
                format!("{stdout}\n{stderr}")
            };

            if output.status.success() {
                JobResult {
                    success: true,
                    output: truncate_output(&combined),
                    started_at,
                    finished_at,
                    duration_ms,
                }
            } else {
                JobResult {
                    success: false,
                    output: truncate_output(&format!(
                        "Exit code: {}\n{}",
                        output.status.code().unwrap_or(-1),
                        combined
                    )),
                    started_at,
                    finished_at,
                    duration_ms,
                }
            }
        }
        Ok(Err(e)) => JobResult {
            success: false,
            output: format!("Failed to spawn command: {e}"),
            started_at,
            finished_at,
            duration_ms,
        },
        Err(_) => JobResult {
            success: false,
            output: format!("Command timed out after {SHELL_JOB_TIMEOUT_SECS}s"),
            started_at,
            finished_at,
            duration_ms,
        },
    }
}

/// Execute an agent job — creates an isolated session, runs the prompt through
/// the LLM + tool loop, and returns the final assistant output.
async fn execute_agent_job(
    job: &CronJob,
    runtime: &AgentRuntime,
    active_model: &ActiveModel,
    workspace_root: &std::path::Path,
) -> JobResult {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    let prompt = job.prompt.as_deref().unwrap_or("");
    let session_id = Uuid::new_v4();
    let title = format!("Cron: {}", job.name.as_deref().unwrap_or(&job.id));

    // 1. Lock the shared session store and create a new session.
    {
        let store = runtime.store.lock().await;
        if let Err(e) = store.create_session(
            session_id,
            workspace_root,
            &active_model.provider_id,
            &active_model.provider_display_name,
            &active_model.model_id,
            &active_model.display_name,
            &title,
        ) {
            let finished_at = Utc::now();
            let duration_ms = start_instant.elapsed().as_millis() as i64;
            return JobResult {
                success: false,
                output: format!("Failed to create agent session: {e}"),
                started_at,
                finished_at,
                duration_ms,
            };
        }
    }

    // 2. Compose and persist the system prompt.
    {
        let store = runtime.store.lock().await;
        let static_prompt = runtime.compose_static_system_prompt(&active_model.system_prompt);
        if let Err(e) = store.update_session_system_prompt(session_id, &static_prompt) {
            log::warn!("Failed to persist static system prompt for cron job: {e}");
        }
    }

    // 3. Add the user message (the job prompt) to the session.
    {
        let store = runtime.store.lock().await;
        let user_msg = Message::new(MessageRole::User, prompt.to_string());
        if let Err(e) = store.append_message(session_id, &user_msg) {
            log::warn!("Failed to append user message for cron job: {e}");
        }
    }

    // 4. Clone the runtime and set the active model on the tools.
    let mut agent = runtime.clone();
    agent.tools.set_active_model(active_model.clone());

    // 5. Create a fresh context manager.
    let mut context_manager = tidev_engine::context::ContextManager::new();

    // 6. Run the agent loop (discard streaming events, we read the final
    //    result from the session store afterwards).
    let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();

    let loop_result = agent
        .run_agent_loop(AgentLoopConfig {
            session_id,
            model: active_model.clone(),
            context_manager: &mut context_manager,
            mode: SessionMode::Build,
            thinking_level: active_model.thinking_level.clone(),
            event_tx,
            cancel_token: None,
        })
        .await;

    let finished_at = Utc::now();
    let duration_ms = start_instant.elapsed().as_millis() as i64;

    match loop_result {
        Ok(()) => {
            // Read the last assistant message from the session store.
            let store = runtime.store.lock().await;
            let output = match store.load_messages(session_id) {
                Ok(messages) => messages
                    .into_iter()
                    .rev()
                    .find(|m| m.role == MessageRole::Assistant)
                    .map(|m| m.content)
                    .unwrap_or_default(),
                Err(e) => {
                    log::warn!("Failed to load messages for cron agent job: {e}");
                    String::new()
                }
            };
            drop(store);

            JobResult {
                success: true,
                output,
                started_at,
                finished_at,
                duration_ms,
            }
        }
        Err(e) => JobResult {
            success: false,
            output: format!("Agent execution failed: {e}"),
            started_at,
            finished_at,
            duration_ms,
        },
    }
}

fn truncate_output(output: &str) -> String {
    const MAX_BYTES: usize = 16 * 1024;
    if output.len() > MAX_BYTES {
        format!(
            "{}...[truncated {} bytes]",
            &output[..MAX_BYTES],
            output.len() - MAX_BYTES
        )
    } else {
        output.to_string()
    }
}
