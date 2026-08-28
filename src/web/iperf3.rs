use std::{sync::Arc, time::SystemTime};

use keenetic_rci::{KeeneticClient, NetworkTestSession};
use serde::Serialize;
use tokio::sync::{Mutex, watch};

use crate::{config::Iperf3Config, sample::unix_time_ms};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StartOutcome {
    Started,
    AlreadyRunning,
}

enum Run {
    Idle,
    Running {
        started_at_ms: u64,
    },
    Finished {
        started_at_ms: u64,
        finished_at_ms: u64,
        outcome: Outcome,
    },
}

enum Outcome {
    Completed(Arc<[String]>),
    Failed(Arc<str>),
    Cancelled,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Idle,
    Running,
    Finished,
    Failed,
    Cancelled,
}

pub(super) struct Controller {
    client: KeeneticClient,
    config: Iperf3Config,
    shutdown: watch::Receiver<bool>,
    run: Mutex<Run>,
}

impl Controller {
    pub(super) fn new(
        client: KeeneticClient,
        config: Iperf3Config,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            client,
            config,
            shutdown,
            run: Mutex::new(Run::Idle),
        }
    }

    pub(super) fn summary(&self) -> &str {
        &self.config.summary
    }

    pub(super) async fn start(self: &Arc<Self>) -> StartOutcome {
        let started_at_ms = unix_time_ms(SystemTime::now());
        {
            let mut run = self.run.lock().await;
            if matches!(*run, Run::Running { .. }) {
                return StartOutcome::AlreadyRunning;
            }
            *run = Run::Running { started_at_ms };
        }

        let controller = Arc::clone(self);
        let task = tokio::spawn(async move {
            tracing::info!(configuration = %controller.config.summary, "iPerf3 test started");
            let outcome = controller.execute().await;
            let finished_at_ms = unix_time_ms(SystemTime::now());
            match &outcome {
                Outcome::Completed(_) => tracing::info!("iPerf3 test finished"),
                Outcome::Failed(error) => tracing::error!(%error, "iPerf3 test failed"),
                Outcome::Cancelled => tracing::info!("iPerf3 test cancelled during shutdown"),
            }
            *controller.run.lock().await = Run::Finished {
                started_at_ms,
                finished_at_ms,
                outcome,
            };
        });
        drop(task);
        StartOutcome::Started
    }

    async fn execute(&self) -> Outcome {
        let shutdown = self.shutdown.clone();
        if *shutdown.borrow() {
            return Outcome::Cancelled;
        }
        let session = match self
            .client
            .start_network_test(self.config.request.clone())
            .await
        {
            Ok(session) => session,
            Err(error) => return Outcome::Failed(error.to_string().into()),
        };
        collect_until_shutdown(session, shutdown).await
    }

    pub(super) async fn snapshot(&self) -> Snapshot {
        let run = self.run.lock().await;
        let (status, started_at_ms, finished_at_ms, output, error) = match &*run {
            Run::Idle => (Status::Idle, None, None, None, None),
            Run::Running { started_at_ms } => {
                (Status::Running, Some(*started_at_ms), None, None, None)
            }
            Run::Finished {
                started_at_ms,
                finished_at_ms,
                outcome: Outcome::Completed(output),
            } => (
                Status::Finished,
                Some(*started_at_ms),
                Some(*finished_at_ms),
                Some(Arc::clone(output)),
                None,
            ),
            Run::Finished {
                started_at_ms,
                finished_at_ms,
                outcome: Outcome::Failed(error),
            } => (
                Status::Failed,
                Some(*started_at_ms),
                Some(*finished_at_ms),
                None,
                Some(Arc::clone(error)),
            ),
            Run::Finished {
                started_at_ms,
                finished_at_ms,
                outcome: Outcome::Cancelled,
            } => (
                Status::Cancelled,
                Some(*started_at_ms),
                Some(*finished_at_ms),
                None,
                Some(Arc::from("iPerf3 test cancelled during shutdown")),
            ),
        };
        drop(run);
        Snapshot {
            status,
            summary: Arc::clone(&self.config.summary),
            started_at_ms,
            finished_at_ms,
            output,
            error,
        }
    }
}

#[derive(Serialize)]
pub(super) struct Snapshot {
    status: Status,
    summary: Arc<str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<Arc<[String]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Arc<str>>,
}

async fn collect_until_shutdown(
    mut session: NetworkTestSession,
    mut shutdown: watch::Receiver<bool>,
) -> Outcome {
    let shutting_down = *shutdown.borrow();
    if shutting_down {
        return cancel(session).await;
    }
    let mut lines = Vec::new();
    loop {
        tokio::select! {
            chunk = session.next_chunk() => match chunk {
                Ok(Some(chunk)) => lines.extend(chunk),
                Ok(None) => return Outcome::Completed(lines.into()),
                Err(error) => return Outcome::Failed(error.to_string().into()),
            },
            () = crate::shutdown::requested(&mut shutdown) => return cancel(session).await,
        }
    }
}

async fn cancel(session: NetworkTestSession) -> Outcome {
    match session.cancel().await {
        Ok(_) => Outcome::Cancelled,
        Err(error) => {
            Outcome::Failed(format!("failed to cancel iPerf3 during shutdown: {error}").into())
        }
    }
}
