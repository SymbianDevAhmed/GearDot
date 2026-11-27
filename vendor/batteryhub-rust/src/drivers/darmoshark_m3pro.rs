//! Darmoshark M3 PRO on its 2.4 GHz receivers (1915:0723 / 0725 / 0734) and on its USB cable
//! (072D).
//!
//! Vendor collection FF0A:0001, 64-byte frames without a report id; a frame's first 63 bytes sum to
//! 0xA1 (mod 256) and its last byte is the checksum. Protocol as read off Darmoshark's own software
//! in a USB capture and as used by the original C++ BatteryHub (GPL-3.0, src/drivers/darmoshark.cpp);
//! implemented here from those facts, no code taken:
//!
//!   request  [op][00][81][01] 0 ... [csum]       op 0x41 = receiver, 0x01 = cable
//!   reply    [op][00][8C][01][charging][level] ... [csum]
//!
//! reply[6] is the level (0 = the mouse is off, not empty) and reply[5] != 0 means it is charging.
//! The receiver keeps answering while the mouse sleeps but stops forwarding the battery, so a frame
//! without a valid level reports nothing at all: `hw` + `grace_sec` keep the last level, dimmed,
//! for half an hour, and the entry comes back as soon as the mouse answers again. The cable and
//! every receiver share one key: plugging in refreshes the single entry instead of adding a second
//! one.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x1915;
const CABLE: u16 = 0x072D;
const RECEIVERS: [u16; 3] = [0x0723, 0x0725, 0x0734];
const OP_CABLE: u8 = 0x01;
const OP_RECEIVER: u8 = 0x41;
const FRAME: usize = 64;
const SUM: u8 = 0xA1;
const TOTAL_MS: u64 = 600;
const READ_MS: u64 = 100;
const KEY: &str = "darmoshark:m3pro";
const NAME: &str = "Darmoshark M3 PRO";
const GRACE_SEC: u32 = 1800;

/// Checksum of a frame: its bytes without the trailing checksum, summed up to `SUM` (mod 256).
fn checksum(frame: &[u8]) -> u8 {
    frame[..frame.len().saturating_sub(1)].iter().fold(SUM, |acc, b| acc.wrapping_sub(*b))
}

/// Request frame, report id 0 first.
pub fn request(op: u8) -> [u8; FRAME + 1] {
    let mut r = [0u8; FRAME + 1];
    r[1] = op;
    r[3] = 0x81;
    r[4] = 0x01;
    r[FRAME] = checksum(&r[1..]);
    r
}

/// (level %, charging) from a reply to `op`, if it is a valid one.
pub fn level(r: &[u8], op: u8) -> Option<(i32, bool)> {
    if r.len() < FRAME || r[0] != op || r[2] != 0x8C || r[3] != 0x01 {
        return None;
    }
    // The cable's reply has not been observed to carry the trailer checksum.
    if op == OP_RECEIVER && r[FRAME - 1] != checksum(&r[..FRAME]) {
        return None;
    }
    (1..=100).contains(&r[6]).then(|| (r[6] as i32, r[5] != 0))
}

fn read_level(d: &HidDev, op: u8) -> Option<(i32, bool)> {
    d.flush();
    if !d.write(&request(op)) {
        return None;
    }
    let end = Deadline::in_ms(TOTAL_MS);
    while !end.passed() {
        // Unsolicited frames and replies to something else are skipped by `level`.
        if let Some(found) = d.read(end.left_ms().min(READ_MS as u32)).and_then(|r| level(&r, op)) {
            return Some(found);
        }
    }
    None
}

/// The vendor collection: usage page FF0A, 65-byte input and output reports.
fn matching(c: &HidColl) -> bool {
    c.page() == 0xFF0A && c.usage() == 1 && c.in_len() == 65 && c.out_len() == 65
}

fn report(ctx: &mut PollContext, pid: u16, via: &str, (percent, charging): (i32, bool)) {
    let status = DeviceStatus {
        key: KEY.into(),
        name: NAME.into(),
        via: via.into(),
        kind: DeviceKind::Mouse,
        percent,
        charging,
        hw: Some((VID, pid)),
        grace_sec: GRACE_SEC,
        ..Default::default()
    };
    ctx.add(status, VID, pid);
}

