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
    /// Read or update desktop-independent per-user policy settings.
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// Evaluate policy offline without connecting to the sensor or desktop.
    Simulate {
        #[arg(long, action = clap::ArgAction::Set)]
        present: bool,
        #[arg(long, default_value_t = 15_000)]
        stable_ms: u64,
        #[arg(long, default_value_t = 15_000)]
        idle_ms: u64,
        #[arg(long, default_value_t = 30_000)]
        runtime_ms: u64,
        #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
        screen_locked: bool,
        #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
        locked_by_hpd: bool,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        osd_announced_present: bool,
        #[arg(long)]
        since_last_osd_ms: Option<u64>,
    },
    /// Print read-only configuration, sensor and desktop safety diagnostics.
    Diagnose,
}

#[derive(Debug, Subcommand)]
enum SettingsCommand {
    Get,
    Defaults,
    Set {
        #[arg(long)]
        json: String,
    },
    Reset,
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
    match cli.command {
        Command::Daemon => service::run_daemon(Config::load(&cli.config)?).await,
        Command::Agent => agent::run_agent(Config::load_for_agent(&cli.config)?).await,
        Command::Status => status().await,
        Command::Monitor => monitor().await,
        Command::Probe => probe(&Config::load(&cli.config)?),
        Command::Settings { command } => settings(&cli.config, command),
        Command::Simulate {
            present,
            stable_ms,
            idle_ms,
            runtime_ms,
            screen_locked,
            locked_by_hpd,
            osd_announced_present,
            since_last_osd_ms,
        } => {
            let config = Config::load_for_agent(&cli.config)?;
            let decision = agent::simulate_policy(
                &config.policy,
                present,
                std::time::Duration::from_millis(stable_ms),
                std::time::Duration::from_millis(idle_ms),
                std::time::Duration::from_millis(runtime_ms),
                screen_locked,
                locked_by_hpd,
                osd_announced_present,
                since_last_osd_ms.map(std::time::Duration::from_millis),
            );
            println!("{}", serde_json::to_string_pretty(&decision)?);
            Ok(())
        }
        Command::Diagnose => diagnose(&cli.config),
    }
}

fn diagnose(system_path: &std::path::Path) -> Result<()> {
    let config = Config::load_for_agent(system_path)?;
    let user_path = Config::user_path();
    let screen_off_reason = screensaver::automatic_screen_off_block_reason();
    let effective_policy = agent::effective_policy(&config.policy, screen_off_reason.is_none());
    let sensor = match iio::SensorPaths::discover(&config.sensor) {
        Ok(sensor) => serde_json::json!({
            "available": true,
            "sysfs": sensor.sysfs_dir,
            "device": sensor.dev_path,
            "raw": sensor.read_current().ok(),
        }),
        Err(error) => serde_json::json!({
            "available": false,
            "error": error.to_string(),
        }),
    };
    let mut warnings = Vec::new();
    if config.policy.enabled && !config.policy.dry_run {
        warnings.push("live desktop automation is enabled");
    }
    if config.policy.turn_off_screen
        && let Some(reason) = screen_off_reason
    {
        warnings.push(reason);
    }
    if config.policy.wake_manual_lock {
        warnings.push("presence may wake screens that the user locked manually");
    }
    let output = serde_json::json!({
        "system_config": system_path,
        "user_config": user_path,
        "user_config_exists": user_path.exists(),
        "session_type": std::env::var("XDG_SESSION_TYPE").ok(),
        "desktop": std::env::var("XDG_CURRENT_DESKTOP").ok(),
        "screen_off_supported": screen_off_reason.is_none(),
        "screen_off_block_reason": screen_off_reason,
        "sensor": sensor,
        "policy": config.policy,
        "effective_policy": effective_policy,
        "warnings": warnings,
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn settings(system_path: &std::path::Path, command: SettingsCommand) -> Result<()> {
    match command {
        SettingsCommand::Get => println!(
            "{}",
            serde_json::to_string(&Config::load_for_agent(system_path)?.policy)?
        ),
        SettingsCommand::Defaults => println!(
            "{}",
            serde_json::to_string(&config::PolicyConfig::default())?
        ),
        SettingsCommand::Set { json } => {
            let policy: config::PolicyConfig = serde_json::from_str(&json)?;
            Config::save_user_policy(&policy)?;
        }
        SettingsCommand::Reset => {
            let path = Config::user_path();
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
    }
    Ok(())
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
