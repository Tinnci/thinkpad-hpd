use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use tokio::{sync::watch, time};
use tracing::{debug, info, warn};
use zbus::Connection;

use crate::{activity, config::Config, screensaver::ScreenController, service::HumanPresenceProxy};

const RUNTIME_DIRECTORY_NAME: &str = "thinkpad-hpd";
const LOCK_MARKER_NAME: &str = "locked-by-agent";

struct LockMarker {
    path: PathBuf,
}

impl LockMarker {
    fn for_current_session() -> Result<Self> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .context("XDG_RUNTIME_DIR is not set")?;
        Self::in_runtime_directory(&PathBuf::from(runtime))
    }

    fn in_runtime_directory(runtime: &Path) -> Result<Self> {
        let directory = runtime.join(RUNTIME_DIRECTORY_NAME);
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "failed to create lock marker directory {}",
                directory.display()
            )
        })?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).with_context(|| {
            format!(
                "failed to secure lock marker directory {}",
                directory.display()
            )
        })?;
        Ok(Self {
            path: directory.join(LOCK_MARKER_NAME),
        })
    }

    #[cfg(test)]
    fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn is_marked(&self) -> bool {
        self.path.is_file()
    }

    fn mark(&self) -> Result<()> {
        fs::write(&self.path, b"locked\n")
            .with_context(|| format!("failed to write lock marker {}", self.path.display()))
    }

    fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("failed to remove lock marker {}", self.path.display())),
        }
    }
}

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
    let lock_marker = LockMarker::for_current_session()?;
    let initially_locked = controller
        .is_locked()
        .await
        .context("failed to read initial screen lock state")?;
    let mut locked_by_hpd = initially_locked && lock_marker.is_marked();
    if !initially_locked {
        lock_marker.clear()?;
    }
    let (activity_tx, activity_rx) = watch::channel(Instant::now());
    let _activity_task =
        should_monitor_input(&config.policy).then(|| activity::start_input_monitor(activity_tx));
    let mut presence_signals = proxy.receive_presence_changed().await?;
    let mut owner_changes = proxy.inner().receive_owner_changed().await?;
    let mut tick = time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    let (mut away_since, mut present_since) =
        presence_deadlines(sensor_available, present, Instant::now());
    let mut lock_requested = false;
    let mut wake_requested = false;
    let mut osd_announced_present = present;
    let mut last_osd_at: Option<Instant> = None;
    let started_at = Instant::now();

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
                let availability_changed = !sensor_available;
                sensor_available = true;
                if availability_changed || args.present != present {
                    present = args.present;
                    info!(present, raw = args.raw, "agent received presence transition");
                    (away_since, present_since) =
                        presence_deadlines(true, present, Instant::now());
                    wake_requested = false;
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
                        (away_since, present_since) =
                            presence_deadlines(available, present, Instant::now());
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
                let presence_stable_for = if present {
                    present_since.map(|start| now.saturating_duration_since(start))
                } else {
                    away_since.map(|start| now.saturating_duration_since(start))
                }.unwrap_or_default();
                if should_show_osd(
                    config.policy.enabled && config.policy.show_osd,
                    osd_announced_present,
                    present,
                    presence_stable_for,
                    last_osd_at.map(|last| now.saturating_duration_since(last)),
                    &config.policy,
                ) {
                    let text = if present {
                        &config.policy.osd_present_text
                    } else {
                        &config.policy.osd_away_text
                    };
                    if config.policy.dry_run {
                        info!(present, %text, "dry-run: would display presence OSD");
                    } else {
                        controller.show_presence_osd(present, text).await;
                    }
                    osd_announced_present = present;
                    last_osd_at = Some(now);
                }
                if !present {
                    let away_for = away_since.map(|start| now.saturating_duration_since(start)).unwrap_or_default();
                    let idle_for = now.saturating_duration_since(last_activity);
                    let running_for = now.saturating_duration_since(started_at);
                    if should_lock(lock_requested, away_for, idle_for, running_for, &config.policy) {
                        let screen_locked = match controller.is_locked().await {
                            Ok(locked) => locked,
                            Err(error) => {
                                warn!(%error, "screen lock state unavailable; skipping away action");
                                continue;
                            }
                        };
                        if !screen_locked {
                            if config.policy.dry_run {
                                info!("dry-run: would request screen lock");
                            } else {
                                lock_marker.mark()?;
                                if let Err(error) = controller.lock().await {
                                    lock_marker.clear()?;
                                    return Err(error);
                                }
                            }
                            lock_requested = true;
                            locked_by_hpd = true;
                            if config.policy.turn_off_screen && !config.policy.dry_run {
                                time::sleep(config.policy.screen_off_delay()).await;
                                controller.turn_off_screen().await;
                            } else if config.policy.turn_off_screen {
                                info!("dry-run: would request display power-off");
                            }
                        } else {
                            lock_requested = true;
                        }
                    } else {
                        debug!(?away_for, ?idle_for, lock_requested, "away policy pending");
                    }
                } else if !wake_requested {
                    let present_for = present_since.map(|start| now.saturating_duration_since(start)).unwrap_or_default();
                    let screen_locked = match controller.is_locked().await {
                        Ok(locked) => locked,
                        Err(error) => {
                            warn!(%error, "screen lock state unavailable; skipping return action");
                            continue;
                        }
                    };
                    if should_wake(wake_requested, locked_by_hpd, present_for, &config.policy)
                        && screen_locked {
                        if config.policy.dry_run {
                            info!("dry-run: would simulate user activity");
                        } else {
                            controller.wake().await?;
                            lock_marker.clear()?;
                        }
                        wake_requested = true;
                        locked_by_hpd = false;
                    } else if !screen_locked {
                        lock_marker.clear()?;
                        locked_by_hpd = false;
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
    running_for: Duration,
    policy: &crate::config::PolicyConfig,
) -> bool {
    policy.enabled
        && policy.lock_screen
        && !lock_requested
        && running_for >= policy.startup_grace()
        && away_for >= policy.away_confirm()
        && idle_for >= policy.idle_confirm()
}

fn should_monitor_input(policy: &crate::config::PolicyConfig) -> bool {
    policy.enabled && policy.lock_screen
}

#[derive(Debug, Serialize)]
pub struct EffectivePolicy {
    pub mode: &'static str,
    pub live: bool,
    pub dry_run: bool,
    pub input_monitor: bool,
    pub lock_screen: bool,
    pub turn_off_screen: bool,
    pub wake_screen: bool,
    pub wake_manual_lock: bool,
    pub show_osd: bool,
}

pub fn effective_policy(
    policy: &crate::config::PolicyConfig,
    screen_off_supported: bool,
) -> EffectivePolicy {
    let lock_screen = policy.enabled && policy.lock_screen;
    let wake_screen = policy.enabled && policy.wake_screen;
    let show_osd = policy.enabled && policy.show_osd;
    let mode = if !policy.enabled {
        "disabled"
    } else if policy.dry_run {
        "dry-run"
    } else {
        match (lock_screen, wake_screen, show_osd) {
            (true, true, _) => "lock-and-wake",
            (true, false, _) => "lock-only",
            (false, true, false) => "wake-only",
            (false, true, true) => "wake-and-osd",
            (false, false, true) => "osd-only",
            (false, false, false) => "monitor-only",
        }
    };
    EffectivePolicy {
        mode,
        live: policy.enabled && !policy.dry_run,
        dry_run: policy.dry_run,
        input_monitor: should_monitor_input(policy),
        lock_screen,
        turn_off_screen: lock_screen && policy.turn_off_screen && screen_off_supported,
        wake_screen,
        wake_manual_lock: wake_screen && policy.wake_manual_lock,
        show_osd,
    }
}

fn presence_deadlines(
    available: bool,
    present: bool,
    now: Instant,
) -> (Option<Instant>, Option<Instant>) {
    if !available {
        (None, None)
    } else if present {
        (None, Some(now))
    } else {
        (Some(now), None)
    }
}

fn should_wake(
    wake_requested: bool,
    locked_by_hpd: bool,
    present_for: Duration,
    policy: &crate::config::PolicyConfig,
) -> bool {
    policy.enabled
        && !wake_requested
        && policy.wake_screen
        && (locked_by_hpd || policy.wake_manual_lock)
        && present_for >= policy.present_confirm()
}

fn should_show_osd(
    enabled: bool,
    announced_present: bool,
    present: bool,
    stable_for: Duration,
    since_last_osd: Option<Duration>,
    policy: &crate::config::PolicyConfig,
) -> bool {
    enabled
        && announced_present != present
        && stable_for >= policy.osd_confirm()
        && since_last_osd.is_none_or(|elapsed| elapsed >= policy.osd_cooldown())
}

#[derive(Debug, Serialize)]
pub struct SimulationDecision {
    pub lock: bool,
    pub wake: bool,
    pub show_osd: bool,
    pub request_screen_off: bool,
    pub screen_off_supported: bool,
    pub dry_run: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn simulate_policy(
    policy: &crate::config::PolicyConfig,
    present: bool,
    stable_for: Duration,
    idle_for: Duration,
    running_for: Duration,
    screen_locked: bool,
    locked_by_hpd: bool,
    osd_announced_present: bool,
    since_last_osd: Option<Duration>,
) -> SimulationDecision {
    let lock = !present && should_lock(false, stable_for, idle_for, running_for, policy);
    let wake = present && screen_locked && should_wake(false, locked_by_hpd, stable_for, policy);
    let show_osd = should_show_osd(
        policy.enabled && policy.show_osd,
        osd_announced_present,
        present,
        stable_for,
        since_last_osd,
        policy,
    );
    let screen_off_supported = crate::screensaver::automatic_screen_off_supported();
    SimulationDecision {
        lock,
        wake,
        show_osd,
        request_screen_off: lock && policy.turn_off_screen && screen_off_supported,
        screen_off_supported,
        dry_run: policy.dry_run,
    }
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
    use std::{os::unix::fs::PermissionsExt, time::Duration};

    use crate::config::PolicyConfig;

    use super::{
        LOCK_MARKER_NAME, LockMarker, RUNTIME_DIRECTORY_NAME, effective_policy, presence_deadlines,
        should_lock, should_monitor_input, should_show_osd, should_wake,
    };

    #[test]
    fn lock_requires_both_presence_and_input_deadlines() {
        let mut policy = PolicyConfig::default();
        assert!(!should_lock(
            false,
            Duration::from_secs(14),
            Duration::from_secs(30),
            Duration::from_secs(30),
            &policy
        ));
        assert!(!should_lock(
            false,
            Duration::from_secs(30),
            Duration::from_secs(14),
            Duration::from_secs(30),
            &policy
        ));
        assert!(should_lock(
            false,
            Duration::from_secs(15),
            Duration::from_secs(15),
            Duration::from_secs(30),
            &policy
        ));
        assert!(!should_lock(
            true,
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(30),
            &policy
        ));
        assert!(!should_lock(
            false,
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(9),
            &policy
        ));
        policy.lock_screen = false;
        assert!(!should_lock(
            false,
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(30),
            &policy
        ));
    }

    #[test]
    fn input_monitor_only_runs_for_enabled_lock_policy() {
        let mut policy = PolicyConfig::default();
        assert!(should_monitor_input(&policy));

        policy.dry_run = false;
        assert!(should_monitor_input(&policy));

        policy.lock_screen = false;
        assert!(!should_monitor_input(&policy));

        policy.lock_screen = true;
        policy.enabled = false;
        assert!(!should_monitor_input(&policy));
    }

    #[test]
    fn effective_policy_reports_wake_only_mode() {
        let policy = PolicyConfig {
            enabled: true,
            dry_run: false,
            lock_screen: false,
            turn_off_screen: true,
            wake_screen: true,
            wake_manual_lock: true,
            show_osd: false,
            ..PolicyConfig::default()
        };
        let effective = effective_policy(&policy, false);
        assert_eq!(effective.mode, "wake-only");
        assert!(effective.live);
        assert!(!effective.input_monitor);
        assert!(!effective.lock_screen);
        assert!(!effective.turn_off_screen);
        assert!(effective.wake_screen);
        assert!(effective.wake_manual_lock);
        assert!(!effective.show_osd);
    }

    #[test]
    fn effective_policy_distinguishes_disabled_and_dry_run() {
        let disabled = PolicyConfig {
            enabled: false,
            ..PolicyConfig::default()
        };
        let effective = effective_policy(&disabled, true);
        assert_eq!(effective.mode, "disabled");
        assert!(!effective.live);
        assert!(!effective.input_monitor);
        assert!(!effective.lock_screen);
        assert!(!effective.wake_screen);
        assert!(!effective.show_osd);

        let dry_run = PolicyConfig::default();
        let effective = effective_policy(&dry_run, true);
        assert_eq!(effective.mode, "dry-run");
        assert!(!effective.live);
        assert!(effective.dry_run);
        assert!(effective.input_monitor);
        assert!(effective.lock_screen);
        assert!(effective.turn_off_screen == dry_run.turn_off_screen);
    }

    #[test]
    fn unavailable_sensor_never_starts_presence_deadlines() {
        let now = std::time::Instant::now();
        assert_eq!(presence_deadlines(false, false, now), (None, None));
        assert_eq!(presence_deadlines(false, true, now), (None, None));
        assert_eq!(presence_deadlines(true, false, now), (Some(now), None));
        assert_eq!(presence_deadlines(true, true, now), (None, Some(now)));
    }

    #[test]
    fn wake_respects_confirmation_and_policy() {
        let mut policy = PolicyConfig::default();
        assert!(!should_wake(
            false,
            true,
            Duration::from_millis(749),
            &policy
        ));
        assert!(should_wake(
            false,
            true,
            Duration::from_millis(750),
            &policy
        ));
        assert!(!should_wake(false, false, Duration::from_secs(5), &policy));
        policy.wake_manual_lock = true;
        assert!(should_wake(false, false, Duration::from_secs(5), &policy));
        policy.wake_screen = false;
        assert!(!should_wake(false, true, Duration::from_secs(5), &policy));
    }

    #[test]
    fn osd_requires_a_stable_new_state() {
        let policy = PolicyConfig::default();
        assert!(!should_show_osd(
            true,
            true,
            false,
            Duration::from_millis(999),
            None,
            &policy
        ));
        assert!(should_show_osd(
            true,
            true,
            false,
            Duration::from_millis(1000),
            None,
            &policy
        ));
        assert!(!should_show_osd(
            true,
            true,
            true,
            Duration::from_secs(5),
            None,
            &policy
        ));
        assert!(!should_show_osd(
            false,
            true,
            false,
            Duration::from_secs(5),
            None,
            &policy
        ));
        assert!(!should_show_osd(
            true,
            true,
            false,
            Duration::from_secs(5),
            Some(Duration::from_secs(4)),
            &policy
        ));
    }

    #[test]
    fn wake_only_mode_never_locks_or_requests_screen_off() {
        let policy = PolicyConfig {
            enabled: true,
            dry_run: false,
            lock_screen: false,
            turn_off_screen: true,
            wake_screen: true,
            wake_manual_lock: true,
            show_osd: false,
            ..PolicyConfig::default()
        };
        let away = super::simulate_policy(
            &policy,
            false,
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(60),
            false,
            false,
            true,
            None,
        );
        assert!(!away.lock);
        assert!(!away.wake);
        assert!(!away.show_osd);
        assert!(!away.request_screen_off);

        let returned = super::simulate_policy(
            &policy,
            true,
            Duration::from_millis(750),
            Duration::ZERO,
            Duration::from_secs(60),
            true,
            false,
            false,
            None,
        );
        assert!(returned.wake);
        assert!(!returned.lock);
        assert!(!returned.request_screen_off);
    }

    #[test]
    fn lock_marker_survives_restart_and_clears_idempotently() {
        let directory = tempfile::tempdir().unwrap();
        let marker = LockMarker::at(directory.path().join("locked-by-agent"));
        assert!(!marker.is_marked());

        marker.mark().unwrap();
        assert!(marker.is_marked());
        assert_eq!(std::fs::read_to_string(&marker.path).unwrap(), "locked\n");

        marker.clear().unwrap();
        marker.clear().unwrap();
        assert!(!marker.is_marked());
    }

    #[test]
    fn lock_marker_uses_a_private_runtime_directory() {
        let runtime = tempfile::tempdir().unwrap();
        let marker = LockMarker::in_runtime_directory(runtime.path()).unwrap();
        let directory = runtime.path().join(RUNTIME_DIRECTORY_NAME);

        assert_eq!(marker.path, directory.join(LOCK_MARKER_NAME));
        assert_eq!(
            std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn systemd_unit_provisions_the_agent_runtime_directory() {
        let unit = include_str!("../packaging/thinkpad-hpd-agent.service");

        assert!(
            unit.lines()
                .any(|line| line == "RuntimeDirectory=thinkpad-hpd")
        );
        assert!(unit.lines().any(|line| line == "RuntimeDirectoryMode=0700"));
        assert!(
            unit.lines()
                .any(|line| line == "RuntimeDirectoryPreserve=yes")
        );
    }
}
