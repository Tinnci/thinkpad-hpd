use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::{sync::watch, time};
use tracing::{debug, info, warn};
use zbus::Connection;

use crate::{activity, config::Config, screensaver::ScreenController, service::HumanPresenceProxy};

pub async fn run_agent(config: Config) -> Result<()> {
    let system = Connection::system()
        .await
        .context("failed to connect to the system D-Bus")?;
    let proxy = connect_presence_proxy(&system).await?;
    let (mut sensor_available, mut present, raw) = proxy.get_state().await?;
    info!(
        available = sensor_available,
        present, raw, "connected to presence daemon"
    );

    let controller = ScreenController::connect().await?;
    let (activity_tx, activity_rx) = watch::channel(Instant::now());
    let _activity_task = activity::start_input_monitor(activity_tx);
    let mut presence_signals = proxy.receive_presence_changed().await?;
    let mut owner_changes = proxy.inner().receive_owner_changed().await?;
    let mut tick = time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    let mut away_since = (!present).then(Instant::now);
    let mut present_since = present.then(Instant::now);
    let mut lock_requested = false;
    let mut wake_requested = false;

    loop {
        tokio::select! {
            signal = presence_signals.next() => {
                let Some(signal) = signal else {
                    anyhow::bail!("presence D-Bus signal stream ended");
                };
                let args = signal.args()?;
                if !args.available {
                    sensor_available = false;
                    away_since = None;
                    present_since = None;
                    warn!("presence sensor became unavailable");
                    continue;
                }
                sensor_available = true;
                if args.present != present {
                    present = args.present;
                    info!(present, raw = args.raw, "agent received presence transition");
                    if present {
                        present_since = Some(Instant::now());
                        away_since = None;
                        lock_requested = false;
                        wake_requested = false;
                    } else {
                        away_since = Some(Instant::now());
                        present_since = None;
                        wake_requested = false;
                    }
                }
            }
            owner = owner_changes.next() => {
                let Some(owner) = owner else {
                    anyhow::bail!("presence daemon owner-change stream ended");
                };
                if owner.is_none() {
                    sensor_available = false;
                    away_since = None;
                    present_since = None;
                    lock_requested = false;
                    wake_requested = false;
                    warn!("presence daemon disconnected; policy paused");
                    continue;
                }

                match proxy.get_state().await {
                    Ok((available, new_present, raw)) => {
                        sensor_available = available;
                        present = new_present;
                        away_since = (available && !present).then(Instant::now);
                        present_since = (available && present).then(Instant::now);
                        lock_requested = false;
                        wake_requested = false;
                        info!(available, present, raw, "presence daemon reconnected");
                    }
                    Err(error) => {
                        sensor_available = false;
                        warn!(%error, "presence daemon returned before state was readable");
                    }
                }
            }
            _ = tick.tick() => {
                if !sensor_available {
                    continue;
                }
                let now = Instant::now();
                let last_activity = *activity_rx.borrow();
                if !present {
                    let away_for = away_since.map(|start| now.saturating_duration_since(start)).unwrap_or_default();
                    let idle_for = now.saturating_duration_since(last_activity);
                    if should_lock(lock_requested, away_for, idle_for, &config.policy) {
                        if !controller.is_locked().await.unwrap_or(false) {
                            controller.lock().await?;
                            lock_requested = true;
                            if config.policy.turn_off_screen {
                                time::sleep(Duration::from_millis(750)).await;
                                controller.turn_off_screen().await;
                            }
                        } else {
                            lock_requested = true;
                        }
                    } else {
                        debug!(?away_for, ?idle_for, lock_requested, "away policy pending");
                    }
                } else if !wake_requested {
                    let present_for = present_since.map(|start| now.saturating_duration_since(start)).unwrap_or_default();
                    if should_wake(wake_requested, present_for, &config.policy)
                        && controller.is_locked().await.unwrap_or(false) {
                        controller.wake().await?;
                        wake_requested = true;
                    }
                }
            }
            _ = shutdown_signal() => {
                info!("agent shutdown requested");
                break;
            }
        }
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn should_lock(
    lock_requested: bool,
    away_for: Duration,
    idle_for: Duration,
    policy: &crate::config::PolicyConfig,
) -> bool {
    !lock_requested && away_for >= policy.away_confirm() && idle_for >= policy.idle_confirm()
}

fn should_wake(
    wake_requested: bool,
    present_for: Duration,
    policy: &crate::config::PolicyConfig,
) -> bool {
    !wake_requested && policy.wake_screen && present_for >= policy.present_confirm()
}

async fn connect_presence_proxy(connection: &Connection) -> Result<HumanPresenceProxy<'_>> {
    let mut delay = Duration::from_millis(250);
    loop {
        match HumanPresenceProxy::new(connection).await {
            Ok(proxy) => match proxy.get_state().await {
                Ok(_) => return Ok(proxy),
                Err(error) => debug!(%error, "presence daemon not ready"),
            },
            Err(error) => debug!(%error, "presence proxy unavailable"),
        }
        time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(5));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::config::PolicyConfig;

    use super::{should_lock, should_wake};

    #[test]
    fn lock_requires_both_presence_and_input_deadlines() {
        let policy = PolicyConfig::default();
        assert!(!should_lock(
            false,
            Duration::from_secs(14),
            Duration::from_secs(30),
            &policy
        ));
        assert!(!should_lock(
            false,
            Duration::from_secs(30),
            Duration::from_secs(14),
            &policy
        ));
        assert!(should_lock(
            false,
            Duration::from_secs(15),
            Duration::from_secs(15),
            &policy
        ));
        assert!(!should_lock(
            true,
            Duration::from_secs(30),
            Duration::from_secs(30),
            &policy
        ));
    }

    #[test]
    fn wake_respects_confirmation_and_policy() {
        let mut policy = PolicyConfig::default();
        assert!(!should_wake(false, Duration::from_millis(749), &policy));
        assert!(should_wake(false, Duration::from_millis(750), &policy));
        policy.wake_screen = false;
        assert!(!should_wake(false, Duration::from_secs(5), &policy));
    }
}
