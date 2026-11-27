//! BEITONG (BETOP) BTP-KP70A controller on its 2.4 GHz dongle (20BC:5130, "BTP-KP70A XINPUT DONGLE").
//!
//! Protocol measured on a KP70 by the reporter of issue #212 (as-shipped firmware, Windows 11): the
//! same frame as the KP20 in betop-battery (MIT, github.com/oitsukiii/betop-battery), in plain text.
//! The channel (interface 1, collection 00FF:0003, report 02, 64 bytes) is also described in a
//! KP70 write-up (nekotail.moe, "betop-kp70-nte-protocol"). XInput sees the dongle as a wired pad,
//! so it has no level there.
//!
//!   request  02 15 00 ...              get base info
//!   reply    02 15 LL .. SS ...        LL = level 0..100, SS & 0F = 1 charging
//!
//! Only this read is sent. While the dongle starts up it shows as 20DD:5130, which does not answer
//! and is left alone.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x20BC;
const PID: u16 = 0x5130;
const REPORT: u8 = 0x02;
const BASE_INFO: u8 = 0x15;
const LEN: usize = 64;
const REPLY_MS: u64 = 500;

pub fn request() -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[0] = REPORT;
    r[1] = BASE_INFO;
    r
}

/// (level, charging) from a reply (report id included).
pub fn parse(r: &[u8]) -> Option<(i32, bool)> {
    (r.len() >= 5 && r[0] == REPORT && r[1] == BASE_INFO && r[2] <= 100 && r[2..].iter().any(|&b| b != 0)).then(|| (r[2] as i32, r[4] & 0x0F == 1))
}

fn read(c: &HidColl) -> Option<(i32, bool)> {
    let d = HidDev::open(c)?;
    d.flush();
    if !d.write(&request()) {
        return None;
    }
    let end = Deadline::in_ms(REPLY_MS);
    while !end.passed() {
        if let Some(v) = d.read(end.left_ms()).and_then(|r| parse(&r)) {
            return Some(v);
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0x00FF && c.usage() == 0x0003 && c.in_len() as usize == LEN && c.out_len() as usize == LEN);
    if colls.is_empty() {
        return;
    }
    let base = DeviceStatus {
        key: "betop:5130".into(),
        name: "BEITONG KP70".into(),
        kind: DeviceKind::Gamepad,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 60,
        ..Default::default()
    };
    let s = match colls.iter().find_map(read) {
        Some((level, charging)) => DeviceStatus { percent: level, charging, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        // Measured on the reporter's KP70 (#212).
        let mut r = vec![0u8; LEN];
        r[..16].copy_from_slice(&[0x02, 0x15, 0x5A, 0x00, 0x50, 0x01, 0x00, 0xF0, 0x0F, 0x00, 0x00, 0xD4, 0x72, 0xE7, 0x00, 0x00]);
        assert_eq!(parse(&r), Some((90, false)));
        r[4] = 0x51;
        assert_eq!(parse(&r), Some((90, true)));
        // Our own request read back, another command.
        assert_eq!(parse(&request()), None);
        r[1] = 0x16;
        assert_eq!(parse(&r), None);
    }
}
