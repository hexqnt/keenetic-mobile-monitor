use std::{convert::Infallible, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use keenetic_rci::KeeneticClient;
use tokio::sync::{broadcast, watch};
use tower_http::cors::CorsLayer;

use crate::{
    collector::{History, InitialHistory},
    config::{Iperf3Config, PlotlySource, WebConfig},
};

mod dashboard;
mod iperf3;

#[derive(Clone)]
struct WebState {
    history: Arc<History>,
    page: Bytes,
    plotly_js: Option<Bytes>,
    shutdown: watch::Receiver<bool>,
    iperf3: Option<Arc<iperf3::Controller>>,
}

fn sample_event(sample: &crate::sample::LteSample) -> Event {
    Event::default()
        .event("sample")
        .id(sample.id().to_string())
        .data(serde_json::to_string(sample).expect("samples serialize"))
}

fn extract_plotly_js() -> Result<String> {
    const OPEN: &str = "<script type=\"text/javascript\">";
    let scripts = plotly::Plot::offline_js_sources();
    let (_, after_open) = scripts
        .split_once(OPEN)
        .context("plotly crate returned no embedded script")?;
    let (javascript, _) = after_open
        .split_once("</script>")
        .context("plotly embedded script is malformed")?;
    Ok(javascript.to_owned())
}

pub async fn build(
    history: Arc<History>,
    config: &WebConfig,
    iperf3: Option<(KeeneticClient, Iperf3Config)>,
    ping_host: Option<&str>,
    shutdown: watch::Receiver<bool>,
) -> Result<Router> {
    let plotly_js = (config.plotly_source == PlotlySource::Local)
        .then(extract_plotly_js)
        .transpose()?
        .map(Bytes::from);
    let iperf3 = iperf3.map(|(client, config)| {
        Arc::new(iperf3::Controller::new(client, config, shutdown.clone()))
    });
    let page = Bytes::from(dashboard::render_page(
        config.plotly_source,
        &config.api_base_url,
        iperf3.as_ref().map(|controller| controller.summary()),
        ping_host,
        config.debug,
    ));

    if let Some(directory) = &config.output_dir {
        tokio::fs::create_dir_all(directory)
            .await
            .with_context(|| {
                format!(
                    "failed to create static output directory {}",
                    directory.display()
                )
            })?;
        write_atomic(&directory.join("index.html"), &page).await?;
        if let Some(plotly_js) = &plotly_js {
            write_atomic(&directory.join("plotly.min.js"), plotly_js).await?;
        }
        tracing::info!(directory = %directory.display(), "Static dashboard written");
    }

    let state = WebState {
        history,
        page,
        plotly_js,
        shutdown,
        iperf3,
    };
    let mut router = Router::new().route("/api/events", get(events));
    if state.iperf3.is_some() {
        router = router.route("/api/iperf3", get(iperf3_status).post(start_iperf3));
    }
    if config.output_dir.is_none() {
        router = router.route("/", get(index));
        if config.plotly_source == PlotlySource::Local {
            router = router.route("/plotly.min.js", get(plotly_asset));
        }
    }
    if let Some(origin) = &config.cors_origin {
        let origin =
            HeaderValue::from_str(origin).context("web.cors_origin is not a valid HTTP origin")?;
        router = router.layer(
            CorsLayer::new()
                .allow_origin(origin)
                .allow_methods([Method::GET, Method::POST]),
        );
    }
    Ok(router.with_state(state))
}

async fn iperf3_status(State(state): State<WebState>) -> Response {
    let snapshot = state
        .iperf3
        .expect("the iPerf3 route is only installed when configured")
        .snapshot()
        .await;
    ([(header::CACHE_CONTROL, "no-store")], Json(snapshot)).into_response()
}

async fn start_iperf3(State(state): State<WebState>) -> Response {
    let controller = state
        .iperf3
        .expect("the iPerf3 route is only installed when configured");
    let status = match controller.start().await {
        iperf3::StartOutcome::Started => StatusCode::ACCEPTED,
        iperf3::StartOutcome::AlreadyRunning => StatusCode::CONFLICT,
    };
    (status, Json(controller.snapshot().await)).into_response()
}

async fn index(State(state): State<WebState>) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        state.page,
    )
        .into_response()
}

