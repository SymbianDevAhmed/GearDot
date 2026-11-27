//! Logitech G PRO X 2 LIGHTSPEED (VID 046D, PID 0AF7), G522 LIGHTSPEED (PID 0B18) and ASTRO A20 X
//! (PID 0B35) headsets over their 2.4 GHz receivers.
//!
//! Not HID++ on FF43 (logitech.rs): Logitech's "Centurion" transport on the vendor collection
//! FFA0:0001 (interface 3), 64-byte output / input report 51. Protocol as documented by HeadsetControl
//! (GPL-3.0, github.com/Sapd/HeadsetControl, lib/devices/logitech_gpro_x2_lightspeed.hpp,
//! lib/devices/logitech_g522_lightspeed.hpp, lib/devices/logitech_astro_a20x.hpp (verified on Windows with the
//! A20 X: prefix 50 23, bridge, battery feature 0104 on interface 5) and
//! lib/devices/protocols/logitech_centurion_protocol.hpp), Solaar (GPL-2.0, its PRO X 2 0AF7 device
//! dump) and HaloBattery (MIT, github.com/HeyOkay/HaloBattery, providers/logitech_centurion.py,
//! confirmed on hardware in HaloBattery issue #103); implemented here from those facts.
//!
//!   frame     51 LL 00 <payload>                LL = payload length + 1 (G522, A20 X: 50 23 LL 00 <payload>)
//!   direct    payload <feature index> <function | 01> <params>; the reply echoes the index, data at 2
//!   bridge    payload <bridge> 11 <size hi> <size lo> 00 <headset feature index> <function | 01> <params>
//!             the receiver acknowledges (<bridge> 11), the headset answers
//!             <bridge> 10 .. .. 00 <index> <function> <data>   (<index> FF = rejected)
//!   event     <bridge> 00 00 CC                 headset connected (CC 1) / disconnected (CC 0)
//!
//! Read: Root (index 0) function 0 with feature id 0003 gives the receiver's bridge index; through the
//! bridge, Root function 0 with 0104 gives the headset's battery feature index; its function 0 answers
//! [level %] [..] [state], state 1 / 2 = charging, 3 = charged. Firmware without 0104 gets
//! HeadsetControl's fixed request 51 08 00 03 1A 00 03 00 04 0A: 51 03 = acknowledgement, 51 05 with
//! byte 6 = 00 = headset off, 51 0B with byte 8 = 04 = battery (byte 10 level, byte 12 = 02 charging).
//! The fixed request is the PRO X 2's own and is never sent to the G522 or the A20 X. Only these reads are sent; no
//! setting is changed.
use std::collections::HashMap;
use std::sync::Mutex;

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x046D;
const PID: u16 = 0x0AF7;
const REPORT: u8 = 0x51;

/// (PID, frame prefix, name, PRO X 2 fixed request as fallback)
const MODELS: [(u16, &[u8], &str, bool); 3] = [
    (PID, &[REPORT], "Logitech G PRO X 2 LIGHTSPEED", true),
    (0x0B18, &[0x50, 0x23], "Logitech G522 LIGHTSPEED", false), (0x0B35, &[0x50, 0x23], "Logitech ASTRO A20 X", false),
];
const SW: u8 = 0x01;
const BRIDGE_SEND: u8 = 0x10;
const FEATURE_BRIDGE: u16 = 0x0003;
const FEATURE_BATTERY: u16 = 0x0104;
const LEGACY: [u8; 10] = [0x51, 0x08, 0x00, 0x03, 0x1A, 0x00, 0x03, 0x00, 0x04, 0x0A];
const WAIT_MS: u64 = 1500;

pub fn frame(pre: &[u8], payload: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    let h = pre.len();
    f[..h].copy_from_slice(pre);
    f[h] = payload.len() as u8 + 1;
    let n = payload.len().min(62 - h);
    f[h + 2..h + 2 + n].copy_from_slice(&payload[..n]);
    f
}

/// The payload of a received frame (after the length and flags bytes).
pub fn payload<'a>(pre: &[u8], r: &'a [u8]) -> Option<&'a [u8]> {
    let h = pre.len();
    if r.len() < h + 3 || !r.starts_with(pre) {
        return None;
    }
    let len = r[h] as usize;
    (len > 1 && len + h < r.len()).then(|| &r[h + 2..len + h + 1])
}

