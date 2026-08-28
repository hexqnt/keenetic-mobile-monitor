use std::{
    fmt,
    num::NonZeroU16,
    sync::Arc,
    time::{Instant, SystemTime},
};

use keenetic_rci::{
    Error as KeeneticError, KeeneticClient, NetworkTestRequest, NetworkTestSession,
};
use tokio::{
    sync::watch,
    time::{MissedTickBehavior, interval_at},
};

use crate::{
    config::{PingConfig, PingRequest},
    sample::{PingOutcome, PingSnapshot, PollFailure, duration_ms, unix_time_ms},
};

enum ProbeRun {
    Completed(PingOutput),
    Failed(KeeneticError),
    Shutdown,
}

#[derive(Debug, Eq, PartialEq)]
enum PingOutputError {
    MissingPacketCounts,
    UnexpectedTransmitted { expected: u16, actual: u16 },
    TooManyReplies { transmitted: u16, received: u16 },
    MissingAverageRtt,
}

impl fmt::Display for PingOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPacketCounts => {
                formatter.write_str("ping output contains no packet statistics")
            }
            Self::UnexpectedTransmitted { expected, actual } => write!(
                formatter,
                "ping reported {actual} transmitted packets, expected {expected}"
            ),
            Self::TooManyReplies {
                transmitted,
                received,
            } => write!(
                formatter,
                "ping reported {received} received packets after {transmitted} transmissions"
            ),
            Self::MissingAverageRtt => {
                formatter.write_str("ping output contains replies but no average RTT")
            }
        }
    }
}

#[derive(Default)]
struct PingOutput {
    packet_counts: Option<(u16, u16)>,
    average_rtt_ms: Option<f64>,
}

impl PingOutput {
    fn observe(&mut self, line: &str) {
        if self.packet_counts.is_none() {
            self.packet_counts = parse_packet_counts(line);
        }
        if self.average_rtt_ms.is_none() {
            self.average_rtt_ms = parse_average_rtt(line);
        }
    }

    fn finish(self, expected_count: NonZeroU16) -> Result<PingOutcome, PingOutputError> {
        let (transmitted, received) = self
            .packet_counts
            .ok_or(PingOutputError::MissingPacketCounts)?;
        if transmitted != expected_count.get() {
            return Err(PingOutputError::UnexpectedTransmitted {
                expected: expected_count.get(),
                actual: transmitted,
            });
        }
        if received > transmitted {
            return Err(PingOutputError::TooManyReplies {
                transmitted,
                received,
            });
        }
        if received == 0 {
            return Ok(PingOutcome::NoReply {
                transmitted: expected_count,
            });
        }
        let received = NonZeroU16::new(received).expect("zero replies returned above");
        let average_rtt_ms = self
            .average_rtt_ms
            .ok_or(PingOutputError::MissingAverageRtt)?;
        Ok(PingOutcome::Replies {
            transmitted: expected_count,
            received,
            average_rtt_ms,
        })
    }
}

#[cfg(test)]
fn parse_output(lines: &[String], expected_count: u16) -> Result<PingOutcome, PingOutputError> {
    let mut output = PingOutput::default();
    for line in lines {
        output.observe(line);
    }
    output.finish(NonZeroU16::new(expected_count).expect("test count must be non-zero"))
}

fn parse_packet_counts(line: &str) -> Option<(u16, u16)> {
    let mut fields = line.split(',');
    let transmitted_field = fields.next()?.trim_ascii();
    let received_field = fields.next()?.trim_ascii();
    Some((
        parse_packet_count(
            transmitted_field,
            &["packets transmitted", "packet transmitted"],
        )?,
        parse_packet_count(
            received_field,
            &["packets received", "packet received", "received"],
        )?,
    ))
}

fn parse_packet_count(field: &str, suffixes: &[&str]) -> Option<u16> {
    suffixes
        .iter()
        .find_map(|suffix| field.strip_suffix(suffix))?
        .trim_ascii()
        .parse()
        .ok()
}

fn parse_average_rtt(line: &str) -> Option<f64> {
    let (label, values) = line.split_once('=')?;
    if !label
        .trim_ascii()
        .eq_ignore_ascii_case("round-trip min/avg/max")
    {
        return None;
    }
    let average = values.trim_ascii().split('/').nth(1)?.parse::<f64>().ok()?;
    (average.is_finite() && average >= 0.0).then_some(average)
}

