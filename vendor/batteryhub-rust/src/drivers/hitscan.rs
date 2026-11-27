//! Contributed in issue #427 by Dyzson, measured on a Hyperlight X over the 8K dongle.
//!
//! Hitscan mice (VID 3770: Hyperlight, Hyperlight N, Hyperlight X) over their receivers, e.g. the
//! "Hitscan 8K Dongle" 3770:0300, or on their USB cables (3770:0136 Hyperlight X, 0194 Hyperlight N).
//!
//! The supplier status query WLMOUSE and the DriverCore mice answer too (wlmouse.rs, drivercore.rs),
//! as the original C++ BatteryHub reads Hitscan mice (src/drivers/glorious.cpp, after Hitscan's web
//! configurator control.hitscan.com). The vendor collection with a 65-byte feature report (report
//! ID 0; FFFF:0000 on the 8K dongle) answers:
//!
//!   request  (feature SET)  00 | 00 00 02 02 00 83 | 00 ...
//!   reply    (feature GET)  00 | SS 00 02 02 00 83 CC NN ...
//!            SS  A1 / A2 awake, A0 waking up (asked once more), any other Ax asleep
//!            CC  charging (01)
//!            NN  battery level 0..100 % (0 is shown as 1 %, as in the original)
//!
//! Measured on a Hyperlight X over the 8K dongle, 200 ms after the request:
//!
//!   00 A1 00 02 02 00 83 00 59 00 ..   awake, not charging, 89 %
//!
//! A report that is not this protocol (status byte not Ax, no 83) is ignored, and the product is
//! left to the other drivers. Only this query is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{hex4, sleep_ms};

const VID: u16 = 0x3770;
const QUERY: [u8; 7] = [0x00, 0x00, 0x00, 0x02, 0x02, 0x00, 0x83];
/// The receiver needs a moment to fetch the answer from the mouse (the original waits as long).
const REPLY_MS: u64 = 200;
/// A mouse that is just waking up is asked once more after this.
const WAKING_MS: u64 = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading {
    /// Level 1..=100 % and the charging flag.
    Awake(i32, bool),
    Waking,
    Asleep,
}

fn name(pid: u16) -> &'static str {
    match pid {
        0x0136 => "Hitscan Hyperlight X",
        0x0194 => "Hitscan Hyperlight N",
        _ => "Hitscan Hyperlight",
    }
}

/// The mouse itself on its USB cable (it charges there), not a receiver.
fn is_cable(pid: u16) -> bool {
    matches!(pid, 0x0136 | 0x0194)
}

/// Firmware-update (DFU) mode of the Hyperlight X (1136, "Hitscan Hyperlight X DFU") and of the 8K
/// dongle (1300, "Hitscan 8K Dongle DFU"), seen while Hitscan Control updated them. Each has an
/// FFFF:0000 collection; nothing is sent to them, and no other driver gets them either.
fn is_dfu(pid: u16) -> bool {
    matches!(pid, 0x1136 | 0x1300)
}

/// A status reply (report ID in byte 0). None: not an answer of this protocol.
pub fn parse(r: &[u8]) -> Option<Reading> {
    if r.len() < 9 || r[6] != 0x83 || (r[1] & 0xF0) != 0xA0 {
        return None;
    }
    Some(match r[1] {
        0xA1 | 0xA2 if r[8] <= 100 => Reading::Awake(i32::from(r[8].max(1)), r[7] == 0x01),
        0xA0 => Reading::Waking,
        _ => Reading::Asleep,
    })
}

fn query(c: &HidColl) -> Option<Reading> {
    let d = HidDev::open_feature_only(c)?;
    for attempt in 0..2 {
        if !d.set_feature(&QUERY) {
            return None;
        }
        sleep_ms(REPLY_MS);
        match parse(&d.get_feature(0)?)? {
            Reading::Waking if attempt == 0 => sleep_ms(WAKING_MS),
            Reading::Waking => return Some(Reading::Asleep),
            r => return Some(r),
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    for (vid, pid) in ctx.products(VID) {
        if is_dfu(pid) {
            ctx.claim(vid, pid);
            continue;
        }
        let colls = ctx.find(vid, pid, |c| c.feat_len() == 65);
        let Some(reading) = colls.iter().find_map(query) else { continue };
        let cable = is_cable(pid);
        let base = DeviceStatus {
            key: format!("hitscan:{}", hex4(pid)),
            name: name(pid).into(),
            kind: DeviceKind::Mouse,
            via: if cable { "USB cable" } else { "2.4 GHz receiver" }.into(),
            charging: cable,
            hw: Some((vid, pid)),
            grace_sec: 60,
            ..Default::default()
        };
        let s = match reading {
            Reading::Awake(level, charging) => DeviceStatus { percent: level, charging: charging || cable, ..base },
            Reading::Waking | Reading::Asleep => DeviceStatus { asleep: true, ..base },
        };
        ctx.add(s, vid, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        // Measured on a Hyperlight X over the 8K dongle (3770:0300).
        let mut r = vec![0u8; 65];
        r[..9].copy_from_slice(&[0x00, 0xA1, 0x00, 0x02, 0x02, 0x00, 0x83, 0x00, 0x59]);
        assert_eq!(parse(&r), Some(Reading::Awake(89, false)));
        r[7] = 0x01;
        assert_eq!(parse(&r), Some(Reading::Awake(89, true)));
        r[8] = 0;
        assert_eq!(parse(&r), Some(Reading::Awake(1, true)));
        r[1] = 0xA4;
        assert_eq!(parse(&r), Some(Reading::Asleep));
        r[1] = 0xA0;
        assert_eq!(parse(&r), Some(Reading::Waking));
    }

    #[test]
    fn not_a_reply() {
        // Our own query read back, and a report too short to hold a reply.
        let mut q = vec![0u8; 65];
        q[..7].copy_from_slice(&QUERY);
        assert_eq!(parse(&q), None);
        let r = [0x00, 0xA1, 0x00, 0x02, 0x02, 0x00, 0x83, 0x00];
        assert_eq!(parse(&r), None);
        // A level out of range is not shown (asleep: the last value stays), as in the original.
        let r = [0x00, 0xA1, 0x00, 0x02, 0x02, 0x00, 0x83, 0x00, 0xFF];
        assert_eq!(parse(&r), Some(Reading::Asleep));
    }

    #[test]
    fn names_and_cables() {
        assert_eq!(name(0x0136), "Hitscan Hyperlight X");
        assert_eq!(name(0x0300), "Hitscan Hyperlight");
        assert!(is_cable(0x0136) && is_cable(0x0194) && !is_cable(0x0300));
        assert!(is_dfu(0x1136) && is_dfu(0x1300) && !is_dfu(0x0136) && !is_dfu(0x0300));
    }
}
