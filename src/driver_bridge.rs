//! Normalize readings from vendor backends before matching the Windows device list.
use serde::{Deserialize, Serialize};
use wireless_battery_monitor::preferences::{
    local_id, normalized_device_name, CachedPhoneBattery, InputBattery, InputCharge, InputKind,
    InputPeripheral, InputStatus,
};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Reading {
    key: String,
    name: String,
    via: String,
    kind: String,
    level: String,
    #[serde(default = "missing_percent")]
    percent: i32,
    charging: bool,
    full: bool,
    connected: bool,
    asleep: bool,
    millivolts: i32,
    vid: u16,
    pid: u16,
}

fn missing_percent() -> i32 {
    -1
}

impl Reading {
    fn into_device(self, provider: &str) -> Option<InputPeripheral> {
        if !self.connected || self.asleep || self.key.is_empty() || self.name.trim().is_empty() {
            return None;
        }
        let kind = match self.kind.as_str() {
            "mouse" => InputKind::Mouse,
            "keyboard" => InputKind::Keyboard,
            "gamepad" => InputKind::Gamepad,
            "headset" | "earbuds" => InputKind::Headset,
            _ => InputKind::Other,
        };
        let via = self.via.to_ascii_lowercase();
        let connection = if via.contains(" / ") {
            InputStatus::Unknown
        } else if via.contains("bluetooth") || via == "ble" {
            InputStatus::Bluetooth
        } else if [
            "receiver",
            "dongle",
            "2.4",
            "adapter",
            "lightspeed",
            "unifying",
        ]
        .iter()
        .any(|part| via.contains(part))
        {
            InputStatus::Receiver
        } else if via.contains("usb") || via.contains("wired") {
            InputStatus::Usb
        } else {
            InputStatus::Unknown
        };
        let status = match self.level.trim().to_ascii_lowercase().as_str() {
            "empty" | "critical" => InputStatus::Empty,
            "low" => InputStatus::Low,
            "medium" | "good" => InputStatus::Medium,
            "full" => InputStatus::Full,
            _ => connection,
        };
        let battery = u8::try_from(self.percent)
            .ok()
            .filter(|value| *value <= 100)
            .filter(|_| status.level_label().is_none())
            .map(|percent| InputBattery {
                percent,
                charge: if self.full {
                    InputCharge::Full
                } else if self.charging {
                    InputCharge::Charging
                } else {
                    InputCharge::Discharging
                },
                estimated: self.millivolts > 0,
                checked_at: 0,
                stale: false,
            });
        if battery.is_none() && status.level_label().is_none() {
            return None;
        }
        let id = self
            .key
            .strip_prefix("xinput:")
            .and_then(|key| key.split_once(':'))
            .filter(|(slot, ty)| {
                kind == InputKind::Gamepad && *ty == "0" && matches!(*slot, "0" | "1" | "2" | "3")
            })
            .map_or_else(
                || local_id(provider, self.key.as_bytes()),
                |(slot, _)| format!("xinput-{slot}"),
            );
        Some(InputPeripheral {
            id,
            name: self
                .name
                .chars()
                .filter(|c| !c.is_control())
                .take(100)
                .collect(),
            kind,
            status,
            connection_override: status.level_label().map(|_| connection.connection()),
            usb_ids: (self.vid != 0 && self.pid != 0).then_some((self.vid, self.pid)),
            battery,
        })
    }
}

pub fn same_named_device(left: &InputPeripheral, right: &InputPeripheral) -> bool {
    left.kind == right.kind
        && !left.name.trim().is_empty()
        && !normalized_device_name(&left.name).is_empty()
        && normalized_device_name(&left.name) == normalized_device_name(&right.name)
        && match (left.usb_ids, right.usb_ids) {
            (Some(a), Some(b)) => a == b,
            _ => left.connection() == right.connection(),
        }
}

fn merge(devices: &mut Vec<InputPeripheral>, readings: Vec<Reading>, provider: &str) {
    let entries: Vec<_> = readings
        .into_iter()
        .filter_map(|r| r.into_device(provider))
        .collect();
    // Compare only to the prior source. Preserve two identical devices reported by one reader.
    let prior = devices.len();
    let mut claimed = vec![false; prior];
    for new in entries {
        let mut candidates = devices[..prior].iter().enumerate().filter(|(index, old)| {
            !claimed[*index]
                && (old.id == new.id
                    || same_named_device(old, &new)
                    || (old.kind == new.kind
                        && old.usb_ids.is_some()
                        && old.usb_ids == new.usb_ids
                        && (provider == "headsetcontrol-"
                            || (old.battery.is_none() && old.status.level_label().is_none()))))
        });
        if let Some((index, _)) = candidates.next().filter(|_| candidates.next().is_none()) {
            claimed[index] = true;
            if devices[index].status != InputStatus::WiredOnly
                && devices[index].battery.is_none()
                && (new.battery.is_some() || devices[index].status.level_label().is_none())
            {
                devices[index].battery = new.battery;
                if devices[index].usb_ids.is_some() && devices[index].usb_ids == new.usb_ids {
                    devices[index].name = new.name;
                }
                if new.status != InputStatus::Unknown {
                    devices[index].status = new.status;
                    devices[index].connection_override = new.connection_override;
                }
            }
        } else {
            devices.push(new);
        }
    }
}

