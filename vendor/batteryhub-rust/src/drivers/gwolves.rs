//! G-Wolves mice on their own 2.4 GHz receivers or USB cables (VID 33E4), e.g. the HTS Ultra on its
//! "HTS Ultra 8K Receiver-N" 33E4:5317 (issue #376).
//!
//! Protocol and ids from G-Wolves' own web driver (mouse.xyz, Config/env-models.json), as
//! documented by HaloBattery (MIT, github.com/HeyOkay/HaloBattery, providers/gwolves.py; the old
//! exchange confirmed on an HSK Pro ACE in its #105); implemented here from those facts. On the
//! collection with a 64-byte feature report and no report id (65 bytes with it), the web driver
//! uses one of two exchanges, chosen by the model:
//!
//!   new  request  00 | 00 00 02 02 00 83 00 ...      reply 00 | A1 00 02 02 00 83 CH LL
//!   old  request  00 | 00 02 8F RR 00 ...            reply 00 | A1 02 8F .. CH LL
//!        RR = 01 over a receiver, 00 on the cable
//!
//! CH = charging, LL = level; a status byte other than A1 (A2 for the new one) means the mouse
//! sleeps. The reply is fetched up to 15 times, 50 ms apart, as the web driver does. Only this read
//! is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::sleep_ms;

const VID: u16 = 0x33E4;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Proto {
    New,
    Old,
}
use Proto::{New, Old};

/// (pid, name, the model's receiver id (its entry), exchange, on the cable).
const MODELS: [(u16, &str, u16, Proto, bool); 48] = [
    (0x3808, "G-Wolves HTM Plus", 0x3817, New, true),
    (0x3817, "G-Wolves HTM Plus", 0x3817, New, false),
    (0x6808, "G-Wolves HSK Pro 2.0", 0x6817, New, true),
    (0x6817, "G-Wolves HSK Pro 2.0", 0x6817, New, false),
    (0x5608, "G-Wolves HTXU", 0x5617, New, true),
    (0x5617, "G-Wolves HTXU", 0x5617, New, false),
    (0x3608, "G-Wolves Fenrir Pro", 0x3617, New, true),
    (0x3617, "G-Wolves Fenrir Pro", 0x3617, New, false),
    (0x3908, "G-Wolves VUK", 0x3917, Old, true),
    (0x3917, "G-Wolves VUK", 0x3917, Old, false),
    (0x7904, "G-Wolves HT-S2", 0x7913, Old, true),
    (0x7913, "G-Wolves HT-S2", 0x7913, Old, false),
    (0x3708, "G-Wolves Fenrir Max", 0x3717, Old, true),
    (0x3717, "G-Wolves Fenrir Max", 0x3717, Old, false),
    (0x5308, "G-Wolves HTS Ultra", 0x5317, Old, true),
    (0x5317, "G-Wolves HTS Ultra", 0x5317, Old, false),
    (0x7704, "G-Wolves HTR", 0x7713, Old, true),
    (0x7713, "G-Wolves HTR", 0x7713, Old, false),
    (0x3508, "G-Wolves Fenrir", 0x3517, Old, true),
    (0x3517, "G-Wolves Fenrir", 0x3517, Old, false),
    (0x7908, "G-Wolves HT-S2 Pro", 0x7917, Old, true),
    (0x7917, "G-Wolves HT-S2 Pro", 0x7917, Old, false),
    (0x5808, "G-Wolves HSK Pro", 0x5817, Old, true),
    (0x5817, "G-Wolves HSK Pro", 0x5817, Old, false),
    (0x5807, "G-Wolves HSK Pro", 0x5817, Old, false),
    (0x2708, "G-Wolves HTX Mini", 0x2717, Old, true),
    (0x2717, "G-Wolves HTX Mini", 0x2717, Old, false),
    (0x5408, "G-Wolves HTS Plus", 0x5417, Old, true),
    (0x5417, "G-Wolves HTS Plus", 0x5417, Old, false),
    (0x5407, "G-Wolves HTS Plus", 0x5417, Old, false),
    (0x5708, "G-Wolves HTX", 0x5717, Old, true),
    (0x5717, "G-Wolves HTX", 0x5717, Old, false),
    (0x5707, "G-Wolves HTX", 0x5717, Old, false),
    (0x5908, "G-Wolves HSK Plus", 0x5917, Old, true),
    (0x5917, "G-Wolves HSK Plus", 0x5917, Old, false),
    (0x5907, "G-Wolves HSK Plus", 0x5917, Old, false),
    (0x7204, "G-Wolves HSK Lite", 0x7203, Old, true),
    (0x7203, "G-Wolves HSK Lite", 0x7203, Old, false),
    (0x7708, "G-Wolves HTR Pro", 0x7717, Old, true),
    (0x7717, "G-Wolves HTR Pro", 0x7717, Old, false),
    (0x5804, "G-Wolves HSK Pro ACE", 0x5803, Old, true),
    (0x5803, "G-Wolves HSK Pro ACE", 0x5803, Old, false),
    (0x5404, "G-Wolves HTS Plus ACE", 0x5403, Old, true),
    (0x5403, "G-Wolves HTS Plus ACE", 0x5403, Old, false),
    (0x5704, "G-Wolves HTX ACE", 0x5703, Old, true),
    (0x5703, "G-Wolves HTX ACE", 0x5703, Old, false),
    (0x5904, "G-Wolves HSK Plus ACE", 0x5903, Old, true),
    (0x5903, "G-Wolves HSK Plus ACE", 0x5903, Old, false),
];

