use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::fd::AsFd,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, anyhow, bail};
use nix::poll::{PollFd, PollFlags, poll};

use crate::config::SensorConfig;

#[derive(Clone, Debug)]
pub struct ScanType {
    pub raw: String,
    little_endian: bool,
    signed: bool,
    real_bits: u8,
    storage_bits: u8,
    shift: u8,
}

#[derive(Clone, Debug)]
pub struct SensorPaths {
    pub sysfs_dir: PathBuf,
    pub dev_path: PathBuf,
    pub raw_path: PathBuf,
    buffer_enable: PathBuf,
    buffer_length: PathBuf,
    scan_enable: PathBuf,
    pub scan_type: ScanType,
}

pub struct IioBuffer {
    sensor: SensorPaths,
    file: File,
    sample: Vec<u8>,
}

impl ScanType {
    pub fn parse(text: &str) -> Result<Self> {
        let raw = text.trim().to_string();
        let (endian, rest) = raw
            .split_once(':')
            .ok_or_else(|| anyhow!("invalid IIO scan type: {raw}"))?;
        let little_endian = match endian {
            "le" => true,
            "be" => false,
            _ => bail!("unsupported IIO endianness in {raw}"),
        };
        let signed = match rest.as_bytes().first() {
            Some(b's') => true,
            Some(b'u') => false,
            _ => bail!("unsupported IIO signedness in {raw}"),
        };
        let rest = &rest[1..];
        let (bits, shift) = rest
            .split_once(">>")
            .ok_or_else(|| anyhow!("missing IIO shift in {raw}"))?;
        let (real_bits, storage_bits) = bits
            .split_once('/')
            .ok_or_else(|| anyhow!("missing IIO storage width in {raw}"))?;
        let real_bits = real_bits.parse::<u8>()?;
        let storage_bits = storage_bits.parse::<u8>()?;
        let shift = shift.parse::<u8>()?;
        if storage_bits == 0 || storage_bits % 8 != 0 || storage_bits > 64 {
            bail!("unsupported IIO storage width in {raw}");
        }
        if real_bits == 0 || real_bits > storage_bits || shift >= storage_bits {
            bail!("invalid IIO bit layout in {raw}");
        }
        Ok(Self {
            raw,
            little_endian,
            signed,
            real_bits,
            storage_bits,
            shift,
        })
    }

    pub fn storage_bytes(&self) -> usize {
        usize::from(self.storage_bits / 8)
    }

    pub fn decode(&self, bytes: &[u8]) -> Result<i32> {
        if bytes.len() < self.storage_bytes() {
            bail!("short IIO sample");
        }
        let mut padded = [0_u8; 8];
        let width = self.storage_bytes();
        if self.little_endian {
            padded[..width].copy_from_slice(&bytes[..width]);
        } else {
            padded[8 - width..].copy_from_slice(&bytes[..width]);
        }
        let storage = if self.little_endian {
            u64::from_le_bytes(padded)
        } else {
            u64::from_be_bytes(padded)
        };
        let mask = if self.real_bits == 64 {
            u64::MAX
        } else {
            (1_u64 << self.real_bits) - 1
        };
        let value = (storage >> self.shift) & mask;
        if self.signed {
            let sign = 1_u64 << (self.real_bits - 1);
            let signed = if value & sign != 0 {
                (value | !mask) as i64
            } else {
                value as i64
            };
            i32::try_from(signed).context("IIO sample does not fit in i32")
        } else {
            i32::try_from(value).context("IIO sample does not fit in i32")
        }
    }
}

