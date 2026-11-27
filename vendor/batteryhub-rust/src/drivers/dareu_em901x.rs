//! DAREU EM901X mouse on its 2.4 GHz receiver (VID 260D, PID 1074) or USB cable (1090), and the
//! Fantech Helios Go XD5 on its dongle (25A7:FA7C) or cable (FA7B), which takes the same request.
//!
//! Protocol as documented by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! src/drivers/dareu.cpp, verified on hardware; taken from the official EM901X AIM-WL driver's
//! GetPower, analysed in myfinder-device-battery's upstream-protocols.md); implemented here from those
//! facts. On interface 1 the request goes out as feature report 08 on FF02:0002 (17 bytes) and the
//! answer comes back as input report 09 on the sibling FF01:0000 collection:
//!
//!   request  08 04 00 ... 49          byte 16 = 55 minus the sum of bytes 0..15
//!   reply    09 04 00 .. .. .. LL CC  LL = level %, CC non-zero = charging
//!
//! The XD5's request and reply layout ([6] level, [7] charging) are documented by TaskbarVitals
//! (src/drivers/fantech.cpp, confirmed on the FA7C dongle) after xd5-go-web (reverse engineered from
//! Fantech's OemDrv.exe); its reply carries report id 08 or 09.
//!
//! Only this read is sent. The cable is asked first; a reading there stands for the receiver too.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::HidDev;
use crate::util::{Deadline, hex4};

/// (VID, cable PID, receiver PID, name, EM901X reply)
const MODELS: [(u16, u16, u16, &str, bool); 2] = [
    (0x260D, 0x1090, 0x1074, "DAREU EM901X", true),
    (0x25A7, 0xFA7B, 0xFA7C, "Fantech Helios Go XD5", false),
];
const LEN: u16 = 17;
const REPLY_MS: u64 = 1000;

pub fn request() -> [u8; 17] {
    let mut r = [0u8; 17];
    r[0] = 0x08;
    r[1] = 0x04;
    r[16] = 0x55u8.wrapping_sub(r[..16].iter().fold(0u8, |a, b| a.wrapping_add(*b)));
    r
}

/// (level, charging) from one input report.
pub fn decode(r: &[u8], em901x: bool) -> Option<(i32, bool)> {
    if r.len() != LEN as usize || r[1] != 0x04 || r[6] > 100 {
        return None;
    }
    let ok = if em901x { r[0] == 0x09 && r[2] == 0 } else { (r[0] == 0x08 || r[0] == 0x09) && r[6] >= 1 && r[7] <= 1 };
    ok.then(|| (r[6] as i32, r[7] != 0))
}

pub fn poll(ctx: &mut PollContext) {
    for (vid, cable, receiver, name, em901x) in MODELS {
        poll_model(ctx, vid, cable, receiver, name, em901x);
    }
}

fn poll_model(ctx: &mut PollContext, vid: u16, cable: u16, receiver: u16, name: &str, em901x: bool) {
    let mut cable_read = false;
    for pid in [cable, receiver] {
        if ctx.is_claimed(vid, pid) || (cable_read && pid == receiver) {
            continue;
        }
        let inputs = ctx.find(vid, pid, |c| c.iface == 1 && c.page() == 0xFF01 && c.usage() == 0x0000 && c.in_len() == LEN);
        let features = ctx.find(vid, pid, |c| c.iface == 1 && c.page() == 0xFF02 && c.usage() == 0x0002 && c.feat_len() == LEN);
        // Never pair collections of different devices that share the id.
        if inputs.len() != 1 || features.len() != 1 {
            continue;
        }
        let (Some(input), Some(feature)) = (HidDev::open(&inputs[0]), HidDev::open_feature_only(&features[0])) else { continue };
        input.flush();
        if !feature.set_feature(&request()) {
            continue;
        }
        let end = Deadline::in_ms(REPLY_MS);
        let mut found = None;
        while !end.passed() {
            if let Some(v) = input.read(end.left_ms()).and_then(|r| decode(&r, em901x)) {
                found = Some(v);
                break;
            }
        }
        let base = DeviceStatus {
            key: format!("feat17:{}:{}", hex4(vid), hex4(receiver)),
            name: name.into(),
            kind: DeviceKind::Mouse,
            via: if pid == cable { "USB cable" } else { "2.4 GHz receiver" }.into(),
            hw: Some((vid, pid)),
            grace_sec: 1800,
            ..Default::default()
        };
        match found {
            Some((level, charging)) => {
                ctx.add(DeviceStatus { percent: level, charging: charging || pid == cable, ..base }, vid, pid);
                if pid == cable {
                    cable_read = true;
                    ctx.claim(vid, receiver);
                }
            }
            None if pid == receiver => ctx.add(DeviceStatus { connected: false, ..base }, vid, pid),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame() {
        let r = request();
        assert_eq!(r[..2], [0x08, 0x04]);
        assert_eq!(r[16], 0x49);
    }

    #[test]
    fn replies() {
        let mut r = vec![0u8; 17];
        r[..8].copy_from_slice(&[0x09, 0x04, 0x00, 0, 0, 0, 77, 1]);
        assert_eq!(decode(&r, true), Some((77, true)));
        r[2] = 1;
        assert_eq!(decode(&r, true), None);
        r[0] = 0x08;
        assert_eq!(decode(&r, false), Some((77, true)));
        assert_eq!(decode(&r, true), None);
        r[2] = 0;
        r[6] = 101;
        assert_eq!(decode(&r, true), None);
        assert_eq!(decode(&r[..16], true), None);
    }
}