pub fn direct(pre: &[u8], index: u8, function: u8, params: &[u8]) -> [u8; 64] {
    let mut p = vec![index, (function & 0xF0) | SW];
    p.extend_from_slice(params);
    frame(pre, &p)
}

pub fn bridged(pre: &[u8], bridge: u8, index: u8, function: u8, params: &[u8]) -> [u8; 64] {
    let size = 3 + params.len();
    let mut p = vec![bridge, BRIDGE_SEND | SW, (size >> 8) as u8 & 0x0F, size as u8, 0x00, index, (function & 0xF0) | SW];
    p.extend_from_slice(params);
    frame(pre, &p)
}

/// Data of a direct reply for feature `index`.
pub fn direct_reply(p: &[u8], index: u8) -> Option<Vec<u8>> {
    (p.len() >= 2 && p[0] == index).then(|| p[2..].to_vec())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Bridge {
    Ack,
    Answer(Vec<u8>),
    Rejected,
    Disconnected,
    Other,
}

/// Classifies one received payload while waiting for the headset's answer for its feature `index`.
pub fn bridge_reply(p: &[u8], bridge: u8, index: u8) -> Bridge {
    if p.len() >= 4 && p[0] == 0xFF && p[1] == bridge && p[2] == (BRIDGE_SEND | SW) {
        return Bridge::Rejected;
    }
    if p.len() < 2 || p[0] != bridge {
        return Bridge::Other;
    }
    if p[1] == 0x00 {
        return if p.len() >= 4 && p[3] == 0x00 { Bridge::Disconnected } else { Bridge::Other };
    }
    if p[1] >> 4 != BRIDGE_SEND >> 4 {
        return Bridge::Other;
    }
    match p[1] & 0x0F {
        SW => return Bridge::Ack,
        0 => {}
        _ => return Bridge::Other,
    }
    if p.len() < 7 || p[4] != 0x00 {
        return Bridge::Other;
    }
    if p[5] == index {
        Bridge::Answer(p[7..].to_vec())
    } else if p[5] == 0xFF && p[6] == index {
        Bridge::Rejected
    } else {
        Bridge::Other
    }
}

/// Battery feature data: Some((level, charging, charged)).
pub fn parse_battery(data: &[u8]) -> Option<(i32, bool, bool)> {
    let level = *data.first()?;
    let state = data.get(2).copied().unwrap_or(0);
    (level <= 100).then_some((level as i32, state == 1 || state == 2, state == 3))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Legacy {
    Battery(i32, bool),
    Off,
    Skip,
}

pub fn legacy_reply(r: &[u8]) -> Option<Legacy> {
    if r.len() < 2 || r[0] != REPORT {
        return None;
    }
    match r[1] {
        0x03 => Some(Legacy::Skip),
        0x05 if r.len() >= 7 && r[6] == 0x00 => Some(Legacy::Off),
        0x05 => Some(Legacy::Skip),
        0x0B if r.len() >= 13 && r[8] == 0x04 => (r[10] <= 100).then(|| Legacy::Battery(r[10] as i32, r[12] == 0x02)),
        _ => None,
    }
}

fn feature_params(id: u16) -> [u8; 2] {
    id.to_be_bytes()
}

enum Error {
    /// The headset is off or the request was rejected: show the receiver as disconnected.
    Offline,
    /// The headset has no such feature.
    NotSupported,
    /// No usable answer.
    Failed,
}

fn ask_direct(d: &HidDev, pre: &[u8], index: u8, function: u8, params: &[u8]) -> Result<Vec<u8>, Error> {
    d.flush();
    if !d.write(&direct(pre, index, function, params)) {
        return Err(Error::Failed);
    }
    let end = Deadline::in_ms(WAIT_MS);
    while !end.passed() {
        let Some(r) = d.read(end.left_ms()) else { continue };
        if let Some(data) = payload(pre, &r).and_then(|p| direct_reply(p, index)) {
            return Ok(data);
        }
    }
    Err(Error::Failed)
}

fn ask_bridged(d: &HidDev, pre: &[u8], bridge: u8, index: u8, function: u8, params: &[u8]) -> Result<Vec<u8>, Error> {
    d.flush();
    if !d.write(&bridged(pre, bridge, index, function, params)) {
        return Err(Error::Failed);
    }
    let end = Deadline::in_ms(WAIT_MS);
    let mut acked = false;
    while !end.passed() {
        let Some(r) = d.read(end.left_ms()) else { continue };
        match payload(pre, &r).map(|p| bridge_reply(p, bridge, index)) {
            Some(Bridge::Answer(data)) => return Ok(data),
            Some(Bridge::Ack) => acked = true,
            Some(Bridge::Rejected) | Some(Bridge::Disconnected) => return Err(Error::Offline),
            _ => {}
        }
    }
    // Acknowledged by the receiver but never answered by the headset: off or out of range.
    Err(if acked { Error::Offline } else { Error::Failed })
}

fn ask_legacy(d: &HidDev) -> Result<(i32, bool, bool), Error> {
    d.flush();
    if !d.write(&LEGACY) {
        return Err(Error::Failed);
    }
    let end = Deadline::in_ms(WAIT_MS);
    let mut reads = 0;
    while !end.passed() && reads < 4 {
        let Some(r) = d.read(end.left_ms()) else { continue };
        match legacy_reply(&r) {
            Some(Legacy::Battery(level, charging)) => return Ok((level, charging, false)),
            Some(Legacy::Off) => return Err(Error::Offline),
            Some(Legacy::Skip) => reads += 1,
            None => {}
        }
    }
    Err(Error::Failed)
}

/// Bridge and headset battery feature indexes, kept while they keep working.
type Indexes = HashMap<String, (u8, Option<u8>)>;
static INDEXES: Mutex<Option<Indexes>> = Mutex::new(None);

fn read(c: &HidColl, pre: &[u8], legacy: bool) -> Result<(i32, bool, bool), Error> {
    let d = HidDev::open(c).ok_or(Error::Failed)?;
    let cached = INDEXES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&c.path).copied());
    let bridge = match cached {
        Some((b, _)) => b,
        None => match ask_direct(&d, pre, 0x00, 0x00, &feature_params(FEATURE_BRIDGE))?.first() {
            Some(&b) if b != 0 => b,
            _ => return Err(Error::Failed),
        },
    };
    let battery = match cached.and_then(|(_, i)| i) {
        Some(i) => Ok(i),
        None => match ask_bridged(&d, pre, bridge, 0x00, 0x00, &feature_params(FEATURE_BATTERY)) {
            Ok(data) => match data.first() {
                Some(&i) if i != 0 => Ok(i),
                _ => Err(Error::NotSupported),
            },
            Err(e) => Err(e),
        },
    };
    let remember = |battery: Option<u8>| {
        let mut g = INDEXES.lock().unwrap_or_else(|e| e.into_inner());
        g.get_or_insert_with(HashMap::new).insert(c.path.clone(), (bridge, battery));
    };
    match battery {
        Ok(index) => match ask_bridged(&d, pre, bridge, index, 0x00, &[]) {
            Ok(data) => {
                remember(Some(index));
                parse_battery(&data).ok_or(Error::Failed)
            }
            Err(e) => {
                remember(None); // look the feature up again next time
                Err(e)
            }
        },
        Err(Error::NotSupported) => {
            remember(None);
            if legacy { ask_legacy(&d) } else { Err(Error::Failed) }
        }
        Err(e) => {
            remember(None);
            Err(e)
        }
    }
}

