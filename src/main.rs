use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use keenetic_rci::KeeneticClient;
use tokio::sync::watch;

use crate::{
    collector::History,
    config::{Config, RouterConfig},
};

mod collector;
mod config;
mod modem;
mod ping;
mod sample;
mod shutdown;
mod web;

#[derive(Debug, Subcommand)]
enum Command {
    /// Collect samples in memory until the process is stopped.
    Collect {
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Collect samples and expose the dashboard and streaming API.
    Serve {
        #[arg(short, long)]
        config: PathBuf,
    },
}

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

fn print_greeting() {
    println!(
        "\n{} {}\n{}\n",
        "◆".bright_cyan(),
        "Keenetic Mobile Monitor".bright_blue().bold(),
        "Welcome! Mobile/LTE telemetry monitoring is starting...".dimmed(),
    );
}

// Two workers keep large history responses from delaying telemetry collection
// without creating a worker for every CPU core.
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let cli = Cli::parse();
    print_greeting();
    let (path, serve) = match cli.command {
        Command::Collect { config } => (config, false),
        Command::Serve { config } => (config, true),
    };
    let Config {
        router,
        web: web_config,
        iperf3,
        ping,
    } = Config::load(&path, serve).await?;
    let RouterConfig {
        id,
        url,
        login,
        password,
        poll_interval,
        interface: configured_interface,
    } = router;
    let client = KeeneticClient::builder()
        .base_url(url)
        .credentials(login, password.into_inner())
        .build()
        .context("failed to configure Keenetic client")?;
    let interface = collector::resolve_interface(&client, configured_interface).await?;
    tracing::info!(interface = %interface, "Mobile interface selected");
    let ping = ping
        .map(|config| config.with_interface(interface.clone()))
        .transpose()?;

    let history = Arc::new(History::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (ping_updates, latest_ping) = watch::channel(None);
    let web = if serve {
        let web_config = web_config.expect("serve mode validates the web section");
        let listener = tokio::net::TcpListener::bind(web_config.bind)
            .await
            .with_context(|| format!("failed to bind HTTP server to {}", web_config.bind))?;
        let iperf3 = iperf3.map(|config| (client.clone(), config));
        let app = web::build(
            Arc::clone(&history),
            &web_config,
            iperf3,
            ping.as_ref().map(|config| config.host.as_ref()),
            shutdown_rx.clone(),
        )
        .await?;
        Some((app, listener, web_config.bind))
    } else {
        None
    };

    let ping_collector = ping.map(|config| {
        tokio::spawn(ping::run(
            client.clone(),
            config,
            ping_updates,
            shutdown_rx.clone(),
        ))
    });
    let collector = tokio::spawn(collector::run(
        client,
        id,
        poll_interval,
        interface,
        Arc::clone(&history),
        latest_ping,
        shutdown_rx.clone(),
    ));

    if let Some((app, listener, bind)) = web {
        tracing::info!(address = %bind, "HTTP server listening");
        let server_shutdown = shutdown_tx.clone();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                let _ = server_shutdown.send(true);
            })
            .await
            .context("HTTP server failed")?;
    } else {
        shutdown_signal().await;
        let _ = shutdown_tx.send(true);
    }

    collector.await.context("collector task panicked")?;
    if let Some(ping_collector) = ping_collector {
        ping_collector
            .await
            .context("ping collector task panicked")?;
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("Shutdown requested");
}
