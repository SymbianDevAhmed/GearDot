//! Audeze Maxwell and Maxwell 2 headsets over their 2.4 GHz dongles (VID 3329).
//!
//! Protocol from HeadsetControl (GPL-3.0, github.com/Sapd/HeadsetControl, lib/devices/
//! audeze_maxwell_device.hpp, recovered from Audeze's app and a USB capture; it lists the Maxwell 2's
//! 4B29 / 4B28) and HaloBattery (MIT, github.com/HeyOkay/HaloBattery, providers/audeze.py, measured
//! on a Maxwell dongle 4B19); implemented here from those facts. Vendor collection FF13:0001,
//! 62-byte reports:
//!
//!   output 06 <len> 80 <fragment>          80 = to the headset through the dongle
//!   input  07 <len> .. <fragment>          fetched with GetInputReport; len 0 = nothing new yet
//!   the fragments form messages  05 <type> <len lo> <len hi> <body>
//!   battery  request 05 5A 03 00 D6 0C 00
//!            reply   05 5B 03 00 D6 0C <status>, then 05 5D 05 00 D6 0C 00 00 <percent>
//!
//! The input report is polled every 50 ms for up to 1.25 s, like Audeze's app does; Audeze HQ polls
//! the same dongle and can take a reply, so the read is tried up to three times. No reply: the
//! headset is off. No charging flag. Only this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{self, HidColl, HidDev};
use crate::util::{Deadline, hex4, sleep_ms};

const VID: u16 = 0x3329;
const MODELS: [(u16, &str); 4] = [
    (0x4B19, "Audeze Maxwell"),
    (0x4B18, "Audeze Maxwell (Xbox)"),
    (0x4B29, "Audeze Maxwell 2"),
    (0x4B28, "Audeze Maxwell 2 (Xbox)"),
];
const REQUEST: [u8; 10] = [0x06, 0x07, 0x80, 0x05, 0x5A, 0x03, 0x00, 0xD6, 0x0C, 0x00];
const ATTEMPTS: usize = 3;
const REPLY_MS: u64 = 1250;
const POLL_MS: u64 = 50;

/// The valid fragment of an input report (`07 <len> <route> <fragment>`).
pub fn fragment(r: &[u8]) -> &[u8] {
    if r.len() < 3 || r[0] != 0x07 {
        return &[];
    }
    let len = r[1] as usize;
    if 3 + len > r.len() { &[] } else { &r[3..3 + len] }
}

/// Takes complete messages off the front of `stream`; Some(Some(level)) for the battery result,
/// Some(None) when the headset refused the read.
pub fn take_battery(stream: &mut Vec<u8>) -> Option<Option<i32>> {
    loop {
        while !stream.is_empty() && stream[0] != 0x05 {
            stream.remove(0);
        }
        if stream.len() < 4 {
            return None;
        }
        let size = 4 + u16::from_le_bytes([stream[2], stream[3]]) as usize;
        if size > 1024 {
            stream.remove(0);
            continue;
        }
        if stream.len() < size {
            return None;
        }
        let m: Vec<u8> = stream.drain(..size).collect();
        let battery = m.len() >= 6 && m[4..6] == [0xD6, 0x0C];
        if battery && m[1] == 0x5D && m.len() > 8 {
            return Some(Some((m[8] as i32).min(100)));
        }
        if battery && m[1] == 0x5B && m.len() > 6 && m[6] != 0 {
            return Some(None);
        }
    }
}

fn read(c: &HidColl) -> Option<i32> {
    let d = HidDev::open(c)?;
    for _ in 0..ATTEMPTS {
        if !d.write(&REQUEST) {
            return None;
        }
        let end = Deadline::in_ms(REPLY_MS);
        let mut stream = Vec::new();
        while !end.passed() {
            sleep_ms(POLL_MS);
            let r = d.get_input_report(0x07)?;
            stream.extend_from_slice(fragment(&r));
            if let Some(level) = take_battery(&mut stream) {
                return level;
            }
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    for (pid, name) in MODELS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.page() == 0xFF13 && c.usage() == 0x0001 && c.in_len() >= 16 && c.out_len() >= 16);
        let Some(c) = colls.first() else { continue };
        let base = DeviceStatus {
            key: format!("audeze:{}", hex4(pid)),
            name: name.into(),
            kind: DeviceKind::Headset,
            via: "2.4 GHz dongle".into(),
            hw: Some((VID, pid)),
            grace_sec: 60,
            ..Default::default()
        };
        // "… Dongle": no headset linked (the dongle would answer its last value).
        let linked = !hid::product_string(c).to_lowercase().contains("dongle");
        let s = match linked.then(|| colls.iter().find_map(read)).flatten() {
            Some(level) => DeviceStatus { percent: level, ..base },
            None => DeviceStatus { connected: false, ..base },
        };
        ctx.add(s, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        // The acknowledgement and the result in one report, with leftovers after len.
        let mut r = vec![0u8; 62];
        let frag = [0x05, 0x5B, 0x03, 0x00, 0xD6, 0x0C, 0x00, 0x05, 0x5D, 0x05, 0x00, 0xD6, 0x0C, 0x00, 0x00, 66];
        r[0] = 0x07;
        r[1] = frag.len() as u8;
        r[2] = 0x80;
        r[3..3 + frag.len()].copy_from_slice(&frag);
        r[40] = 0x05;
        let mut stream = fragment(&r).to_vec();
        assert_eq!(take_battery(&mut stream), Some(Some(66)));
        // Split over two reports.
        let mut s = frag[..10].to_vec();
        assert_eq!(take_battery(&mut s), None);
        s.extend_from_slice(&frag[10..]);
        assert_eq!(take_battery(&mut s), Some(Some(66)));
        // Refused.
        let mut s = vec![0x05, 0x5B, 0x03, 0x00, 0xD6, 0x0C, 0x02];
        assert_eq!(take_battery(&mut s), Some(None));
        // Nothing new yet.
        let mut z = vec![0u8; 62];
        z[0] = 0x07;
        assert!(fragment(&z).is_empty());
    }
}