impl SensorPaths {
    pub fn discover(config: &SensorConfig) -> Result<Self> {
        let root = Path::new("/sys/bus/iio/devices");
        for entry in fs::read_dir(root).context("failed to enumerate IIO devices")? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if !name.starts_with("iio:device") {
                continue;
            }
            let sysfs_dir = entry.path();
            let sensor_name = fs::read_to_string(sysfs_dir.join("name")).unwrap_or_default();
            if sensor_name.trim() != config.sysfs_name {
                continue;
            }
            let canonical = fs::canonicalize(&sysfs_dir)?;
            if !canonical.to_string_lossy().contains(&config.platform_match) {
                continue;
            }

            let scan_type_path = sysfs_dir.join("scan_elements/in_proximity0_type");
            let scan_type = ScanType::parse(
                &fs::read_to_string(&scan_type_path)
                    .with_context(|| format!("failed to read {}", scan_type_path.display()))?,
            )?;
            return Ok(Self {
                dev_path: PathBuf::from("/dev").join(&*name),
                raw_path: sysfs_dir.join("in_proximity0_raw"),
                buffer_enable: sysfs_dir.join("buffer0/enable"),
                buffer_length: sysfs_dir.join("buffer0/length"),
                scan_enable: sysfs_dir.join("scan_elements/in_proximity0_en"),
                scan_type,
                sysfs_dir,
            });
        }
        bail!(
            "no IIO sensor named '{}' under platform '{}'",
            config.sysfs_name,
            config.platform_match
        )
    }

    pub fn read_current(&self) -> Result<i32> {
        let value = fs::read_to_string(&self.raw_path)
            .with_context(|| format!("failed to read {}", self.raw_path.display()))?;
        value
            .trim()
            .parse::<i32>()
            .with_context(|| format!("invalid value in {}", self.raw_path.display()))
    }
}

impl IioBuffer {
    pub fn open(sensor: SensorPaths, length: u32) -> Result<Self> {
        write_sysfs(&sensor.buffer_enable, "0")?;
        write_sysfs(&sensor.scan_enable, "1")?;
        write_sysfs(&sensor.buffer_length, &length.to_string())?;
        if let Err(error) = write_sysfs(&sensor.buffer_enable, "1") {
            let _ = write_sysfs(&sensor.scan_enable, "0");
            return Err(error);
        }
        let file = match OpenOptions::new().read(true).open(&sensor.dev_path) {
            Ok(file) => file,
            Err(error) => {
                let _ = write_sysfs(&sensor.buffer_enable, "0");
                let _ = write_sysfs(&sensor.scan_enable, "0");
                return Err(error)
                    .with_context(|| format!("failed to open {}", sensor.dev_path.display()));
            }
        };
        let sample = vec![0; sensor.scan_type.storage_bytes()];
        Ok(Self {
            sensor,
            file,
            sample,
        })
    }

    pub fn read_sample_interruptible(&mut self, running: &Arc<AtomicBool>) -> Result<Option<i32>> {
        while running.load(Ordering::Acquire) {
            let mut descriptors = [PollFd::new(self.file.as_fd(), PollFlags::POLLIN)];
            let ready = poll(&mut descriptors, 250_u16).context("IIO poll failed")?;
            if ready == 0 {
                continue;
            }
            let events = descriptors[0].revents().unwrap_or_else(PollFlags::empty);
            if events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
                bail!("IIO device reported poll error: {events:?}");
            }
            if events.contains(PollFlags::POLLIN) {
                return self.read_sample().map(Some);
            }
        }
        Ok(None)
    }

    fn read_sample(&mut self) -> Result<i32> {
        self.file.read_exact(&mut self.sample).or_else(|error| {
            if error.kind() == io::ErrorKind::Interrupted {
                self.file.read_exact(&mut self.sample)
            } else {
                Err(error)
            }
        })?;
        self.sensor.scan_type.decode(&self.sample)
    }
}

impl Drop for IioBuffer {
    fn drop(&mut self) {
        let _ = write_sysfs(&self.sensor.buffer_enable, "0");
        let _ = write_sysfs(&self.sensor.scan_enable, "0");
    }
}

fn write_sysfs(path: &Path, value: &str) -> Result<()> {
    fs::write(path, value).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::ScanType;

    #[test]
    fn decodes_thinkpad_scan_format() {
        let scan = ScanType::parse("le:s8/32>>0\n").unwrap();
        assert_eq!(scan.storage_bytes(), 4);
        assert_eq!(scan.decode(&[1, 0, 0, 0]).unwrap(), 1);
        assert_eq!(scan.decode(&[2, 0, 0, 0]).unwrap(), 2);
        assert_eq!(scan.decode(&[0xff, 0, 0, 0]).unwrap(), -1);
    }

    #[test]
    fn rejects_short_samples() {
        let scan = ScanType::parse("le:s8/32>>0").unwrap();
        assert!(scan.decode(&[1]).is_err());
    }
}
