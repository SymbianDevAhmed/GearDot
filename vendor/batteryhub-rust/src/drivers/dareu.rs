//! DAREU EK75 Pro keyboard on its 2.4 GHz receiver 260D:0042 or USB cable 260D:0046 (DAREU's "Togran"
//! devices).
//!
//! Protocol as documented by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! src/drivers/dareu.cpp: read from DAREU's own web driver, the EK75 Pro's cable verified on hardware
//! there); implemented here from those facts. Feature report 0A (65 bytes) on the consumer collection
//! 000C:0001:
//!
//!   set feature  0A SL LN CL CM          SL = slot << 4, LN length, CL class, CM command
//!   get feature  0A x2 .. CL CM .. ..    low nibble of byte 1 = 2 once answered
//!
//!   slot 0, 06 00 A0   [7] = paired devices, then from [8] three bytes each: present, pid (big endian)
//!   slot n, 04 07 80   [7] = 1 charging, [8] level, [9] its maximum (0 = 100)
//!
//! Only these two reads are sent. The receiver lists its devices by slot; the cable answers on slot 0.
//! Not tested on hardware by this project (issue #63).
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::sleep_ms;

const VID: u16 = 0x260D;
const RECEIVER: u16 = 0x0042;
const CABLE: u16 = 0x0046;
const KEY: &str = "dareu:ek75pro";
const REPORT: u8 = 0x0A;
const LEN: usize = 65;
/// Length, class, command.
const CMD_PAIRED: [u8; 3] = [0x06, 0x00, 0xA0];
const CMD_BATTERY: [u8; 3] = [0x04, 0x07, 0x80];
const MAX_SLOTS: usize = 5;

pub fn request(slot: u8, cmd: [u8; 3]) -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[..5].copy_from_slice(&[REPORT, slot << 4, cmd[0], cmd[1], cmd[2]]);
    r
}

/// The report read back is the answer to `cmd`.
pub fn answered(r: &[u8], cmd: [u8; 3]) -> bool {
    r.len() >= 11 && r[0] == REPORT && r[1] & 0x0F == 2 && r[3] == cmd[1] && r[4] == cmd[2]
}

/// Slots (1 based) with a device present.
pub fn slots(r: &[u8]) -> Vec<u8> {
    let count = (r[7] as usize).min(MAX_SLOTS);
    (0..count).filter(|i| r.get(8 + 3 * i).is_some_and(|&present| present != 0)).map(|i| i as u8 + 1).collect()
}

/// Level in percent, charging.
pub fn battery(r: &[u8]) -> Option<(i32, bool)> {
    let max = if r[9] == 0 { 100 } else { r[9] as i32 };
    let level = r[8] as i32;
    (level <= max).then(|| (level * 100 / max, r[7] == 1))
}

fn query(d: &HidDev, slot: u8, cmd: [u8; 3]) -> Option<Vec<u8>> {
    if !d.set_feature(&request(slot, cmd)) {
        return None;
    }
    (0..15).find_map(|_| {
        sleep_ms(30);
        d.get_feature(REPORT).filter(|r| answered(r, cmd))
    })
}

fn read(c: &HidColl, cable: bool) -> Option<(i32, bool)> {
    let d = HidDev::open_feature_only(c)?;
    let slots = if cable { vec![0] } else { slots(&query(&d, 0, CMD_PAIRED)?) };
    slots.into_iter().find_map(|s| query(&d, s, CMD_BATTERY).and_then(|r| battery(&r)))
}

pub fn poll(ctx: &mut PollContext) {
    for pid in [CABLE, RECEIVER] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.page() == 0x000C && c.usage() == 0x0001 && c.feat_len() as usize == LEN);
        let Some((percent, charging)) = colls.iter().find_map(|c| read(c, pid == CABLE)) else { continue };
        if ctx.out.iter().any(|o| o.key == KEY) {
            ctx.claim(VID, pid); // on the cable as well: listed once
            continue;
        }
        let s = DeviceStatus {
            key: KEY.into(),
            name: "DAREU EK75 Pro".into(),
            kind: DeviceKind::Keyboard,
            via: if pid == CABLE { "USB cable" } else { "2.4 GHz receiver" }.into(),
            percent,
            charging,
            hw: Some((VID, pid)),
            grace_sec: 1800,
            ..Default::default()
        };
        ctx.add(s, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(cmd: [u8; 3], data: &[u8]) -> Vec<u8> {
        let mut r = request(1, cmd).to_vec();
        r[1] |= 2;
        r[7..7 + data.len()].copy_from_slice(data);
        r
    }

    #[test]
    fn requests() {
        assert_eq!(request(0, CMD_PAIRED)[..6], [0x0A, 0x00, 0x06, 0x00, 0xA0, 0]);
        assert_eq!(request(2, CMD_BATTERY)[..6], [0x0A, 0x20, 0x04, 0x07, 0x80, 0]);
        assert!(!answered(&request(1, CMD_BATTERY), CMD_BATTERY)); // not answered yet
    }

    #[test]
    fn replies() {
        let r = reply(CMD_BATTERY, &[1, 82, 100]);
        assert!(answered(&r, CMD_BATTERY) && !answered(&r, CMD_PAIRED));
        assert_eq!(battery(&r), Some((82, true)));
        assert_eq!(battery(&reply(CMD_BATTERY, &[0, 3, 4])), Some((75, false)));
        assert_eq!(battery(&reply(CMD_BATTERY, &[0, 60, 0])), Some((60, false)));
        assert_eq!(battery(&reply(CMD_BATTERY, &[0, 101, 100])), None);
        assert_eq!(slots(&reply(CMD_PAIRED, &[2, 0, 0, 0, 1, 0x00, 0x46])), [2]);
        assert!(slots(&reply(CMD_PAIRED, &[0])).is_empty());
    }
}
