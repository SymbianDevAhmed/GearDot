//! IPI PIAO mouse on its "PIAO-2.4G" receiver (372E:1014).
//!
//! Protocol as documented by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! src/drivers/piao.cpp: captures on real hardware and IPI's web driver) and Joule
//! (github.com/DillanCidral/Joule, ChineseBrandsProvider.cs, the same 4F 81 read with byte 4 = 02);
//! implemented here from TaskbarVitals' facts. Feature report 03 (64 bytes) on FF00:0001:
//!
//!   set feature  03 SS 50 00 00 CM SB 00 ...    SS = sum of bytes 2..63
//!   get feature  03 50 ST LN 00 ...             ST = 0 answered (2 = mouse asleep), LN = data length
//!
//!   47 00  link      data from byte 7: [0] = 0 when the mouse is off or asleep
//!   4F 81  battery   data from byte 7 when byte 5 echoes 4F, from byte 6 when it echoes 81 (or 01):
//!                    bit 7 = charging, bits 0..6 = level
//!
//! Only these two reads are sent. Other 372E products (AULA, EWEADN) are not touched. Not tested on
//! hardware by this project (issue #47).
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::sleep_ms;

const VID: u16 = 0x372E;
const PID: u16 = 0x1014;
const REPORT: u8 = 0x03;
const LEN: usize = 64;
const LINK: (u8, u8) = (0x47, 0x00);
const BATTERY: (u8, u8) = (0x4F, 0x81);
const ATTEMPTS: usize = 4;

pub fn request((cmd, sub): (u8, u8)) -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[0] = REPORT;
    r[2] = 0x50;
    r[5] = cmd;
    r[6] = sub;
    r[1] = r[2..].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    r
}

/// First data byte of the answer to a request.
pub fn data(r: &[u8], (cmd, sub): (u8, u8)) -> Option<u8> {
    if r.len() < 8 || r[0] != REPORT || r[1] != 0x50 || r[2] != 0 || r[3] == 0 || r[4] != 0 {
        return None;
    }
    if r[5] == cmd {
        Some(r[7])
    } else if sub != 0 && (r[5] == sub || r[5] == sub & 0x7F) {
        Some(r[6])
    } else {
        None
    }
}

/// Level, charging.
pub fn battery(b: u8) -> Option<(i32, bool)> {
    (b & 0x7F <= 100).then_some(((b & 0x7F) as i32, b & 0x80 != 0))
}

fn query(d: &HidDev, what: (u8, u8)) -> Option<u8> {
    if !d.set_feature(&request(what)) {
        return None;
    }
    (0..ATTEMPTS).find_map(|_| {
        sleep_ms(120);
        d.get_feature(REPORT).and_then(|r| data(&r, what))
    })
}

enum Reading {
    Battery(i32, bool),
    Asleep,
    Away,
}

fn read(c: &HidColl) -> Option<Reading> {
    let d = HidDev::open_feature_only(c)?;
    if query(&d, LINK)? == 0 {
        return Some(Reading::Away);
    }
    Some(match query(&d, BATTERY).and_then(battery) {
        Some((level, charging)) => Reading::Battery(level, charging),
        None => Reading::Asleep,
    })
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF00 && c.usage() == 0x0001 && c.feat_len() as usize == LEN);
    let Some(reading) = colls.iter().find_map(read) else { return };
    let base = DeviceStatus {
        key: "piao:1014".into(),
        name: "IPI PIAO".into(),
        kind: DeviceKind::Mouse,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 600,
        ..Default::default()
    };
    let s = match reading {
        Reading::Battery(percent, charging) => DeviceStatus { percent, charging, ..base },
        Reading::Asleep => DeviceStatus { asleep: true, ..base },
        Reading::Away => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_frame() {
        let r = request(BATTERY);
        assert_eq!(r[..8], [0x03, 0x20, 0x50, 0, 0, 0x4F, 0x81, 0]);
        assert_eq!(request(LINK)[1], 0x97);
    }

    fn reply(head: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; LEN];
        r[..head.len()].copy_from_slice(head);
        r
    }

    #[test]
    fn replies() {
        assert_eq!(data(&reply(&[0x03, 0x50, 0, 1, 0, 0x47, 0x09, 1]), LINK), Some(1));
        assert_eq!(data(&reply(&[0x03, 0x50, 0, 1, 0, 0x81, 0xE4]), BATTERY), Some(0xE4));
        assert_eq!(data(&reply(&[0x03, 0x50, 0, 1, 0, 0x4F, 0x81, 64]), BATTERY), Some(64));
        assert_eq!(data(&reply(&[0x03, 0x50, 2, 0, 0, 0x4F, 0x81, 64]), BATTERY), None); // asleep
        assert_eq!(data(&request(BATTERY), BATTERY), None); // the request read back
        assert_eq!(battery(0xE4), Some((100, true)));
        assert_eq!(battery(64), Some((64, false)));
        assert_eq!(battery(0x7F), None);
    }
}
