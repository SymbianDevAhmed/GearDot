//! Older HyperX wireless headsets on their 2.4 GHz dongles: Cloud II Wireless (HP 03F0:0696 / 018B),
//! Cloud Flight (Kingston 0951:16C4 /
//! 0951:1723) and Cloud Stinger Core Wireless (0951:170B; PlayStation receivers 03F0:0A93,
//! 0951:1713, 03F0:0B93). The Cloud III S is in hyperx.rs, the Cloud II Core Wireless in hyperx_core.rs.
//!
//! Implemented from documented facts, as also used by the original C++ BatteryHub (GPL-3.0,
//! src/drivers/hyperx.cpp):
//!
//! * Cloud Flight, from HeadsetControl (GPL-3.0, github.com/Sapd/HeadsetControl,
//!   lib/devices/hyperx_cloud_flight.hpp) and HyperHeadset (MIT, github.com/LennardKittner/HyperHeadset,
//!   src/devices/cloud_flight_wireless.rs). 20-byte report 21 on the FF90:0303 collection:
//!     request  21 FF 05 00 ...    reply  21 FF 05 HH LL ...   HHLL = voltage in mV (big endian)
//!   Above 0x100B mV the headset is charging (no level then); otherwise the level comes from
//!   HeadsetControl's voltage polynomial.
//! * Cloud Stinger Core Wireless, from HyperBatteryMonitor (MIT,
//!   github.com/davidsantanaalves/HyperBatteryMonitor, Devices/CloudStingerCoreWirelessDevice.cs).
//!   Feature report FF on FF00:0001: SetFeature(command), wait 20 ms, GetFeature(FF).
//!     connection  reply [11] == 1: headset on
//!     battery     reply [9] = 03 discharging / 05 charging / 06 full, [11..12] = mV (little endian)
//!   The PlayStation receivers can hand back a previous report; their battery reply starts FF 12 05,
//!   so there it is read again (up to 10 x 20 ms) until it does.
//! * Cloud Alpha Wireless (03F0:098D), from HeadsetControl (lib/devices/hyperx_cloud_alpha_wireless.hpp).
//!   Report 21 on the 31-byte FF43:0202 collection: `21 BB CC`, reply `21 .. CC VV`:
//!     03 = link (VV 01: headset not connected), 0C = charging (VV 01), 0B = level (VV %)
//!
//! * Cloud II Wireless (HP), from HeadsetControl (lib/devices/hyperx_cloud_2_wireless.hpp). Report 06 on
//!   the 20-byte FF90:0303 collection: `06 FF BB <cmd>`, reply `06 FF BB <cmd> ..`:
//!     02 = battery: [5..6] = mV (big endian), [7] = level %;   03 = charging ([4] == 1)
//!   No reply: the headset is off.
//!
//! Read-only: only these queries are sent. A dongle whose headset does not answer is shown off.
#![allow(clippy::doc_overindented_list_items)]
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{Deadline, hex4, interpolate, sleep_ms};

#[derive(Clone, Copy, PartialEq)]
enum Proto {
    Flight,
    Stinger { ps: bool },
    Alpha,
    Cloud2,
}

const MODELS: [(u16, u16, &str, Proto); 10] = [
    (0x03F0, 0x0696, "HyperX Cloud II Wireless", Proto::Cloud2),
    (0x03F0, 0x018B, "HyperX Cloud II Wireless", Proto::Cloud2),
    (0x03F0, 0x098D, "HyperX Cloud Alpha Wireless", Proto::Alpha),
    (0x0951, 0x16C4, "HyperX Cloud Flight", Proto::Flight),
    (0x0951, 0x1723, "HyperX Cloud Flight", Proto::Flight),
    (0x0951, 0x170B, "HyperX Cloud Stinger Core Wireless", Proto::Stinger { ps: false }),
    (0x0951, 0x1713, "HyperX Cloud Stinger Core Wireless", Proto::Stinger { ps: true }),
    // Cloud Core Wireless DTS: the Stinger Core exchange, hardware-verified by TaskbarVitals (GPL-3.0,
    // github.com/symNistaken/TaskbarVitals, src/drivers/hyperx.cpp); it can return a stale frame
    // first, so it gets the retrying read.
    (0x0951, 0x173F, "HyperX Cloud Core Wireless", Proto::Stinger { ps: true }),
    (0x03F0, 0x0A93, "HyperX Cloud Stinger Core Wireless", Proto::Stinger { ps: true }),
    // Same headset's PlayStation receiver (issue #94); same collection, untested.
    (0x03F0, 0x0B93, "HyperX Cloud Stinger Core Wireless", Proto::Stinger { ps: true }),
];

