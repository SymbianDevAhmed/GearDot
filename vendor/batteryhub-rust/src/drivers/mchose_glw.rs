//! MCHOSE keyboards on the common 2.4 GHz receiver 3837:3033 (product string "MCHOSE <model> 2.4G":
//! G75 V2, G87 V2, K99 V3, and the K87S, which speaks another protocol and is not read).
//!
//! Protocol as documented by the original C++ BatteryHub (GPL-3.0, src/drivers/mchose_keyboard.cpp),
//! read off MCHOSE's own M HUB web driver ("Channel:Keyboard" GLW class: _simpleGetCommand /
//! getFuncConfig / parseBaseFuncConfigStruct) and confirmed there on a real G75 V2 receiver;
//! implemented here from those facts. "GLW" frames on the id-less 64-byte collection 0001:0000 (the
//! FF31:0074 collection answers the same frames):
//!
//!   request  55 CC 00 SS LL OO OO 00 ...   (64 bytes, no report id)
//!            SS = checksum = (LL + offset + payload) & FF; reads have no payload
//!   reply    AA CC 00 SS LL OO OO 00 | payload from byte 8
//!   05       "func config" read, offset 0, 56 bytes: payload[32] bits 0..6 = level %, bit 7 = charging
//!
//! Only this read is sent. The keyboard decides on its own when to sleep, so the receiver is asked at
//! most every ASK_EVERY and the last reading is kept in between. A receiver that never answered
//! (a K87S, or a dead link) is not shown.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{self, HidColl, HidDev};
use crate::util::{Deadline, sleep_ms};

const VID: u16 = 0x3837;
const PID: u16 = 0x3033;
const FLAG_REQUEST: u8 = 0x55;
const FLAG_REPLY: u8 = 0xAA;
pub const CMD_FUNC_CONFIG: u8 = 0x05;
const PAYLOAD: usize = 8;
const BATTERY: usize = 32;
const READ_SIZE: u8 = 56;
const ASK_EVERY: Duration = Duration::from_secs(10 * 60);

/// Read request (report id 0 first): `cmd`, `size` bytes at offset 0.
pub fn request(cmd: u8, size: u8) -> [u8; 9] {
    // checksum = size + offset (0) with no payload
    [0x00, FLAG_REQUEST, cmd, 0x00, size, size, 0x00, 0x00, 0x00]
}

/// (level %, charging) from a func-config reply (report id 0 stripped by the reader).
pub fn parse_func_config(r: &[u8]) -> Option<(i32, bool)> {
    if r.len() < PAYLOAD + BATTERY + 1 || r[0] != FLAG_REPLY || r[1] != CMD_FUNC_CONFIG || r[5] != 0 || r[6] != 0 {
        return None;
    }
    let size = r[4] as usize;
    if size < BATTERY + 1 || PAYLOAD + size > r.len() {
        return None;
    }
    let sum = r[4..8].iter().chain(&r[PAYLOAD..PAYLOAD + size]).fold(0u8, |a, &b| a.wrapping_add(b));
    if sum != r[3] {
        return None;
    }
    let b = r[PAYLOAD + BATTERY];
    let level = (b & 0x7F) as i32;
    (level <= 100).then_some((level, b & 0x80 != 0))
}

/// "MCHOSE G75 V2 2.4G" -> "MCHOSE G75 V2".
pub fn display_name(product: &str) -> String {
    let n = product.trim().trim_end_matches(" 2.4G").trim();
    if n.is_empty() { "MCHOSE keyboard".into() } else { n.to_string() }
}

fn query(c: &HidColl) -> Option<(i32, bool)> {
    let d = HidDev::open(c)?;
    for attempt in 0..2 {
        if attempt > 0 {
            sleep_ms(150);
        }
        d.flush();
        if !d.write(&request(CMD_FUNC_CONFIG, READ_SIZE)) {
            continue;
        }
        let end = Deadline::in_ms(500);
        while !end.passed() {
            let Some(r) = d.read(end.left_ms()) else { break };
            if let Some(v) = parse_func_config(&r) {
                return Some(v);
            }
        }
    }
    None
}

struct State {
    asked: Option<Instant>,
    last: Option<(i32, bool)>,
}

static STATE: Mutex<State> = Mutex::new(State { asked: None, last: None });

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| {
        c.in_len() == 65 && c.out_len() == 65 && ((c.page() == 0x0001 && c.usage() == 0x0000) || (c.page() == 0xFF31 && c.usage() == 0x0074))
    });
    let Some(first) = colls.first() else { return };
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let ask = st.asked.is_none_or(|t| t.elapsed() >= ASK_EVERY);
    let mut fresh = None;
    if ask {
        st.asked = Some(Instant::now());
        fresh = colls.iter().find_map(query);
        if fresh.is_some() {
            st.last = fresh;
        }
    }
    let Some((level, charging)) = st.last else { return }; // never answered
    drop(st);
    let base = DeviceStatus {
        key: "mchose:3837:3033".into(),
        name: display_name(&hid::product_string(first)),
        kind: DeviceKind::Keyboard,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 1800,
        ..Default::default()
    };
    // Asked now and no answer: keyboard off, asleep or busy (the app keeps the last level dimmed).
    let s = if ask && fresh.is_none() { DeviceStatus { connected: false, ..base } } else { DeviceStatus { percent: level, charging, ..base } };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(battery: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..8].copy_from_slice(&[FLAG_REPLY, CMD_FUNC_CONFIG, 0x00, 0x00, READ_SIZE, 0x00, 0x00, 0x00]);
        r[PAYLOAD + 3] = 0x12; // some other func-config byte
        r[PAYLOAD + BATTERY] = battery;
        let sum = r[4..8].iter().chain(&r[PAYLOAD..PAYLOAD + READ_SIZE as usize]).fold(0u8, |a, &b| a.wrapping_add(b));
        r[3] = sum;
        r
    }

    #[test]
    fn request_frame() {
        assert_eq!(request(CMD_FUNC_CONFIG, 56), [0x00, 0x55, 0x05, 0x00, 56, 56, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn replies() {
        assert_eq!(parse_func_config(&reply(85)), Some((85, false))); // the test G75 V2 on battery
        assert_eq!(parse_func_config(&reply(0x80 | 60)), Some((60, true)));
        let mut bad = reply(85);
        bad[3] ^= 1;
        assert_eq!(parse_func_config(&bad), None); // checksum
        let mut other = reply(85);
        other[1] = 0x01;
        assert_eq!(parse_func_config(&other), None);
        let mut req = reply(85);
        req[0] = FLAG_REQUEST; // our own request echoed
        assert_eq!(parse_func_config(&req), None);
        assert_eq!(parse_func_config(&reply(0x7F)), None); // 127 %
    }

    #[test]
    fn names() {
        assert_eq!(display_name("MCHOSE G75 V2 2.4G"), "MCHOSE G75 V2");
        assert_eq!(display_name("MCHOSE K99 V3 2.4G"), "MCHOSE K99 V3");
        assert_eq!(display_name(""), "MCHOSE keyboard");
    }
}
