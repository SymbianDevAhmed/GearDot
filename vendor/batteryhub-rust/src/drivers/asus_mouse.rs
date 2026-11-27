//! ASUS ROG / TUF wireless mice over their 2.4 GHz receiver or USB cable (VID 0B05): Strix Impact II
//! Wireless, Gladius III (Wireless, Aimpoint, Eva 2), Chakram (X), Keris (Wireless, Aimpoint, EVA),
//! Harpe Ace Aim Lab, Spatha X, Pugio II, TUF M4 Wireless, TX Gaming Mouse (Mini).
//!
//! Protocol and ids as used by G-Helper (GPL-3.0, github.com/seerge/g-helper, app/Peripherals/Mouse/
//! AsusMouse.cs and the model files in Models/), also listed by HaloBattery (MIT,
//! github.com/HeyOkay/HaloBattery, providers/asus.py); implemented here from those facts. Vendor
//! collection on interface 0, 65-byte output / input report 00:
//!
//!   request  00 12 07 00 ...       battery
//!   reply    00 12 07 .. .. [5] .. .. .. .. [10]
//!            [5] = level: percent, or steps 0..4 (x 25 %) on older models; [10] != 0 = charging
//!
//! A level of 0 without charging means the mouse is off or asleep (the receiver answers for it).
//! Only this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x0B05;
const REPLY_MS: u64 = 600;

/// Scale of the battery byte: percent, or a level 0..4.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scale {
    Percent,
    Steps,
}

/// (pid, name, scale, on its cable). G-Helper's wired id of a mouse is two below its receiver's
/// (the TUF Mini Miku's two ids are not told apart: both are shown as on the receiver).
const KNOWN: [(u16, &str, Scale, bool); 30] = [
    (0x1A72, "ROG Gladius III Aimpoint", Scale::Percent, false),
    (0x1A70, "ROG Gladius III Aimpoint", Scale::Percent, true),
    (0x1B0C, "ROG Gladius III Eva 2", Scale::Percent, false),
    (0x1B0A, "ROG Gladius III Eva 2", Scale::Percent, true),
    (0x197F, "ROG Gladius III Wireless", Scale::Percent, false),
    (0x197D, "ROG Gladius III Wireless", Scale::Percent, true),
    (0x1A1A, "ROG Chakram X", Scale::Percent, false),
    (0x1A18, "ROG Chakram X", Scale::Percent, true),
    (0x1A94, "ROG Harpe Ace Aim Lab Edition", Scale::Percent, false),
    (0x1A92, "ROG Harpe Ace Aim Lab Edition", Scale::Percent, true),
    (0x1A68, "ROG Keris Wireless Aimpoint", Scale::Percent, false),
    (0x1A66, "ROG Keris Wireless Aimpoint", Scale::Percent, true),
    (0x1979, "ROG Spatha X", Scale::Percent, false),
    (0x1977, "ROG Spatha X", Scale::Percent, true),
    (0x19F4, "TUF Gaming M4 Wireless", Scale::Percent, false),
    (0x1A8D, "TX Gaming Mouse", Scale::Percent, false),
    (0x1AF5, "TX Gaming Mouse Mini", Scale::Percent, false),
    (0x1AF3, "TX Gaming Mouse Mini", Scale::Percent, true),
    (0x18E5, "ROG Chakram", Scale::Steps, false),
    (0x18E3, "ROG Chakram", Scale::Steps, true),
    (0x1960, "ROG Keris Wireless", Scale::Steps, false),
    (0x195E, "ROG Keris Wireless", Scale::Steps, true),
    (0x1A59, "ROG Keris EVA Edition", Scale::Steps, false),
    (0x1A57, "ROG Keris EVA Edition", Scale::Steps, true),
    (0x1908, "ROG Pugio II", Scale::Steps, false),
    (0x1906, "ROG Pugio II", Scale::Steps, true),
    (0x1949, "ROG Strix Impact II Wireless", Scale::Steps, false),
    (0x1947, "ROG Strix Impact II Wireless", Scale::Steps, true),
    (0x1C57, "TUF Gaming Mini Miku Edition", Scale::Percent, false),
    (0x1C56, "TUF Gaming Mini Miku Edition", Scale::Percent, false),
];

pub fn request() -> [u8; 3] {
    [0x00, 0x12, 0x07]
}

/// Some((level, charging)) from a reply (report id included); Some(None) for "asleep".
pub fn decode(r: &[u8], scale: Scale) -> Option<Option<(i32, bool)>> {
    let max = if scale == Scale::Steps { 4 } else { 100 };
    if r.len() < 11 || r[..3] != request() || r[5] > max {
        return None;
    }
    let charging = r[10] != 0;
    let level = if scale == Scale::Steps { r[5] as i32 * 25 } else { r[5] as i32 };
    Some((r[5] > 0 || charging).then_some((level, charging)))
}

fn read(c: &HidColl, scale: Scale) -> Option<Option<(i32, bool)>> {
    let d = HidDev::open(c)?;
    d.flush();
    if !d.write(&request()) {
        return None;
    }
    let end = Deadline::in_ms(REPLY_MS);
    while !end.passed() {
        if let Some(v) = d.read(end.left_ms()).and_then(|r| decode(&r, scale)) {
            return Some(v);
        }
    }
    Some(None)
}

pub fn poll(ctx: &mut PollContext) {
    for (pid, name, scale, cable) in KNOWN {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.iface == 0 && c.page() >= 0xFF00 && c.in_len() == 65 && c.out_len() == 65);
        let Some(found) = colls.iter().find_map(|c| read(c, scale)) else { continue };
        // The receiver's id names the mouse, so a mouse on its cable and on its receiver is one entry.
        let key_pid = KNOWN.iter().find(|k| k.1 == name && !k.3).map_or(pid, |k| k.0);
        let base = DeviceStatus {
            key: format!("asus:{}", crate::util::hex4(key_pid).to_lowercase()),
            name: name.into(),
            kind: DeviceKind::Mouse,
            via: if cable { "USB cable" } else { "2.4 GHz receiver" }.into(),
            hw: Some((VID, pid)),
            grace_sec: 300,
            ..Default::default()
        };
        let s = match found {
            Some((level, charging)) => DeviceStatus { percent: level, charging: charging || cable, ..base },
            None if cable => continue,
            None => DeviceStatus { connected: false, ..base },
        };
        ctx.add(s, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(level: u8, charging: u8) -> Vec<u8> {
        let mut r = vec![0u8; 65];
        r[..3].copy_from_slice(&[0x00, 0x12, 0x07]);
        r[5] = level;
        r[10] = charging;
        r
    }

    #[test]
    fn replies() {
        assert_eq!(decode(&reply(3, 0), Scale::Steps), Some(Some((75, false))));
        assert_eq!(decode(&reply(2, 1), Scale::Steps), Some(Some((50, true))));
        assert_eq!(decode(&reply(0, 0), Scale::Steps), Some(None)); // asleep
        assert_eq!(decode(&reply(9, 0), Scale::Steps), None);
        assert_eq!(decode(&reply(87, 0), Scale::Percent), Some(Some((87, false))));
        assert_eq!(decode(&reply(101, 0), Scale::Percent), None);
        let mut e = reply(3, 0);
        e[1] = 0xFF; // error reply
        assert_eq!(decode(&e, Scale::Steps), None);
        // Every wired id has its receiver's id two above it.
        for (pid, name, _, cable) in KNOWN {
            if cable {
                assert!(KNOWN.iter().any(|k| k.0 == pid + 2 && k.1 == name && !k.3), "{pid:04X}");
            }
        }
    }
}
