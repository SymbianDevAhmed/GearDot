//! Bounded local diagnostic event log. No telemetry or hardware identifiers.
use crate::{
    battery::{Connection, DeviceKind},
    preferences::{data_dir, InputCharge, LiveSnapshot},
};
use std::{
    fs, io,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
const MAX_BYTES: u64 = 256 * 1024;

#[derive(Clone, Copy)]
pub enum Log {
    Monitor,
    Ui,
}

impl Log {
    fn filename(self) -> &'static str {
        match self {
            Self::Monitor => "monitor.log",
            Self::Ui => "ui.log",
        }
    }
    pub fn event(self, category: &str, message: &str) -> io::Result<()> {
        let dir = data_dir()?;
        fs::create_dir_all(&dir)?;
        append(&dir, self.filename(), category, message)
    }
}

fn single_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(300)
        .collect()
}

pub fn report(snapshot: &LiveSnapshot, now: u64) -> String {
    use std::fmt::Write as _;
    let mut report = format!(
        "GearDot {} · Device diagnostics\n",
        env!("CARGO_PKG_VERSION")
    );
    let running = !snapshot.monitor_session.is_empty();
    let _ = writeln!(
        report,
        "Monitor: {}",
        if running { "running" } else { "unavailable" }
    );
    for (source, timestamp, limit) in [
        ("Bluetooth", snapshot.bluetooth_checked_at, 45),
        ("USB / Audio / XInput", snapshot.inputs_checked_at, 60),
    ] {
        let age = now.saturating_sub(timestamp);
        let status = if timestamp == 0 {
            "waiting for first scan".into()
        } else if age > limit {
            format!("last scan {age}s ago (stale)")
        } else {
            format!("last scan {age}s ago")
        };
        let _ = writeln!(report, "{source}: {status}");
    }
    for (source, error) in [
        ("Bluetooth", &snapshot.bluetooth_error),
        ("USB HID", &snapshot.inputs_hid_error),
        ("Windows Audio", &snapshot.audio_error),
    ] {
        if let Some(error) = error {
            let reason = if error.to_ascii_lowercase().contains("timed out") {
                "scan timed out"
            } else {
                "scan failed"
            };
            let _ = writeln!(report, "{source}: {reason}");
        }
    }

    let mut count = 0;
    let mut add =
        |name: &str, connection: Connection, battery: &str, usb_ids: Option<(u16, u16)>| {
            count += 1;
            let _ = write!(
                report,
                "- {} | {} | {battery}",
                report_name(name),
                connection.label()
            );
            if let Some((vid, pid)) = usb_ids {
                let _ = write!(report, " | USB {vid:04X}:{pid:04X}");
            }
            report.push('\n');
        };
    for kind in DeviceKind::ALL {
        let reading = snapshot.reading(kind, now);
        if let Some(level) = reading.level.filter(|_| reading.online()) {
            let battery = if reading.estimated {
                format!("~{level}% estimated")
            } else {
                format!("{level}% device-reported")
            };
            let usb_ids = match (kind, reading.connection) {
                (DeviceKind::Headset, Connection::Dongle) => Some((0x046d, 0x0adf)),
                (DeviceKind::Mouse, Connection::Dongle) => Some((0x373b, 0x1040)),
                (DeviceKind::Mouse, Connection::Wired) => Some((0x373b, 0x103f)),
                _ => None,
            };
            add(kind.name(), reading.connection, &battery, usb_ids);
        }
    }
    for reading in snapshot.connected_bluetooth(now) {
        let battery = reading.percent.map_or_else(
            || "Battery unavailable".into(),
            |level| format!("{level}% Windows-reported"),
        );
        add(&reading.name, Connection::Bluetooth, &battery, None);
    }
    for input in snapshot.connected_inputs(now) {
        let battery = if let Some(reading) = input.battery {
            let source = if reading.estimated {
                "estimated from device level"
            } else {
                "device-reported"
            };
            let prefix = if reading.estimated { "~" } else { "" };
            let status = match reading.charge {
                InputCharge::Charging => "charging",
                InputCharge::Discharging => "not charging",
                InputCharge::Full => "full",
            };
            let freshness = if reading.stale {
                format!("last read {}s ago", now.saturating_sub(reading.checked_at))
            } else {
                status.to_owned()
            };
            format!("{prefix}{}% {source} · {freshness}", reading.percent)
        } else if input.status == crate::preferences::InputStatus::WiredOnly {
            "No battery".into()
        } else {
            input.status.level_label().map_or_else(
                || "Battery unavailable".into(),
                |level| format!("{level} (no percentage)"),
            )
        };
        add(&input.name, input.connection(), &battery, input.usb_ids);
    }
    if count == 0 {
        report.push_str("No connected devices detected.\n");
    }
    if let Some(phone) = snapshot.cached_phone.filter(|_| {
        snapshot.inputs_checked_at != 0 && now.saturating_sub(snapshot.inputs_checked_at) <= 90
    }) {
        let charge = if phone.charging {
            " · charging when cached"
        } else {
            ""
        };
        let _ = writeln!(
            report,
            "Phone Link cache: {}%{charge} · connection unconfirmed",
            phone.percent
        );
    }
    report
}

