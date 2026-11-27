//! HyperX Cloud II Wireless headset (Kingston edition) on its 2.4 GHz dongle (VID 0951, PID 1718).
//!
//! Protocol as documented by HeadsetControl (GPL-3.0, github.com/Sapd/HeadsetControl,
//! lib/devices/hyperx_cloud_2_wireless_kingston.hpp); implemented here from those facts. Output report
//! 06 (62 bytes), reply on input report 0B:
//!
//!   request  06 00 02 00 9A 00 00 68 4A 8E 0A 00 00 00 BB CC 00 ...   CC = 02 level, 03 charging
//!   reply    0B .. BB CC VV .. .. LL ...                              LL = level, VV = 1 charging
//!
//! Only these two reads are sent. No reply means the headset is off or out of range.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x0951;
const PID: u16 = 0x1718;
const OUT_LEN: usize = 62;
const LEVEL: u8 = 0x02;
const CHARGING: u8 = 0x03;
const REPLY_MS: u64 = 1000;

pub fn request(cmd: u8) -> [u8; OUT_LEN] {
    let mut r = [0u8; OUT_LEN];
    r[..16].copy_from_slice(&[0x06, 0x00, 0x02, 0x00, 0x9A, 0x00, 0x00, 0x68, 0x4A, 0x8E, 0x0A, 0x00, 0x00, 0x00, 0xBB, cmd]);
    r
}

pub fn is_reply(r: &[u8], cmd: u8) -> bool {
    r.len() >= 8 && r[0] == 0x0B && r[2] == 0xBB && r[3] == cmd
}

fn ask(d: &HidDev, cmd: u8) -> Option<Vec<u8>> {
    let _ = d.get_input_report(0x06); // what HeadsetControl does before every command
    d.flush();
    if !d.write(&request(cmd)) {
        return None;
    }
    let end = Deadline::in_ms(REPLY_MS);
    while !end.passed() {
        if let Some(r) = d.read(end.left_ms()).filter(|r| is_reply(r, cmd)) {
            return Some(r);
        }
    }
    None
}

fn read(c: &HidColl) -> Option<(i32, bool)> {
    let d = HidDev::open(c)?;
    let level = ask(&d, LEVEL).map(|r| r[7]).filter(|&l| l <= 100)?;
    let charging = ask(&d, CHARGING).is_some_and(|r| r[4] == 1);
    Some((level as i32, charging))
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.out_len() as usize == OUT_LEN && c.in_len() >= 8);
    if colls.is_empty() {
        return;
    }
    let base = DeviceStatus {
        key: "hyperx:cloud2w-1718".into(),
        name: "HyperX Cloud II Wireless".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz dongle".into(),
        hw: Some((VID, PID)),
        grace_sec: 600,
        ..Default::default()
    };
    // Vendor page first; the report goes to whichever collection declares output report 06.
    let mut colls = colls;
    colls.sort_by_key(|c| c.page() < 0xFF00);
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
    fn request_frame() {
        let r = request(LEVEL);
        assert_eq!(r[..16], [0x06, 0x00, 0x02, 0x00, 0x9A, 0x00, 0x00, 0x68, 0x4A, 0x8E, 0x0A, 0x00, 0x00, 0x00, 0xBB, 0x02]);
        assert!(r[16..].iter().all(|&b| b == 0));
    }

    #[test]
    fn reply() {
        let mut r = vec![0u8; 64];
        r[..4].copy_from_slice(&[0x0B, 0x00, 0xBB, 0x02]);
        r[7] = 73;
        assert!(is_reply(&r, LEVEL));
        assert!(!is_reply(&r, CHARGING));
        r[0] = 0x06;
        assert!(!is_reply(&r, LEVEL));
    }
}
