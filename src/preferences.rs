//! Persistent user preferences and platform-independent charging/alert policy.
use crate::battery::{BatteryReading, DeviceKind};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub start_with_windows: bool,
    pub low_notifications: bool,
    pub full_notifications: bool,
    pub low_threshold: u8,
    pub refresh_seconds: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { start_with_windows: false, low_notifications: true, full_notifications: true, low_threshold: 20, refresh_seconds: 60 }
    }
}

impl Settings {
    pub fn validated(mut self) -> Self {
        self.low_threshold = self.low_threshold.clamp(5, 50);
        self.refresh_seconds = self.refresh_seconds.clamp(15, 300);
        self
    }
    pub fn load() -> Self { read_json::<Self>("settings.json").unwrap_or_default().validated() }
    pub fn save(&self) -> io::Result<()> { write_json("settings.json", &self.clone().validated()) }
}

pub fn data_dir() -> io::Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("G435Battery"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA unavailable"))
}

fn read_json<T: serde::de::DeserializeOwned>(name: &str) -> io::Result<T> {
    serde_json::from_slice(&fs::read(data_dir()?.join(name))?).map_err(io::Error::other)
}

fn write_json<T: Serialize>(name: &str, value: &T) -> io::Result<()> {
    let dir = data_dir()?;
    fs::create_dir_all(&dir)?;
    let destination = dir.join(name);
    let temporary = dir.join(format!("{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &destination)
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LiveReading {
    pub checked_at: u64,
    pub level: Option<u8>,
    pub millivolts: Option<u16>,
    pub charging: Option<bool>,
    pub estimated: bool,
    pub stale: bool,
    pub message: String,
}

impl LiveReading {
    pub fn from_reading(reading: BatteryReading, now: u64) -> Self {
        Self { checked_at: now, level: Some(reading.percentage()), millivolts: Some(reading.millivolts()),
            charging: reading.charging(), estimated: reading.estimated(), stale: false, message: String::new() }
    }
    pub fn confirmed_full(&self) -> bool {
        !self.stale && !self.estimated && self.level == Some(100) && self.charging.is_some()
    }
    pub fn state_label(&self) -> &'static str {
        if self.stale { "Last recorded reading" }
        else if self.level.is_none() { "Not connected" }
        else if self.confirmed_full() { "Full • charging complete" }
        else if self.charging == Some(true) { "Charging" }
        else if self.estimated && self.level == Some(100) { "Estimated full • charging status unavailable" }
        else if self.estimated { "Voltage estimate • charging status unavailable" }
        else { "Battery powered" }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LiveSnapshot {
    pub devices: [LiveReading; 2],
    pub refresh_seconds: u64,
}

impl LiveSnapshot {
    pub fn load() -> Self { read_json("live-status.json").unwrap_or_default() }
    pub fn save(&self) -> io::Result<()> { write_json("live-status.json", self) }
    pub fn reading(&self, kind: DeviceKind, now: u64) -> LiveReading {
        let mut reading = self.devices[kind.index()].clone();
        if reading.checked_at == 0 || now.saturating_sub(reading.checked_at) > self.refresh_seconds.max(15) * 2 + 30 {
            reading.stale = true;
            reading.message = "No recent live update; start the monitor or reconnect the device.".into();
        }
        reading
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert { Low, Full }

/// A low-level alert is latched until recovery; a full alert requires a known charging session.
#[derive(Default)]
pub struct AlertTracker { low_alerted: bool, charge_seen: bool, full_alerted: bool }

impl AlertTracker {
    pub fn observe(&mut self, reading: BatteryReading, settings: &Settings) -> Option<Alert> {
        let level = reading.percentage();
        let charging = reading.charging();
        if level >= settings.low_threshold.saturating_add(5) || charging == Some(true) { self.low_alerted = false; }
        if level < 95 { self.full_alerted = false; }
        if charging == Some(true) && level < 100 { self.charge_seen = true; }
        // A device can stop reporting charging at precisely 100%.
        if !reading.estimated() && level == 100 && self.charge_seen && !self.full_alerted {
            self.full_alerted = true;
            self.charge_seen = false;
            if settings.full_notifications { return Some(Alert::Full); }
        }
        if charging == Some(false) && level < 100 { self.charge_seen = false; }
        if settings.low_notifications && charging != Some(true) && level <= settings.low_threshold && !self.low_alerted {
            self.low_alerted = true;
            return Some(Alert::Low);
        }
        None
    }
    pub fn disconnected(&mut self) { self.charge_seen = false; }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mouse(level: u8, charging: bool) -> BatteryReading { BatteryReading::from_reported(3900, level, charging).unwrap() }
    #[test]
    fn preferences_are_bounded_and_default_to_opt_in_startup() {
        let s = Settings { low_threshold: 255, refresh_seconds: 0, ..Default::default() }.validated();
        assert_eq!(s.low_threshold, 50);
        assert_eq!(s.refresh_seconds, 15);
        assert!(!s.start_with_windows);
    }
    #[test]
    fn low_alert_is_latched_until_recovery() {
        let s = Settings::default();
        let mut tracker = AlertTracker::default();
        assert_eq!(tracker.observe(mouse(20, false), &s), Some(Alert::Low));
        assert_eq!(tracker.observe(mouse(19, false), &s), None);
        assert_eq!(tracker.observe(mouse(25, false), &s), None);
        assert_eq!(tracker.observe(mouse(20, false), &s), Some(Alert::Low));
    }
    #[test]
    fn full_requires_observed_charge_and_is_not_repeated() {
        let s = Settings::default();
        let mut tracker = AlertTracker::default();
        assert_eq!(tracker.observe(mouse(100, false), &s), None);
        assert_eq!(tracker.observe(mouse(80, true), &s), None);
        assert_eq!(tracker.observe(mouse(100, false), &s), Some(Alert::Full));
        assert_eq!(tracker.observe(mouse(100, true), &s), None);
        assert_eq!(tracker.observe(mouse(99, true), &s), None);
        assert_eq!(tracker.observe(mouse(100, true), &s), None);
    }
    #[test]
    fn disabled_and_disconnected_alerts_are_not_emitted() {
        let s = Settings { low_notifications: false, full_notifications: false, ..Default::default() };
        let mut tracker = AlertTracker::default();
        assert_eq!(tracker.observe(mouse(10, false), &s), None);
        assert_eq!(tracker.observe(mouse(80, true), &s), None);
        assert_eq!(tracker.observe(mouse(100, false), &s), None);
        let s = Settings::default();
        tracker.observe(mouse(80, true), &s);
        tracker.disconnected();
        assert_eq!(tracker.observe(mouse(100, false), &s), None);
    }
    #[test]
    fn settings_roundtrip_and_missing_fields_use_defaults() {
        let settings = Settings { refresh_seconds: 30, low_threshold: 15, ..Default::default() };
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&settings).unwrap()).unwrap(), settings);
        let partial = serde_json::from_str::<Settings>(r#"{"refresh_seconds":30}"#).unwrap();
        assert!(partial.low_notifications);
        assert!(!partial.start_with_windows);
        assert_eq!(partial.refresh_seconds, 30);
    }
    #[test]
    fn estimated_voltage_never_claims_confirmed_full() {
        let headset = LiveReading::from_reading(BatteryReading::from_voltage(4200).unwrap(), 1);
        assert!(!headset.confirmed_full());
        assert!(headset.state_label().contains("Estimated full"));
        let mouse = LiveReading::from_reading(mouse(100, true), 1);
        assert!(mouse.confirmed_full());
    }
}
