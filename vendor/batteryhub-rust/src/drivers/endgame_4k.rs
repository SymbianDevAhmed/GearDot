//! Endgame Gear 4K wireless mice (OP1w 4k v1 / v2, XM2w 4k v1 / v2) on the shared 4K dongle 3367:1970.
//!
//! Protocol as documented by vole (MIT, github.com/johanneszab/vole, re/PROTOCOL.md, verified on an
//! OP1w 4k v1 and v2 with dongle firmware 1.01) and Endgame_Gear_Web_Configurator (GPL-3.0,
//! github.com/Janlhin/Endgame_Gear_Web_Configurator, src/session.ts / transport.ts), as used by the
//! original C++ BatteryHub (GPL-3.0, src/drivers/endgame.cpp); implemented here from those facts.
//! Config channel = vendor collection FF01:0002, feature report A1 (64 bytes, no checksum):
//!
//!   request  A1 CC TT 00 ...           CC = command, TT = target
//!   SET_FEATURE the request, wait ~150 ms, GET_FEATURE A1. Reply byte 1 = status:
//!            01 ok, 03 busy (read again), 08 / 11 mouse asleep or out of range
//!   B4       battery: reply [16] = level %. No charging flag is known for this family.
//!   0E       mouse info: reply [16..17] = 67 33 (VID), [18..19] = the mouse's PID (little endian)
//!   0F       probe: after a failed command the dongle stays silent until a 0F gets through, so it
//!            is sent only after a failure (sending it every poll keeps the mouse awake).
//!
//! Only these reads are sent; no write, reset or re-pair command.
use std::collections::HashMap;
use std::sync::Mutex;

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{hex4, sleep_ms};

const VID: u16 = 0x3367;
const DONGLE_4K: u16 = 0x1970;
const REPORT: u8 = 0xA1;
pub const CMD_BATTERY: u8 = 0xB4;
pub const CMD_INFO: u8 = 0x0E;
const CMD_PROBE: u8 = 0x0F;
pub const ST_OK: u8 = 0x01;
const ST_BUSY: u8 = 0x03;

pub fn request(cmd: u8, target: u8) -> [u8; 64] {
    let mut r = [0u8; 64];
    r[..3].copy_from_slice(&[REPORT, cmd, target]);
    r
}

pub fn mouse_name(pid: u16) -> Option<&'static str> {
    match pid {
        0x1972 => Some("Endgame Gear OP1w 4k"),
        0x1984 => Some("Endgame Gear OP1w 4k v2"),
        0x1968 => Some("Endgame Gear XM2w 4k"),
        0x1982 => Some("Endgame Gear XM2w 4k v2"),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub enum Battery {
    Level(i32),
    Asleep,
}

/// Battery from a B4 reply (report id first). None = unexpected status or not a reply.
pub fn parse_battery(r: &[u8]) -> Option<Battery> {
    if r.len() < 20 || r[0] != REPORT {
        return None;
    }
    match r[1] {
        ST_OK if r[16] <= 100 => Some(Battery::Level(r[16] as i32)),
        0x08 | 0x11 => Some(Battery::Asleep),
        _ => None,
    }
}

/// The mouse's PID from a 0E reply.
pub fn parse_info(r: &[u8]) -> Option<u16> {
    (r.len() >= 20 && r[0] == REPORT && r[1] == ST_OK && r[16] == 0x67 && r[17] == 0x33).then(|| u16::from_le_bytes([r[18], r[19]]))
}

fn command(d: &HidDev, cmd: u8, target: u8) -> Option<Vec<u8>> {
    if !d.set_feature(&request(cmd, target)) {
        return None;
    }
    let mut last = None;
    for attempt in 0..3u64 {
        sleep_ms(150 + attempt * 100);
        let r = d.get_feature(REPORT).filter(|r| r.len() >= 20 && r[0] == REPORT)?;
        if r[1] != ST_BUSY {
            return Some(r);
        }
        last = Some(r);
    }
    last
}

/// Last mouse name seen per dongle path: the mouse only identifies itself while awake.
static NAMES: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);

fn read(c: &HidColl) -> Option<(Battery, Option<&'static str>)> {
    let d = HidDev::open_feature_only(c)?;
    let mut b = command(&d, CMD_BATTERY, 0x00).and_then(|r| parse_battery(&r));
    if b.is_none() {
        // Unexpected status or no answer: probe the dongle and the mouse once, then retry.
        let _ = command(&d, CMD_PROBE, 0x01);
        let _ = command(&d, CMD_PROBE, 0x0F);
        b = command(&d, CMD_BATTERY, 0x00).and_then(|r| parse_battery(&r));
    }
    let b = b?;
    let name = match b {
        Battery::Level(_) => command(&d, CMD_INFO, 0x00).and_then(|r| parse_info(&r)).and_then(mouse_name),
        Battery::Asleep => None,
    };
    Some((b, name))
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, DONGLE_4K) {
        return;
    }
    let colls = ctx.find(VID, DONGLE_4K, |c| c.page() == 0xFF01 && c.usage() == 0x0002 && c.feat_len() >= 64);
    let Some((c, (battery, name))) = colls.iter().find_map(|c| read(c).map(|r| (c, r))) else { return };
    let name = {
        let mut g = NAMES.lock().unwrap_or_else(|e| e.into_inner());
        let names = g.get_or_insert_with(HashMap::new);
        if let Some(n) = name {
            names.insert(c.path.clone(), n);
        }
        names.get(&c.path).copied().unwrap_or("Endgame Gear 4k wireless mouse")
    };
    let base = DeviceStatus {
        key: format!("endgame:{}", hex4(DONGLE_4K)),
        name: name.into(),
        kind: DeviceKind::Mouse,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, DONGLE_4K)),
        grace_sec: 1800,
        ..Default::default()
    };
    let s = match battery {
        Battery::Level(level) => DeviceStatus { percent: level, ..base },
        Battery::Asleep => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, DONGLE_4K);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u8, at16: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = REPORT;
        r[1] = status;
        r[16..16 + at16.len()].copy_from_slice(at16);
        r
    }

    #[test]
    fn request_frame() {
        let r = request(CMD_BATTERY, 0x00);
        assert_eq!(&r[..4], &[0xA1, 0xB4, 0x00, 0x00]);
        assert!(r[3..].iter().all(|&b| b == 0));
    }

    #[test]
    fn battery_replies() {
        assert_eq!(parse_battery(&reply(0x01, &[73])), Some(Battery::Level(73)));
        assert_eq!(parse_battery(&reply(0x08, &[0])), Some(Battery::Asleep));
        assert_eq!(parse_battery(&reply(0x11, &[0])), Some(Battery::Asleep));
        assert_eq!(parse_battery(&reply(0x03, &[73])), None); // still busy
        assert_eq!(parse_battery(&reply(0x01, &[150])), None);
        assert_eq!(parse_battery(&[0xA1, 0x01, 0x00]), None);
    }

    #[test]
    fn info_replies() {
        assert_eq!(parse_info(&reply(0x01, &[0x67, 0x33, 0x82, 0x19])), Some(0x1982));
        assert_eq!(mouse_name(0x1982), Some("Endgame Gear XM2w 4k v2"));
        assert_eq!(parse_info(&reply(0x01, &[0x00, 0x00, 0x82, 0x19])), None);
        assert_eq!(parse_info(&reply(0x08, &[0x67, 0x33, 0x82, 0x19])), None);
    }
}
