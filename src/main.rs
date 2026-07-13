mod activity;
mod agent;
mod config;
mod iio;
mod screensaver;
mod service;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;
use futures_util::StreamExt;
use service::HumanPresenceProxy;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, global = true, default_value = "/etc/thinkpad-hpd/config.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the privileged IIO reader and system D-Bus service.
    Daemon,
    /// Run the per-user KDE integration agent.
    Agent,
    /// Print the current sensor state from the system D-Bus service.
    Status,
    /// Stream presence changes from the system D-Bus service.
    Monitor,
    /// Discover the HID/IIO device and print its current raw value.
    Probe,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("thinkpad_hpd=info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let config = Config::load(&cli.config)?;

    match cli.command {
        Command::Daemon => service::run_daemon(config).await,
        Command::Agent => agent::run_agent(config).await,
        Command::Status => status().await,
        Command::Monitor => monitor().await,
        Command::Probe => probe(&config),
    }
}

async fn status() -> Result<()> {
    let connection = zbus::Connection::system().await?;
    let proxy = HumanPresenceProxy::new(&connection).await?;
    let (available, present, raw) = proxy.get_state().await?;
    println!("available={available} present={present} raw={raw}");
    Ok(())
}

async fn monitor() -> Result<()> {
    let connection = zbus::Connection::system().await?;
    let proxy = HumanPresenceProxy::new(&connection).await?;
    let (available, present, raw) = proxy.get_state().await?;
    println!("available={available} present={present} raw={raw}");

    let mut signals = proxy.receive_presence_changed().await?;
    while let Some(signal) = signals.next().await {
        let args = signal.args()?;
        println!(
            "available={} present={} raw={}",
            args.available, args.present, args.raw
        );
    }
    Ok(())
}

fn probe(config: &Config) -> Result<()> {
    let sensor = iio::SensorPaths::discover(&config.sensor)?;
    println!("sysfs={}", sensor.sysfs_dir.display());
    println!("device={}", sensor.dev_path.display());
    println!("scan_type={}", sensor.scan_type.raw);
    println!("raw={}", sensor.read_current()?);
    Ok(())
}