#[derive(Deserialize)]
struct HeadsetControlResponse {
    #[serde(default)]
    devices: Vec<HeadsetControlDevice>,
}

#[derive(Deserialize)]
struct HeadsetControlDevice {
    device: String,
    id_vendor: String,
    id_product: String,
    battery: Option<HeadsetControlBattery>,
}

#[derive(Deserialize)]
struct HeadsetControlBattery {
    status: String,
    level: i32,
}

impl HeadsetControlDevice {
    fn into_reading(self) -> Option<Reading> {
        let battery = self.battery?;
        if !matches!(
            battery.status.as_str(),
            "BATTERY_AVAILABLE" | "BATTERY_CHARGING"
        ) || !(0..=100).contains(&battery.level)
        {
            return None;
        }
        let vid = u16::from_str_radix(self.id_vendor.strip_prefix("0x")?, 16).ok()?;
        let pid = u16::from_str_radix(self.id_product.strip_prefix("0x")?, 16).ok()?;
        Some(Reading {
            key: format!("{vid:04x}:{pid:04x}"),
            name: self.device,
            kind: "headset".into(),
            percent: battery.level,
            charging: battery.status == "BATTERY_CHARGING",
            connected: true,
            vid,
            pid,
            ..Reading::default()
        })
    }
}

#[cfg(windows)]
pub fn poll(devices: &mut Vec<InputPeripheral>) -> Option<CachedPhoneBattery> {
    use std::time::Duration;
    let exe = std::env::current_exe().ok()?;
    let mut cached_phone = None;
    let sources = [
        (exe.clone(), "--batteryhub-probe", "batteryhub-"),
        (
            exe.with_file_name("GearDot-DeviceProbe.exe"),
            "--json",
            "wdbm-",
        ),
    ];
    for (path, arg, prefix) in sources {
        match crate::bluetooth::run_probe::<Vec<Reading>>(
            &path,
            &[arg],
            Duration::from_secs(20),
            512 * 1024,
            None,
        ) {
            Ok(mut readings) => {
                readings.retain(|reading| {
                    if reading.key == "phonelink" && reading.kind == "phone" {
                        cached_phone = reading.cached_phone();
                        return false;
                    }
                    true
                });
                let _ = wireless_battery_monitor::diagnostics::Log::Monitor
                    .event("device driver", &format!("{prefix} responded"));
                merge(devices, readings, prefix);
            }
            Err(error) => {
                let _ = wireless_battery_monitor::diagnostics::Log::Monitor
                    .event("device driver", &format!("{prefix}: {error}"));
            }
        }
    }
    // Use the packaged executable at an absolute path. No search on PATH or runtime download.
    let result = crate::bluetooth::run_probe::<HeadsetControlResponse>(
        &exe.with_file_name("HeadsetControl.exe"),
        &["-b", "-o", "json", "--timeout", "1500"],
        Duration::from_secs(12),
        128 * 1024,
        Some(1),
    );
    let log = wireless_battery_monitor::diagnostics::Log::Monitor;
    match result {
        Ok(response) => {
            let _ = log.event("device driver", "headsetcontrol- responded");
            merge(
                devices,
                response
                    .devices
                    .into_iter()
                    .filter_map(HeadsetControlDevice::into_reading)
                    .collect(),
                "headsetcontrol-",
            );
        }
        Err(error) => {
            let _ = log.event("device driver", &format!("headsetcontrol-: {error}"));
        }
    }
    cached_phone
}

impl Reading {
    fn cached_phone(&self) -> Option<CachedPhoneBattery> {
        let percent = u8::try_from(self.percent)
            .ok()
            .filter(|&level| level <= 100)?;
        (self.connected && !self.asleep).then_some(CachedPhoneBattery {
            percent,
            charging: self.charging,
        })
    }
}

#[cfg(windows)]
pub fn g435_fallback() -> Option<wireless_battery_monitor::battery::BatteryReading> {
    use std::time::Duration;
    use wireless_battery_monitor::battery::{BatteryReading, Connection};
    let exe = std::env::current_exe().ok()?;
    for (executable, argument, driver) in [
        (exe.clone(), "--batteryhub-g435-probe", "BatteryHub"),
        (
            exe.with_file_name("GearDot-DeviceProbe.exe"),
            "--g435-json",
            "W.D.B.M",
        ),
    ] {
        let Ok(readings) = crate::bluetooth::run_probe::<Vec<Reading>>(
            &executable,
            &[argument],
            Duration::from_secs(5),
            16 * 1024,
            None,
        ) else {
            continue;
        };
        if let Some(reading) = readings.into_iter().find_map(|reading| {
            if !reading.connected
                || reading.asleep
                || (reading.vid, reading.pid) != (0x046d, 0x0adf)
            {
                return None;
            }
            BatteryReading::from_voltage(u16::try_from(reading.millivolts).ok()?).ok()
        }) {
            let _ = wireless_battery_monitor::diagnostics::Log::Monitor.event(
                "device driver",
                &format!("Logitech G435 fallback: {driver}"),
            );
            return Some(reading.with_connection(Connection::Dongle));
        }
    }
    None
}

