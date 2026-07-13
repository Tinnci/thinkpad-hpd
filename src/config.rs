use std::{fs, path::Path, time::Duration};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub sensor: SensorConfig,
    pub policy: PolicyConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SensorConfig {
    pub sysfs_name: String,
    pub platform_match: String,
    pub present_values: Vec<i32>,
    pub away_values: Vec<i32>,
    pub buffer_length: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    pub away_confirm_seconds: u64,
    pub idle_confirm_seconds: u64,
    pub present_confirm_milliseconds: u64,
    pub turn_off_screen: bool,
    pub wake_screen: bool,
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
            away_confirm_seconds: 15,
            idle_confirm_seconds: 15,
            present_confirm_milliseconds: 750,
            turn_off_screen: true,
            wake_screen: true,
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
}

impl PolicyConfig {
    pub fn away_confirm(&self) -> Duration {
        Duration::from_secs(self.away_confirm_seconds)
    }

    pub fn idle_confirm(&self) -> Duration {
        Duration::from_secs(self.idle_confirm_seconds)
    }

    pub fn present_confirm(&self) -> Duration {
        Duration::from_millis(self.present_confirm_milliseconds)
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
