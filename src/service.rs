use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use zbus::{connection, object_server::SignalEmitter};

use crate::{
    config::Config,
    iio::{IioBuffer, SensorPaths},
};

pub const SERVICE_NAME: &str = "org.thinkpad.HumanPresence1";
pub const OBJECT_PATH: &str = "/org/thinkpad/HumanPresence1";

#[derive(Clone, Copy, Debug, Default)]
struct PresenceState {
    available: bool,
    present: bool,
    raw: i32,
}

#[derive(Clone)]
struct HumanPresence {
    state: Arc<RwLock<PresenceState>>,
}

#[zbus::interface(name = "org.thinkpad.HumanPresence1")]
impl HumanPresence {
    fn get_state(&self) -> (bool, bool, i32) {
        let state = *self.state.read().expect("presence state lock poisoned");
        (state.available, state.present, state.raw)
    }

    #[zbus(property)]
    fn available(&self) -> bool {
        self.state
            .read()
            .expect("presence state lock poisoned")
            .available
    }

    #[zbus(property)]
    fn present(&self) -> bool {
        self.state
            .read()
            .expect("presence state lock poisoned")
            .present
    }

    #[zbus(property)]
    fn raw_value(&self) -> i32 {
        self.state.read().expect("presence state lock poisoned").raw
    }

    #[zbus(signal)]
    async fn presence_changed(
        emitter: &SignalEmitter<'_>,
        available: bool,
        present: bool,
        raw: i32,
    ) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.thinkpad.HumanPresence1",
    default_service = "org.thinkpad.HumanPresence1",
    default_path = "/org/thinkpad/HumanPresence1"
)]
pub trait HumanPresence {
    fn get_state(&self) -> zbus::Result<(bool, bool, i32)>;

    #[zbus(signal)]
    fn presence_changed(&self, available: bool, present: bool, raw: i32) -> zbus::Result<()>;
}

pub async fn run_daemon(config: Config) -> Result<()> {
    let sensor = SensorPaths::discover(&config.sensor)?;
    let initial_raw = sensor.read_current()?;
    let initial_present = config.sensor.classify(initial_raw).unwrap_or(false);
    info!(
        sysfs = %sensor.sysfs_dir.display(),
        device = %sensor.dev_path.display(),
        scan_type = %sensor.scan_type.raw,
        initial_raw,
        initial_present,
        "discovered HID human-presence sensor"
    );

    let state = Arc::new(RwLock::new(PresenceState {
        available: true,
        present: initial_present,
        raw: initial_raw,
    }));
    let interface = HumanPresence {
        state: Arc::clone(&state),
    };
    let connection = connection::Builder::system()?
        .name(SERVICE_NAME)?
        .serve_at(OBJECT_PATH, interface)?
        .build()
        .await
        .context("failed to publish system D-Bus service")?;
    let interface_ref = connection
        .object_server()
        .interface::<_, HumanPresence>(OBJECT_PATH)
        .await?;
    HumanPresence::presence_changed(
        interface_ref.signal_emitter(),
        true,
        initial_present,
        initial_raw,
    )
    .await?;

    let running = Arc::new(AtomicBool::new(true));
    let reader_running = Arc::clone(&running);
    let (sample_tx, mut sample_rx) = mpsc::channel::<Result<i32>>(32);
    let buffer_length = config.sensor.buffer_length;
    let reader = tokio::task::spawn_blocking(move || {
        let mut buffer = match IioBuffer::open(sensor, buffer_length) {
            Ok(buffer) => buffer,
            Err(error) => {
                let _ = sample_tx.blocking_send(Err(error));
                return;
            }
        };
        while reader_running.load(Ordering::Acquire) {
            match buffer.read_sample_interruptible(&reader_running) {
                Ok(Some(raw)) => {
                    if sample_tx.blocking_send(Ok(raw)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = sample_tx.blocking_send(Err(error));
                    break;
                }
            }
        }
    });

    let run_result: Result<()> = async {
        let mut last_classified = config.sensor.classify(initial_raw);
        loop {
            tokio::select! {
                sample = sample_rx.recv() => {
                    let Some(sample) = sample else { break; };
                    let raw = sample.context("IIO reader failed")?;
                    let Some(present) = config.sensor.classify(raw) else {
                        warn!(raw, "ignoring unmapped presence value");
                        continue;
                    };
                    let changed = {
                        let mut current = state.write().expect("presence state lock poisoned");
                        let changed = current.raw != raw || Some(present) != last_classified;
                        current.available = true;
                        current.raw = raw;
                        current.present = present;
                        changed
                    };
                    if changed {
                        info!(raw, present, "presence state changed");
                        {
                            let interface = interface_ref.get().await;
                            interface
                                .available_changed(interface_ref.signal_emitter())
                                .await?;
                            interface
                                .present_changed(interface_ref.signal_emitter())
                                .await?;
                            interface
                                .raw_value_changed(interface_ref.signal_emitter())
                                .await?;
                        }
                        HumanPresence::presence_changed(
                            interface_ref.signal_emitter(),
                            true,
                            present,
                            raw,
                        ).await?;
                    } else {
                        debug!(raw, present, "presence sample");
                    }
                    last_classified = Some(present);
                }
                _ = shutdown_signal() => {
                    info!("shutdown requested");
                    break;
                }
            }
        }
        Ok(())
    }
    .await;

    running.store(false, Ordering::Release);
    let reader_result = time_out_reader(reader).await;
    drop(connection);
    run_result?;
    reader_result
}

async fn time_out_reader(reader: tokio::task::JoinHandle<()>) -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(2), reader)
        .await
        .context("timed out waiting for IIO reader")?
        .context("IIO reader task failed")?;
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
