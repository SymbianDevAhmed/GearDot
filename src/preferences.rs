//! Persistent user preferences and platform-independent charging/alert policy.
use crate::battery::{BatteryReading, DeviceKind};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    System,
    Mint,
    Blue,
    Violet,
}
impl Accent {
    pub const ALL: [Self; 4] = [Self::System, Self::Mint, Self::Blue, Self::Violet];
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Mint => "Mint",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
        }
    }
    pub fn rgb(self, light: bool) -> (u32, u32, u32) {
        match (self, light) {
            (Self::System, true) => (32, 32, 32),
            (Self::System, false) => (248, 248, 248),
            (Self::Mint, true) => (0, 106, 79),
            (Self::Mint, false) => (105, 222, 184),
            (Self::Blue, true) => (16, 82, 160),
            (Self::Blue, false) => (130, 190, 255),
            (Self::Violet, true) => (92, 45, 155),
            (Self::Violet, false) => (194, 154, 250),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}
impl Theme {
    pub const ALL: [Self; 3] = [Self::System, Self::Dark, Self::Light];
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidgetOptions {
    pub visible: [bool; 2],
    pub scale: u16,
    pub accent: Accent,
    pub animate: bool,
    pub monitor_slot: u8,
    pub offset: u16,
    pub power_saving: bool,
}
impl Default for WidgetOptions {
    fn default() -> Self {
        Self {
            visible: [true, true],
            scale: 100,
            accent: Accent::System,
            animate: true,
            monitor_slot: 0,
            offset: 4,
            power_saving: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceProfile {
    pub custom: bool,
    pub low_threshold: u8,
    pub refresh_seconds: u64,
    pub low_notifications: bool,
    pub full_notifications: bool,
}
impl Default for DeviceProfile {
    fn default() -> Self {
        Self {
            custom: false,
            low_threshold: 20,
            refresh_seconds: 60,
            low_notifications: true,
            full_notifications: true,
        }
    }
}
impl DeviceProfile {
    fn validated(mut self) -> Self {
        self.low_threshold = self.low_threshold.clamp(5, 50);
        self.refresh_seconds = self.refresh_seconds.clamp(15, 300);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub start_with_windows: bool,
    pub low_notifications: bool,
    pub full_notifications: bool,
    pub low_threshold: u8,
    pub refresh_seconds: u64,
    pub show_headset: bool,
    pub show_mouse: bool,
    pub widget_scale: u16,
    pub widget_accent: Accent,
    pub animate_charging: bool,
    pub theme: Theme,
    pub chart_hours: u8,
    pub headset_name: String,
    pub mouse_name: String,
    pub power_saving: bool,
    pub monitor_slot: u8,
    pub widget_offset: u16,
    pub notification_sound: bool,
    pub profiles: [DeviceProfile; 2],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            low_notifications: true,
            full_notifications: true,
            low_threshold: 20,
            refresh_seconds: 60,
            show_headset: true,
            show_mouse: true,
            widget_scale: 100,
            widget_accent: Accent::System,
            animate_charging: true,
            theme: Theme::System,
            chart_hours: 12,
            headset_name: String::new(),
            mouse_name: String::new(),
            power_saving: false,
            monitor_slot: 0,
            widget_offset: 4,
            notification_sound: true,
            profiles: [DeviceProfile::default(); 2],
        }
    }
}

impl Settings {
    pub fn validated(mut self) -> Self {
        self.low_threshold = self.low_threshold.clamp(5, 50);
        self.refresh_seconds = self.refresh_seconds.clamp(15, 300);
        self.widget_scale = self.widget_scale.clamp(80, 130);
        if ![3, 6, 12, 24, 48].contains(&self.chart_hours) {
            self.chart_hours = 12;
        }
        self.headset_name = validated_name(&self.headset_name);
        self.mouse_name = validated_name(&self.mouse_name);
        self.monitor_slot = self.monitor_slot.min(8);
        self.widget_offset = self.widget_offset.min(240);
        self.profiles = self.profiles.map(DeviceProfile::validated);
        self
    }
    pub fn device_name(&self, kind: DeviceKind) -> &str {
        let custom = match kind {
            DeviceKind::Headset => &self.headset_name,
            DeviceKind::Mouse => &self.mouse_name,
        };
        if custom.is_empty() {
            kind.name()
        } else {
            custom
        }
    }
    pub fn backup(&self) -> io::Result<()> {
        write_json("settings-backup.json", &self.clone().validated())
    }
    pub fn restore_backup() -> io::Result<Self> {
        read_json::<Self>("settings-backup.json").map(Self::validated)
    }
    pub fn default_profile(&self) -> DeviceProfile {
        DeviceProfile {
            custom: false,
            low_threshold: self.low_threshold,
            refresh_seconds: self.refresh_seconds,
            low_notifications: self.low_notifications,
            full_notifications: self.full_notifications,
        }
    }
    pub fn profile(&self, kind: DeviceKind) -> DeviceProfile {
        let p = self.profiles[kind.index()];
        if p.custom {
            p
        } else {
            self.default_profile()
        }
    }
    pub fn poll_seconds(&self, kind: DeviceKind) -> u64 {
        let base = self.profile(kind).refresh_seconds;
        if self.power_saving {
            base.max(120)
        } else {
            base
        }
    }
    pub fn reconnect_seconds(&self) -> u64 {
        if self.power_saving {
            45
        } else {
            15
        }
    }
    pub fn charge_seconds(&self) -> u64 {
        if self.power_saving {
            30
        } else {
            15
        }
    }
    pub fn widget_options(&self) -> WidgetOptions {
        WidgetOptions {
            visible: [self.show_headset, self.show_mouse],
            scale: self.widget_scale,
            accent: self.widget_accent,
            animate: self.animate_charging && !self.power_saving,
            monitor_slot: self.monitor_slot,
            offset: self.widget_offset,
            power_saving: self.power_saving,
        }
    }
    pub fn load() -> Self {
        read_json::<Self>("settings.json")
            .unwrap_or_default()
            .validated()
    }
    pub fn save(&self) -> io::Result<()> {
        write_json("settings.json", &self.clone().validated())
    }
}

fn validated_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(30)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub fn portable_root() -> Option<PathBuf> {
    static ROOT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let parent = exe.parent()?;
        parent
            .join("portable.flag")
            .is_file()
            .then(|| parent.to_owned())
    })
    .clone()
}
pub fn is_portable() -> bool {
    portable_root().is_some()
}

pub fn data_dir() -> io::Result<PathBuf> {
    if let Some(root) = portable_root() {
        return Ok(root.join("data"));
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("G435Battery"))
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
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
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
        Self {
            checked_at: now,
            level: Some(reading.percentage()),
            millivolts: Some(reading.millivolts()),
            charging: reading.charging(),
            estimated: reading.estimated(),
            stale: false,
            message: String::new(),
        }
    }
    pub fn confirmed_full(&self) -> bool {
        !self.stale && !self.estimated && self.level == Some(100) && self.charging.is_some()
    }
    pub fn state_label(&self) -> &'static str {
        if self.stale {
            "Last recorded reading"
        } else if self.level.is_none() {
            "Not connected"
        } else if self.confirmed_full() {
            "Full • charging complete"
        } else if self.charging == Some(true) {
            "Charging"
        } else if self.estimated && self.level == Some(100) {
            "Estimated full • charging status unavailable"
        } else if self.estimated {
            "Voltage estimate • charging status unavailable"
        } else {
            "Battery powered"
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LiveSnapshot {
    pub devices: [LiveReading; 2],
    pub refresh_seconds: u64,
    #[serde(default)]
    pub device_intervals: [u64; 2],
}

impl LiveSnapshot {
    pub fn load() -> Self {
        read_json("live-status.json").unwrap_or_default()
    }
    pub fn save(&self) -> io::Result<()> {
        write_json("live-status.json", self)
    }
    pub fn reading(&self, kind: DeviceKind, now: u64) -> LiveReading {
        let mut reading = self.devices[kind.index()].clone();
        let interval = self.device_intervals[kind.index()];
        let interval = if interval == 0 {
            self.refresh_seconds
        } else {
            interval
        };
        if reading.checked_at == 0
            || now.saturating_sub(reading.checked_at) > interval.clamp(15, 300) * 2 + 30
        {
            reading.stale = true;
            reading.message =
                "No recent live update; start the monitor or reconnect the device.".into();
        }
        reading
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    Low,
    Full,
}

/// A low-level alert is latched until recovery; a full alert requires a known charging session.
#[derive(Default)]
pub struct AlertTracker {
    low_alerted: bool,
    charge_seen: bool,
    full_alerted: bool,
}

impl AlertTracker {
    pub fn observe(&mut self, reading: BatteryReading, settings: &Settings) -> Option<Alert> {
        self.observe_profile(reading, settings.default_profile())
    }
    pub fn observe_profile(
        &mut self,
        reading: BatteryReading,
        profile: DeviceProfile,
    ) -> Option<Alert> {
        let level = reading.percentage();
        let charging = reading.charging();
        if level >= profile.low_threshold.saturating_add(5) || charging == Some(true) {
            self.low_alerted = false;
        }
        if level < 95 {
            self.full_alerted = false;
        }
        if charging == Some(true) && level < 100 {
            self.charge_seen = true;
        }
        // A device can stop reporting charging at precisely 100%.
        if !reading.estimated() && level == 100 && self.charge_seen && !self.full_alerted {
            self.full_alerted = true;
            self.charge_seen = false;
            if profile.full_notifications {
                return Some(Alert::Full);
            }
        }
        if charging == Some(false) && level < 100 {
            self.charge_seen = false;
        }
        if profile.low_notifications
            && charging != Some(true)
            && level <= profile.low_threshold
            && !self.low_alerted
        {
            self.low_alerted = true;
            return Some(Alert::Low);
        }
        None
    }
    pub fn disconnected(&mut self) {
        self.charge_seen = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mouse(level: u8, charging: bool) -> BatteryReading {
        BatteryReading::from_reported(3900, level, charging).unwrap()
    }
    #[test]
    fn preferences_are_bounded_and_default_to_opt_in_startup() {
        let s = Settings {
            low_threshold: 255,
            refresh_seconds: 0,
            ..Default::default()
        }
        .validated();
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
        let s = Settings {
            low_notifications: false,
            full_notifications: false,
            ..Default::default()
        };
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
        let settings = Settings {
            refresh_seconds: 30,
            low_threshold: 15,
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<Settings>(&serde_json::to_string(&settings).unwrap()).unwrap(),
            settings
        );
        let partial = serde_json::from_str::<Settings>(r#"{"refresh_seconds":30}"#).unwrap();
        assert!(partial.low_notifications);
        assert!(!partial.start_with_windows);
        assert_eq!(partial.widget_options(), WidgetOptions::default());
        assert_eq!(partial.theme, Theme::System);
        assert_eq!(partial.chart_hours, 12);
        assert_eq!(partial.refresh_seconds, 30);
    }
    #[test]
    fn appearance_preferences_are_bounded() {
        let s = Settings {
            widget_scale: 250,
            chart_hours: 13,
            show_headset: false,
            widget_accent: Accent::Violet,
            theme: Theme::Light,
            ..Default::default()
        }
        .validated();
        assert_eq!(s.widget_scale, 130);
        assert_eq!(s.chart_hours, 12);
        assert_eq!(s.widget_options().visible, [false, true]);
        assert_eq!(s.widget_options().accent, Accent::Violet);
        assert_eq!(s.theme, Theme::Light);
    }
    #[test]
    fn custom_device_names_do_not_change_device_identity() {
        let s = Settings {
            headset_name: " Studio Headset ".into(),
            mouse_name: "\n".into(),
            ..Default::default()
        }
        .validated();
        assert_eq!(s.device_name(DeviceKind::Headset), "Studio Headset");
        assert_eq!(s.device_name(DeviceKind::Mouse), "VXE MAD R");
        assert_eq!(DeviceKind::Headset.index(), 0);
    }
    #[test]
    fn snapshot_freshness_uses_each_devices_poll_interval() {
        let reading = LiveReading::from_reading(mouse(55, false), 100);
        let snapshot = LiveSnapshot {
            devices: [reading.clone(), reading],
            refresh_seconds: 60,
            device_intervals: [30, 300],
        };
        assert!(snapshot.reading(DeviceKind::Headset, 200).stale);
        assert!(!snapshot.reading(DeviceKind::Mouse, 200).stale);
    }
    #[test]
    fn per_device_profiles_and_saver_are_independent() {
        let s = Settings {
            power_saving: true,
            profiles: [
                DeviceProfile {
                    custom: true,
                    low_threshold: 35,
                    refresh_seconds: 30,
                    ..Default::default()
                },
                DeviceProfile::default(),
            ],
            ..Default::default()
        }
        .validated();
        assert_eq!(s.profile(DeviceKind::Headset).low_threshold, 35);
        assert_eq!(s.profile(DeviceKind::Mouse).low_threshold, 20);
        assert_eq!(s.poll_seconds(DeviceKind::Headset), 120);
        assert_eq!(s.reconnect_seconds(), 45);
        assert_eq!(s.charge_seconds(), 30);
        assert!(!s.widget_options().animate);
        let mut tracker = AlertTracker::default();
        assert_eq!(
            tracker.observe_profile(mouse(30, false), s.profile(DeviceKind::Headset)),
            Some(Alert::Low)
        );
        let mut other = AlertTracker::default();
        assert_eq!(
            other.observe_profile(mouse(30, false), s.profile(DeviceKind::Mouse)),
            None
        );
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