async fn plotly_asset(State(state): State<WebState>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        state
            .plotly_js
            .expect("the asset route is only installed in local mode"),
    )
        .into_response()
}

async fn events(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let last_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let mut updates = state.history.subscribe();
    let mut shutdown = state.shutdown.clone();
    let history = Arc::clone(&state.history);
    let stream = async_stream::stream! {
        let mut delivered = last_id.unwrap_or(0);
        match history.initial(last_id).await {
            InitialHistory::Replay(samples) => for sample in samples {
                delivered = sample.id();
                yield Ok(sample_event(&sample));
            },
            InitialHistory::Snapshot(samples) => {
                delivered = samples.last().map_or(0, |sample| sample.id());
                yield Ok(Event::default()
                    .event("snapshot")
                    .id(delivered.to_string())
                    .data(serde_json::to_string(&samples).expect("samples serialize")));
            }
        }

        let shutdown_requested = crate::shutdown::requested(&mut shutdown);
        tokio::pin!(shutdown_requested);
        loop {
            tokio::select! {
                update = updates.recv() => match update {
                    Ok(sample) if sample.id() > delivered => {
                        delivered = sample.id();
                        yield Ok(sample_event(&sample));
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for sample in history.after(delivered).await {
                            delivered = sample.id();
                            yield Ok(sample_event(&sample));
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                () = &mut shutdown_requested => break,
            }
        }
    };
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

async fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("static");
    let temporary = path.with_file_name(format!(".{file_name}.tmp"));
    tokio::fs::write(&temporary, contents)
        .await
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    tokio::fs::rename(&temporary, path)
        .await
        .with_context(|| format!("failed to replace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        num::NonZeroU16,
        sync::atomic::{AtomicUsize, Ordering},
        time::SystemTime,
    };

    use axum::{
        Json,
        body::Body,
        extract::State,
        http::{Request, StatusCode},
        routing::post,
    };
    use http_body_util::BodyExt;
    use keenetic_rci::{Iperf3, IperfLimit, KeeneticClient};
    use serde_json::{Value, json};
    use tokio::sync::{Mutex, Notify};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        config::{Iperf3Config, WebConfig},
        sample::{
            FailureKind, PendingSample, PingOutcome, PingSnapshot, PollFailure, PollOutcome,
            unix_time_ms,
        },
    };

    #[test]
    fn pages_select_requested_plotly_source() {
        let cdn = dashboard::render_page(
            PlotlySource::Cdn,
            "https://api.example.test",
            Some("server.example:5201 · TCP upload · 10s · IPv4"),
            Some("one.one.one.one"),
            true,
        );
        assert!(cdn.contains(dashboard::PLOTLY_CDN));
        assert!(cdn.contains("data-api-base=\"https://api.example.test\""));
        assert!(cdn.contains("id=\"iperf-start\""));
        assert!(cdn.contains("server.example:5201 · TCP upload · 10s · IPv4"));
        assert!(cdn.contains("data-ping-host=\"one.one.one.one\""));
        assert!(cdn.contains("id=\"metadata\""));
        let local = dashboard::render_page(PlotlySource::Local, "", None, None, false);
        assert!(local.contains("src=\"plotly.min.js\""));
        assert!(!local.contains("data-ping-host"));
        for id in [
            "signal-chart",
            "signal-level",
            "traffic-chart",
            "temperature-chart",
            "context-chart",
            "carrier-table",
            "copy-carriers",
            "carrier-statistics-table",
            "context-table",
            "event-table",
        ] {
            assert!(local.contains(&format!("id=\"{id}\"")));
        }
        assert!(local.contains("Max DL / UL"));
        assert!(local.contains("All traffic levels"));
        assert!(local.contains(
            "The installed equipment does not support collecting carrier radio statistics."
        ));
        assert!(!local.contains("rssi-now"));
        assert!(!local.contains("id=\"primary\""));
        assert!(!local.contains("id=\"aggregation\""));
        assert!(!local.contains("id=\"iperf-start\""));
        assert!(!local.contains("Latest raw mobile sample"));
        assert!(!local.contains("id=\"metadata\""));
    }

    #[test]
    fn embedded_plotly_can_be_extracted() {
        let javascript = extract_plotly_js().unwrap();
        assert!(javascript.contains("Plotly"));
        assert!(javascript.len() > 1_000_000);
    }

    fn web_config(cors_origin: Option<&str>) -> WebConfig {
        WebConfig {
            bind: "127.0.0.1:3000".parse::<SocketAddr>().unwrap(),
            output_dir: None,
            api_base_url: String::new(),
            cors_origin: cors_origin.map(str::to_owned),
            plotly_source: PlotlySource::Cdn,
            debug: false,
        }
    }

    async fn add_failure(history: &History, message: &str, ping: Option<Arc<PingSnapshot>>) {
        history
            .push(PendingSample {
                collected_at_ms: unix_time_ms(SystemTime::now()),
                poll_duration_ms: 1,
                router_id: Arc::from("test"),
                interface: Arc::from("UsbLte0"),
                traffic: None,
                ping,
                outcome: PollOutcome::Failure {
                    error: PollFailure {
                        kind: FailureKind::Transport,
                        message: message.to_owned(),
                    },
                },
            })
            .await;
    }

    fn ping(average_rtt_ms: f64) -> Arc<PingSnapshot> {
        Arc::new(PingSnapshot::new(
            unix_time_ms(SystemTime::now()),
            10,
            Arc::from("one.one.one.one"),
            PingOutcome::Replies {
                transmitted: NonZeroU16::new(3).unwrap(),
                received: NonZeroU16::new(3).unwrap(),
                average_rtt_ms,
            },
        ))
    }

    #[tokio::test]
    async fn sse_sends_snapshot_and_replays_after_last_event_id() {
        let history = Arc::new(History::new());
        add_failure(&history, "first", None).await;
        add_failure(&history, "second", Some(ping(12.5))).await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let app = build(
            Arc::clone(&history),
            &web_config(None),
            None,
            None,
            shutdown_rx,
        )
        .await
        .unwrap();

        let response = app
            .oneshot(
                Request::get("/api/events")
                    .header("last-event-id", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        shutdown_tx.send(true).unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.contains("event: sample"));
        assert!(body.contains("id: 2"));
        assert!(body.contains("\"ping\":{"));
        assert!(body.contains("\"average_rtt_ms\":12.5"));
        assert!(body.contains("second"));
        assert!(!body.contains("event: snapshot"));
        assert!(!body.contains("first"));
    }

    #[tokio::test]
    async fn sse_replaces_a_cursor_from_an_older_server_run() {
        let history = Arc::new(History::new());
        add_failure(&history, "current-run", None).await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let app = build(history, &web_config(None), None, None, shutdown_rx)
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::get("/api/events")
                    .header("last-event-id", "99")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        shutdown_tx.send(true).unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.contains("event: snapshot"));
        assert!(body.contains("id: 1"));
        assert!(body.contains("current-run"));
    }

    #[tokio::test]
    async fn sse_snapshot_and_cors_are_available() {
        let history = Arc::new(History::new());
        add_failure(&history, "snapshot-value", None).await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let app = build(
            history,
            &web_config(Some("https://dashboard.example")),
            None,
            None,
            shutdown_rx,
        )
        .await
        .unwrap();
        let response = app
            .oneshot(
                Request::get("/api/events")
                    .header(header::ORIGIN, "https://dashboard.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("https://dashboard.example"))
        );
        shutdown_tx.send(true).unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.contains("event: snapshot"));
        assert!(body.contains("snapshot-value"));
    }

    #[tokio::test]
    async fn local_mode_exports_page_and_plotly_asset() {
        let directory = tempfile::tempdir().unwrap();
        let (_, shutdown_rx) = watch::channel(false);
        let config = WebConfig {
            bind: "127.0.0.1:3000".parse().unwrap(),
            output_dir: Some(directory.path().to_owned()),
            api_base_url: "https://api.example".to_owned(),
            cors_origin: None,
            plotly_source: PlotlySource::Local,
            debug: false,
        };
        let _router = build(Arc::new(History::new()), &config, None, None, shutdown_rx)
            .await
            .unwrap();

        let page = tokio::fs::read_to_string(directory.path().join("index.html"))
            .await
            .unwrap();
        let plotly = tokio::fs::metadata(directory.path().join("plotly.min.js"))
            .await
            .unwrap();
        assert!(page.contains("data-api-base=\"https://api.example\""));
        assert!(plotly.len() > 1_000_000);
    }

    #[tokio::test]
    async fn iperf3_api_runs_the_configured_typed_request() {
        let received = Arc::new(Mutex::new(None));
        let router = Router::new()
            .route(
                "/rci/tools/iperf3",
                post(
                    |State(received): State<Arc<Mutex<Option<Value>>>>,
                     Json(request): Json<Value>| async move {
                        *received.lock().await = Some(request);
                        Json(json!({"message": ["router output"]}))
                    },
                ),
            )
            .with_state(Arc::clone(&received));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router_task = tokio::spawn(async move { axum::serve(listener, router).await });

        let client = KeeneticClient::builder()
            .base_url(format!("http://{address}"))
            .build()
            .unwrap();
        let iperf3 = Iperf3Config {
            request: Iperf3::new("server.example", IperfLimit::time(3).unwrap())
                .unwrap()
                .port(5202)
                .unwrap(),
            summary: Arc::from("server.example:5202 · TCP upload · 3s · IPv4"),
        };
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let app = build(
            Arc::new(History::new()),
            &web_config(None),
            Some((client, iperf3)),
            None,
            shutdown_rx,
        )
        .await
        .unwrap();

        let response = app
            .clone()
            .oneshot(Request::post("/api/iperf3").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let snapshot = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let response = app
                    .clone()
                    .oneshot(Request::get("/api/iperf3").body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                let body = response.into_body().collect().await.unwrap().to_bytes();
                let snapshot: Value = serde_json::from_slice(&body).unwrap();
                if snapshot["status"] != "running" {
                    break snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        assert_eq!(snapshot["status"], "finished");
        assert_eq!(snapshot["output"], json!(["router output"]));
        assert_eq!(
            *received.lock().await,
            Some(json!({
                "host": "server.example",
                "ipv4": true,
                "tcp": true,
                "port": 5202,
                "time": 3
            }))
        );
        router_task.abort();
    }

    #[tokio::test]
    async fn shutdown_cancels_an_active_router_iperf3_session() {
        #[derive(Clone)]
        struct CancellationState {
            started: Arc<Notify>,
            cancellations: Arc<AtomicUsize>,
        }

        let state = CancellationState {
            started: Arc::new(Notify::new()),
            cancellations: Arc::new(AtomicUsize::new(0)),
        };
        let router = Router::new()
            .route(
                "/rci/tools/iperf3",
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router_task = tokio::spawn(async move { axum::serve(listener, router).await });
        let client = KeeneticClient::builder()
            .base_url(format!("http://{address}"))
            .build()
            .unwrap();
        let config = Iperf3Config {
            request: Iperf3::new("server.example", IperfLimit::time(30).unwrap()).unwrap(),
            summary: Arc::from("server.example:5201 · TCP upload · 30s · IPv4"),
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let controller = Arc::new(iperf3::Controller::new(client, config, shutdown_rx));

        assert_eq!(controller.start().await, iperf3::StartOutcome::Started);
        state.started.notified().await;
        assert_eq!(
            controller.start().await,
            iperf3::StartOutcome::AlreadyRunning
        );
        shutdown_tx.send(true).unwrap();
        let snapshot = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = serde_json::to_value(controller.snapshot().await).unwrap();
                if snapshot["status"] != "running" {
                    break snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        assert_eq!(snapshot["status"], "cancelled");
        assert_eq!(snapshot["error"], "iPerf3 test cancelled during shutdown");
        assert_eq!(state.cancellations.load(Ordering::SeqCst), 1);
        router_task.abort();
    }
}
