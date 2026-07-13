use std::{
    env, fs,
    io::Write,
    os::unix::fs::PermissionsExt,
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
        let config = if path.exists() {
            let text = fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?
        } else {
            Self::default()
        };
        config.validate()?;
        Ok(config)
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
        Self::save_user_policy_at(&path, policy)
    }

    fn save_user_policy_at(path: &Path, policy: &PolicyConfig) -> Result<()> {
        let parent = path.parent().context("user policy path has no parent")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to protect {}", parent.display()))?;
        let text = toml::to_string_pretty(&UserConfig {
            policy: policy.clone(),
        })?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        temporary.write_all(text.as_bytes())?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        self.sensor.validate()?;
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
            self.osd_present_text.chars().count() <= 120
                && self.osd_away_text.chars().count() <= 120,
            "OSD text must not exceed 120 characters"
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
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.sysfs_name.trim().is_empty(),
            "sensor sysfs name must not be empty"
        );
        anyhow::ensure!(
            !self.platform_match.trim().is_empty(),
            "sensor platform match must not be empty"
        );
        anyhow::ensure!(
            !self.present_values.is_empty(),
            "sensor present values must not be empty"
        );
        anyhow::ensure!(
            !self.away_values.is_empty(),
            "sensor away values must not be empty"
        );
        anyhow::ensure!(
            self.present_values
                .iter()
                .all(|value| !self.away_values.contains(value)),
            "sensor present and away values must not overlap"
        );
        anyhow::ensure!(
            (2..=4096).contains(&self.buffer_length),
            "sensor buffer length must be between 2 and 4096"
        );
        Ok(())
    }

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
    use std::fs;

    use super::{Config, PolicyConfig, SensorConfig};

    #[test]
    fn default_mapping_matches_thinkpad_firmware() {
        let config = SensorConfig::default();
        assert_eq!(config.classify(1), Some(true));
        assert_eq!(config.classify(2), Some(false));
        assert_eq!(config.classify(0), None);
    }

    #[test]
    fn rejects_overlapping_sensor_mappings() {
        let mut config = SensorConfig::default();
        config.away_values.push(1);
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "sensor present and away values must not overlap"
        );
    }

    #[test]
    fn rejects_empty_sensor_mappings_and_invalid_buffer_lengths() {
        let mut config = SensorConfig::default();
        config.present_values.clear();
        assert!(config.validate().is_err());

        let config = SensorConfig {
            buffer_length: 1,
            ..SensorConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn system_config_is_validated_when_loaded() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            r#"
[sensor]
present_values = [1]
away_values = [1]
"#,
        )
        .unwrap();
        assert_eq!(
            Config::load(&path).unwrap_err().to_string(),
            "sensor present and away values must not overlap"
        );
    }

    #[test]
    fn osd_text_limit_counts_unicode_characters() {
        let valid = PolicyConfig {
            osd_present_text: "人".repeat(120),
            osd_away_text: "离".repeat(120),
            ..PolicyConfig::default()
        };
        assert!(valid.validate().is_ok());

        let invalid = PolicyConfig {
            osd_present_text: "人".repeat(121),
            ..PolicyConfig::default()
        };
        assert_eq!(
            invalid.validate().unwrap_err().to_string(),
            "OSD text must not exceed 120 characters"
        );
    }

    #[test]
    fn saved_user_policy_is_private_and_atomically_replaced() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("thinkpad-hpd");
        let path = parent.join("config.toml");
        Config::save_user_policy_at(&path, &PolicyConfig::default()).unwrap();

        assert_eq!(
            fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_dir(&parent).unwrap().count(),
            1,
            "temporary files must not remain after a successful save"
        );

        let replacement = PolicyConfig {
            enabled: false,
            ..PolicyConfig::default()
        };
        Config::save_user_policy_at(&path, &replacement).unwrap();
        let saved: super::UserConfig = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(!saved.policy.enabled);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
