//! ASTRO A50 (Gen 4) headset on its base station (VID 9886, PID 002C, base switched to PC mode).
//!
//! Protocol as documented by HeadsetControl (GPL-3.0, github.com/Sapd/HeadsetControl,
//! lib/devices/logitech_astro_a50_gen4.hpp, hardware-verified; command codes from eh-fifty, MIT);
//! implemented here from those facts. Vendor collection FF32:0074, report 02, 64-byte frames, one
//! reply per request:
//!
//!   request  02 CC 00 ...                CC = 54 headset status, 7C battery
//!   reply    02 SS LL PP ...             SS = 02 ok with payload, 00 accepted, 01 error
//!   54       PP bit 1 = live link to the headset (bit 0 = docked)
//!   7C       PP bits 0-6 = level in percent, bit 7 = charging
//!
//! Only these two reads are sent. Without a link the base keeps answering the last level, so the
//! headset is shown off then.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x9886;
const PID: u16 = 0x002C;
const LEN: usize = 64;
const REPORT: u8 = 0x02;
const CMD_STATUS: u8 = 0x54;
const CMD_BATTERY: u8 = 0x7C;
const REPLY_MS: u64 = 1000;

pub fn request(cmd: u8) -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[0] = REPORT;
    r[1] = cmd;
    r
}

/// First payload byte of an "ok" reply.
pub fn payload(r: &[u8]) -> Option<u8> {
    (r.len() >= 4 && r[0] == REPORT && r[1] == 0x02).then(|| r[3])
}

/// (level, charging) from the battery byte.
pub fn battery(raw: u8) -> Option<(i32, bool)> {
    let level = (raw & 0x7F) as i32;
    (level <= 100).then_some((level, raw & 0x80 != 0))
}

fn ask(d: &HidDev, cmd: u8) -> Option<u8> {
    d.flush();
    if !d.write(&request(cmd)) {
        return None;
    }
    let end = Deadline::in_ms(REPLY_MS);
    while !end.passed() {
        if let Some(r) = d.read(end.left_ms()) {
            // One reply per request: the first frame of report 02 is the answer.
            if r.first() == Some(&REPORT) {
                return payload(&r);
            }
        }
    }
    None
}

/// Some(None) = base answers, headset not linked.
fn read(c: &HidColl) -> Option<Option<(i32, bool)>> {
    let d = HidDev::open(c)?;
    let status = ask(&d, CMD_STATUS)?;
    if status & 0x02 == 0 {
        return Some(None);
    }
    Some(ask(&d, CMD_BATTERY).and_then(battery))
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF32 && c.usage() == 0x0074 && c.out_len() as usize == LEN && c.in_len() as usize == LEN);
    let Some(first) = colls.first() else { return };
    let base = DeviceStatus {
        key: "astro:a50gen4".into(),
        name: "ASTRO A50".into(),
        kind: DeviceKind::Headset,
        via: "USB base station".into(),
        hw: Some((VID, PID)),
        grace_sec: 300,
        ..Default::default()
    };
    let s = match read(first) {
        Some(Some((level, charging))) => DeviceStatus { percent: level, charging, ..base },
        Some(None) => DeviceStatus { connected: false, ..base },
        None => return,
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames() {
        assert_eq!(request(CMD_BATTERY)[..3], [0x02, 0x7C, 0x00]);
        assert!(request(CMD_STATUS)[2..].iter().all(|&b| b == 0));
    }

    #[test]
    fn replies() {
        let mut r = vec![0u8; 64];
        r[..4].copy_from_slice(&[0x02, 0x02, 0x01, 0xD5]);
        assert_eq!(payload(&r).and_then(battery), Some((85, true)));
        r[3] = 0x37;
        assert_eq!(payload(&r).and_then(battery), Some((55, false)));
        r[1] = 0x01; // error
        assert_eq!(payload(&r), None);
    }
}
