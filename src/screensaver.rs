use std::{env, fs};

use anyhow::{Context, Result};
use tracing::{info, warn};
use zbus::{Connection, Proxy};

const SCREENSAVER_SERVICE: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH: &str = "/ScreenSaver";
const SCREENSAVER_INTERFACE: &str = "org.freedesktop.ScreenSaver";

const PLASMASHELL_SERVICE: &str = "org.kde.plasmashell";
const OSD_PATH: &str = "/org/kde/osdService";
const OSD_INTERFACE: &str = "org.kde.osdService";

const GLOBAL_ACCEL_SERVICE: &str = "org.kde.kglobalaccel";
const POWERDEVIL_PATH: &str = "/component/org_kde_powerdevil";
const POWERDEVIL_INTERFACE: &str = "org.kde.kglobalaccel.Component";

pub struct ScreenController {
    connection: Connection,
}

impl ScreenController {
    pub async fn connect() -> Result<Self> {
        Ok(Self {
            connection: Connection::session()
                .await
                .context("failed to connect to the user D-Bus")?,
        })
    }

    async fn screensaver(&self) -> Result<Proxy<'_>> {
        Proxy::new(
            &self.connection,
            SCREENSAVER_SERVICE,
            SCREENSAVER_PATH,
            SCREENSAVER_INTERFACE,
        )
        .await
        .context("failed to create ScreenSaver proxy")
    }

    pub async fn is_locked(&self) -> Result<bool> {
        let proxy = self.screensaver().await?;
        proxy
            .call("GetActive", &())
            .await
            .context("ScreenSaver.GetActive failed")
    }

    pub async fn lock(&self) -> Result<()> {
        info!("requesting KDE screen lock");
        let proxy = self.screensaver().await?;
        let _: () = proxy
            .call("Lock", &())
            .await
            .context("ScreenSaver.Lock failed")?;
        Ok(())
    }

    pub async fn wake(&self) -> Result<()> {
        info!("requesting KDE user-activity simulation");
        let proxy = self.screensaver().await?;
        let _: () = proxy
            .call("SimulateUserActivity", &())
            .await
            .context("ScreenSaver.SimulateUserActivity failed")?;
        Ok(())
    }

    pub async fn turn_off_screen(&self) {
        if !automatic_screen_off_supported() {
            warn!(
                "refusing automatic display power-off on AMDGPU Wayland due to pageflip/DMCUB crash risk"
            );
            return;
        }
        let result = async {
            let proxy = Proxy::new(
                &self.connection,
                GLOBAL_ACCEL_SERVICE,
                POWERDEVIL_PATH,
                POWERDEVIL_INTERFACE,
            )
            .await?;
            let _: () = proxy.call("invokeShortcut", &("Turn Off Screen",)).await?;
            Ok::<(), zbus::Error>(())
        }
        .await;
        if let Err(error) = result {
            warn!(%error, "failed to invoke the KDE Turn Off Screen shortcut");
        }
    }

    pub async fn show_presence_osd(&self, present: bool, text: &str) {
        info!(present, "displaying KDE presence OSD");
        let icon = if present {
            "preferences-desktop-user"
        } else {
            "system-lock-screen"
        };
        let result = async {
            let proxy = Proxy::new(
                &self.connection,
                PLASMASHELL_SERVICE,
                OSD_PATH,
                OSD_INTERFACE,
            )
            .await?;
            let _: () = proxy.call("showText", &(icon, text)).await?;
            Ok::<(), zbus::Error>(())
        }
        .await;
        if let Err(error) = result {
            warn!(%error, "failed to display the KDE presence OSD");
        }
    }
}

pub fn automatic_screen_off_supported() -> bool {
    automatic_screen_off_block_reason().is_none()
}

pub fn automatic_screen_off_block_reason() -> Option<&'static str> {
    if env::var("XDG_SESSION_TYPE")
        .map(|value| value != "wayland")
        .unwrap_or(true)
    {
        return None;
    }
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return None;
    };
    let amd_gpu = entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with("card")
            && !name.contains('-')
            && fs::read_to_string(entry.path().join("device/vendor"))
                .map(|vendor| vendor.trim().eq_ignore_ascii_case("0x1002"))
                .unwrap_or(false)
    });
    amd_gpu.then_some("automatic display power-off is blocked on AMDGPU Wayland after observed DMCUB/pageflip failures")
}

#[cfg(test)]
mod tests {
    #[test]
    fn screen_off_guard_is_conservative_without_wayland_environment() {
        unsafe { std::env::remove_var("XDG_SESSION_TYPE") };
        assert!(super::automatic_screen_off_supported());
    }
}
