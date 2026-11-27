//! Logitech G735 headset on its LIGHTSPEED dongle (VID 046D, PID 0AD8).
//!
//! Protocol as documented by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! src/drivers/logitech_g735.cpp: from a USB capture of G HUB, then reproduced on the headset with
//! these two read-only requests); implemented here from those facts. The dongle has no classic HID++
//! collections; the headset is reached through the vendor collection FF03:0001, report F2 (271 bytes
//! in and out), which wraps Centurion-style HID++ 2.0 messages after a 16-byte header:
//!
//!   request  F2 01 05 1E FA 82 40 CE 00 00 00 20 00 00 00 00 | idx  fn<<4|sw  params...
//!   reply    F2 00 .. (16 bytes)                              | idx  fn<<4|sw  data...
//!   error    F2 00 .. (16 bytes)                              | FF   idx  fn<<4|sw  code
//!
//! Root (index 0) getFeature(0104, battery) gives the battery feature's index; its function 0 answers
//! [level %] [next level] [state], state 1 / 2 = charging. Software id 0B keeps the replies apart from
//! G HUB's. No answer means the headset is off or out of range. Only these two reads are sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x046D;
const PID: u16 = 0x0AD8;
const REPORT: u8 = 0xF2;
const SW: u8 = 0x0B;
const LEN: u16 = 271;
const HEADER: [u8; 16] = [0xF2, 0x01, 0x05, 0x1E, 0xFA, 0x82, 0x40, 0xCE, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00];
const DEADLINE_MS: u64 = 1000;

pub fn request(index: u8, function: u8, params: &[u8]) -> Vec<u8> {
    let mut r = HEADER.to_vec();
    r.push(index);
    r.push((function << 4) | SW);
    r.extend_from_slice(params);
    r
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Data(Vec<u8>),
    Error,
}

/// Classifies one input report as the answer to (index, function), if it is one.
pub fn reply(r: &[u8], index: u8, function: u8) -> Option<Reply> {
    let fnsw = (function << 4) | SW;
    if r.len() < HEADER.len() + 4 || r[0] != REPORT || r[1] != 0x00 {
        return None;
    }
    let m = &r[HEADER.len()..];
    if m[0] == index && m[1] == fnsw {
        return Some(Reply::Data(m[2..].to_vec()));
    }
    (m[0] == 0xFF && m[1] == index && m[2] == fnsw).then_some(Reply::Error)
}

/// (level, charging) from the battery feature's answer.
pub fn battery(data: &[u8]) -> Option<(i32, bool)> {
    let level = *data.first()?;
    let state = data.get(2).copied().unwrap_or(0);
    (level <= 100).then_some((level as i32, state == 1 || state == 2))
}

fn call(d: &HidDev, end: &Deadline, index: u8, function: u8, params: &[u8]) -> Option<Vec<u8>> {
    d.flush();
    if !d.write_timeout(&request(index, function, params), 250) {
        return None;
    }
    while !end.passed() {
        let Some(r) = d.read(end.left_ms()) else { continue };
        match reply(&r, index, function) {
            Some(Reply::Data(data)) => return Some(data),
            Some(Reply::Error) => return None,
            None => {}
        }
    }
    None
}

fn read(c: &HidColl) -> Option<(i32, bool)> {
    let d = HidDev::open(c)?;
    let end = Deadline::in_ms(DEADLINE_MS);
    let index = *call(&d, &end, 0x00, 0, &[0x01, 0x04])?.first()?;
    if index == 0 {
        return None;
    }
    call(&d, &end, index, 0, &[]).and_then(|data| battery(&data))
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF03 && c.usage() == 0x0001 && c.in_len() == LEN && c.out_len() == LEN);
    let Some(first) = colls.first() else { return };
    let base = DeviceStatus {
        key: format!("g735:{}", hex4(PID)),
        name: "Logitech G735".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 60,
        ..Default::default()
    };
    let s = match read(first) {
        Some((level, charging)) => DeviceStatus { percent: level, charging, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames() {
        let r = request(0, 0, &[0x01, 0x04]);
        assert_eq!(r[..16], HEADER);
        assert_eq!(r[16..], [0x00, 0x0B, 0x01, 0x04]);
        assert_eq!(request(2, 0, &[])[16..], [0x02, 0x0B]);
    }

    #[test]
    fn replies() {
        let mut r = vec![0u8; 271];
        r[0] = 0xF2;
        r[16..21].copy_from_slice(&[0x02, 0x0B, 41, 40, 0x01]);
        assert_eq!(reply(&r, 2, 0), Some(Reply::Data(r[18..].to_vec())));
        assert_eq!(battery(&r[18..]), Some((41, true)));
        assert_eq!(reply(&r, 3, 0), None);
        r[16..19].copy_from_slice(&[0xFF, 0x02, 0x0B]);
        assert_eq!(reply(&r, 2, 0), Some(Reply::Error));
        r[1] = 0x01; // our own request echoed
        assert_eq!(reply(&r, 2, 0), None);
        assert_eq!(battery(&[101]), None);
    }
}