#[derive(Debug, PartialEq, Default)]
pub struct Reading {
    pub on: bool,
    pub percent: Option<i32>,
    pub mv: i32,
    pub charging: bool,
    pub full: bool,
}

// ---- Cloud Flight ------------------------------------------------------------------------------

const FLIGHT_CHARGING_MV: i32 = 0x100B;

pub fn flight_request() -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..3].copy_from_slice(&[0x21, 0xFF, 0x05]);
    r
}

/// HeadsetControl's voltage -> % estimate.
pub fn flight_percent(mv: i32) -> i32 {
    if mv <= 3648 {
        return (0.00125 * mv as f64) as i32;
    }
    if mv > 3975 {
        return 100;
    }
    let v = mv as f64;
    let p = 0.00000002547505 * v.powi(4) - 0.0003900299 * v.powi(3) + 2.238321 * v * v - 5706.256 * v + 5452299.0;
    (p.round() as i32).clamp(0, 100)
}

/// Voltage from a reply (report id included).
pub fn flight_parse(r: &[u8]) -> Option<Reading> {
    if r.len() < 5 || r[..3] != [0x21, 0xFF, 0x05] {
        return None;
    }
    let mv = ((r[3] as i32) << 8) | r[4] as i32;
    if mv == 0 {
        return Some(Reading::default());
    }
    let charging = mv > FLIGHT_CHARGING_MV;
    Some(Reading { on: true, percent: (!charging).then(|| flight_percent(mv)), mv, charging, full: false })
}

fn flight(c: &HidColl) -> Option<Reading> {
    let d = HidDev::open(c)?;
    d.flush();
    if !d.write(&flight_request()) {
        return None;
    }
    let end = Deadline::in_ms(1000);
    while !end.passed() {
        if let Some(r) = d.read(end.left_ms()).and_then(|r| flight_parse(&r)) {
            return Some(r);
        }
    }
    Some(Reading::default()) // dongle present, headset silent
}

// ---- Cloud Stinger Core ------------------------------------------------------------------------

const STINGER_CONNECTION: [u8; 20] = [0xFF, 0x07, 0x00, 0xFD, 0x04, 0x00, 0x00, 0x02, 0x82, 0x03, 0x05, 0x29, 0x0F, 0x00, 0x00, 0x03, 0x68, 0x10, 0x00, 0x00];
const STINGER_BATTERY: [u8; 12] = [0xFF, 0x07, 0x00, 0xFD, 0x04, 0x0C, 0xF1, 0x02, 0x01, 0x04, 0xF0, 0x0C];
const STINGER_PCT: [i32; 11] = [100, 90, 80, 70, 60, 50, 40, 30, 20, 10, 0];
const STINGER_CHARGING_MV: [i32; 11] = [4130, 4090, 4050, 4000, 3967, 3946, 3930, 3916, 3871, 3831, 3400];
const STINGER_DISCHARGING_MV: [i32; 11] = [4121, 4042, 3954, 3905, 3838, 3785, 3763, 3738, 3700, 3651, 3517];

/// Battery from the GetFeature(FF) reply to the battery command.
pub fn stinger_battery(r: &[u8]) -> Option<Reading> {
    if r.len() < 13 || r[0] != 0xFF {
        return None;
    }
    let code = r[9];
    if !matches!(code, 0x03 | 0x05 | 0x06) {
        return None;
    }
    let mv = u16::from_le_bytes([r[11], r[12]]) as i32;
    let percent = match code {
        0x06 => 100,
        0x05 => interpolate(mv, &STINGER_CHARGING_MV, &STINGER_PCT),
        _ => interpolate(mv, &STINGER_DISCHARGING_MV, &STINGER_PCT),
    };
    Some(Reading { on: true, percent: Some(percent), mv, charging: code == 0x05, full: code == 0x06 })
}

fn stinger(c: &HidColl, ps: bool) -> Option<Reading> {
    let d = HidDev::open(c).or_else(|| HidDev::open_feature_only(c))?;
    let exchange = |cmd: &[u8], good: &dyn Fn(&[u8]) -> bool| -> Option<Vec<u8>> {
        if !d.set_feature(cmd) {
            return None;
        }
        for _ in 0..if ps { 10 } else { 1 } {
            sleep_ms(20);
            let r = d.get_feature(0xFF).filter(|r| r.len() >= 13)?;
            if !ps || good(&r) {
                return Some(r);
            }
        }
        None
    };
    let fresh = |r: &[u8]| !(r[1] == 0x01 && r[2] == 0x00);
    let link = match exchange(&STINGER_CONNECTION, &fresh) {
        Some(r) => r,
        // The connection reply's header isn't documented: on the PlayStation receivers the last
        // read is used after 10 stale-looking ones.
        None if ps => {
            sleep_ms(20);
            d.get_feature(0xFF).filter(|r| r.len() >= 13)?
        }
        None => return None,
    };
    if link[11] != 0x01 {
        return Some(Reading::default());
    }
    let ok = |r: &[u8]| r[0] == 0xFF && r[1] == 0x12 && r[2] == 0x05;
    Some(exchange(&STINGER_BATTERY, &ok).and_then(|r| stinger_battery(&r)).unwrap_or(Reading { on: true, ..Default::default() }))
}

