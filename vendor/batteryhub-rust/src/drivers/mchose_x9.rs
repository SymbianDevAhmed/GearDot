//! MCHOSE X9 headset on its 2.4 GHz dongle (3837:6045), and the V9 Turbo+ (3837:600A), which answers
//! the same read: `55 65 64 02` (100 %), matching M HUB, on its owner's hardware in HaloBattery
//! (MIT, github.com/HeyOkay/HaloBattery) issue #193 / pull request #215.
//!
//! Protocol as documented by HeadsetControl (GPL-3.0, github.com/Sapd/HeadsetControl,
//! lib/devices/mchose_x9.hpp: from a USB capture of M HUB, checked against its percentage); implemented
//! here from those facts. Vendor collection FF90:0001, report 55, 64 bytes each way:
//!
//!   request  55 65 01 00 ...
//!   reply    55 65 LL 02 ...     LL = level in percent; byte 3 says which field it is (02 = battery)
//!
//! Only this read is sent; no charging state. Not tested on hardware by this project (issue #147).
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::HidDev;

const VID: u16 = 0x3837;
const MODELS: [(u16, &str, &str); 2] = [(0x6045, "mchose:x9", "MCHOSE X9"), (0x600A, "mchose:v9turbo", "MCHOSE V9 Turbo+")];
const REQUEST: [u8; 3] = [0x55, 0x65, 0x01];
const FIELD_BATTERY: u8 = 0x02;
const REPLY_MS: u64 = 1500;

/// The level in a reply (report id included).
pub fn parse(r: &[u8]) -> Option<i32> {
    (r.len() >= 4 && r[..2] == REQUEST[..2] && r[3] == FIELD_BATTERY && r[2] <= 100).then(|| r[2] as i32)
}

pub fn poll(ctx: &mut PollContext) {
    for (pid, key, name) in MODELS {
        poll_model(ctx, pid, key, name);
    }
}

fn poll_model(ctx: &mut PollContext, pid: u16, key: &str, name: &str) {
    if ctx.is_claimed(VID, pid) {
        return;
    }
    let colls = ctx.find(VID, pid, |c| c.page() == 0xFF90 && c.usage() == 0x0001 && c.in_len() == 64 && c.out_len() == 64);
    if colls.is_empty() {
        return;
    }
    let base = DeviceStatus {
        key: key.into(),
        name: name.into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz dongle".into(),
        hw: Some((VID, pid)),
        grace_sec: 600,
        ..Default::default()
    };
    let s = match colls.iter().find_map(|c| HidDev::open(c)?.ask(&REQUEST, REPLY_MS, parse)) {
        Some(percent) => DeviceStatus { percent, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, pid);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        assert_eq!(parse(&[0x55, 0x65, 76, 0x02, 0]), Some(76));
        assert_eq!(parse(&[0x55, 0x65, 1, 0x00, 0]), None); // the request, or another field
        assert_eq!(parse(&[0x55, 0x65, 101, 0x02, 0]), None);
        assert_eq!(parse(&[0x55, 0x66, 76, 0x02, 0]), None);
    }
}
