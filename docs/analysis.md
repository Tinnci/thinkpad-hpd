# HID-SENSOR-200011 analysis

## Tested system

- Computer: Lenovo ThinkPad Z13 Gen 1
- Distribution: CachyOS
- Kernel: `7.1.2-3-cachyos`
- Desktop: KDE Plasma on Wayland
- Sensor path:
  `/sys/devices/0020:1022:0001.0001/HID-SENSOR-200011.1.auto/iio:device0`
- Kernel module: `hid_sensor_prox`
- IIO device: `/dev/iio:device0`
- IIO name: `prox`
- Scan type: `le:s8/32>>0`

The report payload has one signed 8-bit value in a 32-bit little-endian IIO
storage slot. A buffered sample is therefore four bytes. The tested firmware
reports:

| Raw value | Observed meaning |
| --- | --- |
| `1` | A person is present |
| `2` | The person is away |

These are discrete firmware states, not a distance measurement.

## Implementation languages

The hardware path does not use Zig:

- Linux support is implemented in C in
  `drivers/iio/light/hid-sensor-prox.c`.
- The kernel module is `hid-sensor-prox.ko.zst`, with platform alias
  `HID-SENSOR-200011`.
- `iio-sensor-proxy` is implemented in C using GLib/GUdev.
- `thinkpad-hpd` is the new native Rust userspace daemon and KDE agent.

The kernel driver recognizes HID Human Presence, Human Proximity, and Human
Attention usages. It copies HID input reports into an IIO scan buffer and calls
`iio_push_to_buffers()`, so userspace can block on the IIO character device
instead of polling a sysfs attribute.

## Protocol mismatch

USB HID Usage Tables 1.5 describes the Human Presence field as Boolean: true
when a human is using the computer and false otherwise. This laptop's firmware
instead exposes the enum-like values `1` and `2`. The mapping above was verified
against physical present/away transitions and buffered byte captures.

`iio-sensor-proxy` 3.9 cannot model this device correctly. Its
`drv-iio-poll-proximity.c` driver:

1. Requires a nonzero `PROXIMITY_NEAR_LEVEL` udev property or
   `in_proximity_nearlevel` sysfs attribute.
2. Polls `in_proximity*_raw` every 700 ms.
3. Classifies `prox > near_level` as near, with 0.9/1.1 hysteresis.

No threshold can produce the required mapping where `1` means present and the
larger value `2` means away. Adding a fabricated udev threshold would make the
service start but would invert or otherwise misclassify the hardware state.

Relevant upstream issues, all open on 2026-07-13:

- [#425: Proximity and attention id support for Human Presence detection](https://gitlab.freedesktop.org/hadess/iio-sensor-proxy/-/issues/425)
- [#403: Support buffered proximity sensors](https://gitlab.freedesktop.org/hadess/iio-sensor-proxy/-/issues/403)
- [#361: Wrong near level criterion](https://gitlab.freedesktop.org/hadess/iio-sensor-proxy/-/issues/361)

## Previous service failure

The old `hpd-kde-bridge.sh` implementation read sysfs once per second and used
X11-oriented idle/display commands. On KDE Wayland, `xprintidle` reported that
the screen saver extension was unavailable, `GetSessionIdleTime` returned
`NotSupported`, and the script repeatedly logged `couldn't open display`.
After six days it had consumed about 56 CPU minutes without reliable lock/wake
behavior.

## Native design

The replacement has two privilege-separated Rust processes:

- The root daemon discovers the matching IIO device, enables only its proximity
  scan channel, blocks in `poll(2)` for buffered reports, decodes the declared
  IIO scan type, and publishes state on the system D-Bus.
- The user agent receives D-Bus transitions, reads real keyboard and pointer
  activity from evdev, watches `/dev/input` with inotify for hotplug, and calls
  KDE's `org.freedesktop.ScreenSaver` interface.

System D-Bus API:

- Service/interface: `org.thinkpad.HumanPresence1`
- Object: `/org/thinkpad/HumanPresence1`
- Method: `GetState() -> (available, present, raw)`
- Signal: `PresenceChanged(available, present, raw)`

The default policy requires both 15 seconds of confirmed absence and 15 seconds
without actual input before locking. Presence must remain stable for 750 ms
before waking the lock screen. Wake only simulates activity; it does not unlock
the session or bypass authentication. Presence transitions are also shown using
KDE's native `org.kde.osdService` after a separate 1-second debounce.

The IIO reader uses a bounded poll timeout so SIGTERM can stop it cleanly. Its
RAII cleanup disables both `buffer0/enable` and the scan element before the
daemon exits.

## Validation results

Validation performed on 2026-07-13:

- `cargo test`: 5 passed, 0 failed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- Optimized release build: passed.
- SIGTERM cleanup: process stopped cleanly; buffer and scan enable both became
  `0`.
- Input activity: keyboard, mouse, touchpad, TrackPoint, and virtual mouse
  devices were opened; a generated relative mouse event reset idle duration.
- Hotplug: inotify observed `/dev/input` create and delete events.
- D-Bus loss: agent paused policy when the daemon stopped and refreshed
  `available=true, present=false, raw=2` when it returned.
- KDE wake call: `SimulateUserActivity` left `GetActive=true`, confirming that
  it wakes the lock screen without unlocking it.
- systemd security exposure: system daemon `1.5 OK`; user agent `1.3 OK`.
- Idle resource use after startup: approximately 2 MiB resident memory per
  process and under 120 ms accumulated CPU during the validation window.

Suspend/resume logic was exercised through equivalent IIO teardown and daemon
reconnection. A full machine suspend or reboot was not forced during the
interactive installation.

## Source references

- Linux kernel: `drivers/iio/light/hid-sensor-prox.c`
- iio-sensor-proxy 3.9 commit:
  `0085ddf8ecb173a1c5fcf2344aa40e561125354f`
- iio-sensor-proxy proximity driver: `src/drv-iio-poll-proximity.c`
- USB-IF HID Usage Tables 1.5, Sensors page, Human Presence data field