// ---- Cloud Alpha Wireless ----------------------------------------------------------------------

const ALPHA_LINK: u8 = 0x03;
const ALPHA_LEVEL: u8 = 0x0B;
const ALPHA_CHARGING: u8 = 0x0C;

pub fn alpha_request(cmd: u8) -> [u8; 3] {
    [0x21, 0xBB, cmd]
}

/// Value byte of the reply to `cmd` (report id included).
pub fn alpha_parse(r: &[u8], cmd: u8) -> Option<u8> {
    (r.len() >= 4 && r[0] == 0x21 && r[2] == cmd).then(|| r[3])
}

fn alpha_query(d: &HidDev, cmd: u8) -> Option<u8> {
    d.flush();
    if !d.write(&alpha_request(cmd)) {
        return None;
    }
    let end = Deadline::in_ms(1000);
    while !end.passed() {
        if let Some(v) = d.read(end.left_ms()).and_then(|r| alpha_parse(&r, cmd)) {
            return Some(v);
        }
    }
    None
}

fn alpha(c: &HidColl) -> Option<Reading> {
    let d = HidDev::open(c)?;
    if alpha_query(&d, ALPHA_LINK)? == 0x01 {
        return Some(Reading::default()); // headset not connected
    }
    let mut out = Reading { on: true, ..Default::default() };
    out.charging = alpha_query(&d, ALPHA_CHARGING) == Some(0x01);
    out.percent = alpha_query(&d, ALPHA_LEVEL).filter(|l| *l <= 100).map(|l| l as i32);
    Some(out)
}

// ---- Cloud II Wireless (HP) -------------------------------------------------------------------

const CLOUD2_LEVEL: u8 = 0x02;
const CLOUD2_CHARGING: u8 = 0x03;

pub fn cloud2_request(cmd: u8) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..4].copy_from_slice(&[0x06, 0xFF, 0xBB, cmd]);
    r
}

/// The reply to `cmd` (report id included), if this is it.
pub fn cloud2_parse(r: &[u8], cmd: u8) -> Option<&[u8]> {
    (r.len() >= 8 && r[..4] == [0x06, 0xFF, 0xBB, cmd]).then_some(r)
}

fn cloud2_query(d: &HidDev, cmd: u8) -> Option<Vec<u8>> {
    d.flush();
    if !d.write(&cloud2_request(cmd)) {
        return None;
    }
    let end = Deadline::in_ms(1000);
    while !end.passed() {
        if let Some(r) = d.read(end.left_ms()).filter(|r| cloud2_parse(r, cmd).is_some()) {
            return Some(r);
        }
    }
    None
}

fn cloud2(c: &HidColl) -> Option<Reading> {
    let d = HidDev::open(c)?;
    let Some(r) = cloud2_query(&d, CLOUD2_LEVEL) else {
        return Some(Reading::default()); // dongle present, headset silent
    };
    let mv = ((r[5] as i32) << 8) | r[6] as i32;
    let percent = (r[7] <= 100).then_some(r[7] as i32);
    let charging = cloud2_query(&d, CLOUD2_CHARGING).is_some_and(|c| c[4] == 1);
    Some(Reading { on: true, percent, mv, charging, full: false })
}

// ---- Polling -----------------------------------------------------------------------------------

fn collections(ctx: &PollContext, vid: u16, pid: u16, proto: Proto) -> Vec<HidColl> {
    match proto {
        Proto::Alpha => ctx.find(vid, pid, |c| c.page() == 0xFF43 && c.usage() == 0x0202 && c.out_len() >= 4 && c.in_len() >= 4),
        Proto::Flight => ctx.find(vid, pid, |c| c.page() == 0xFF90 && c.usage() == 0x0303 && c.out_len() == 20 && c.in_len() >= 5),
        Proto::Cloud2 => ctx.find(vid, pid, |c| c.page() == 0xFF90 && c.usage() == 0x0303 && c.out_len() >= 8 && c.in_len() >= 8),
        Proto::Stinger { .. } => ctx.find(vid, pid, |c| c.page() == 0xFF00 && c.usage() == 0x0001 && c.feat_len() >= 32),
    }
}

