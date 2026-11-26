//! Local battery history and conservative discharge estimates.
use crate::{battery::{BatteryReading, DeviceKind}, preferences::data_dir};
use std::{fs, io, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

const RETENTION: u64 = 48 * 3600;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub at: u64,
    pub percent: u8,
    pub millivolts: u16,
    pub charging: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryStats {
    pub count: usize, pub minimum: u8, pub maximum: u8, pub net_change: i16,
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
        Ok(data_dir()?.join(match device { DeviceKind::Headset => "history.csv", DeviceKind::Mouse => "mad-r-history.csv" }))
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

    pub fn export_csv(&self) -> io::Result<PathBuf> {
        let dir = data_dir()?.join("exports");
        fs::create_dir_all(&dir)?;
        let model = if self.device == DeviceKind::Headset { "g435" } else { "mad-r" };
        let target = dir.join(format!("{model}-history-{}.csv", Self::now()));
        let mut csv = String::from("unix_timestamp,percentage,millivolts,charging\n");
        use std::fmt::Write;
        for sample in &self.samples {
            let _ = writeln!(csv, "{},{},{},{}", sample.at, sample.percent, sample.millivolts,
                match sample.charging { Some(true) => "true", Some(false) => "false", None => "unknown" });
        }
        fs::write(&target, csv)?;
        Ok(target)
    }

    pub fn samples(&self) -> &[Sample] { &self.samples }
    pub fn latest(&self) -> Option<Sample> { self.samples.last().copied() }

    pub fn statistics(&self, now: u64, hours: u8) -> Option<BatteryStats> {
        let start = now.saturating_sub(u64::from(hours.clamp(1, 48)) * 3600);
        let mut samples = self.samples.iter().filter(|s| s.at >= start && s.at <= now);
        let first = samples.next()?;
        let mut stats = BatteryStats { count: 1, minimum: first.percent, maximum: first.percent, net_change: 0 };
        for sample in samples {
            stats.count += 1;
            stats.minimum = stats.minimum.min(sample.percent);
            stats.maximum = stats.maximum.max(sample.percent);
            stats.net_change = i16::from(sample.percent) - i16::from(first.percent);
        }
        Some(stats)
    }

    /// Fit a linear regression to recent uninterrupted falling samples, not just endpoints.
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
        let n = segment.len() as f64;
        let (sum_x, sum_y, sum_xy, sum_x2) = segment.iter().fold((0.0,0.0,0.0,0.0), |(sx,sy,sxy,sxx), sample| {
            let x = (sample.at - first.at) as f64 / 3600.0;
            let y = f64::from(sample.percent);
            (sx + x, sy + y, sxy + x*y, sxx + x*x)
        });
        let denominator = n * sum_x2 - sum_x * sum_x;
        if denominator <= f64::EPSILON { return None; }
        let rate = -(n * sum_xy - sum_x * sum_y) / denominator;
        if rate <= 0.0 { return None; }
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
        assert!((history.hours_remaining(now).unwrap() - 5.5).abs() < 1e-8);
        assert_eq!(history.hours_remaining(now + 181), None);
        let insufficient = BatteryHistory { samples: history.samples[..3].to_vec(), device: DeviceKind::Headset };
        assert_eq!(insufficient.hours_remaining(10_600), None);
        assert_eq!(history.statistics(now, 3), Some(BatteryStats { count: 25, minimum: 66, maximum: 90, net_change: -24 }));
    }

    #[test]
    fn no_forecast_while_charging_or_after_data_gap() {
        let samples = vec![
            Sample { at: 100, percent: 90, millivolts: 4000, charging: None },
            Sample { at: 7_300, percent: 70, millivolts: 3800, charging: Some(true) },
        ];
        let history = BatteryHistory { samples, device: DeviceKind::Mouse };
        assert_eq!(history.hours_remaining(7_300), None);
        assert_eq!(history.statistics(100_000, 3), None);
    }
}
