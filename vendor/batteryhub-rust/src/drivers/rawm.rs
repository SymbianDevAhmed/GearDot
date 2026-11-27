//! RAWM mice on the "RAWM HS Receiver" (VID 1915, PID 2346), e.g. the Leviathan V4 (issue #188).
//!
//! Protocol from USB captures of RAWM's own software with a Leviathan V4 on this receiver (issue #188,
//! taken by the reporter); only the requests that software sends are used. Vendor collection FF00:0001,
//! unnumbered 64-byte reports. The first byte of a report says where it goes: 8L = to the receiver,
//! C0 8L = to / from the mouse over the air (L = payload length, low 7 bits):
//!
//!   request  C0 8D 01 0D 03 00 00 T0 T1 T2 T3 00 00 00 00    get info; T = Unix time, little endian
//!            87 06 07 42 00 00 00 00                         sent right after it by RAWM's software
//!   reply    C0 9E ...  the mouse's settings as JSON over several reports, among them
//!                       "dn":"LEVIATHAN V4", "battery":92, "chr":0 (1 = charging)
//!   event    C0 89 FF FF FF FF 0B 05 17 LL CC                 sent by the mouse when its level changes
//!                                                             (LL = level, CC = 1 on the cable)
//!
//! No reply means the mouse is off, asleep or out of range.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x1915;
const PID: u16 = 0x2346;
const REPLY_MS: u64 = 1500;

/// Output reports, report ID 0 first.
pub fn info_request(unix: u32) -> [u8; 65] {
    let mut r = [0u8; 65];
    r[1..7].copy_from_slice(&[0xC0, 0x8D, 0x01, 0x0D, 0x03, 0x00]);
    r[8..12].copy_from_slice(&unix.to_le_bytes());
    r
}

pub fn status_request() -> [u8; 65] {
    let mut r = [0u8; 65];
    r[1..9].copy_from_slice(&[0x87, 0x06, 0x07, 0x42, 0x00, 0x00, 0x00, 0x00]);
    r
}

/// Payload of a report coming from the mouse (C0 8L ...), else empty.
pub fn from_mouse(r: &[u8]) -> &[u8] {
    if r.len() < 2 || r[0] != 0xC0 || r[1] & 0x80 == 0 {
        return &[];
    }
    let n = (r[1] & 0x7F) as usize;
    &r[2..(2 + n).min(r.len())]
}

fn json_num(s: &str, key: &str) -> Option<i32> {
    let at = s.find(&format!("\"{key}\":"))? + key.len() + 3;
    let digits: String = s[at..].chars().take_while(|c| c.is_ascii_digit()).collect();
    // A number cut off at the end of what has arrived so far isn't complete yet.
    (!digits.is_empty() && s.len() > at + digits.len()).then(|| digits.parse().ok()).flatten()
}

fn json_str(s: &str, key: &str) -> Option<String> {
    let at = s.find(&format!("\"{key}\":\""))? + key.len() + 4;
    let end = s[at..].find('"')?;
    Some(s[at..at + end].to_string())
}

/// What the mouse sent so far: Some((level, charging, name)).
pub fn decode(stream: &[u8]) -> Option<(i32, bool, Option<String>)> {
    let s = String::from_utf8_lossy(stream);
    if let Some(level) = json_num(&s, "battery").filter(|l| (0..=100).contains(l)) {
        return Some((level, json_num(&s, "chr").is_some_and(|c| c != 0), json_str(&s, "dn")));
    }
    stream
        .windows(9)
        .rev()
        .find(|w| w[..7] == [0xFF, 0xFF, 0xFF, 0xFF, 0x0B, 0x05, 0x17] && w[7] <= 100)
        .map(|w| (w[7] as i32, w[8] == 1, None))
}

fn read(c: &HidColl) -> Option<(i32, bool, Option<String>)> {
    let d = HidDev::open(c)?;
    d.flush();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t| t.as_secs() as u32).unwrap_or(0);
    if !d.write(&info_request(now)) || !d.write(&status_request()) {
        return None;
    }
    let mut stream = Vec::new();
    let end = Deadline::in_ms(REPLY_MS);
    while !end.passed() {
        let Some(r) = d.read(end.left_ms()) else { continue };
        stream.extend_from_slice(from_mouse(&r));
        if let Some(v) = decode(&stream) {
            return Some(v);
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF00 && c.out_len() >= 65 && c.in_len() >= 65);
    let Some(first) = colls.first() else { return };
    let base = DeviceStatus {
        key: "rawm:2346".into(),
        name: "RAWM mouse".into(),
        kind: DeviceKind::Mouse,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 1800,
        ..Default::default()
    };
    let s = match read(first) {
        Some((level, charging, name)) => DeviceStatus {
            percent: level,
            charging,
            name: name.filter(|n| !n.is_empty()).map(|n| format!("RAWM {n}")).unwrap_or(base.name.clone()),
            ..base
        },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    #[test]
    fn requests_match_the_capture() {
        let r = info_request(0x6B05_F7FD);
        assert_eq!(r[..16], hex("00 c0 8d 01 0d 03 00 00 fd f7 05 6b 00 00 00 00")[..]);
        assert_eq!(status_request()[..9], hex("00 87 06 07 42 00 00 00 00")[..]);
    }

    #[test]
    fn json_reply_from_the_capture() {
        let reports = [
            "c0 9e ff ff ff ff 22 29 7b 22 72 22 3a 22 31 2e 32 2e 33 22 2c 22 72 63 22 3a 39 2c 22 64 6e 22",
            "c0 9e 3a 22 4c 45 56 49 41 54 48 41 4e 20 56 34 22 2c 22 70 69 22 3a 39 30 33 34 2c 22 76 69 22",
            "c0 9e 3a 36 34 32 31 2c 22 62 61 74 74 65 72 79 22 3a 39 32 2c 22 63 68 72 22 3a 30 2c 22 63 70",
        ];
        let mut stream = Vec::new();
        for (i, r) in reports.iter().enumerate() {
            stream.extend_from_slice(from_mouse(&hex(r)));
            let got = decode(&stream);
            if i < 2 {
                assert_eq!(got, None);
            } else {
                assert_eq!(got, Some((92, false, Some("LEVIATHAN V4".into()))));
            }
        }
    }

    #[test]
    fn level_event_and_receiver_reports() {
        let ev = hex("c0 89 ff ff ff ff 0b 05 17 5c 01 00 00 00 00 00 00 0c 00 00");
        assert_eq!(decode(from_mouse(&ev)), Some((92, true, None)));
        // receiver's own info (no C0) and the C0 52 status report carry nothing from the mouse
        assert!(from_mouse(&hex("bf ff ff ff ff 02 a1 7b 22 72 22")).is_empty());
        assert!(from_mouse(&hex("c0 52 00 0c 00 00")).is_empty());
    }
}
