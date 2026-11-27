//! ASUS ROG Pelta headset over its 2.4 GHz dongle (VID 0B05, PID 1B84) or its USB cable (PID 1B82).
//!
//! Protocol as documented by pelta-control (MIT, github.com/jimm0thy/pelta-control, README
//! "Protocol": worked out from the dongle's HID descriptor and USB captures of Armoury Crate, the
//! tool tested on a real headset) and as used by the original C++ BatteryHub (GPL-3.0,
//! src/drivers/rog.cpp, with BatteryWatch, MIT, github.com/B1Chat/BatteryWatch); implemented here
//! from those facts. Vendor collection FF00:0001 on interface 3, report CC, 64 bytes each way:
//!
//!   CC 12 07  ->  CC 12 07 00 00 [5] sleep timer [6] battery % (FE = not known yet) [7] alert %
//!   CC 12 08  ->  CC 12 08 00 00 [5] 1 = charging
//!   a sleeping headset answers FF AA in bytes 5..6 (or 1..2)
//!
//! The dongle also pushes events on the same collection (CC 12 0E wake / sleep, CC 12 09 level
//! change); they are skipped while waiting for the answer. Only these two reads are sent; nothing
//! that changes a setting (CC 51 .., CC 41 ..).
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x0B05;
const DONGLE: u16 = 0x1B84;
const CABLE: u16 = 0x1B82;
const KEY: &str = "rog:pelta";
const REPORT: u8 = 0xCC;
const CMD_STATUS: u8 = 0x07;
const CMD_CHARGING: u8 = 0x08;
const WAIT_MS: u64 = 1200; // the first answer after a reconnect can take ~750 ms

pub fn request(cmd: u8) -> [u8; 64] {
    let mut r = [0u8; 64];
    r[..3].copy_from_slice(&[REPORT, 0x12, cmd]);
    r
}

#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// The answer to `cmd`: bytes from index 5 on.
    Data(Vec<u8>),
    Asleep,
}

/// Decodes one input report (report ID included) while waiting for the answer to `cmd`.
pub fn decode(r: &[u8], cmd: u8) -> Option<Answer> {
    if r.len() < 8 || r[0] != REPORT {
        return None;
    }
    if r[1] == 0xFF && r[2] == 0xAA {
        return Some(Answer::Asleep);
    }
    if r[1] != 0x12 {
        return None;
    }
    if r[5] == 0xFF && r[6] == 0xAA {
        return Some(Answer::Asleep);
    }
    (r[2] == cmd && r[3] == 0 && r[4] == 0).then(|| Answer::Data(r[5..].to_vec()))
}

/// Status answer data: battery percent, None while not known (FE).
pub fn level(data: &[u8]) -> Option<i32> {
    data.get(1).filter(|&&b| b <= 100).map(|&b| b as i32)
}

fn ask(d: &HidDev, cmd: u8) -> Option<Answer> {
    d.flush();
    if !d.write(&request(cmd)) {
        return None;
    }
    let end = Deadline::in_ms(WAIT_MS);
    while !end.passed() {
        if let Some(a) = d.read(end.left_ms()).and_then(|r| decode(&r, cmd)) {
            return Some(a);
        }
    }
    None
}

enum Reading {
    /// Level (None = not known yet), charging.
    Battery(Option<i32>, bool),
    Asleep,
}

fn read(c: &HidColl) -> Option<Reading> {
    let d = HidDev::open(c)?;
    match ask(&d, CMD_STATUS)? {
        Answer::Asleep => Some(Reading::Asleep),
        Answer::Data(data) => {
            let charging = matches!(ask(&d, CMD_CHARGING), Some(Answer::Data(ch)) if ch.first() == Some(&1));
            Some(Reading::Battery(level(&data), charging))
        }
    }
}

pub fn poll(ctx: &mut PollContext) {
    for pid in [DONGLE, CABLE] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.iface == 3 && c.page() == 0xFF00 && c.usage() == 0x0001 && c.in_len() >= 8 && c.out_len() >= 8);
        if colls.is_empty() {
            continue;
        }
        let cable = pid == CABLE;
        let base = DeviceStatus {
            key: KEY.into(),
            name: "ROG Pelta".into(),
            kind: DeviceKind::Headset,
            via: if cable { "USB cable" } else { "2.4 GHz dongle" }.into(),
            hw: Some((VID, pid)),
            grace_sec: 1800,
            ..Default::default()
        };
        let s = match colls.iter().find_map(read) {
            Some(Reading::Battery(level, charging)) => DeviceStatus { percent: level.unwrap_or(-1), charging: charging || cable, ..base },
            Some(Reading::Asleep) => DeviceStatus { asleep: true, ..base },
            None => DeviceStatus { connected: false, ..base },
        };
        // Dongle and cable both present (charging on the cable): one entry, the cable's reading wins.
        if let Some(o) = ctx.out.iter_mut().find(|o| o.key == KEY) {
            if s.connected && s.percent >= 0 {
                *o = DeviceStatus { hw: o.hw, ..s };
            }
            ctx.claim(VID, pid);
        } else {
            ctx.add(s, VID, pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(head: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 64];
        f[..head.len()].copy_from_slice(head);
        f
    }

    #[test]
    fn requests() {
        assert_eq!(request(CMD_STATUS)[..4], [0xCC, 0x12, 0x07, 0x00]);
        assert_eq!(request(CMD_CHARGING)[..3], [0xCC, 0x12, 0x08]);
    }

    #[test]
    fn status_and_charging() {
        // Sleep timer 10 min, 64 %, alert at 15 %.
        let Some(Answer::Data(d)) = decode(&frame(&[0xCC, 0x12, 0x07, 0x00, 0x00, 10, 64, 15, 1]), CMD_STATUS) else { panic!() };
        assert_eq!(level(&d), Some(64));
        let Some(Answer::Data(d)) = decode(&frame(&[0xCC, 0x12, 0x07, 0x00, 0x00, 10, 0xFE, 15]), CMD_STATUS) else { panic!() };
        assert_eq!(level(&d), None); // not known yet
        assert_eq!(decode(&frame(&[0xCC, 0x12, 0x08, 0x00, 0x00, 0x01]), CMD_CHARGING), Some(Answer::Data(frame(&[0x01])[..59].to_vec())));
    }

    #[test]
    fn asleep() {
        assert_eq!(decode(&frame(&[0xCC, 0x12, 0x07, 0x00, 0x00, 0xFF, 0xAA]), CMD_STATUS), Some(Answer::Asleep));
        assert_eq!(decode(&frame(&[0xCC, 0xFF, 0xAA]), CMD_STATUS), Some(Answer::Asleep));
    }

    #[test]
    fn events_are_skipped() {
        assert_eq!(decode(&frame(&[0xCC, 0x12, 0x0E, 0x00, 0x00, 0x00]), CMD_STATUS), None); // wake
        assert_eq!(decode(&frame(&[0xCC, 0x12, 0x09, 0x00, 0x00, 50]), CMD_STATUS), None); // level push
        assert_eq!(decode(&frame(&[0xCB, 0x12, 0x07, 0x00, 0x00, 10, 64]), CMD_STATUS), None);
    }
}
