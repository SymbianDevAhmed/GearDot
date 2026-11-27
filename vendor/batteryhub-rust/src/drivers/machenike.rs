//! Machenike L7 Mix (L7 Arctic) mouse on its 2.4 GHz receiver 248A:FF4C or USB cable 248A:FF4E.
//!
//! Protocol as documented by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! docs/machenike.md: from Machenike's own software and read-only queries on a real mouse, both
//! connections tested there); implemented here from those facts. Vendor collection 008C:0001, feature
//! report 51:
//!
//!   set feature  51 06 00 ...
//!   get feature  51 06 .. .. 8A 24 4E FF .. .. LK LL CC
//!                8A 24 4E FF  the mouse's own ids
//!                LK           bits 0..2 = 1 receiver / 0 cable, bit 3 = mouse linked (receiver only)
//!                LL, CC       level 0..100, 1 = charging
//!
//! With the link bit clear the receiver repeats its last level, which is not used. The vendor id is
//! Telink's, shared by many makers: only these ids with the product string "L7 Mix" are asked. Not
//! tested on hardware by this project (issue #207).
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{self, HidColl, HidDev};
use crate::util::sleep_ms;

const VID: u16 = 0x248A;
/// Product id, interface, cable.
const PRODUCTS: [(u16, i32, bool); 2] = [(0xFF4E, 2, true), (0xFF4C, 1, false)];
const KEY: &str = "machenike:l7mix";
const REPORT: u8 = 0x51;
const CMD_STATE: u8 = 0x06;

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    /// Level, charging.
    Battery(i32, bool),
    Away,
}

/// Decodes the feature report read back (report id included).
pub fn decode(r: &[u8], cable: bool) -> Option<State> {
    if r.len() < 13 || r[..2] != [REPORT, CMD_STATE] || r[4..8] != [0x8A, 0x24, 0x4E, 0xFF] || r[10] & 0x07 != !cable as u8 {
        return None;
    }
    if !cable && r[10] & 0x08 == 0 {
        return Some(State::Away);
    }
    (r[11] <= 100 && r[12] <= 1).then(|| State::Battery(r[11] as i32, r[12] == 1))
}

fn read(c: &HidColl, cable: bool) -> Option<State> {
    let d = HidDev::open_feature_only(c)?;
    if !d.set_feature(&[REPORT, CMD_STATE]) {
        return None;
    }
    (0..5).find_map(|_| {
        sleep_ms(40);
        d.get_feature(REPORT).and_then(|r| decode(&r, cable))
    })
}

pub fn poll(ctx: &mut PollContext) {
    for (pid, iface, cable) in PRODUCTS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.iface == iface && c.page() == 0x008C && c.usage() == 0x0001 && c.feat_len() >= 21);
        let colls: Vec<HidColl> = colls.into_iter().filter(|c| hid::product_string(c).contains("L7 Mix")).collect();
        if colls.is_empty() {
            continue;
        }
        let base = DeviceStatus {
            key: KEY.into(),
            name: "Machenike L7 Mix".into(),
            kind: DeviceKind::Mouse,
            via: if cable { "USB cable" } else { "2.4 GHz receiver" }.into(),
            hw: Some((VID, pid)),
            grace_sec: 1800,
            ..Default::default()
        };
        let s = match colls.iter().find_map(|c| read(c, cable)) {
            Some(State::Battery(percent, charging)) => DeviceStatus { percent, charging, ..base },
            _ => DeviceStatus { connected: false, ..base },
        };
        // The cable (asked first) speaks for the mouse while it is plugged in.
        if cable && !s.connected || ctx.out.iter().any(|o| o.key == KEY) {
            ctx.claim(VID, pid);
        } else {
            ctx.add(s, VID, pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_replies() {
        // TaskbarVitals' captures: receiver 64 %, cable 74 % charging, receiver without link.
        let rx = [0x51, 0x06, 0x01, 0x00, 0x8A, 0x24, 0x4E, 0xFF, 0x05, 0x01, 0x09, 0x40, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode(&rx, false), Some(State::Battery(64, false)));
        let cable = [0x51, 0x06, 0x01, 0x00, 0x8A, 0x24, 0x4E, 0xFF, 0x05, 0x01, 0x00, 0x4A, 0x01, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode(&cable, true), Some(State::Battery(74, true)));
        let away = [0x51, 0x06, 0x01, 0x00, 0x8A, 0x24, 0x4E, 0xFF, 0x05, 0x01, 0x01, 0x40, 0x01, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode(&away, false), Some(State::Away));
    }

    #[test]
    fn rejected() {
        let mut r = [0x51, 0x06, 0x01, 0x00, 0x8A, 0x24, 0x4E, 0xFF, 0x05, 0x01, 0x09, 101, 0x00];
        assert_eq!(decode(&r, false), None);
        r[11] = 50;
        assert_eq!(decode(&r, true), None); // a receiver's mode on the cable
        r[4] = 0x34; // another maker's mouse
        assert_eq!(decode(&r, false), None);
        assert_eq!(decode(&[0x51, 0x06], false), None);
    }
}