pub fn poll(ctx: &mut PollContext) {
    let cables = ctx.find(VID, CABLE, matching);
    if let Some(c) = cables.first() {
        // The cable answers its own frame and claims the receivers, so plugging in only refreshes the
        // entry instead of adding a second one. Both are claimed even while nothing answers: the
        // entry then keeps its last level through the grace period instead of a generic fallback
        // reporting the collection.
        ctx.claim(VID, CABLE);
        for pid in RECEIVERS {
            ctx.claim(VID, pid);
        }
        if let Some(found) = HidDev::open(c).and_then(|d| read_level(&d, OP_CABLE)) {
            report(ctx, CABLE, "USB cable", found);
        }
        return;
    }

    for pid in RECEIVERS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, matching);
        let Some(c) = colls.first() else { continue };
        ctx.claim(VID, pid);
        // No valid reply: the mouse sleeps or is off. Nothing is reported, so the app keeps the last
        // level for the grace period (the receiver itself is still there).
        if let Some(found) = HidDev::open(c).and_then(|d| read_level(&d, OP_RECEIVER)) {
            report(ctx, pid, "2.4 GHz receiver", found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(op: u8, level: u8, charging: u8) -> Vec<u8> {
        let mut r = vec![0u8; FRAME];
        r[..7].copy_from_slice(&[op, 0x00, 0x8C, 0x01, 0x00, charging, level]);
        let sum = checksum(&r);
        r[FRAME - 1] = sum;
        r
    }

    #[test]
    fn request_frames() {
        let r = request(OP_RECEIVER);
        assert_eq!(&r[..5], &[0x00, 0x41, 0x00, 0x81, 0x01]);
        assert_eq!(r[FRAME], checksum(&r[1..]));
        // Every byte of the frame, report id aside, sums up to 0xA1.
        assert_eq!(r[1..].iter().fold(0u8, |a, b| a.wrapping_add(*b)), SUM);
        assert_eq!(request(OP_CABLE)[1], 0x01);
    }

    #[test]
    fn receiver_replies_need_the_checksum() {
        let r = reply(OP_RECEIVER, 92, 0);
        assert_eq!(level(&r, OP_RECEIVER), Some((92, false)));
        assert_eq!(level(&reply(OP_RECEIVER, 92, 1), OP_RECEIVER), Some((92, true)));
        assert_eq!(level(&reply(OP_CABLE, 92, 0), OP_RECEIVER), None); // other opcode
        let mut bad = r.clone();
        bad[FRAME - 1] ^= 0xFF;
        assert_eq!(level(&bad, OP_RECEIVER), None);
    }

    #[test]
    fn cable_replies_carry_no_checksum() {
        let mut r = reply(OP_CABLE, 55, 0);
        r[FRAME - 1] = 0; // not the sum: the cable does not fill it
        assert_eq!(level(&r, OP_CABLE), Some((55, false)));
    }

    #[test]
    fn levels_out_of_range_are_not_a_reading() {
        // 0 % = the mouse is off, 101+ = not a level.
        assert_eq!(level(&reply(OP_RECEIVER, 0, 0), OP_RECEIVER), None);
        assert_eq!(level(&reply(OP_RECEIVER, 101, 0), OP_RECEIVER), None);
        assert_eq!(level(&reply(OP_RECEIVER, 100, 0), OP_RECEIVER), Some((100, false)));
        assert_eq!(level(&[0x41, 0x00], OP_RECEIVER), None);
    }

    #[test]
    fn one_collection_shape() {
        let coll = |page, usage, input_len, output_len| HidColl {
            path: "x".into(),
            vid: VID,
            pid: CABLE,
            iface: 3,
            usage_page: page,
            usage_id: usage,
            input_len,
            output_len,
            feature_len: 0,
            n_input_value_caps: 1,
            n_feature_value_caps: 0,
        };
        assert!(matching(&coll(0xFF0A, 1, 65, 65)));
        assert!(!matching(&coll(0xFF0A, 1, 64, 65)));
        assert!(!matching(&coll(0xFF0A, 2, 65, 65)));
    }
}
