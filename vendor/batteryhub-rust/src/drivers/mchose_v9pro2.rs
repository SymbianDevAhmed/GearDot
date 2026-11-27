//! MCHOSE V9 Pro 2 headset on its 2.4 GHz dongle (VID 3837, PID 6011).
//!
//! Protocol as documented by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! src/drivers/mchose_headset.cpp: taken from MCHOSE's M HUB bundle, buildMCFrame /
//! createReadBatteryCommand / parseBatteryResponse, and verified on a V9 Pro 2); implemented here from
//! those facts. Vendor collection FF07:0222, report 55, 64 bytes in and out. After the report id:
//!
//!   AA 01 FL LL 00 TT SC <body> CK   FL 1 = XOR checksum (CK over FL..body), 0 = CK is 0
//!                                    LL = length of SC + body, TT 01 reply / 04 notification
//!   request  55 AA 01 00 01 00 01 0B 00        read battery (sub-command 0B)
//!   reply    sub-command 0B (or notification 22): body[0] 0 = headset off or asleep,
//!            body[1] bit 7 = charging, bits 0-6 = level %
//!
//! Only this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};

const VID: u16 = 0x3837;
const PID: u16 = 0x6011;
const REPORT: u8 = 0x55;
const START: u8 = 0xAA;
const VERSION: u8 = 0x01;
const READ_BATTERY: u8 = 0x0B;
const NOTIFY_BATTERY: u8 = 0x22;
const REPLY_MS: u64 = 1000;

pub const REQUEST: [u8; 9] = [REPORT, START, VERSION, 0x00, 0x01, 0x00, 0x01, READ_BATTERY, 0x00];

#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    Off,
    Level(i32, bool),
}

/// One input report (report id first): a battery reply or notification.
pub fn parse(r: &[u8]) -> Option<Reading> {
    if r.len() < 9 || r[0] != REPORT {
        return None;
    }
    let f = &r[1..];
    if f[0] != START || f[1] != VERSION || f[4] != 0x00 {
        return None;
    }
    let reply = f[5] == 0x01 && f[6] == READ_BATTERY;
    let notify = f[5] == 0x04 && f[6] == NOTIFY_BATTERY;
    if !reply && !notify {
        return None;
    }
    let len = f[3] as usize;
    if len < 3 || 6 + len >= f.len() {
        return None;
    }
    let check = if f[2] == 1 { f[2..6 + len].iter().fold(0u8, |a, b| a ^ b) } else { 0 };
    if f[6 + len] != check {
        return None;
    }
    let body = &f[7..];
    if body[0] == 0 {
        return Some(Reading::Off);
    }
    let level = (body[1] & 0x7F) as i32;
    (level <= 100).then_some(Reading::Level(level, body[1] & 0x80 != 0))
}

fn read(c: &HidColl) -> Option<Reading> {
    HidDev::open(c)?.ask(&REQUEST, REPLY_MS, parse)
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF07 && c.usage() == 0x0222 && c.out_len() >= 64 && c.in_len() >= 64);
    let Some(reading) = colls.iter().find_map(read) else { return };
    let base = DeviceStatus {
        key: "mchose:v9pro2".into(),
        name: "MCHOSE V9 Pro 2".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 1800,
        ..Default::default()
    };
    let s = match reading {
        Reading::Level(level, charging) => DeviceStatus { percent: level, charging, ..base },
        Reading::Off => DeviceStatus { asleep: true, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(flags: u8, kind: u8, sub: u8, body: &[u8]) -> Vec<u8> {
        let mut f = vec![START, VERSION, flags, (1 + body.len()) as u8, 0x00, kind, sub];
        f.extend_from_slice(body);
        let ck = if flags == 1 { f[2..].iter().fold(0u8, |a, b| a ^ b) } else { 0 };
        f.push(ck);
        let mut r = vec![REPORT];
        r.extend_from_slice(&f);
        r.resize(64, 0);
        r
    }

    #[test]
    fn replies() {
        assert_eq!(parse(&frame(0, 0x01, READ_BATTERY, &[1, 80, 0])), Some(Reading::Level(80, false)));
        assert_eq!(parse(&frame(1, 0x01, READ_BATTERY, &[1, 0x80 | 55, 0])), Some(Reading::Level(55, true)));
        assert_eq!(parse(&frame(1, 0x04, NOTIFY_BATTERY, &[1, 30, 0])), Some(Reading::Level(30, false)));
        assert_eq!(parse(&frame(0, 0x01, READ_BATTERY, &[0, 0, 0])), Some(Reading::Off));
        let mut bad = frame(1, 0x01, READ_BATTERY, &[1, 80, 0]);
        bad[11] ^= 0xFF;
        assert_eq!(parse(&bad), None);
        assert_eq!(parse(&frame(0, 0x01, 0x0C, &[1, 80, 0])), None);
        assert_eq!(parse(&REQUEST), None);
    }
}
