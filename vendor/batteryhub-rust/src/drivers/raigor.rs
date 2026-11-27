//! Fantech Raigor III and other mice on the reused 1EA7:0066 / 0064 dongle.
//!
//! Protocol from t20-battery (github.com/tuttizpro218/t20-battery, docs/PROTOCOL.md), as implemented
//! and verified live on a Raigor III dongle 1EA7:0066 by TaskbarVitals (GPL-3.0,
//! github.com/symNistaken/TaskbarVitals, src/drivers/raigor.cpp); implemented here from those facts.
//! Vendor collection FFB5:0001, report B5, 8-byte input and output reports:
//!
//!   request  B5 05 00 00 00 00 00 00   the dongle streams its 256-byte status page as 64 frames
//!            B5 05 AA 04 d0 d1 d2 d3   AA = 00, 04 .. FC; chunk 94 holds [id] [state] [level %]
//!   push     B5 67 id id id id ST LL   on wake, re-link and as heartbeat; LL = level %
//!
//! The page holds the last radio report, so a sleeping mouse keeps its last value. No charging flag.
//! Only this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x1EA7;
const PIDS: [u16; 2] = [0x0066, 0x0064];
const REPORT: u8 = 0xB5;
const REQUEST: [u8; 8] = [REPORT, 0x05, 0, 0, 0, 0, 0, 0];
const WAIT_MS: u64 = 1200;

/// Level % from one input report, if it carries it.
pub fn decode(r: &[u8]) -> Option<i32> {
    if r.len() != 8 || r[0] != REPORT {
        return None;
    }
    if r[1] == 0x05 && r[2] == 0x94 && r[3] == 0x04 && r[6] <= 100 {
        return Some(r[6] as i32);
    }
    (r[1] == 0x67 && r[7] <= 100).then_some(r[7] as i32)
}

fn read(c: &HidColl) -> Option<i32> {
    let d = HidDev::open(c)?;
    d.flush();
    if !d.write(&REQUEST) {
        return None;
    }
    let end = Deadline::in_ms(WAIT_MS);
    while !end.passed() {
        if let Some(level) = d.read(end.left_ms().min(200)).and_then(|r| decode(&r)) {
            return Some(level);
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    for pid in PIDS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.page() == 0xFFB5 && c.usage() == 0x0001 && c.in_len() == 8 && c.out_len() == 8);
        let Some(level) = colls.iter().find_map(read) else { continue };
        ctx.add(
            DeviceStatus {
                key: format!("raigor:{}:{}", hex4(VID), hex4(pid)),
                name: "Fantech Raigor".into(),
                kind: DeviceKind::Mouse,
                via: "2.4 GHz receiver".into(),
                hw: Some((VID, pid)),
                grace_sec: 1800,
                percent: level,
                ..Default::default()
            },
            VID,
            pid,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        // The documented chunk 94: 35 09 40 38 -> 64 %.
        assert_eq!(decode(&[0xB5, 0x05, 0x94, 0x04, 0x35, 0x09, 0x40, 0x38]), Some(64));
        assert_eq!(decode(&[0xB5, 0x05, 0x90, 0x04, 0x35, 0x09, 0x40, 0x38]), None);
        assert_eq!(decode(&[0xB5, 0x67, 1, 2, 3, 4, 0, 55]), Some(55));
        assert_eq!(decode(&[0xB5, 0x67, 1, 2, 3, 4, 0, 0xFF]), None);
        assert_eq!(decode(&[0xB4, 0x05, 0x94, 0x04, 0, 0, 50, 0]), None);
    }
}
