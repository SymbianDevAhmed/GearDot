//! Razer Nari headset over its 2.4 GHz dongle (VID 1532, PID 051C).
//!
//! The dongle's HID interface (interface 5, vendor collection FF00:0001) carries a 64-byte feature
//! report FF. Razer Synapse writes a status request and reads the answer about 100 ms later:
//!
//!   request  set feature  FF 0A 00 FD 04 12 F1 02 05 00 ...
//!   answer   get feature  FF 0F 05 FE 12 04 1F 08 05 .. .. SS VV VV LL ...
//!            SS = power state (01 / 02 / 03 on battery, 09 charging, 06 charged), VV VV = battery
//!            voltage in mV (big endian; 0 with the headset off), LL = level in coarse steps
//!            (100, 80, 50, 30, 10)
//!   stale    get feature  FF 01 00 FE 12 04 1F 08 05 ...   the dongle's old copy: no new answer
//!
//! Request, interface and the Nari's answer (FF 0F 05 FE 12 04 1F 08 05 03 05 02 0D E0 1E: 3552 mV,
//! level 30) from the Synapse capture in Modzeleczek/RazerNariBatteryLevel (README, no license
//! stated; only the facts are used); the field meaning from HaloBattery (MIT,
//! github.com/HeyOkay/HaloBattery, providers/blackshark2020.py, measured on the BlackShark V2 Pro 2020,
//! which uses the same request and layout) and BrimDeck (Apache-2.0, github.com/MoFeng2223/BrimDeck,
//! src/BrimDeck.Core/Brands/RazerHeadsets.cs, which reads the Nari 051C the same way); implemented
//! here from those facts. While charging the level byte is not the battery's, so only charging is
//! shown then. Only the status request is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::sleep_ms;

const VID: u16 = 0x1532;
const PID: u16 = 0x051C;
const REPORT: u8 = 0xFF;
const LEN: usize = 64;
const REQUEST: [u8; 9] = [0xFF, 0x0A, 0x00, 0xFD, 0x04, 0x12, 0xF1, 0x02, 0x05];
const ANSWER: [u8; 9] = [0xFF, 0x0F, 0x05, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05];
const STALE: [u8; 9] = [0xFF, 0x01, 0x00, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05];
const CHARGING: u8 = 0x09;
const CHARGED: u8 = 0x06;

#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// Level (None while charging or not on the 0..100 scale), charging, charged, millivolts.
    Battery { level: Option<i32>, charging: bool, full: bool, mv: i32 },
    /// The headset is off (no fresh answer, or 0 mV).
    Off,
    /// The dongle's stale copy: no new answer yet.
    Stale,
}

/// Decodes the feature report (report ID FF first).
pub fn decode(r: &[u8]) -> Option<Answer> {
    if r.len() >= 15 && r[..9] == STALE {
        return Some(Answer::Stale);
    }
    if r.len() < 15 || r[..9] != ANSWER {
        return None;
    }
    let (state, mv, level) = (r[11], u16::from_be_bytes([r[12], r[13]]) as i32, r[14]);
    if mv == 0 {
        return Some(Answer::Off);
    }
    let charging = state == CHARGING;
    let level = (!charging && level <= 100).then_some(level as i32);
    Some(Answer::Battery { level, charging, full: state == CHARGED, mv })
}

fn query(c: &HidColl) -> Option<Answer> {
    let d = HidDev::open_feature_only(c).or_else(|| HidDev::open(c))?;
    let mut req = [0u8; LEN];
    req[..REQUEST.len()].copy_from_slice(&REQUEST);
    if !d.set_feature(&req) {
        return None;
    }
    let mut last = None;
    for _ in 0..5 {
        sleep_ms(100); // Synapse waits about 100 ms
        match d.get_feature(REPORT).and_then(|r| decode(&r)) {
            Some(Answer::Stale) => last = Some(Answer::Stale),
            Some(a) => return Some(a),
            None => {}
        }
    }
    last.map(|_| Answer::Off)
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF00 && c.usage() == 0x0001 && c.feat_len() as usize >= LEN);
    if colls.is_empty() {
        return;
    }
    let base = DeviceStatus {
        key: "razer:nari:051c".into(),
        name: "Razer Nari".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz dongle".into(),
        hw: Some((VID, PID)),
        grace_sec: 120,
        ..Default::default()
    };
    let s = match colls.iter().find_map(query) {
        Some(Answer::Battery { level, charging, full, mv }) => DeviceStatus { percent: level.unwrap_or(-1), charging, full, millivolts: mv, ..base },
        _ => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(head: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 64];
        f[..head.len()].copy_from_slice(head);
        f
    }

    #[test]
    fn nari_capture() {
        // Modzeleczek/RazerNariBatteryLevel: ... 03 05 02 0D E0 1E = 3552 mV, level 30.
        let r = frame(&[0xFF, 0x0F, 0x05, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05, 0x03, 0x05, 0x02, 0x0D, 0xE0, 0x1E]);
        assert_eq!(decode(&r), Some(Answer::Battery { level: Some(30), charging: false, full: false, mv: 3552 }));
    }

    #[test]
    fn power_states() {
        // HaloBattery's frames of the same layout: charging (level hidden), charge complete.
        let chg = frame(&[0xFF, 0x0F, 0x05, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05, 0x05, 0x03, 0x09, 0x10, 0x80, 0x50]);
        assert_eq!(decode(&chg), Some(Answer::Battery { level: None, charging: true, full: false, mv: 4224 }));
        let full = frame(&[0xFF, 0x0F, 0x05, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05, 0x06, 0x05, 0x06, 0x10, 0x88, 0x64]);
        assert_eq!(decode(&full), Some(Answer::Battery { level: Some(100), charging: false, full: true, mv: 4232 }));
    }

    #[test]
    fn off_and_stale() {
        let off = frame(&[0xFF, 0x0F, 0x05, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05, 0x03, 0x05, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(decode(&off), Some(Answer::Off));
        let stale = frame(&[0xFF, 0x01, 0x00, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05, 0x05, 0x03, 0x09, 0x10, 0x88, 0x50]);
        assert_eq!(decode(&stale), Some(Answer::Stale));
        let mut req = frame(&REQUEST);
        assert_eq!(decode(&req), None); // our own request
        req[0] = 0x00;
        assert_eq!(decode(&req), None);
    }
}
