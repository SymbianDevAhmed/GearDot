//! Mice on SinoWealth's stock 2.4 GHz receiver (VID 258A, PID 002F): Redragon M602-KS, M690 Pro,
//! M910-KS, M811 Pro and others. Contributed by NeroHole (pull request #111, checked on an M602-KS
//! receiver: `05 80 01 01` and `05 90 11 LL`), reworked here.
//!
//! Protocol as documented by BatteryTray (MIT, github.com/qu1ncky/BatteryTray,
//! src/providers/devices/mice_platforms.rs) and as used by the original C++ BatteryHub (GPL-3.0,
//! src/drivers/sinowealth_receiver.cpp); both follow OpenMouse's notes on the M690 Pro
//! (github.com/OpenMouse-Project/mouse-protocol, docs/redragon-m690-pro-testing.md, read off
//! Redragon's own app; facts only). The receiver's interface 1 has a vendor collection FF00:0001
//! with only an 8-byte feature report (report id 05 + 7 bytes):
//!
//!   SetFeature 05 80, ~100 ms, GetFeature 05  ->  05 80 01 ..   mouse linked
//!                                                 05 80 00 ..   mouse off or asleep
//!   SetFeature 05 90, ~100 ms, GetFeature 05  ->  05 90 11 LL   LL = level 0..100
//!
//! Both are status reads. The receiver answers 90 from its own copy even with the mouse off, so the
//! link is asked first. The app's writes use report 8 (another collection), which is never touched.
//! The USB id is shared by other receivers, so only the exact collection shape is used.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{self, HidColl, HidDev};
use crate::util::{hex4, sleep_ms};

const VID: u16 = 0x258A;
const PID: u16 = 0x002F;
const REPORT: u8 = 0x05;
pub const CMD_LINK: u8 = 0x80;
pub const CMD_BATTERY: u8 = 0x90;

pub fn request(cmd: u8) -> [u8; 2] {
    [REPORT, cmd]
}

/// Link reply: Some(true) linked, Some(false) mouse off / asleep, None = not a link reply.
pub fn parse_link(r: &[u8]) -> Option<bool> {
    (r.len() >= 3 && r[0] == REPORT && r[1] == CMD_LINK).then(|| r[2] == 0x01)
}

/// Battery reply: the level, or None.
pub fn parse_battery(r: &[u8]) -> Option<u8> {
    (r.len() >= 4 && r[0] == REPORT && r[1] == CMD_BATTERY && r[2] == 0x11 && r[3] <= 100).then(|| r[3])
}

fn ask(d: &HidDev, cmd: u8) -> Option<Vec<u8>> {
    if !d.set_feature(&request(cmd)) {
        return None;
    }
    sleep_ms(100);
    d.get_feature(REPORT)
}

/// Some(Some(level)) / Some(None) = receiver answered, mouse off; None = not this receiver.
fn read(c: &HidColl) -> Option<Option<u8>> {
    // Zero-access open: the receiver's mouse collections are held by Windows.
    let d = HidDev::open_feature_only(c)?;
    if !parse_link(&ask(&d, CMD_LINK)?)? {
        return Some(None);
    }
    Some(ask(&d, CMD_BATTERY).and_then(|r| parse_battery(&r)))
}

/// Model from the product string ("M690-PRO" on that receiver); most only say "2.4G Wireless Receiver".
pub fn name(product: &str) -> String {
    let p = product.trim();
    let generic = p.is_empty() || p.to_ascii_lowercase().contains("receiver") || p.to_ascii_lowercase().contains("2.4g");
    if generic { "Wireless mouse".into() } else { p.to_string() }
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.iface == 1 && c.page() == 0xFF00 && c.usage() == 0x0001 && c.feat_len() == 8 && c.in_len() == 0 && c.out_len() == 0);
    let Some((c, found)) = colls.iter().find_map(|c| read(c).map(|f| (c, f))) else { return };
    let base = DeviceStatus {
        key: format!("sinowealth:{}", hex4(PID)),
        name: name(&hid::product_string(c)),
        kind: DeviceKind::Mouse,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 1800,
        ..Default::default()
    };
    let s = match found {
        Some(level) => DeviceStatus { percent: level as i32, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        assert_eq!(request(CMD_LINK), [0x05, 0x80]);
        assert_eq!(request(CMD_BATTERY), [0x05, 0x90]);
    }

    #[test]
    fn link_replies() {
        // PR #111 / OpenMouse: 05 80 01 01 linked, 05 80 00 .. off or asleep.
        assert_eq!(parse_link(&[0x05, 0x80, 0x01, 0x01, 0, 0, 0, 0]), Some(true));
        assert_eq!(parse_link(&[0x05, 0x80, 0x00, 0x02, 0, 0, 0, 0]), Some(false));
        assert_eq!(parse_link(&[0x05, 0x90, 0x11, 0x64]), None);
    }

    #[test]
    fn battery_replies() {
        assert_eq!(parse_battery(&[0x05, 0x90, 0x11, 0x64, 0, 0, 0, 0]), Some(100));
        assert_eq!(parse_battery(&[0x05, 0x90, 0x11, 0x3A]), Some(58));
        assert_eq!(parse_battery(&[0x05, 0x90, 0x10, 0x01]), None); // the cable's charging reply
        assert_eq!(parse_battery(&[0x05, 0x90, 0x11, 0xC8]), None); // out of range
        assert_eq!(parse_battery(&[0x05, 0x80, 0x11, 0x64]), None);
    }

    #[test]
    fn names() {
        assert_eq!(name("2.4G Wireless Receiver"), "Wireless mouse");
        assert_eq!(name(""), "Wireless mouse");
        assert_eq!(name("M690-PRO"), "M690-PRO");
    }
}
