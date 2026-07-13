use std::{collections::HashMap, path::PathBuf, time::Instant};

use anyhow::{Context, Result};
use evdev::{AbsoluteAxisCode, EventType, KeyCode, RelativeAxisCode};
use futures_util::StreamExt;
use inotify::{Inotify, WatchMask};
use tokio::{sync::watch, task::JoinHandle, time};
use tracing::{debug, info, warn};

const INPUT_DIRECTORY: &str = "/dev/input";

pub fn start_input_monitor(sender: watch::Sender<Instant>) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = run_input_monitor(sender).await {
            warn!(%error, "input activity monitor stopped");
        }
    })
}

async fn run_input_monitor(sender: watch::Sender<Instant>) -> Result<()> {
    let inotify = Inotify::init().context("failed to initialize input hotplug watcher")?;
    inotify
        .watches()
        .add(
            INPUT_DIRECTORY,
            WatchMask::CREATE | WatchMask::DELETE | WatchMask::MOVED_FROM | WatchMask::MOVED_TO,
        )
        .context("failed to watch /dev/input")?;
    let mut hotplug_events = inotify
        .into_event_stream([0_u8; 4096])
        .context("failed to create input hotplug event stream")?;
    let mut monitors = HashMap::new();
    reconcile_devices(&mut monitors, &sender);

    let mut audit = time::interval(std::time::Duration::from_secs(30));
    audit.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            event = hotplug_events.next() => {
                match event {
                    Some(Ok(event)) => {
                        debug!(name = ?event.name, mask = ?event.mask, "input hotplug event");
                        reconcile_devices(&mut monitors, &sender);
                    }
                    Some(Err(error)) => return Err(error).context("input hotplug watcher failed"),
                    None => anyhow::bail!("input hotplug watcher ended"),
                }
            }
            _ = audit.tick() => reconcile_devices(&mut monitors, &sender),
        }
    }
}

fn reconcile_devices(
    monitors: &mut HashMap<PathBuf, JoinHandle<()>>,
    sender: &watch::Sender<Instant>,
) {
    monitors.retain(|path, handle| {
        let keep = path.exists() && !handle.is_finished();
        if !keep {
            handle.abort();
            info!(device = %path.display(), "stopped input activity monitor");
        }
        keep
    });

    for (path, device) in evdev::enumerate() {
        if monitors.contains_key(&path) || !is_user_input(&device) {
            continue;
        }
        let name = device.name().unwrap_or("unnamed input device").to_string();
        info!(device = %path.display(), name, "monitoring input activity");
        let sender = sender.clone();
        monitors.insert(
            path.clone(),
            tokio::spawn(monitor_device(path, device, sender)),
        );
    }
}

fn is_user_input(device: &evdev::Device) -> bool {
    let keys = device.supported_keys();
    let keyboard = keys.is_some_and(|keys| {
        keys.contains(KeyCode::KEY_A)
            && keys.contains(KeyCode::KEY_ENTER)
            && keys.contains(KeyCode::KEY_SPACE)
    });
    let pointer_keys = keys
        .is_some_and(|keys| keys.contains(KeyCode::BTN_LEFT) || keys.contains(KeyCode::BTN_TOUCH));
    let relative_pointer = device.supported_relative_axes().is_some_and(|axes| {
        axes.contains(RelativeAxisCode::REL_X) || axes.contains(RelativeAxisCode::REL_Y)
    });
    let absolute_pointer = device.supported_absolute_axes().is_some_and(|axes| {
        axes.contains(AbsoluteAxisCode::ABS_X) && axes.contains(AbsoluteAxisCode::ABS_Y)
    });
    keyboard || pointer_keys || relative_pointer || absolute_pointer
}

async fn monitor_device(path: PathBuf, device: evdev::Device, sender: watch::Sender<Instant>) {
    let mut stream = match device.into_event_stream() {
        Ok(stream) => stream,
        Err(error) => {
            warn!(device = %path.display(), %error, "failed to open input event stream");
            return;
        }
    };
    loop {
        match stream.next_event().await {
            Ok(event) => {
                if event.event_type() != EventType::SYNCHRONIZATION {
                    sender.send_replace(Instant::now());
                    debug!(device = %path.display(), ?event, "input activity");
                }
            }
            Err(error) => {
                warn!(device = %path.display(), %error, "input event stream ended");
                return;
            }
        }
    }
}
