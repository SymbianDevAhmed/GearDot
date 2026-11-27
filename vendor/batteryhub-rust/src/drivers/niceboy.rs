//! Mice on the Telink "Wireless Receiver" 248A:FA02: the Niceboy MW400, and the Empire Gaming RF908,
//! sold on the same receiver.
//!
//! Protocol as documented by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! src/drivers/niceboy.cpp: from Niceboy's MW400 configuration app, WillGetBatteryThread /
//! RefreshBatteryStatus, and replies read from an MW400); implemented here from those facts.
//! Interface 0, vendor collection 0001:0000 with report 05 (33 bytes in and out):
//!
//!   request  05 24 FA 48 00 ...
//!   reply    05 .. FA 48 LO HI CC     raw = LO | HI << 8, mV = raw * 990 / 240, CC = charging;
//!                                     LO, HI and CC all 0 = mouse asleep or off
//!   level    the app's table in 10 % steps: 4200 mV 100 %, 4100 90, 4000 80, 3900 70, 3800 60,
//!            3700 50, 3500 40, 3400 30, 3200 20, below 10 %
//!
//! The 248A vendor id is shared by other Telink receivers, so only this product id is matched. Only
//! this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};

const VID: u16 = 0x248A;
const PID: u16 = 0xFA02;
const REPORT: u8 = 0x05;
const REPLY_MS: u64 = 500;
const REQUEST: [u8; 4] = [REPORT, 0x24, 0xFA, 0x48];

#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    Asleep,
    Level { percent: i32, mv: i32, charging: bool },
}

pub fn percent(mv: i32) -> i32 {
    const MV: [i32; 9] = [4200, 4100, 4000, 3900, 3800, 3700, 3500, 3400, 3200];
    const PCT: [i32; 9] = [100, 90, 80, 70, 60, 50, 40, 30, 20];
    MV.iter().zip(PCT).find(|(m, _)| mv >= **m).map(|(_, p)| p).unwrap_or(10)
}

pub fn decode(r: &[u8]) -> Option<Reading> {
    if r.len() < 7 || r[0] != REPORT || r[2] != 0xFA || r[3] != 0x48 {
        return None;
    }
    if r[4] | r[5] | r[6] == 0 {
        return Some(Reading::Asleep);
    }
    let mv = (r[4] as i32 | (r[5] as i32) << 8) * 990 / 240;
    Some(Reading::Level { percent: percent(mv), mv, charging: r[6] != 0 })
}

fn read(c: &HidColl) -> Option<Reading> {
    HidDev::open(c)?.ask(&REQUEST, REPLY_MS, decode)
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.iface == 0 && c.page() == 0x0001 && c.usage() == 0x0000 && c.in_len() == 33 && c.out_len() == 33);
    let Some(reading) = colls.iter().find_map(read) else { return };
    let base = DeviceStatus {
        key: "niceboy:fa02".into(),
        name: "Wireless mouse (Niceboy MW400 receiver)".into(),
        kind: DeviceKind::Mouse,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 1800,
        ..Default::default()
    };
    let s = match reading {
        Reading::Asleep => DeviceStatus { asleep: true, ..base },
        Reading::Level { percent, mv, charging } => DeviceStatus { percent, millivolts: mv, charging, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        // The documented example: raw 1002 = 4133 mV = 90 %, not charging.
        assert_eq!(decode(&[0x05, 0x49, 0xFA, 0x48, 0xEA, 0x03, 0x00]), Some(Reading::Level { percent: 90, mv: 4133, charging: false }));
        assert_eq!(decode(&[0x05, 0x49, 0xFA, 0x48, 0, 0, 0]), Some(Reading::Asleep));
        assert_eq!(decode(&[0x05, 0x49, 0xFA, 0x47, 0xEA, 0x03, 0x00]), None);
        assert_eq!(percent(3000), 10);
        assert_eq!(percent(4300), 100);
    }
}