pub async fn run(
    client: KeeneticClient,
    config: PingConfig,
    updates: watch::Sender<Option<Arc<PingSnapshot>>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = interval_at(tokio::time::Instant::now(), config.interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let started = Instant::now();
                let result = match &config.request {
                    PingRequest::Ipv4(request) => {
                        execute(&client, request.clone(), &mut shutdown).await
                    }
                    PingRequest::Ipv6(request) => {
                        execute(&client, request.clone(), &mut shutdown).await
                    }
                };
                let outcome = match result {
                    ProbeRun::Completed(output) => output.finish(config.count).unwrap_or_else(|error| {
                        tracing::warn!(%error, "Keenetic ping output could not be parsed");
                        PingOutcome::Failure {
                            error: PollFailure::invalid_response(error.to_string()),
                        }
                    }),
                    ProbeRun::Failed(error) => {
                        tracing::warn!(error = ?error, "Keenetic ping failed");
                        PingOutcome::Failure {
                            error: PollFailure::from_error(&error),
                        }
                    }
                    ProbeRun::Shutdown => break,
                };
                updates.send_replace(Some(Arc::new(PingSnapshot::new(
                    unix_time_ms(SystemTime::now()),
                    duration_ms(started.elapsed()),
                    Arc::clone(&config.host),
                    outcome,
                ))));
            }
            () = crate::shutdown::requested(&mut shutdown) => break,
        }
    }
}

async fn execute<R>(
    client: &KeeneticClient,
    request: R,
    shutdown: &mut watch::Receiver<bool>,
) -> ProbeRun
where
    R: NetworkTestRequest,
{
    if *shutdown.borrow() {
        return ProbeRun::Shutdown;
    }
    let session = match client.start_network_test(request).await {
        Ok(session) => session,
        Err(error) => return ProbeRun::Failed(error),
    };
    collect_until_shutdown(session, shutdown).await
}

async fn collect_until_shutdown(
    mut session: NetworkTestSession,
    shutdown: &mut watch::Receiver<bool>,
) -> ProbeRun {
    if *shutdown.borrow() {
        cancel(session, "shutdown").await;
        return ProbeRun::Shutdown;
    }
    let mut output = PingOutput::default();
    loop {
        tokio::select! {
            chunk = session.next_chunk() => match chunk {
                Ok(Some(chunk)) => {
                    for line in chunk {
                        output.observe(&line);
                    }
                }
                Ok(None) => return ProbeRun::Completed(output),
                Err(error) => {
                    cancel(session, "poll failure").await;
                    return ProbeRun::Failed(error);
                }
            },
            () = crate::shutdown::requested(shutdown) => {
                cancel(session, "shutdown").await;
                return ProbeRun::Shutdown;
            }
        }
    }
}

