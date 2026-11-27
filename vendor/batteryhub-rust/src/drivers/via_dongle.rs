//! Keyboards on the shared "Wireless 2.4G Dongle" 36B0:3002 (e.g. Cidoo QK61 V2, PMO / WOBKEY
//! Wave 75, URX Core75), and PMO Hub keyboards on their USB cable.
//!
//! Protocol as used by the original C++ BatteryHub (GPL-3.0, src/drivers/via_dongle.cpp), from
//! pmohub.cn's web driver (as reverse engineered in the original's issue 343, tested on a PMO
//! Wave 75 RGB) and HaloBattery (MIT, github.com/HeyOkay/HaloBattery, providers/urx.py, confirmed on
//! a URX Core75 on this dongle); implemented here from those facts. The dongle passes the keyboard's
//! raw HID channel through on interface 1, FF60:0061, 33-byte reports (65 on a PMO cable). Two
//! firmware families answer on it, both read-only:
//!
//! PMO Hub: `AA <cmd> 00 00 <len>`, replies echo `AA <cmd> 00 00` with 55 (OK) at byte 7.
//!
//! ```text
//! AA 10 .. <len>   start a session (len = payload - 8); the dongle's reply names the paired
//!                  keyboard: .. 55 00 B0 36 <pid lo> <pid hi>
//! AA 30 .. 18      battery: .. 55 <percent> (no charging state)
//! AA 11 .. <len>   end the session (always sent after a start)
//! ```
//!
//! VIA: custom get `08 00 07` (channel 0, value 7) -> `08 00 07 <level>`; FF = not handled.
//!
//! Which firmware is behind the dongle is not visible from its descriptors, so both are tried (VIA
//! first when the product string names a URX board), and from then on only the one that answered.
//! A dongle that answers neither is asked again with a growing back-off. A PMO keyboard on its
//! cable leaves the dongle unanswered, so the dongle is skipped then.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{self, HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x36B0;
const DONGLE: u16 = 0x3002;
const REPLY_MS: u64 = 600;

/// pmohub.cn's keyboards (protocol 1): their cable product id, also reported as paired by the dongle.
const PMO_BOARDS: [(u16, &str); 4] = [(0x3009, "WAVE 75"), (0x3004, "Aurora 80"), (0x302B, "PMO P75"), (0x3012, "PMO 80")];

fn pmo_name(pid: u16) -> Option<&'static str> {
    PMO_BOARDS.iter().find(|b| b.0 == pid).map(|b| b.1)
}

fn is_raw_hid(c: &HidColl) -> bool {
    c.iface == 1 && c.page() == 0xFF60 && c.usage() == 0x0061 && c.in_len() == c.out_len() && (c.out_len() == 33 || c.out_len() == 65)
}

/// One output report (report ID 0 first) with `bytes` at the start of the payload.
fn frame(out_len: usize, bytes: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; out_len];
    f[1..=bytes.len()].copy_from_slice(bytes);
    f
}

/// Sends a request and waits for the reply that starts with `echo` (report ID stripped).
fn ask(d: &HidDev, req: &[u8], echo: &[u8], ms: u64) -> Option<Vec<u8>> {
    d.flush();
    if !d.write(req) {
        return None;
    }
    let end = Deadline::in_ms(ms);
    while !end.passed() {
        if let Some(r) = d.read(end.left_ms())
            && r.len() >= echo.len() + 6
            && r.starts_with(echo)
        {
            return Some(r);
        }
    }
    None
}

// ---- PMO Hub ----------------------------------------------------------------------------------

pub fn pmo_ok(r: &[u8]) -> bool {
    r.len() > 8 && r[7] == 0x55
}

/// The keyboard behind the dongle, from the reply to START (`55 00 B0 36 <pid>`).
pub fn pmo_paired(start: &[u8]) -> Option<u16> {
    (start.len() > 12 && u16::from_le_bytes([start[9], start[10]]) == VID).then(|| u16::from_le_bytes([start[11], start[12]]))
}

/// The level from the reply to GET_BATTERY.
pub fn pmo_level(r: &[u8]) -> Option<i32> {
    (pmo_ok(r) && r[8] <= 100).then_some(r[8] as i32)
}

