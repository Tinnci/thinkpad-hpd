# thinkpad-hpd

Native Linux support for the ThinkPad `HID-SENSOR-200011` human-presence
sensor.

The kernel exposes this HID sensor through IIO. On the ThinkPad Z13 Gen 1 the
firmware reports `1` for present and `2` for away, while `iio-sensor-proxy`
expects a continuous proximity value above a configurable threshold. This
daemon therefore consumes the IIO buffer directly and publishes the decoded
state over system D-Bus.

The per-user agent monitors real keyboard and pointer activity through evdev,
locks through KDE's `org.freedesktop.ScreenSaver` interface after a confirmed
away period, and wakes the lock screen when presence returns.

## Commands

```bash
thinkpad-hpd probe
thinkpad-hpd status
thinkpad-hpd monitor
thinkpad-hpd daemon
thinkpad-hpd agent
```

The system daemon must run as root because IIO buffer configuration and
`/dev/iio:device*` access are privileged. The agent runs as the logged-in user.

## Build and install

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
sudo install -Dm755 target/release/thinkpad-hpd /usr/local/bin/thinkpad-hpd
sudo install -Dm644 config/thinkpad-hpd.toml /etc/thinkpad-hpd/config.toml
sudo install -Dm644 packaging/org.thinkpad.HumanPresence1.conf \
  /etc/dbus-1/system.d/org.thinkpad.HumanPresence1.conf
sudo install -Dm644 packaging/thinkpad-hpd.service \
  /usr/lib/systemd/system/thinkpad-hpd.service
sudo install -Dm644 packaging/99-thinkpad-hpd.rules \
  /usr/lib/udev/rules.d/99-thinkpad-hpd.rules
sudo install -Dm644 packaging/thinkpad-hpd-agent.service \
  /usr/lib/systemd/user/thinkpad-hpd-agent.service
sudo systemctl daemon-reload
sudo systemctl enable --now thinkpad-hpd.service
systemctl --user daemon-reload
systemctl --user enable --now thinkpad-hpd-agent.service
```

See [docs/analysis.md](docs/analysis.md) for the hardware protocol, kernel and
`iio-sensor-proxy` compatibility analysis.
