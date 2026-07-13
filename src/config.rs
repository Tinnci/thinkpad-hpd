use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub sensor: SensorConfig,
    pub policy: PolicyConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct SensorConfig {
    pub sysfs_name: String,
    pub platform_match: String,
    pub present_values: Vec<i32>,
    pub away_values: Vec<i32>,
    pub buffer_length: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct PolicyConfig {
    pub enabled: bool,
    pub dry_run: bool,
    pub lock_screen: bool,
    pub away_confirm_seconds: u64,
    pub idle_confirm_seconds: u64,
    pub startup_grace_seconds: u64,
    pub present_confirm_milliseconds: u64,
    pub osd_confirm_milliseconds: u64,
    pub turn_off_screen: bool,
    pub screen_off_delay_milliseconds: u64,
    pub wake_screen: bool,
    pub wake_manual_lock: bool,
    pub show_osd: bool,
    pub osd_cooldown_seconds: u64,
    pub osd_present_text: String,
    pub osd_away_text: String,
}

impl Default for SensorConfig {
    fn default() -> Self {
        Self {
            sysfs_name: "prox".to_string(),
            platform_match: "HID-SENSOR-200011".to_string(),
            present_values: vec![1],
            away_values: vec![2],
            buffer_length: 16,
        }
    }
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dry_run: true,
            lock_screen: true,
            away_confirm_seconds: 15,
            idle_confirm_seconds: 15,
            startup_grace_seconds: 10,
            present_confirm_milliseconds: 750,
            osd_confirm_milliseconds: 1000,
            turn_off_screen: false,
            screen_off_delay_milliseconds: 750,
            wake_screen: true,
            wake_manual_lock: false,
            show_osd: true,
            osd_cooldown_seconds: 5,
            osd_present_text: "HPD: 检测到用户".to_string(),
            osd_away_text: "HPD: 用户已离开".to_string(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn load_for_agent(system_path: &Path) -> Result<Self> {
        let mut config = Self::load(system_path)?;
        let user_path = Self::user_path();
        if user_path.exists() {
            let text = fs::read_to_string(&user_path)
                .with_context(|| format!("failed to read {}", user_path.display()))?;
            let user: UserConfig = toml::from_str(&text)
                .with_context(|| format!("failed to parse {}", user_path.display()))?;
            config.policy = user.policy;
        }
        config.validate()?;
        Ok(config)
    }

    pub fn user_path() -> PathBuf {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".config")
            })
            .join("thinkpad-hpd/config.toml")
    }

    pub fn save_user_policy(policy: &PolicyConfig) -> Result<()> {
        policy.validate()?;
        let path = Self::user_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(&UserConfig {
            policy: policy.clone(),
        })?;
        let temporary = path.with_extension("toml.tmp");
        fs::write(&temporary, text)?;
        fs::rename(&temporary, &path)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        self.policy.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UserConfig {
    policy: PolicyConfig,
}

impl PolicyConfig {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            (1..=3600).contains(&self.away_confirm_seconds),
            "away confirmation must be between 1 and 3600 seconds"
        );
        anyhow::ensure!(
            self.idle_confirm_seconds <= 3600,
            "idle confirmation must be between 0 and 3600 seconds"
        );
        anyhow::ensure!(
            self.startup_grace_seconds <= 600,
            "startup grace must be between 0 and 600 seconds"
        );
        anyhow::ensure!(
            (100..=60_000).contains(&self.present_confirm_milliseconds),
            "presence confirmation must be between 100 and 60000 milliseconds"
        );
        anyhow::ensure!(
            (100..=60_000).contains(&self.osd_confirm_milliseconds),
            "OSD confirmation must be between 100 and 60000 milliseconds"
        );
        anyhow::ensure!(
            self.screen_off_delay_milliseconds <= 60_000,
            "screen-off delay must be between 0 and 60000 milliseconds"
        );
        anyhow::ensure!(
            self.osd_cooldown_seconds <= 300,
            "OSD cooldown must be between 0 and 300 seconds"
        );
        anyhow::ensure!(
            self.osd_present_text.len() <= 120 && self.osd_away_text.len() <= 120,
            "OSD text must not exceed 120 bytes"
        );
        Ok(())
    }

    pub fn away_confirm(&self) -> Duration {
        Duration::from_secs(self.away_confirm_seconds)
    }

    pub fn idle_confirm(&self) -> Duration {
        Duration::from_secs(self.idle_confirm_seconds)
    }

    pub fn startup_grace(&self) -> Duration {
        Duration::from_secs(self.startup_grace_seconds)
    }

    pub fn screen_off_delay(&self) -> Duration {
        Duration::from_millis(self.screen_off_delay_milliseconds)
    }

    pub fn present_confirm(&self) -> Duration {
        Duration::from_millis(self.present_confirm_milliseconds)
    }

    pub fn osd_confirm(&self) -> Duration {
        Duration::from_millis(self.osd_confirm_milliseconds)
    }

    pub fn osd_cooldown(&self) -> Duration {
        Duration::from_secs(self.osd_cooldown_seconds)
    }
}

impl SensorConfig {
    pub fn classify(&self, raw: i32) -> Option<bool> {
        if self.present_values.contains(&raw) {
            Some(true)
        } else if self.away_values.contains(&raw) {
            Some(false)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SensorConfig;

    #[test]
    fn default_mapping_matches_thinkpad_firmware() {
        let config = SensorConfig::default();
        assert_eq!(config.classify(1), Some(true));
        assert_eq!(config.classify(2), Some(false));
        assert_eq!(config.classify(0), None);
    }
}
