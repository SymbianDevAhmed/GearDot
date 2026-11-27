//! Fractal Design Scape headset over its 2.4 GHz dongle (VID 36BC, PID 0001).
//!
//! Protocol as documented by scapectl (github.com/charlietran/scapectl, README "USB HID Protocol
//! Reference", observed from Fractal's own Adjust Pro web app and used on the headset on Windows and
//! macOS) and as also read by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! src/drivers/fractal.cpp); implemented here from those facts. Vendor collection FF00:0001, 64-byte
//! output / input report 02:
//!
//!   request  02 F1 21 00 ...      F1 = relayed to the headset, 21 = status poll
//!   reply    02 F1 21 00 .. [15]  [15] (status byte 14) = battery %
//!
//! The headset answers through the dongle in about 60 ms; when it is off nothing comes back. The
//! dongle's own state (11 21) flaps and is not used. No charging flag. Only this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x36BC;
const PID: u16 = 0x0001;
const REPORT: u8 = 0x02;
const HEADSET: u8 = 0xF1;
const STATUS: u8 = 0x21;
const REPLY_MS: u64 = 800;

pub fn request() -> [u8; 3] {
    [REPORT, HEADSET, STATUS]
}

/// Battery % from a reply to the status poll (report id included).
pub fn decode(r: &[u8]) -> Option<i32> {
    (r.len() >= 16 && r[..3] == request() && r[3] == 0 && r[15] <= 100).then_some(r[15] as i32)
}

/// Some(level) when the headset answered, Some(None) when only the dongle is there, None on error.
fn read(c: &HidColl) -> Option<Option<i32>> {
    let d = HidDev::open(c)?;
    d.flush();
    if !d.write(&request()) {
        return None;
    }
    let end = Deadline::in_ms(REPLY_MS);
    while !end.passed() {
        if let Some(level) = d.read(end.left_ms()).and_then(|r| decode(&r)) {
            return Some(Some(level));
        }
    }
    Some(None)
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF00 && c.usage() == 0x0001 && c.in_len() >= 16 && c.out_len() >= 4);
    let Some(found) = colls.iter().find_map(read) else { return };
    let base = DeviceStatus {
        key: "fractal:scape".into(),
        name: "Fractal Scape".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz dongle".into(),
        hw: Some((VID, PID)),
        grace_sec: 60,
        ..Default::default()
    };
    let s = match found {
        Some(level) => DeviceStatus { percent: level, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        let mut r = vec![0u8; 64];
        r[..4].copy_from_slice(&[0x02, 0xF1, 0x21, 0x00]);
        r[15] = 73;
        assert_eq!(decode(&r), Some(73));
        r[3] = 1; // status not OK
        assert_eq!(decode(&r), None);
        r[3] = 0;
        r[2] = 0x01; // another command's reply
        assert_eq!(decode(&r), None);
        r[2] = 0x21;
        r[1] = 0x11; // the dongle's own state
        assert_eq!(decode(&r), None);
    }
}
