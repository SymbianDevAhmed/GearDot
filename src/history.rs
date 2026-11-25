//! Local battery history and conservative discharge estimates.
use crate::battery::{BatteryReading, DeviceKind};
use std::{fs, io, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

const RETENTION: u64 = 48 * 3600;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub at: u64,
    pub percent: u8,
    pub millivolts: u16,
    pub charging: Option<bool>,
}

#[derive(Default)]
pub struct BatteryHistory {
    samples: Vec<Sample>,
    device: DeviceKind,
}

impl BatteryHistory {
    pub fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    fn path(device: DeviceKind) -> io::Result<PathBuf> {
        let base = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA unavailable"))?;
        Ok(PathBuf::from(base).join("G435Battery").join(match device { DeviceKind::Headset => "history.csv", DeviceKind::Mouse => "mad-r-history.csv" }))
    }

    pub fn load(device: DeviceKind) -> Self {
        let data = Self::path(device).and_then(fs::read_to_string).unwrap_or_default();
        let now = Self::now();
        let mut samples: Vec<_> = data.lines().filter_map(|line| {
            let mut parts = line.split(',');
            let at: u64 = parts.next()?.parse().ok()?;
            let percent: u8 = parts.next()?.parse().ok()?;
            let millivolts: u16 = parts.next()?.parse().ok()?;
            if !(2500..=4500).contains(&millivolts) || percent > 100 ||
                at > now || now - at > RETENTION { return None; }
            let charging = match parts.next() { Some("1") => Some(true), Some("0") => Some(false), _ => None };
            Some(Sample { at, percent, millivolts, charging })
        }).collect();
        samples.sort_by_key(|s| s.at);
        Self { samples, device }
    }

    pub fn record(&mut self, reading: BatteryReading) -> io::Result<()> {
        let now = Self::now();
        self.samples.retain(|s| s.at <= now && now - s.at <= RETENTION);
        if self.samples.last().is_some_and(|s| now.saturating_sub(s.at) < 60 && s.charging == reading.charging()) {
            return Ok(());
        }
        self.samples.push(Sample { at: now, percent: reading.percentage(), millivolts: reading.millivolts(), charging: reading.charging() });
        let path = Self::path(self.device)?;
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        let csv: String = self.samples.iter()
            .map(|s| format!("{},{},{},{}\n", s.at, s.percent, s.millivolts, match s.charging { Some(true) => "1", Some(false) => "0", None => "" })).collect();
        fs::write(path, csv)
    }

    pub fn samples(&self) -> &[Sample] { &self.samples }
    pub fn latest(&self) -> Option<Sample> { self.samples.last().copied() }

    /// Estimate from a recent uninterrupted falling trend only.
    pub fn hours_remaining(&self, now: u64) -> Option<f64> {
        let last = self.latest()?;
        if last.charging == Some(true) || now < last.at || now - last.at > 180 { return None; }
        let mut segment = vec![last];
        for older in self.samples.iter().rev().skip(1) {
            let newer = *segment.last()?;
            if older.charging == Some(true) || newer.at - older.at > 600 || newer.percent > older.percent.saturating_add(2) ||
                last.at - older.at > 12 * 3600 { break; }
            segment.push(*older);
        }
        let first = *segment.last()?;
        let duration = last.at - first.at;
        if segment.len() < 4 || duration < 7200 || first.percent < last.percent.saturating_add(4) {
            return None;
        }
        let rate = f64::from(first.percent - last.percent) / (duration as f64 / 3600.0);
        let hours = f64::from(last.percent) / rate;
        if hours.is_finite() && hours <= 48.0 { Some(hours) } else { None }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_only_from_sustained_discharge() {
        let samples = (0..=24).map(|i| Sample {
            at: 10_000 + i * 300,
            percent: 90 - i as u8,
            millivolts: 4000,
            charging: None,
        }).collect();
        let history = BatteryHistory { samples, device: DeviceKind::Headset };
        let now = 17_200;
        assert_eq!(history.hours_remaining(now), Some(5.5));
        assert_eq!(history.hours_remaining(now + 181), None);
        let insufficient = BatteryHistory { samples: history.samples[..3].to_vec(), device: DeviceKind::Headset };
        assert_eq!(insufficient.hours_remaining(10_600), None);
    }
}