fn report_name(name: &str) -> String {
    let name = single_line(name);
    let mut words = Vec::new();
    for word in name.split_whitespace() {
        let word_without_punctuation = word.trim_matches(|c: char| {
            matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '"' | ',' | ';')
        });
        let lower = word_without_punctuation.to_ascii_lowercase();
        if lower
            .split([':', '='])
            .next()
            .is_some_and(|label| matches!(label, "serial" | "s/n" | "sn" | "mac" | "address"))
        {
            words.push("[redacted]");
            break;
        }
        let hex_count = lower.bytes().filter(u8::is_ascii_hexdigit).count();
        if lower.contains(['#', '\\'])
            || (hex_count >= 12
                && lower
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b':' | b'-' | b'.')))
        {
            words.push("[redacted]");
        } else {
            words.push(word);
        }
    }
    words.join(" ")
}

fn append(dir: &Path, name: &str, category: &str, message: &str) -> io::Result<()> {
    let path = dir.join(name);
    if fs::metadata(&path).is_ok_and(|file| file.len() >= MAX_BYTES) {
        let previous = dir.join(format!("{name}.previous"));
        fs::rename(&path, previous)?;
    }
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(
        file,
        "{time} [{}] {}",
        single_line(category),
        single_line(message)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_lines_cannot_inject_new_records() {
        assert_eq!(single_line("hello\nworld\t!"), "hello world !");
        assert_eq!(single_line(&"a".repeat(500)).len(), 300);
    }

    #[test]
    fn report_shares_model_ids_without_sharing_device_instance_ids() {
        use crate::preferences::{InputBattery, InputKind, InputPeripheral, InputStatus};
        let snapshot = LiveSnapshot {
            inputs_checked_at: 100,
            input_peripherals: vec![InputPeripheral {
                id: "private-device-instance".into(),
                name: "DualSense".into(),
                kind: InputKind::Gamepad,
                status: InputStatus::Usb,
                connection_override: None,
                usb_ids: Some((0x054c, 0x0ce6)),
                battery: Some(InputBattery {
                    percent: 35,
                    charge: InputCharge::Charging,
                    estimated: true,
                    checked_at: 95,
                    stale: true,
                }),
            }],
            ..Default::default()
        };
        let text = report(&snapshot, 105);
        assert!(text.contains("USB 054C:0CE6"));
        assert!(text.contains("~35% estimated from device level · last read 10s ago"));
        assert!(!text.contains("private-device-instance"));
    }

    #[test]
    fn report_preserves_current_readings_without_addresses_or_raw_errors() {
        use crate::preferences::BluetoothReading;
        let snapshot = LiveSnapshot {
            monitor_session: "private-session".into(),
            bluetooth_checked_at: 100,
            bluetooth: vec![BluetoothReading {
                id: "private-device".into(),
                name: "Headphones 01:23:45:67:89:AB SN: ABC-UNIQUE".into(),
                percent: Some(17),
                kind: None,
            }],
            inputs_hid_error: Some(r"Failed to open C:\Users\private".into()),
            ..Default::default()
        };
        let report_text = report(&snapshot, 120);
        assert!(report_text.contains("17% Windows-reported"));
        assert!(report_text.contains("USB HID: scan failed"));
        for sensitive in ["01:23", "ABC-UNIQUE", "private", "Users"] {
            assert!(!report_text.contains(sensitive));
        }
        assert!(report(&snapshot, 146).contains("No connected devices detected"));
    }
}
