# thinkpad-hpd

Native Linux support for the ThinkPad `HID-SENSOR-200011` human-presence
sensor.

The kernel exposes this HID sensor through IIO. On the ThinkPad Z13 Gen 1 the
firmware reports `1` for present and `2` for away, while `iio-sensor-proxy`
expects a continuous proximity value above a configurable threshold. This
daemon therefore consumes the IIO buffer directly and publishes the decoded
state over system D-Bus.

The per-user agent locks and wakes through the standard
`org.freedesktop.ScreenSaver` interface. It monitors keyboard and pointer
activity through evdev only when away locking is enabled. Plasma OSD and
PowerDevil integration are optional desktop enhancements.

## Commands

```bash
thinkpad-hpd probe
thinkpad-hpd status
thinkpad-hpd monitor
thinkpad-hpd daemon
thinkpad-hpd agent
thinkpad-hpd settings get
thinkpad-hpd diagnose
thinkpad-hpd simulate --present true --screen-locked true --locked-by-hpd false
```

The system daemon must run as root because IIO buffer configuration and
`/dev/iio:device*` access are privileged. The agent runs as the logged-in user.
Per-user policy overrides are stored in
`$XDG_CONFIG_HOME/thinkpad-hpd/config.toml`. This interface is desktop
independent; the optional KDE System Settings module in `kcm/` is only a thin
frontend over the same Rust CLI. Lock and wake use the standard
`org.freedesktop.ScreenSaver` D-Bus interface. Plasma OSD and PowerDevil screen
off support are optional enhancements.

The IIO udev rule starts the system daemon when compatible hardware appears,
so the system unit can be active while its install state is `disabled`. Mask
the unit to prevent hardware activation. The user agent is managed separately
with `systemctl --user`.

Automatic display power-off defaults to disabled. It is forcibly blocked on
AMDGPU Wayland systems where DMCUB/pageflip failures have been observed.
`dry_run` evaluates and logs policy decisions without controlling the desktop.

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
sudo systemctl start thinkpad-hpd.service
systemctl --user daemon-reload
systemctl --user enable --now thinkpad-hpd-agent.service
```

To install the optional Plasma 6 settings module:

```bash
cmake -S kcm -B build-kcm -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build-kcm
sudo cmake --install build-kcm
```

The KCM uses KDE KI18n/gettext. English source strings and Simplified Chinese
translations are included. Additional languages can be added as
`kcm/po/<locale>/kcm_thinkpadhpd.po`.

See [docs/analysis.md](docs/analysis.md) for the hardware protocol, kernel and
`iio-sensor-proxy` compatibility analysis.