const TRIES: usize = 15;
const GAP_MS: u64 = 50;

pub fn request(p: Proto, cable: bool) -> [u8; 7] {
    match p {
        New => [0x00, 0x00, 0x00, 0x02, 0x02, 0x00, 0x83],
        Old => [0x00, 0x00, 0x02, 0x8F, if cable { 0x00 } else { 0x01 }, 0x00, 0x00],
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reading {
    Level(i32, bool),
    Asleep,
}

/// A reply (report id 0 in byte 0). None: not a reply to `p`.
pub fn parse(r: &[u8], p: Proto) -> Option<Reading> {
    if r.len() < 9 {
        return None;
    }
    let (ok, charge, level) = match p {
        New if r[3..7] == [0x02, 0x02, 0x00, 0x83] => (matches!(r[1], 0xA1 | 0xA2), r[7], r[8]),
        Old if r[2..4] == [0x02, 0x8F] => (r[1] == 0xA1, r[5], r[6]),
        _ => return None,
    };
    if r[1] & 0xF0 != 0xA0 {
        return None; // our own request read back (status 00)
    }
    if !ok {
        return Some(Reading::Asleep);
    }
    (level <= 100).then_some(Reading::Level(level as i32, charge != 0))
}

fn read(c: &HidColl, p: Proto, cable: bool) -> Option<Reading> {
    let d = HidDev::open_feature_only(c)?;
    if !d.set_feature(&request(p, cable)) {
        return None;
    }
    for _ in 0..TRIES {
        sleep_ms(GAP_MS);
        if let Some(r) = d.get_feature(0).and_then(|r| parse(&r, p)) {
            return Some(r);
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    for (pid, name, receiver, p, cable) in MODELS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.feat_len() == 65);
        if colls.is_empty() {
            continue;
        }
        let base = DeviceStatus {
            key: format!("gwolves:{}", crate::util::hex4(receiver).to_lowercase()),
            name: name.into(),
            kind: DeviceKind::Mouse,
            via: if cable { "USB cable" } else { "2.4 GHz receiver" }.into(),
            hw: Some((VID, pid)),
            grace_sec: 300,
            ..Default::default()
        };
        let s = match colls.iter().find_map(|c| read(c, p, cable)) {
            Some(Reading::Level(level, charging)) => DeviceStatus { percent: level, charging: charging || cable, ..base },
            Some(Reading::Asleep) => DeviceStatus { asleep: true, ..base },
            None if cable => continue,
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
        let mut r = vec![0u8; 65];
        r[..9].copy_from_slice(&[0x00, 0xA1, 0x00, 0x02, 0x02, 0x00, 0x83, 0x00, 0x4B]);
        assert_eq!(parse(&r, New), Some(Reading::Level(75, false)));
        let mut o = vec![0u8; 65];
        o[..7].copy_from_slice(&[0x00, 0xA1, 0x02, 0x8F, 0x01, 0x01, 0x3C]);
        assert_eq!(parse(&o, Old), Some(Reading::Level(60, true)));
        o[1] = 0xA4;
        assert_eq!(parse(&o, Old), Some(Reading::Asleep));
        // Our own requests read back are not replies; the other exchange's reply neither.
        let mut q = vec![0u8; 65];
        q[..7].copy_from_slice(&request(Old, false));
        assert_eq!(parse(&q, Old), None);
        assert_eq!(parse(&r, Old), None);
    }

    #[test]
    fn models() {
        for (pid, name, receiver, p, _) in MODELS {
            let rx = MODELS.iter().find(|m| m.0 == receiver).unwrap_or_else(|| panic!("{pid:04X}"));
            assert!(rx.1 == name && rx.3 == p && !rx.4, "{pid:04X}");
        }
    }
}