pub fn poll(ctx: &mut PollContext) {
    for (vid, pid, name, proto) in MODELS {
        if ctx.is_claimed(vid, pid) {
            continue;
        }
        let colls = collections(ctx, vid, pid, proto);
        let found = colls.iter().find_map(|c| match proto {
            Proto::Flight => flight(c),
            Proto::Stinger { ps } => stinger(c, ps),
            Proto::Alpha => alpha(c),
            Proto::Cloud2 => cloud2(c),
        });
        let Some(r) = found else { continue };
        let base = DeviceStatus {
            key: format!("hyperx:{}", hex4(pid)),
            name: name.into(),
            kind: DeviceKind::Headset,
            via: "2.4 GHz dongle".into(),
            hw: Some((vid, pid)),
            grace_sec: 60,
            ..Default::default()
        };
        let s = if r.on {
            DeviceStatus { percent: r.percent.unwrap_or(-1), millivolts: r.mv, charging: r.charging, full: r.full, ..base }
        } else {
            DeviceStatus { connected: false, ..base }
        };
        ctx.add(s, vid, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(head: &[u8], len: usize) -> Vec<u8> {
        let mut r = vec![0u8; len];
        r[..head.len()].copy_from_slice(head);
        r
    }

    #[test]
    fn flight_frames() {
        assert_eq!(&flight_request()[..4], &[0x21, 0xFF, 0x05, 0x00]);
        let r = flight_parse(&pad(&[0x21, 0xFF, 0x05, 0x0E, 0xD8], 20)).unwrap(); // 3800 mV
        assert!(r.on && !r.charging && r.mv == 3800);
        let p = r.percent.unwrap();
        assert!((1..100).contains(&p));
        let c = flight_parse(&pad(&[0x21, 0xFF, 0x05, 0x10, 0x20], 20)).unwrap(); // 4128 mV: charging
        assert!(c.charging && c.percent.is_none());
        assert!(!flight_parse(&pad(&[0x21, 0xFF, 0x05, 0x00, 0x00], 20)).unwrap().on);
        assert_eq!(flight_parse(&pad(&[0x64, 0x01], 20)), None); // power push
        assert_eq!(flight_parse(&pad(&[0x21, 0xFF, 0x06, 0x0E, 0xD8], 20)), None);
    }

    #[test]
    fn flight_curve() {
        assert_eq!(flight_percent(4000), 100);
        assert_eq!(flight_percent(3600), 4);
        assert!(flight_percent(3900) > flight_percent(3750));
    }

    #[test]
    fn stinger_frames() {
        let mut r = pad(&[0xFF, 0x12, 0x05], 64);
        r[9] = 0x03;
        r[11..13].copy_from_slice(&3905u16.to_le_bytes());
        assert_eq!(stinger_battery(&r), Some(Reading { on: true, percent: Some(70), mv: 3905, charging: false, full: false }));
        r[9] = 0x06;
        assert_eq!(stinger_battery(&r).map(|b| (b.percent, b.full)), Some((Some(100), true)));
        r[9] = 0x05;
        r[11..13].copy_from_slice(&4000u16.to_le_bytes());
        assert_eq!(stinger_battery(&r).map(|b| (b.percent, b.charging)), Some((Some(70), true)));
        r[9] = 0x01;
        assert_eq!(stinger_battery(&r), None);
    }

    #[test]
    fn cloud2_frames() {
        assert_eq!(&cloud2_request(CLOUD2_LEVEL)[..5], &[0x06, 0xFF, 0xBB, 0x02, 0x00]);
        let r = pad(&[0x06, 0xFF, 0xBB, 0x02, 0x00, 0x0E, 0xD8, 0x4B], 20);
        assert!(cloud2_parse(&r, CLOUD2_LEVEL).is_some());
        assert!(cloud2_parse(&r, CLOUD2_CHARGING).is_none());
        assert!(cloud2_parse(&pad(&[0x21, 0xFF, 0x05], 20), CLOUD2_LEVEL).is_none());
    }

    #[test]
    fn alpha_frames() {
        assert_eq!(alpha_request(ALPHA_LEVEL), [0x21, 0xBB, 0x0B]);
        assert_eq!(alpha_parse(&pad(&[0x21, 0xBB, 0x0B, 77], 31), ALPHA_LEVEL), Some(77));
        assert_eq!(alpha_parse(&pad(&[0x21, 0xBB, 0x03, 0x01], 31), ALPHA_LINK), Some(0x01));
        assert_eq!(alpha_parse(&pad(&[0x21, 0xBB, 0x0C, 0x01], 31), ALPHA_LEVEL), None); // another command
        assert_eq!(alpha_parse(&pad(&[0x22, 0xBB, 0x0B, 77], 31), ALPHA_LEVEL), None);
        assert_eq!(alpha_parse(&[0x21, 0xBB, 0x0B], ALPHA_LEVEL), None);
    }
}