/// Some((level, paired keyboard pid)) when the PMO firmware answered.
fn pmo_query(d: &HidDev) -> Option<(i32, Option<u16>)> {
    let out = d.coll().out_len() as usize;
    let len = (out - 1 - 8) as u8; // 0x18 on the dongle, 0x38 on the cable
    let start = frame(out, &[0xAA, 0x10, 0, 0, len]);
    let stop = frame(out, &[0xAA, 0x11, 0, 0, len]);
    let echo = |cmd: u8| [0xAA, cmd, 0, 0];
    // The first request can be missed while the link wakes up.
    let reply = ask(d, &start, &echo(0x10), REPLY_MS).or_else(|| ask(d, &start, &echo(0x10), REPLY_MS));
    let Some(reply) = reply.filter(|r| pmo_ok(r)) else {
        // End a session the keyboard may have taken; don't wait when nothing answers at all.
        d.write(&stop);
        return None;
    };
    let level = ask(d, &frame(out, &[0xAA, 0x30, 0, 0, 0x18]), &echo(0x30), REPLY_MS).and_then(|r| pmo_level(&r));
    ask(d, &stop, &echo(0x11), REPLY_MS); // always end the session
    level.map(|l| (l, pmo_paired(&reply)))
}

// ---- VIA --------------------------------------------------------------------------------------

/// The level from the reply to VIA custom get 08 00 07 (FF or 0 = not handled).
pub fn via_level(r: &[u8]) -> Option<i32> {
    (r.len() >= 4 && r[..3] == [0x08, 0x00, 0x07] && (1..=100).contains(&r[3])).then_some(r[3] as i32)
}

fn via_query(d: &HidDev) -> Option<i32> {
    let req = frame(d.coll().out_len() as usize, &[0x08, 0x00, 0x07]);
    for _ in 0..2 {
        if let Some(r) = ask(d, &req, &[0x08, 0x00, 0x07], REPLY_MS) {
            return via_level(&r);
        }
    }
    None
}

/// "EVO80 2.4G" -> "EVO80", "URX Core75 Wireless" -> "URX Core75"; the generic dongle name -> None.
pub fn via_name(product: &str) -> Option<String> {
    if product.contains("Dongle") {
        return None;
    }
    let mut name = product.trim();
    for suffix in [" 2.4G", " Wireless"] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    (!name.is_empty()).then(|| name.to_string())
}

// ---- state across polls -----------------------------------------------------------------------

#[derive(Clone, Copy, Default, PartialEq, Debug)]
enum Proto {
    #[default]
    Unknown,
    Pmo,
    Via,
}

#[derive(Default)]
struct State {
    proto: Proto,
    misses: u32,
    next_try: Option<Instant>,
    paired: Option<u16>,
}

static STATE: Mutex<Option<HashMap<u16, State>>> = Mutex::new(None);

fn with_state<T>(pid: u16, f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new).entry(pid).or_default())
}

fn forget(pid: u16) {
    if let Some(m) = STATE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&pid);
    }
}

/// Seconds until the next try after `misses` polls without an answer.
pub fn backoff_secs(known: bool, misses: u32) -> u64 {
    if known { 20 } else { (30u64 << (misses.saturating_sub(1)).min(4)).min(300) }
}

fn missed(st: &mut State) {
    st.misses += 1;
    // A known protocol that stops answering is most likely a sleeping keyboard (or one moved to its
    // cable): retry soon. After several misses, forget it in case another keyboard was paired.
    if st.misses >= 6 {
        st.proto = Proto::Unknown;
    }
    st.next_try = Some(Instant::now() + Duration::from_secs(backoff_secs(st.proto != Proto::Unknown, st.misses)));
}

fn answered(st: &mut State, p: Proto) {
    st.proto = p;
    st.misses = 0;
    st.next_try = None;
}

fn due(st: &State) -> bool {
    st.next_try.is_none_or(|t| Instant::now() >= t)
}

fn status(key_pid: u16, name: String, via: &str, level: i32, hw: (u16, u16)) -> DeviceStatus {
    DeviceStatus {
        key: format!("via:{VID:04x}:{key_pid:04x}"), // dongle and cable share the keyboard's pid
        name,
        kind: DeviceKind::Keyboard,
        via: via.into(),
        percent: level,
        hw: Some(hw),
        grace_sec: 1800, // keep the last value while a sleeping keyboard does not answer
        ..Default::default()
    }
}

