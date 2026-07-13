use anyhow::{Context, Result};
use tracing::{info, warn};
use zbus::{Connection, Proxy};

const SCREENSAVER_SERVICE: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH: &str = "/ScreenSaver";
const SCREENSAVER_INTERFACE: &str = "org.freedesktop.ScreenSaver";

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
}
