//! HyperX Pulsefire mice on NGENUITY's "universal" HID protocol, over their 2.4 GHz dongle:
//! Pulsefire Saga Pro (VID 03F0, PID 06BF), and the Pulsefire Haste 2 Wireless (03F0:0D97 /
//! 0F98) that the protocol was first confirmed on.
//!
//! Protocol as documented by ngenuity-linux-cli by Plamen Marinov (MIT,
//! github.com/PMarinov1994/ngenuity-linux-cli; reverse engineered from NGENUITY 3.1.0 and confirmed
//! on a Pulsefire Haste 2 Wireless, its device table lists the Saga Pro on the same protocol) and
//! HaloBattery (MIT, github.com/HeyOkay/HaloBattery, providers/hyperx_ngenuity.py, issue #304);
//! implemented here from those facts. The dongle's vendor collection (usage page FF00, usage 0001,
//! 64-byte packets, the report ID is the command byte) answers a battery query:
//!
//!   request  50 02 | 00 ... (64 bytes)
//!   reply    51 02 LL CC TT TT VV VV ...
//!            LL  battery level 0..100 %
//!            CC  0 not charging, 1 charging, 2 full (on the cable), 3 charger error, FF unknown
//!            TT  temperature, i16 little-endian (not used)
//!            VV  battery voltage in mV, u16 little-endian
//!
//! NGENUITY waits at least 10 ms between packets. No reply means the mouse is asleep or off (the
//! dongle stays enumerated). Only this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x03F0;
const CMD_BATTERY: u8 = 0x50;
const SUB_BATTERY: u8 = 0x02;
const REPLY_BATTERY: u8 = 0x51;
const LEN: u16 = 64;
const WAIT_MS: u64 = 400; // the dongle has to reach the mouse over RF first

fn model(pid: u16) -> Option<&'static str> {
    match pid {
        // As the Saga Pro receiver enumerates on the hardware this was tested on.
        0x06BF => Some("HyperX Pulsefire Saga Pro"),
        0x0D97 | 0x0F98 => Some("HyperX Pulsefire Haste 2 Wireless"),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Reading {
    pub level: i32,
    /// None when the mouse reports no usable charging state.
    pub charging: Option<bool>,
    pub mv: i32,
}

/// Decodes one `51 02` input report (report ID included).
pub fn decode(r: &[u8]) -> Option<Reading> {
    if r.len() < 8 || r[0] != REPLY_BATTERY || r[1] != SUB_BATTERY || r[2] > 100 {
        return None;
    }
    let charging = match r[3] {
        0x00 => Some(false),
        0x01 | 0x02 => Some(true),
        _ => None,
    };
    let mv = u16::from_le_bytes([r[6], r[7]]) as i32;
    Some(Reading { level: r[2] as i32, charging, mv })
}

fn query(c: &HidColl) -> Option<Reading> {
    let d = HidDev::open(c)?;
    d.flush();
    let mut req = [0u8; LEN as usize];
    req[0] = CMD_BATTERY;
    req[1] = SUB_BATTERY;
    if !d.write(&req) {
        return None;
    }
    let end = Deadline::in_ms(WAIT_MS);
    while !end.passed() {
        if let Some(r) = d.read(end.left_ms()).and_then(|r| decode(&r)) {
            return Some(r);
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    for (vid, pid) in ctx.products(VID) {
        let Some(name) = model(pid) else { continue };
        let colls = ctx.find(vid, pid, |c| c.page() == 0xFF00 && c.usage() == 0x0001 && c.out_len() >= LEN && c.in_len() >= 8);
        if colls.is_empty() {
            continue;
        }
        let base = DeviceStatus {
            key: format!("hyperx_ngenuity:{}", hex4(pid)),
            name: name.into(),
            kind: DeviceKind::Mouse,
            via: "2.4 GHz dongle".into(),
            hw: Some((vid, pid)),
            grace_sec: 300, // the mouse sleeps when idle
            ..Default::default()
        };
        let s = match colls.iter().find_map(query) {
            Some(r) => DeviceStatus { percent: r.level, charging: r.charging.unwrap_or(false), millivolts: r.mv, ..base },
            None => DeviceStatus { connected: false, ..base },
        };
        ctx.add(s, vid, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(head: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..head.len()].copy_from_slice(head);
        r
    }

    #[test]
    fn battery_reply() {
        // 87 %, not charging, 25 °C, 4012 mV (0x0FAC).
        assert_eq!(decode(&report(&[0x51, 0x02, 0x57, 0x00, 0x19, 0x00, 0xAC, 0x0F])), Some(Reading { level: 87, charging: Some(false), mv: 4012 }));
        // Charging, and full on the cable.
        assert_eq!(decode(&report(&[0x51, 0x02, 0x32, 0x01])).map(|r| r.charging), Some(Some(true)));
        assert_eq!(decode(&report(&[0x51, 0x02, 0x64, 0x02])).map(|r| r.charging), Some(Some(true)));
        // Unknown charge state keeps the level.
        assert_eq!(decode(&report(&[0x51, 0x02, 0x40, 0xFF])).map(|r| (r.level, r.charging)), Some((64, None)));
    }

    #[test]
    fn not_battery() {
        // Link-state reply, a level above 100, other report IDs, too short.
        assert_eq!(decode(&report(&[0x53, 0x01, 0x01])), None);
        assert_eq!(decode(&report(&[0x51, 0x02, 0xFF])), None);
        assert_eq!(decode(&report(&[0x51, 0x01, 0x50])), None);
        assert_eq!(decode(&[0x51, 0x02, 0x50, 0x00]), None);
    }
}