pub fn poll(ctx: &mut PollContext) {
    for (pid, pre, name, legacy) in MODELS {
        poll_model(ctx, pid, pre, name, legacy);
    }
}

fn poll_model(ctx: &mut PollContext, pid: u16, pre: &[u8], name: &str, legacy: bool) {
    if ctx.is_claimed(VID, pid) {
        return;
    }
    let colls = ctx.find(VID, pid, |c| c.page() == 0xFFA0 && c.usage() == 0x0001 && c.in_len() >= 64 && c.out_len() >= 64);
    if colls.is_empty() {
        return;
    }
    let base = DeviceStatus {
        key: format!("logi-centurion:{}", hex4(pid)),
        name: name.into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, pid)),
        grace_sec: 60,
        ..Default::default()
    };
    let mut found = None;
    for c in &colls {
        match read(c, pre, legacy) {
            Ok(v) => {
                found = Some(v);
                break;
            }
            Err(_) => continue,
        }
    }
    let s = match found {
        Some((level, charging, full)) => DeviceStatus { percent: level, charging, full, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, pid);
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &[u8] = &[REPORT];
    const G522: &[u8] = &[0x50, 0x23];

    fn rx(p: &[u8]) -> [u8; 64] {
        frame(P, p)
    }

    #[test]
    fn frames() {
        // Root.getFeature(0003): 51 05 00 00 01 00 03
        assert_eq!(direct(P, 0, 0, &[0x00, 0x03])[..7], [0x51, 0x05, 0x00, 0x00, 0x01, 0x00, 0x03]);
        assert_eq!(direct(G522, 0, 0, &[0x00, 0x03])[..8], [0x50, 0x23, 0x05, 0x00, 0x00, 0x01, 0x00, 0x03]);
        let g = frame(G522, &[0x00, 0x01, 0x03, 0x00, 0x00]);
        assert_eq!(payload(G522, &g), Some(&[0x00, 0x01, 0x03, 0x00, 0x00][..]));
        assert_eq!(payload(P, &g), None);
        // Through bridge 3, headset Root.getFeature(0104): sub-message 00 00 01 01 04 (5 bytes).
        assert_eq!(bridged(P, 3, 0, 0, &[0x01, 0x04])[..12], [0x51, 0x0A, 0x00, 0x03, 0x11, 0x00, 0x05, 0x00, 0x00, 0x01, 0x01, 0x04]);
        assert_eq!(payload(P, &rx(&[0x00, 0x01, 0x03, 0x00, 0x00])), Some(&[0x00, 0x01, 0x03, 0x00, 0x00][..]));
        assert_eq!(payload(P, &[0x50, 0x02, 0x00, 0x00]), None);
    }

    #[test]
    fn direct_replies() {
        // Root answers the bridge index (3), as in Solaar's dump of a PRO X 2.
        let r = rx(&[0x00, 0x01, 0x03, 0x00, 0x00]);
        assert_eq!(payload(P, &r).and_then(|p| direct_reply(p, 0)), Some(vec![0x03, 0x00, 0x00]));
    }

    #[test]
    fn bridge_replies() {
        assert_eq!(bridge_reply(&[0x03, 0x11, 0x00, 0x05], 3, 4), Bridge::Ack);
        // Battery feature 4: 75 %, discharging.
        let answer = [0x03, 0x10, 0x00, 0x06, 0x00, 0x04, 0x01, 75, 0x00, 0x00];
        assert_eq!(bridge_reply(&answer, 3, 4), Bridge::Answer(vec![75, 0x00, 0x00]));
        assert_eq!(parse_battery(&[75, 0x00, 0x00]), Some((75, false, false)));
        assert_eq!(parse_battery(&[60, 0x00, 0x01]), Some((60, true, false)));
        assert_eq!(parse_battery(&[100, 0x00, 0x03]), Some((100, false, true)));
        assert_eq!(parse_battery(&[101]), None);
        assert_eq!(bridge_reply(&[0x03, 0x10, 0x00, 0x04, 0x00, 0xFF, 0x04, 0x05], 3, 4), Bridge::Rejected);
        assert_eq!(bridge_reply(&[0xFF, 0x03, 0x11, 0x05], 3, 4), Bridge::Rejected);
        assert_eq!(bridge_reply(&[0x03, 0x00, 0x00, 0x00], 3, 4), Bridge::Disconnected);
        assert_eq!(bridge_reply(&[0x03, 0x00, 0x00, 0x01], 3, 4), Bridge::Other);
        assert_eq!(bridge_reply(&[0x03, 0x10, 0x00, 0x06, 0x00, 0x02, 0x01, 75], 3, 4), Bridge::Other);
    }

    #[test]
    fn legacy() {
        assert_eq!(LEGACY, [0x51, 0x08, 0x00, 0x03, 0x1A, 0x00, 0x03, 0x00, 0x04, 0x0A]);
        let mut bat = [0u8; 64];
        bat[..13].copy_from_slice(&[0x51, 0x0B, 0, 0, 0, 0, 0, 0, 0x04, 0, 64, 0, 0x02]);
        assert_eq!(legacy_reply(&bat), Some(Legacy::Battery(64, true)));
        assert_eq!(legacy_reply(&[0x51, 0x05, 0, 0, 0, 0, 0x00]), Some(Legacy::Off));
        assert_eq!(legacy_reply(&[0x51, 0x05, 0, 0, 0, 0, 0x01]), Some(Legacy::Skip));
        assert_eq!(legacy_reply(&[0x51, 0x03, 0, 0]), Some(Legacy::Skip));
        assert_eq!(legacy_reply(&[0x52, 0x0B]), None);
    }
}
