//! Razer BlackShark V3 X headset over its HyperSpeed dongle (VID 1532, PID 057D) or USB cable (057C).
//!
//! Protocol from OpenRazer PR #2899, as implemented and verified on a 057D dongle by TaskbarVitals
//! (GPL-3.0, github.com/symNistaken/TaskbarVitals, src/drivers/razer_headset.cpp); implemented here
//! from those facts. 64-byte feature report 07 on interface 0:
//!
//!   request  07 00 TX 00 00 00 02 07 CC 00 ... [62] = XOR of bytes 3..60   TX = transaction id
//!   reply    07 ST TX .. .. .. AC 07 CC .. VV                             ST 02 ok, 01 busy
//!   CC 80: VV = level %, CC 84: VV = 1 charging
//!
//! Like the other BlackShark headsets it is asked at most every 5 minutes; the last reading is shown
//! in between. Only these two reads are sent.
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::HidDev;
use crate::util::{hex4, sleep_ms};

const VID: u16 = 0x1532;
const PIDS: [u16; 2] = [0x057D, 0x057C];
const REPORT: u8 = 0x07;
const CMD_LEVEL: u8 = 0x80;
const CMD_CHARGING: u8 = 0x84;
const MIN_INTERVAL: Duration = Duration::from_secs(300);

static TXID: AtomicU8 = AtomicU8::new(0x40);
static CACHE: Mutex<Option<HashMap<u16, (Instant, DeviceStatus)>>> = Mutex::new(None);

pub fn request(txid: u8, cmd: u8) -> [u8; 64] {
    let mut r = [0u8; 64];
    r[0] = REPORT;
    r[2] = txid;
    r[6] = 0x02;
    r[7] = 0x07;
    r[8] = cmd;
    r[62] = r[3..=60].iter().fold(0u8, |a, b| a ^ b);
    r
}

/// Some(Some(value)) = answer, Some(None) = failure (e.g. headset off), None = not this request's reply.
pub fn reply(r: &[u8], txid: u8, cmd: u8) -> Option<Option<u8>> {
    if r.len() < 63 || r[0] != REPORT || r[2] != txid || r[7] != 0x07 || r[8] != cmd || r[6] < 2 {
        return None;
    }
    Some((r[1] == 0x02 || r[1] == 0x01).then_some(r[10]))
}

fn get(d: &HidDev, cmd: u8) -> Option<u8> {
    for _ in 0..4 {
        let txid = TXID.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if !d.set_feature(&request(txid, cmd)) {
            return None;
        }
        sleep_ms(60);
        let Some(r) = d.get_feature(REPORT) else { continue };
        match reply(&r, txid, cmd) {
            Some(v) => return v,
            None => continue,
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    for pid in PIDS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.iface == 0 && c.feat_len() >= 64);
        let Some(c) = colls.first() else { continue };
        let cached = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&pid).cloned());
        if let Some((at, s)) = cached.filter(|(at, _)| at.elapsed() < MIN_INTERVAL) {
            let _ = at;
            ctx.add(s, VID, pid);
            continue;
        }
        let Some(d) = HidDev::open_feature_only(c) else { continue };
        let Some(level) = get(&d, CMD_LEVEL).filter(|&l| l <= 100) else { continue };
        let charging = get(&d, CMD_CHARGING) == Some(1);
        let s = DeviceStatus {
            key: format!("blackshark:{}", hex4(pid)),
            name: "Razer BlackShark V3 X".into(),
            kind: DeviceKind::Headset,
            via: if pid == 0x057D { "2.4 GHz receiver" } else { "USB cable" }.into(),
            hw: Some((VID, pid)),
            grace_sec: 600,
            percent: level as i32,
            charging,
            ..Default::default()
        };
        CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(pid, (Instant::now(), s.clone()));
        ctx.add(s, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames() {
        let r = request(0x41, CMD_LEVEL);
        assert_eq!(r[..9], [0x07, 0x00, 0x41, 0x00, 0x00, 0x00, 0x02, 0x07, 0x80]);
        assert_eq!(r[62], 0x02 ^ 0x07 ^ 0x80);
    }

    #[test]
    fn replies() {
        let mut r = vec![0u8; 64];
        r[..11].copy_from_slice(&[0x07, 0x02, 0x41, 0, 0, 0, 0x02, 0x07, 0x80, 0x00, 72]);
        assert_eq!(reply(&r, 0x41, CMD_LEVEL), Some(Some(72)));
        assert_eq!(reply(&r, 0x42, CMD_LEVEL), None);
        r[1] = 0x03;
        assert_eq!(reply(&r, 0x41, CMD_LEVEL), Some(None));
    }
}