/// A PMO Hub keyboard on its USB cable. Returns true when one answered.
fn poll_cable(ctx: &mut PollContext, pid: u16) -> bool {
    if ctx.is_claimed(VID, pid) {
        return false;
    }
    let colls = ctx.find(VID, pid, is_raw_hid);
    if colls.is_empty() {
        forget(pid); // unplugged: start fresh when it comes back
        return false;
    }
    if !with_state(pid, |st| due(st)) {
        return false;
    }
    // A level of 0 on the cable is no battery at all: other wired keyboards share these ids (e.g.
    // the TMKB T68E on 36B0:3009, issue #434) and answer 0; a PMO keyboard charges on its cable.
    let found = colls.iter().find_map(|c| HidDev::open(c).and_then(|d| pmo_query(&d))).filter(|(level, _)| *level > 0);
    let Some((level, _)) = found else {
        with_state(pid, missed);
        return false;
    };
    with_state(pid, |st| answered(st, Proto::Pmo));
    ctx.add(status(pid, pmo_name(pid).unwrap_or("Wireless keyboard").into(), "USB cable", level, (VID, pid)), VID, pid);
    true
}

fn poll_dongle(ctx: &mut PollContext, cable_answered: bool) {
    if ctx.is_claimed(VID, DONGLE) {
        return;
    }
    let colls = ctx.find(VID, DONGLE, |c| is_raw_hid(c) && c.out_len() == 33);
    let Some(c) = colls.first() else {
        forget(DONGLE);
        return;
    };
    let (proto, is_due) = with_state(DONGLE, |st| (st.proto, due(st)));
    if cable_answered && proto != Proto::Via {
        ctx.claim(VID, DONGLE); // the PMO keyboard is on its cable: the dongle does not answer
        return;
    }
    if !is_due {
        return;
    }
    let Some(d) = HidDev::open(c) else { return };
    let product = hid::product_string(c);
    let mut order = [Proto::Pmo, Proto::Via];
    if proto == Proto::Via || (proto == Proto::Unknown && product.contains("URX")) {
        order.swap(0, 1);
    }
    let tries = if proto == Proto::Unknown { 2 } else { 1 };
    for p in order.into_iter().take(tries) {
        let s = match p {
            Proto::Pmo => pmo_query(&d).map(|(level, paired)| {
                let key_pid = with_state(DONGLE, |st| {
                    if paired.is_some() {
                        st.paired = paired;
                    }
                    st.paired.unwrap_or(DONGLE)
                });
                status(key_pid, pmo_name(key_pid).unwrap_or("Wireless keyboard").into(), "2.4 GHz receiver", level, (VID, DONGLE))
            }),
            _ => via_query(&d)
                .map(|level| status(DONGLE, via_name(&product).unwrap_or_else(|| "Wireless keyboard".into()), "2.4 GHz receiver", level, (VID, DONGLE))),
        };
        if let Some(s) = s {
            with_state(DONGLE, |st| answered(st, p));
            ctx.add(s, VID, DONGLE);
            return;
        }
    }
    with_state(DONGLE, missed);
}

pub fn poll(ctx: &mut PollContext) {
    let mut cable = false;
    for (pid, _) in PMO_BOARDS {
        cable |= poll_cable(ctx, pid);
    }
    poll_dongle(ctx, cable);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmo_replies() {
        let mut start = vec![0u8; 32];
        start[..4].copy_from_slice(&[0xAA, 0x10, 0, 0]);
        start[7..13].copy_from_slice(&[0x55, 0x00, 0xB0, 0x36, 0x09, 0x30]);
        assert!(pmo_ok(&start));
        assert_eq!(pmo_paired(&start), Some(0x3009));
        let mut bat = vec![0u8; 32];
        bat[..4].copy_from_slice(&[0xAA, 0x30, 0, 0]);
        bat[7] = 0x55;
        bat[8] = 78;
        assert_eq!(pmo_level(&bat), Some(78));
        bat[7] = 0x0F; // error: no session
        assert_eq!(pmo_level(&bat), None);
        assert_eq!(frame(33, &[0xAA, 0x10, 0, 0, 0x18])[..6], [0, 0xAA, 0x10, 0, 0, 0x18]);
    }

    #[test]
    fn via_replies_and_names() {
        let mut r = vec![0u8; 32];
        r[..4].copy_from_slice(&[0x08, 0x00, 0x07, 64]);
        assert_eq!(via_level(&r), Some(64));
        r[3] = 0xFF; // value not handled by this firmware
        assert_eq!(via_level(&r), None);
        assert_eq!(via_name("EVO80 2.4G").as_deref(), Some("EVO80"));
        assert_eq!(via_name("URX Core75 Wireless").as_deref(), Some("URX Core75"));
        assert_eq!(via_name("Wireless 2.4G Dongle"), None);
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_secs(true, 3), 20);
        assert_eq!(backoff_secs(false, 1), 30);
        assert_eq!(backoff_secs(false, 2), 60);
        assert_eq!(backoff_secs(false, 9), 300);
    }
}