#[cfg(windows)]
pub fn write_batteryhub_response(g435_only: bool) -> Result<(), Box<dyn std::error::Error>> {
    use batteryhub::DeviceKind;
    let devices = if g435_only {
        batteryhub::device::poll_g435_adf()
    } else {
        batteryhub::poll_all_devices()
    };
    let readings: Vec<_> = devices
        .into_iter()
        .map(|r| {
            let (vid, pid) = r.hw.unwrap_or((r.vid, 0));
            let full = r.is_full();
            Reading {
                key: r.key,
                name: r.name,
                via: r.via,
                kind: match r.kind {
                    DeviceKind::Mouse => "mouse",
                    DeviceKind::Keyboard => "keyboard",
                    DeviceKind::Headset | DeviceKind::Earbuds => "headset",
                    DeviceKind::Gamepad => "gamepad",
                    _ => "other",
                }
                .to_owned(),
                level: r.level_text,
                percent: r.percent,
                charging: r.charging,
                full,
                connected: r.connected,
                asleep: r.asleep,
                millivolts: r.millivolts,
                vid,
                pid,
            }
        })
        .collect();
    serde_json::to_writer(std::io::stdout().lock(), &readings)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(name: &str) -> Reading {
        Reading {
            key: name.into(),
            name: name.into(),
            via: "2.4 GHz receiver".into(),
            kind: "mouse".into(),
            percent: 74,
            connected: true,
            vid: 1,
            pid: 2,
            ..Reading::default()
        }
    }

    #[test]
    fn shares_results_across_providers_without_overwriting_a_good_reading() {
        let mut devices = Vec::new();
        merge(&mut devices, vec![reading("Mouse A")], "rust-");
        merge(
            &mut devices,
            vec![Reading {
                percent: 80,
                ..reading("Mouse-A")
            }],
            "cpp-",
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].battery.unwrap().percent, 74);
    }

    #[test]
    fn retains_distinct_devices_and_rejects_unavailable_readings() {
        let mut devices = Vec::new();
        merge(
            &mut devices,
            vec![
                reading("Mouse A"),
                reading("Mouse B"),
                Reading {
                    percent: -1,
                    ..reading("Unknown")
                },
                Reading {
                    connected: false,
                    ..reading("Off")
                },
            ],
            "cpp-",
        );
        assert_eq!(devices.len(), 2);
    }

    #[test]
    fn headsetcontrol_fallback_uses_usb_identity_and_ignores_invalid_levels() {
        let response: HeadsetControlResponse = serde_json::from_str(
            r#"{
            "devices": [
                {"device":"Wireless headset","id_vendor":"0x1234","id_product":"0x5678",
                 "battery":{"status":"BATTERY_CHARGING","level":73}},
                {"device":"Sleeping headset","id_vendor":"0x1234","id_product":"0x5679",
                 "battery":{"status":"BATTERY_UNAVAILABLE","level":-1}}
            ]
        }"#,
        )
        .unwrap();
        let mut devices = vec![Reading {
            key: "vendor-a".into(),
            name: "Previous model name".into(),
            via: "Dongle".into(),
            kind: "headset".into(),
            percent: 60,
            connected: true,
            vid: 0x1234,
            pid: 0x5678,
            ..Reading::default()
        }
        .into_device("wdbm-")
        .unwrap()];
        let entries: Vec<_> = response
            .devices
            .into_iter()
            .filter_map(HeadsetControlDevice::into_reading)
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].charging);
        merge(&mut devices, entries, "headsetcontrol-");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].battery.unwrap().percent, 60);
    }

    #[test]
    fn accepts_recent_phone_link_data_without_exposing_the_phone_name() {
        let mut reading = Reading {
            name: "My private phone name".into(),
            key: "phonelink".into(),
            kind: "phone".into(),
            connected: true,
            charging: true,
            percent: 74,
            ..Reading::default()
        };
        assert_eq!(
            reading.cached_phone(),
            Some(CachedPhoneBattery {
                percent: 74,
                charging: true
            })
        );
        reading.asleep = true;
        assert!(reading.cached_phone().is_none());
        reading.asleep = false;
        reading.percent = 101;
        assert!(reading.cached_phone().is_none());
    }

    #[test]
    fn coarse_readings_do_not_invent_percentages() {
        let device = Reading {
            level: "Low".into(),
            ..reading("Controller")
        }
        .into_device("cpp-")
        .unwrap();
        assert_eq!(device.status, InputStatus::Low);
        assert_eq!(
            device.connection(),
            wireless_battery_monitor::battery::Connection::Dongle
        );
        assert!(device.battery.is_none());
    }
}