async fn cancel(session: NetworkTestSession, reason: &'static str) {
    if let Err(error) = session.cancel().await {
        tracing::warn!(error = ?error, %reason, "Failed to cancel Keenetic ping");
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        time::Duration,
    };

    use axum::{
        Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post,
    };
    use keenetic_rci::{InterfaceId, Ping, PingIpv6};
    use serde_json::{Value, json};
    use tokio::sync::{Mutex, Notify};

    use super::*;

    fn lines(value: &str) -> Vec<String> {
        value.lines().map(str::to_owned).collect()
    }

    fn reply_average(outcome: &PingOutcome) -> f64 {
        let PingOutcome::Replies { average_rtt_ms, .. } = outcome else {
            panic!("expected ping replies");
        };
        *average_rtt_ms
    }

    #[test]
    fn parses_single_and_multiple_reply_averages() {
        let single = lines(
            "1 packet transmitted, 1 received, 0% packet loss,\n\
             Round-trip min/avg/max = 12.34/12.34/12.34 ms.",
        );
        let multiple = lines(
            "3 packets transmitted, 3 packets received, 0% packet loss,\n\
             Round-trip min/avg/max = 10.00/12.50/15.00 ms.",
        );
        assert!((reply_average(&parse_output(&single, 1).unwrap()) - 12.34).abs() < f64::EPSILON);
        assert!((reply_average(&parse_output(&multiple, 3).unwrap()) - 12.5).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_partial_and_total_packet_loss() {
        let partial = lines(
            "3 packets transmitted, 2 packets received, 33% packet loss,\n\
             Round-trip min/avg/max = 10.00/11.25/12.50 ms.",
        );
        let total = lines("3 packets transmitted, 0 packets received, 100% packet loss,");
        let PingOutcome::Replies {
            transmitted,
            received,
            average_rtt_ms,
        } = parse_output(&partial, 3).unwrap()
        else {
            panic!("expected partial replies");
        };
        assert_eq!((transmitted.get(), received.get()), (3, 2));
        assert!((average_rtt_ms - 11.25).abs() < f64::EPSILON);
        assert!(matches!(
            parse_output(&total, 3),
            Ok(PingOutcome::NoReply { transmitted }) if transmitted.get() == 3
        ));
    }

    #[test]
    fn rejects_missing_or_inconsistent_statistics() {
        assert_eq!(
            parse_output(&lines("Process terminated."), 1).unwrap_err(),
            PingOutputError::MissingPacketCounts
        );
        assert_eq!(
            parse_output(
                &lines("2 packets transmitted, 1 packets received, 50% packet loss,"),
                2
            )
            .unwrap_err(),
            PingOutputError::MissingAverageRtt
        );
        assert_eq!(
            parse_output(
                &lines("2 packets transmitted, 3 packets received, 0% packet loss,"),
                2
            )
            .unwrap_err(),
            PingOutputError::TooManyReplies {
                transmitted: 2,
                received: 3,
            }
        );
        assert_eq!(
            parse_output(
                &lines("1 packets transmitted, 0 packets received, 100% packet loss,"),
                2
            )
            .unwrap_err(),
            PingOutputError::UnexpectedTransmitted {
                expected: 2,
                actual: 1,
            }
        );
    }

    async fn spawn_router(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("http://{address}"), task)
    }

    fn config(interval: Duration) -> PingConfig {
        PingConfig {
            request: PingRequest::Ipv4(
                Ping::new("example.com")
                    .unwrap()
                    .count(3)
                    .unwrap()
                    .source(InterfaceId::new("UsbLte0").unwrap())
                    .unwrap(),
            ),
            host: Arc::from("example.com"),
            interval,
            count: NonZeroU16::new(3).unwrap(),
        }
    }

    #[tokio::test]
    async fn collector_sends_typed_request_and_records_average() {
        let received = Arc::new(Mutex::new(None));
        let router = Router::new()
            .route(
                "/rci/tools/ping",
                post(
                    |State(received): State<Arc<Mutex<Option<Value>>>>,
                     Json(request): Json<Value>| async move {
                        *received.lock().await = Some(request);
                        Json(json!({"message": [
                            "3 packets transmitted, 2 packets received, 33% packet loss,",
                            "Round-trip min/avg/max = 10.00/12.50/15.00 ms."
                        ]}))
                    },
                ),
            )
            .with_state(Arc::clone(&received));
        let (origin, router_task) = spawn_router(router).await;
        let client = KeeneticClient::builder().base_url(origin).build().unwrap();
        let (updates, mut latest) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let collector = tokio::spawn(run(
            client,
            config(Duration::from_secs(60)),
            updates,
            shutdown_rx,
        ));

        tokio::time::timeout(Duration::from_secs(1), latest.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            *received.lock().await,
            Some(json!({"host": "example.com", "count": 3, "source": "UsbLte0"}))
        );
        let sample = serde_json::to_value(latest.borrow().as_ref().unwrap()).unwrap();
        assert_eq!(sample["status"], "replies");
        assert_eq!(sample["transmitted"], 3);
        assert_eq!(sample["received"], 2);
        assert_eq!(sample["average_rtt_ms"], 12.5);

        shutdown_tx.send(true).unwrap();
        collector.await.unwrap();
        router_task.abort();
    }

    #[tokio::test]
    async fn ipv6_request_uses_the_ping6_endpoint() {
        let received = Arc::new(Mutex::new(None));
        let router = Router::new()
            .route(
                "/rci/tools/ping6",
                post(
                    |State(received): State<Arc<Mutex<Option<Value>>>>,
                     Json(request): Json<Value>| async move {
                        *received.lock().await = Some(request);
                        Json(json!({"message": ["finished"]}))
                    },
                ),
            )
            .with_state(Arc::clone(&received));
        let (origin, router_task) = spawn_router(router).await;
        let client = KeeneticClient::builder().base_url(origin).build().unwrap();
        let (_shutdown_tx, mut shutdown) = watch::channel(false);
        let request = PingIpv6::new("2001:db8::1")
            .unwrap()
            .count(2)
            .unwrap()
            .source(InterfaceId::new("UsbLte0").unwrap())
            .unwrap();

        assert!(matches!(
            execute(&client, request, &mut shutdown).await,
            ProbeRun::Completed(_)
        ));
        assert_eq!(
            *received.lock().await,
            Some(json!({
                "host": "2001:db8::1",
                "count": 2,
                "source": "UsbLte0"
            }))
        );
        router_task.abort();
    }

    #[derive(Clone)]
    struct RetryState {
        calls: Arc<AtomicUsize>,
        active: Arc<AtomicBool>,
        overlapped: Arc<AtomicBool>,
    }

    #[tokio::test]
    async fn collector_does_not_overlap_and_continues_after_failure() {
        let state = RetryState {
            calls: Arc::new(AtomicUsize::new(0)),
            active: Arc::new(AtomicBool::new(false)),
            overlapped: Arc::new(AtomicBool::new(false)),
        };
        let router = Router::new()
            .route(
                "/rci/tools/ping",
                post(|State(state): State<RetryState>| async move {
                    if state.active.swap(true, Ordering::SeqCst) {
                        state.overlapped.store(true, Ordering::SeqCst);
                    }
                    let call = state.calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    state.active.store(false, Ordering::SeqCst);
                    if call == 0 {
                        (StatusCode::INTERNAL_SERVER_ERROR, "failed").into_response()
                    } else {
                        Json(json!({"message": [
                            "3 packets transmitted, 3 packets received, 0% packet loss,",
                            "Round-trip min/avg/max = 1.00/2.00/3.00 ms."
                        ]}))
                        .into_response()
                    }
                }),
            )
            .with_state(state.clone());
        let (origin, router_task) = spawn_router(router).await;
        let client = KeeneticClient::builder().base_url(origin).build().unwrap();
        let (updates, mut latest) = watch::channel(None);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let collector = tokio::spawn(run(
            client,
            config(Duration::from_millis(5)),
            updates,
            shutdown_rx,
        ));

        tokio::time::timeout(Duration::from_secs(1), latest.changed())
            .await
            .unwrap()
            .unwrap();
        let first = serde_json::to_value(latest.borrow_and_update().as_ref().unwrap()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), latest.changed())
            .await
            .unwrap()
            .unwrap();
        let second = serde_json::to_value(latest.borrow().as_ref().unwrap()).unwrap();
        assert_eq!(first["status"], "failure");
        assert_eq!(second["status"], "replies");
        assert!(!state.overlapped.load(Ordering::SeqCst));

        shutdown_tx.send(true).unwrap();
        collector.await.unwrap();
        router_task.abort();
    }

    #[derive(Clone)]
    struct CancellationState {
        started: Arc<Notify>,
        cancellations: Arc<AtomicUsize>,
    }

    #[tokio::test]
    async fn shutdown_cancels_an_active_router_ping() {
        let state = CancellationState {
            started: Arc::new(Notify::new()),
            cancellations: Arc::new(AtomicUsize::new(0)),
        };
        let router = Router::new()
            .route(
                "/rci/tools/ping",
                post(|State(state): State<CancellationState>| async move {
                    state.started.notify_one();
                    Json(json!({"continued": true}))
                })
                .delete(|State(state): State<CancellationState>| async move {
                    state.cancellations.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"message": ["cancelled"]}))
                }),
            )
            .with_state(state.clone());
        let (origin, router_task) = spawn_router(router).await;
        let client = KeeneticClient::builder().base_url(origin).build().unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let collector = tokio::spawn(run(
            client,
            config(Duration::from_secs(60)),
            watch::channel(None).0,
            shutdown_rx,
        ));

        state.started.notified().await;
        shutdown_tx.send(true).unwrap();
        collector.await.unwrap();
        assert_eq!(state.cancellations.load(Ordering::SeqCst), 1);
        router_task.abort();
    }
}
