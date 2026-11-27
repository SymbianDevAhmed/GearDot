//! Persistent user preferences and platform-independent charging/alert policy.
use crate::battery::{BatteryReading, Connection, DeviceKind};
use crate::palette::{Palette, Rgb, VisualTheme};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

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
            Self::System => "Match theme",
            Self::Mint => "Mint",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
        }
    }
    pub fn rgb(self, light: bool) -> Rgb {
        match (self, light) {
            (Self::System, _) => Palette::for_theme(light).accent,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeSide {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidgetOptions {
    pub visible: [bool; 2],
    pub scale: u16,
    pub accent: Accent,
    pub animate: bool,
    pub monitor_slot: u8,
    pub offset: u16,
    pub taskbar: bool,
    pub edge: bool,
    pub edge_side: EdgeSide,
    pub edge_height_percent: u8,
    pub theme: Theme,
    pub visual_theme: VisualTheme,
}
impl Default for WidgetOptions {
    fn default() -> Self {
        Settings::default().widget_options()
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
pub struct DetectedProfile {
    pub name: String,
    pub visible: bool,
}

impl Default for DetectedProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            visible: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevicePlacement {
    pub devices: bool,
    pub taskbar: Option<bool>,
    pub edge: Option<bool>,
    pub pinned: bool,
}

impl Default for DevicePlacement {
    fn default() -> Self {
        Self {
            devices: true,
            taskbar: None,
            edge: None,
            pinned: false,
        }
    }
}

pub fn device_order(pinned: bool, low: bool, level: Option<u8>) -> (bool, bool, u8) {
    (!pinned, !low, level.unwrap_or(u8::MAX))
}

/// Stable local key without writing the device path or Bluetooth address to disk.
pub fn local_id(namespace: &str, bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |value, byte| {
        (value ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    format!("{namespace}{hash:016x}")
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
    pub visual_theme: VisualTheme,
    pub headset_name: String,
    pub mouse_name: String,
    pub monitor_slot: u8,
    pub widget_offset: u16,
    pub notification_sound: bool,
    pub battery_alerts_muted: bool,
    pub taskbar_widget: bool,
    pub tray_device: Option<String>,
    #[serde(rename = "edge_slider")]
    pub edge_widget: bool,
    pub edge_side: EdgeSide,
    pub edge_height_percent: u8,
    pub connection_alerts: bool,
    pub profiles: [DeviceProfile; 2],
    pub detected_profiles: std::collections::BTreeMap<String, DetectedProfile>,
    pub device_placements: std::collections::BTreeMap<String, DevicePlacement>,
    pub devices_background: Option<String>,
    pub background_opacity: u8,
    pub theme_transparency: std::collections::BTreeMap<VisualTheme, u8>,
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
            visual_theme: VisualTheme::default(),
            headset_name: String::new(),
            mouse_name: String::new(),
            monitor_slot: 0,
            widget_offset: 4,
            notification_sound: true,
            battery_alerts_muted: false,
            taskbar_widget: true,
            edge_widget: true,
            connection_alerts: false,
            tray_device: None,
            edge_side: EdgeSide::Right,
            edge_height_percent: 50,
            profiles: [DeviceProfile::default(); 2],
            detected_profiles: Default::default(),
            device_placements: Default::default(),
            devices_background: None,
            background_opacity: 45,
            theme_transparency: Default::default(),
        }
    }
}

impl Settings {
    pub fn validated(mut self) -> Self {
        self.low_threshold = self.low_threshold.clamp(5, 50);
        self.refresh_seconds = self.refresh_seconds.clamp(15, 300);
        self.widget_scale = self.widget_scale.clamp(80, 130);
        self.headset_name = validated_name(&self.headset_name);
        self.mouse_name = validated_name(&self.mouse_name);
        self.monitor_slot = self.monitor_slot.min(8);
        self.widget_offset = self.widget_offset.min(240);
        self.edge_height_percent = self.edge_height_percent.min(100);
        self.tray_device = self
            .tray_device
            .filter(|id| !id.is_empty() && id.len() <= 64);
        self.profiles = self.profiles.map(DeviceProfile::validated);
        self.background_opacity = self.background_opacity.min(100);
        for value in self.theme_transparency.values_mut() {
            *value = (*value).min(85);
        }
        self.devices_background = self
            .devices_background
            .filter(|id| id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        self.detected_profiles.retain(|id, profile| {
            profile.name = validated_name(&profile.name);
            !id.is_empty() && id.len() <= 64 && *profile != DetectedProfile::default()
        });
        self.device_placements.retain(|id, placement| {
            !id.is_empty() && id.len() <= 64 && *placement != DevicePlacement::default()
        });
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
    pub fn current_theme_transparency(&self) -> u8 {
        self.theme_transparency
            .get(&self.visual_theme)
            .copied()
            .unwrap_or(if self.visual_theme == VisualTheme::Glass {
                70
            } else {
                0
            })
    }

    pub fn detected_name<'a>(&'a self, id: &str, fallback: &'a str) -> &'a str {
        self.detected_profiles
            .get(id)
            .map(|p| p.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(fallback)
    }
    pub fn detected_visible(&self, id: &str) -> bool {
        self.detected_profiles
            .get(id)
            .is_none_or(|profile| profile.visible)
    }
    pub fn placement(&self, id: &str) -> DevicePlacement {
        self.device_placements.get(id).copied().unwrap_or_default()
    }

    pub fn set_placement(&mut self, id: String, placement: DevicePlacement) {
        if placement == DevicePlacement::default() {
            self.device_placements.remove(&id);
        } else {
            self.device_placements.insert(id, placement);
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
        self.profile(kind).refresh_seconds
    }
    pub fn reconnect_seconds(&self) -> u64 {
        4
    }
    pub fn charge_seconds(&self) -> u64 {
        15
    }
    pub fn widget_options(&self) -> WidgetOptions {
        WidgetOptions {
            visible: [self.show_headset, self.show_mouse],
            scale: self.widget_scale,
            accent: self.widget_accent,
            animate: self.animate_charging,
            monitor_slot: self.monitor_slot,
            offset: self.widget_offset,
            taskbar: self.taskbar_widget,
            edge: self.edge_widget,
            edge_side: self.edge_side,
            edge_height_percent: self.edge_height_percent,
            theme: self.theme,
            visual_theme: self.visual_theme,
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

pub fn tray_reading<'a, T>(
    readings: &'a [T],
    target: Option<&str>,
    reading: impl Fn(&T) -> (&str, Option<u8>),
) -> Option<&'a T> {
    readings
        .iter()
        .filter(|item| reading(item).1.is_some())
        .min_by_key(|item| {
            let (id, level) = reading(item);
            (target != Some(id), level)
        })
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
    local_data_dir()
}

fn local_data_dir() -> io::Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("G435Battery"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA unavailable"))
}

pub fn live_data_dir() -> io::Result<PathBuf> {
    // Portable preferences travel with the app. Live observations belong to this PC.
    Ok(local_data_dir()?.join("runtime"))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemporaryMute {
    #[default]
    Off,
    Until(u64),
    Session(String),
}

impl TemporaryMute {
    pub fn active(&self, now: u64, session: &str) -> bool {
        match self {
            Self::Off => false,
            Self::Until(until) => now < *until,
            Self::Session(id) => !session.is_empty() && id == session,
        }
    }

    pub fn load() -> Self {
        read_live_json("temporary-mute.json").unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        write_json_in(&live_data_dir()?, "temporary-mute.json", self)
    }
}

fn read_live_json<T: serde::de::DeserializeOwned>(name: &str) -> io::Result<T> {
    serde_json::from_slice(&fs::read(live_data_dir()?.join(name))?).map_err(io::Error::other)
}

pub fn new_monitor_session() -> String {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{started}", std::process::id())
}

fn read_json<T: serde::de::DeserializeOwned>(name: &str) -> io::Result<T> {
    serde_json::from_slice(&fs::read(data_dir()?.join(name))?).map_err(io::Error::other)
}

fn write_json<T: Serialize>(name: &str, value: &T) -> io::Result<()> {
    write_json_in(&data_dir()?, name, value)
}

fn write_json_in<T: Serialize>(dir: &std::path::Path, name: &str, value: &T) -> io::Result<()> {
    fs::create_dir_all(dir)?;
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
    #[serde(default)]
    pub connection: Connection,
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
            connection: reading.connection(),
            level: Some(reading.percentage()),
            millivolts: Some(reading.millivolts()),
            charging: reading.charging(),
            estimated: reading.estimated(),
            stale: false,
            message: String::new(),
        }
    }
    pub fn online(&self) -> bool {
        self.level.is_some() && !self.stale
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BluetoothReading {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub percent: Option<u8>,
    #[serde(default)]
    pub kind: Option<InputKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputKind {
    Keyboard,
    Gamepad,
    Mouse,
    Headset,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputStatus {
    Usb,
    Receiver,
    Wired,
    WiredOnly,
    Unknown,
    Empty,
    Low,
    Medium,
    Full,
    Bluetooth,
}

impl InputStatus {
    pub fn level_label(self) -> Option<&'static str> {
        match self {
            Self::Empty => Some("Empty"),
            Self::Low => Some("Low"),
            Self::Medium => Some("Medium"),
            Self::Full => Some("Full"),
            _ => None,
        }
    }

    pub fn connection(self) -> Connection {
        match self {
            Self::Usb => Connection::Usb,
            Self::Bluetooth => Connection::Bluetooth,
            Self::Receiver => Connection::Dongle,
            Self::Wired | Self::WiredOnly => Connection::Wired,
            _ => Connection::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputCharge {
    Charging,
    Discharging,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputBattery {
    pub percent: u8,
    pub charge: InputCharge,
    pub estimated: bool,
    pub checked_at: u64,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputPeripheral {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub kind: InputKind,
    pub status: InputStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_override: Option<Connection>,
    #[serde(default)]
    pub usb_ids: Option<(u16, u16)>,
    #[serde(default)]
    pub battery: Option<InputBattery>,
}

impl BluetoothReading {
    pub fn profile_id(&self) -> std::borrow::Cow<'_, str> {
        if self.id.is_empty() {
            local_id("bt-name-", self.name.as_bytes()).into()
        } else {
            self.id.as_str().into()
        }
    }
}

impl InputPeripheral {
    pub fn connection(&self) -> Connection {
        self.connection_override
            .unwrap_or_else(|| self.status.connection())
    }

    pub fn profile_id(&self) -> std::borrow::Cow<'_, str> {
        if self.id.is_empty() {
            local_id("input-name-", self.name.as_bytes()).into()
        } else {
            self.id.as_str().into()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedPhoneBattery {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LiveSnapshot {
    #[serde(default)]
    pub monitor_session: String,
    pub devices: [LiveReading; 2],
    pub refresh_seconds: u64,
    #[serde(default)]
    pub device_intervals: [u64; 2],
    #[serde(default)]
    pub bluetooth: Vec<BluetoothReading>,
    #[serde(default)]
    pub bluetooth_checked_at: u64,
    #[serde(default)]
    pub bluetooth_error: Option<String>,
    #[serde(default)]
    pub input_peripherals: Vec<InputPeripheral>,
    #[serde(default)]
    pub inputs_checked_at: u64,
    #[serde(default)]
    pub cached_phone: Option<CachedPhoneBattery>,
    #[serde(default)]
    pub inputs_hid_error: Option<String>,
    #[serde(default)]
    pub audio_error: Option<String>,
}

pub fn normalized_device_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl LiveSnapshot {
    pub fn load() -> Self {
        read_live_json("live-status.json").unwrap_or_default()
    }
    pub fn save(&self) -> io::Result<()> {
        write_json_in(&live_data_dir()?, "live-status.json", self)
    }
    pub fn connected_bluetooth(&self, now: u64) -> impl Iterator<Item = &BluetoothReading> {
        let active =
            self.bluetooth_checked_at != 0 && now.saturating_sub(self.bluetooth_checked_at) <= 45;
        let inputs = self.connected_inputs(now);
        self.bluetooth.iter().filter(move |reading| {
            if !active {
                return false;
            }
            let name = normalized_device_name(&reading.name);
            if name.is_empty()
                || self
                    .bluetooth
                    .iter()
                    .filter(|candidate| normalized_device_name(&candidate.name) == name)
                    .take(2)
                    .count()
                    != 1
            {
                return true;
            }
            inputs
                .iter()
                .filter(|device| {
                    device.connection() == Connection::Bluetooth
                        && device.battery.is_some()
                        && normalized_device_name(&device.name) == name
                        && reading.kind.is_none_or(|kind| kind == device.kind)
                })
                .take(2)
                .count()
                != 1
        })
    }
    pub fn connected_inputs(&self, now: u64) -> &[InputPeripheral] {
        if self.inputs_checked_at != 0 && now.saturating_sub(self.inputs_checked_at) <= 90 {
            &self.input_peripherals
        } else {
            &[]
        }
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
        self.observe_telemetry(
            reading.percentage(),
            reading.charging(),
            reading.estimated(),
            profile,
        )
    }

    pub fn observe_telemetry(
        &mut self,
        level: u8,
        charging: Option<bool>,
        estimated: bool,
        profile: DeviceProfile,
    ) -> Option<Alert> {
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
        if !estimated && level == 100 && self.charge_seen && !self.full_alerted {
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
    pub fn observe_coarse(&mut self, status: InputStatus, profile: DeviceProfile) -> Option<Alert> {
        match status {
            InputStatus::Empty | InputStatus::Low => self.observe_telemetry(0, None, true, profile),
            InputStatus::Medium | InputStatus::Full => {
                self.low_alerted = false;
                None
            }
            _ => None,
        }
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
    fn tray_selection_falls_back_to_a_reported_level_and_remembers_the_target() {
        let readings = [
            ("mouse", Some(70)),
            ("keyboard", None),
            ("headset", Some(0)),
        ];
        let lookup = |target| tray_reading(&readings, target, |r| *r).copied();
        assert_eq!(lookup(None), Some(("headset", Some(0))));
        assert_eq!(lookup(Some("mouse")), Some(("mouse", Some(70))));
        assert_eq!(lookup(Some("keyboard")), lookup(None));
        assert_eq!(lookup(Some("disconnected")), lookup(None));
        let settings = Settings {
            tray_device: Some("disconnected".into()),
            ..Default::default()
        };
        let roundtrip: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(roundtrip.validated().tray_device, settings.tray_device);
    }

    #[test]
    fn placement_round_trips_and_pinned_devices_precede_low_batteries() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        let id = DeviceKind::Mouse.profile_id();
        assert_eq!(settings.placement(id), DevicePlacement::default());
        let placement = DevicePlacement {
            devices: false,
            taskbar: Some(true),
            edge: Some(false),
            pinned: true,
        };
        settings.set_placement(id.into(), placement);
        let restored: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(restored.placement(id), placement);
        assert!(device_order(true, false, Some(90)) < device_order(false, true, Some(5)));
        assert!(device_order(false, true, Some(12)) < device_order(false, false, Some(6)));
        assert!(device_order(false, false, Some(90)) < device_order(false, false, None));
    }

    #[test]
    fn temporary_mute_expires_at_its_deadline_or_when_the_monitor_restarts() {
        assert!(TemporaryMute::Until(3600).active(3599, "next-session"));
        assert!(!TemporaryMute::Until(3600).active(3600, "next-session"));
        let mute = TemporaryMute::Session("active-session".into());
        assert!(mute.active(0, "active-session"));
        assert!(!mute.active(0, "next-session"));
        assert!(!mute.active(0, ""));
        assert!(!TemporaryMute::Off.active(0, "active-session"));
    }

    #[test]
    fn quick_controls_preserve_old_defaults_and_independent_choices() {
        let mut settings: Settings = serde_json::from_str(r#"{"edge_slider":false}"#).unwrap();
        assert!(settings.widget_options().taskbar);
        assert!(!settings.widget_options().edge);
        assert!(!settings.battery_alerts_muted);

        settings.taskbar_widget = false;
        settings.edge_widget = true;
        settings.battery_alerts_muted = true;
        settings.profiles[DeviceKind::Mouse.index()].custom = true;
        let restored: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert!(!restored.widget_options().taskbar);
        assert!(restored.widget_options().edge);
        assert!(restored.battery_alerts_muted);
        assert!(restored.profile(DeviceKind::Mouse).low_notifications);
    }
    #[test]
    fn transparency_is_independent_for_each_theme_and_the_background_image() {
        let mut settings = Settings::default();
        assert_eq!(settings.current_theme_transparency(), 0);
        settings.theme_transparency.insert(VisualTheme::Ion, 30);
        settings.theme_transparency.insert(VisualTheme::Arctic, 100);
        settings.visual_theme = VisualTheme::Glass;
        assert_eq!(settings.current_theme_transparency(), 70);
        settings.visual_theme = VisualTheme::Arctic;
        let roundtrip: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut roundtrip = roundtrip.validated();
        assert_eq!(roundtrip.current_theme_transparency(), 85);
        roundtrip.visual_theme = VisualTheme::Ion;
        assert_eq!(roundtrip.current_theme_transparency(), 30);
        assert_eq!(roundtrip.background_opacity, 45);
    }

    #[test]
    fn obsidian_is_the_default_and_saved_themes_keep_their_value() {
        assert_eq!(Settings::default().visual_theme, VisualTheme::Obsidian);
        let missing: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.visual_theme, VisualTheme::Obsidian);
        let saved: Settings = serde_json::from_str(r#"{"visual_theme":"Ion"}"#).unwrap();
        assert_eq!(saved.validated().visual_theme, VisualTheme::Ion);
    }

    #[test]
    fn detected_profile_and_background_are_local_persistent_settings() {
        let mut saved: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(saved.background_opacity, 45);
        assert!(saved.devices_background.is_none());
        assert!(saved.detected_visible("hid-example"));

        saved.detected_profiles.insert(
            "hid-example".into(),
            DetectedProfile {
                name: "  Work keyboard  ".into(),
                visible: false,
            },
        );
        saved.devices_background = Some("12ab34cd56ef1234".into());
        saved.background_opacity = 170;
        let saved = saved.validated();
        let restored: Settings =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(restored, saved);
        assert_eq!(restored.background_opacity, 100);
        assert!(!restored.detected_visible("hid-example"));
        assert_eq!(
            restored.detected_name("hid-example", "Keyboard"),
            "Work keyboard"
        );
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
            edge_widget: false,
            edge_side: EdgeSide::Left,
            edge_height_percent: 25,
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
        assert_eq!(partial.refresh_seconds, 30);
    }
    #[test]
    fn appearance_preferences_are_bounded() {
        let s = Settings {
            widget_scale: 250,
            show_headset: false,
            widget_accent: Accent::Violet,
            theme: Theme::Light,
            edge_side: EdgeSide::Left,
            edge_height_percent: 255,
            ..Default::default()
        }
        .validated();
        assert_eq!(s.widget_scale, 130);
        assert_eq!(s.widget_options().visible, [false, true]);
        assert_eq!(s.widget_options().accent, Accent::Violet);
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(s.widget_options().edge_side, EdgeSide::Left);
        assert_eq!(s.widget_options().edge_height_percent, 100);
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
            ..Default::default()
        };
        assert!(snapshot.reading(DeviceKind::Headset, 200).stale);
        assert!(!snapshot.reading(DeviceKind::Mouse, 200).stale);
    }
    #[test]
    fn settings_from_older_versions_ignore_removed_power_saving_mode() {
        let settings: Settings = serde_json::from_str(
            r#"{"power_saving":true,"refresh_seconds":45,"animate_charging":true}"#,
        )
        .unwrap();
        assert_eq!(settings.poll_seconds(DeviceKind::Mouse), 45);
        assert!(settings.widget_options().animate);
        assert!(serde_json::to_value(settings)
            .unwrap()
            .get("power_saving")
            .is_none());
    }

    #[test]
    fn per_device_profiles_keep_their_refresh_intervals() {
        let s = Settings {
            animate_charging: true,
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
        assert_eq!(s.poll_seconds(DeviceKind::Headset), 30);
        assert_eq!(s.reconnect_seconds(), 4);
        assert_eq!(s.charge_seconds(), 15);
        assert!(s.widget_options().animate);
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
