//! AJAZZ AJ159P and other mice on the AJ179-family 2.4 GHz receiver 249A:5C2F.
//!
//! Protocol as documented by HaloBattery (MIT, github.com/HeyOkay/HaloBattery, providers/ajazz.py: USB
//! captures of AJAZZ's driver with an AJ179 V2 MAX on this receiver, level confirmed there) and BatteryTray
//! (MIT, github.com/qu1ncky/BatteryTray, src/providers/devices/mice_platforms.rs, after aj179-linux);
//! implemented here from those facts. Interface 2, 33-byte reports (report id 0 + 32 bytes):
//!
//!   request  10 00 ...
//!   reply    10 ST .. .. MM MM MM MM .. .. .. .. CH LL LK .. SS
//!            ST  0 ok, 2 mouse away         MM  model ("M179", "N625" ...)
//!            CH  1 charging, 2 full         LL  level 0..100
//!            LK  mouse linked               SS  (byte 31) sum of bytes 4..30
//!
//! Only this read is sent. The receiver answers for a mouse that is away with its last level, which is
//! not used: LK tells. Several brands use the receiver. Not tested on hardware by this project.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};

const VID: u16 = 0x249A;
const PID: u16 = 0x5C2F;
const IFACE: i32 = 2;
const CMD_INFO: u8 = 0x10;
const LEN: usize = 32;
const REPLY_MS: u64 = 500;

/// Usage page and usage of the receiver's vendor collection, and the name when there is nothing better.
const COLLECTIONS: [(u16, u16, &str); 2] = [
    (0x0001, 0x0000, "AJAZZ wireless mouse"), // AJAZZ AJ159P (#216)
    // MEETION mice ("MeetionWireless"): the same 33-byte channel declared as FF01:0010, found by the
    // reporter in #216; the original C++ BatteryHub reads it with this request (misc_mice.cpp).
    (0xFF01, 0x0010, "MEETION wireless mouse"),
];

/// Report id 0 first; the checksum of an empty payload is 0.
pub fn request() -> [u8; LEN + 1] {
    let mut r = [0u8; LEN + 1];
    r[1] = CMD_INFO;
    r
}

#[derive(Debug, PartialEq, Eq)]
pub enum Info {
    /// Level, charging, full.
    Battery(i32, bool, bool),
    Away,
}

/// A reply (report id 0 already stripped by the reader).
pub fn parse(r: &[u8]) -> Option<Info> {
    if r.len() < LEN || r[0] != CMD_INFO {
        return None;
    }
    match r[1] {
        2 => Some(Info::Away),
        // r[3] is the data length: 0 in our own request read back.
        0 if r[3] != 0 && r[4..LEN - 1].iter().fold(0u8, |a, &b| a.wrapping_add(b)) == r[LEN - 1] => {
            if r[14] == 0 {
                Some(Info::Away)
            } else {
                (r[13] <= 100).then(|| Info::Battery(r[13] as i32, r[12] == 1, r[12] == 2))
            }
        }
        _ => None,
    }
}

fn read(c: &HidColl) -> Option<Info> {
    let d = HidDev::open(c)?;
    (0..3).find_map(|_| d.ask(&request(), REPLY_MS, parse))
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    for (page, usage, name) in COLLECTIONS {
        let colls = ctx.find(VID, PID, |c| {
            c.iface == IFACE && c.page() == page && c.usage() == usage && c.in_len() as usize == LEN + 1 && c.out_len() as usize == LEN + 1
        });
        let Some(info) = colls.iter().find_map(read) else { continue };
        let base = DeviceStatus {
            key: "ajazz:5c2f".into(),
            name: name.into(),
            kind: DeviceKind::Mouse,
            via: "2.4 GHz receiver".into(),
            hw: Some((VID, PID)),
            grace_sec: 1800,
            ..Default::default()
        };
        let s = match info {
            Info::Battery(percent, charging, full) => DeviceStatus { percent, charging, full, ..base },
            Info::Away => DeviceStatus { asleep: true, ..base },
        };
        ctx.add(s, VID, PID);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(power: u8, level: u8, linked: u8) -> Vec<u8> {
        // HaloBattery's capture: 10 00 01 0b "N625" 00 00 11 01 00 4b 01, checksum 49.
        let mut r = vec![0x10, 0x00, 0x01, 0x0B, 0x4E, 0x36, 0x32, 0x35, 0x00, 0x00, 0x11, 0x01, power, level, linked];
        r.resize(LEN, 0);
        r[31] = r[4..31].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        r
    }

    #[test]
    fn captured_reply() {
        let r = reply(0, 0x4B, 1);
        assert_eq!(r[31], 0x49);
        assert_eq!(parse(&r), Some(Info::Battery(75, false, false)));
    }

    #[test]
    fn states() {
        assert_eq!(parse(&reply(1, 40, 1)), Some(Info::Battery(40, true, false)));
        assert_eq!(parse(&reply(2, 100, 1)), Some(Info::Battery(100, false, true)));
        assert_eq!(parse(&reply(0, 75, 0)), Some(Info::Away)); // last level of a mouse that is away
        let mut away = vec![0u8; LEN];
        away[..2].copy_from_slice(&[0x10, 2]);
        assert_eq!(parse(&away), Some(Info::Away));
    }

    #[test]
    fn rejected() {
        let mut bad = reply(0, 75, 1);
        bad[31] ^= 1;
        assert_eq!(parse(&bad), None);
        assert_eq!(parse(&reply(0, 101, 1)), None);
        assert_eq!(parse(&request()[1..]), None); // the request read back
        let mut push = vec![0u8; LEN]; // the receiver's own announcement
        push[..3].copy_from_slice(&[0xC0, 0x01, 75]);
        assert_eq!(parse(&push), None);
    }
}
