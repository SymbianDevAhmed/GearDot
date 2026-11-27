//! Razer Barracuda X (2022) headset on its 2.4 GHz dongle: 1532:0550 (YS-Tech bridge) or 1532:0552
//! (Macronix bridge). Both forward Airoha commands (50 41 ... out, 50 49 ... back) to the headset and
//! differ only in the HID framing around them.
//!
//! Protocol as documented by barracuda-x-battery (MIT, github.com/nightdevx/barracuda-x-battery,
//! barracuda.py; the GETs were checked on a 1532:0550 receiver), which took the YS-Tech framing from
//! razer-barracuda-x-battery (MIT, github.com/mehdibouchami/razer-barracuda-x-battery,
//! src/Protocol.cs); implemented here from those facts. Vendor collection FF00, 64-byte reports:
//!
//!   YS-Tech   02 80 LL CL CH <command> ...   CRC-16/XMODEM over the 64 bytes with CL CH zero; the
//!             bridge acknowledges through GetInputReport 02 ('O' taken, 'F' busy); replies 02 LL <stream>
//!   Macronix  01 40 handshake first, then 01 80 LL <command> ...; replies 01 80 LL <stream>
//!
//!   remote mode  50 41 0E SQ 02 E1 01 (on) / 00 (off)   routes commands to the headset; no setting changes
//!   GET          50 41 08 SQ 03 CC 00 00                 CC = 21 battery percent, 2A cable (01 plugged)
//!   reply        50 49 08 .. .. .. .. .. NL NH CC 01 VL VV ...    VV = value
//!
//! Only remote mode and these two reads are sent. No reply to remote mode means the headset is off,
//! out of range or in Bluetooth mode.
use std::sync::atomic::{AtomicU8, Ordering};

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x1532;
const DONGLES: [u16; 2] = [0x0550, 0x0552];
const LEN: usize = 64;
const YS_TECH: u8 = 0x02;
const MACRONIX: u8 = 0x01;
const BATTERY: u8 = 0x21;
const CABLE: u8 = 0x2A;

static SEQ: AtomicU8 = AtomicU8::new(0);

fn next_seq() -> u8 {
    let s = SEQ.load(Ordering::Relaxed) % 127 + 1;
    SEQ.store(s, Ordering::Relaxed);
    s
}

pub fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// The output report carrying one command for the given bridge.
pub fn frame(rid: u8, cmd: &[u8]) -> [u8; LEN] {
    let mut f = [0u8; LEN];
    if rid == MACRONIX {
        f[..3].copy_from_slice(&[MACRONIX, 0x80, cmd.len() as u8]);
        f[3..3 + cmd.len()].copy_from_slice(cmd);
    } else {
        f[..3].copy_from_slice(&[YS_TECH, 0x80, cmd.len() as u8]);
        f[5..5 + cmd.len()].copy_from_slice(cmd);
        let crc = crc16_xmodem(&f);
        f[3] = crc as u8;
        f[4] = (crc >> 8) as u8;
    }
    f
}

/// The stream bytes an input report carries; empty for anything else (media keys, acks).
pub fn chunk(r: &[u8], rid: u8) -> &[u8] {
    let (start, n) = match rid {
        YS_TECH if r.len() >= 2 && r[0] == YS_TECH => (2, r[1] as usize),
        MACRONIX if r.len() >= 3 && r[0] == MACRONIX && r[1] == 0x80 => (3, r[2] as usize),
        _ => return &[],
    };
    &r[start..(start + n).min(r.len())]
}

/// Reassembles 50 49 frames (10-byte header, payload length in bytes 8-9) from report chunks.
#[derive(Default)]
pub struct Frames(Vec<u8>);

impl Frames {
    pub fn feed(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.0.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            let Some(start) = self.0.windows(2).position(|w| w == [0x50, 0x49]) else {
                let keep = self.0.last() == Some(&0x50);
                self.0.clear();
                if keep {
                    self.0.push(0x50);
                }
                return out;
            };
            self.0.drain(..start);
            if self.0.len() < 10 {
                return out;
            }
            let end = 10 + (self.0[8] as usize | (self.0[9] as usize) << 8);
            if self.0.len() < end {
                return out;
            }
            out.push(self.0.drain(..end).collect());
        }
    }
}

/// Value of a GET reply frame for `cmd`.
pub fn get_value(f: &[u8], cmd: u8) -> Option<u8> {
    (f.len() >= 14 && f[2] == 0x08 && f[10] == cmd && f[11] == 0x01 && f[12] >= 1).then(|| f[13])
}

struct Dongle {
    d: HidDev,
    rid: u8,
    frames: Frames,
}

impl Dongle {
    fn send_command(&self, cmd: &[u8]) -> bool {
        let f = frame(self.rid, cmd);
        if self.rid == MACRONIX {
            return self.d.write(&f);
        }
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if !self.d.write(&f) {
                return false;
            }
            match self.d.get_input_report(YS_TECH).and_then(|s| s.get(1).copied()) {
                Some(b'O') => return true,
                Some(b'F') => continue,
                _ => return false,
            }
        }
        false
    }

    /// Sends 50 41 group seq body... and returns the first reply frame `matches` accepts.
    fn send(&mut self, group: u8, body: &[u8], ms: u64, matches: impl Fn(&[u8], u8) -> bool) -> Option<Vec<u8>> {
        let seq = next_seq();
        let mut cmd = vec![0x50, 0x41, group, seq];
        cmd.extend_from_slice(body);
        if !self.send_command(&cmd) {
            return None;
        }
        let end = Deadline::in_ms(ms);
        while !end.passed() {
            let Some(r) = self.d.read(end.left_ms()) else { continue };
            let found = self.frames.feed(chunk(&r, self.rid)).into_iter().find(|f| f.len() >= 12 && matches(f, seq));
            if found.is_some() {
                return found;
            }
        }
        None
    }

    fn remote(&mut self, on: bool) -> bool {
        self.send(0x0E, &[0x02, 0xE1, on as u8], 800, |f, seq| f[10] == 0x0E && f[11] == 0x80 | seq)
            .is_some_and(|f| f[12] == 0)
    }

    fn get(&mut self, cmd: u8) -> Option<u8> {
        self.send(0x08, &[0x03, cmd, 0x00, 0x00], 1500, |f, _| get_value(f, cmd).is_some()).and_then(|f| get_value(&f, cmd))
    }
}

fn read(c: &HidColl, pid: u16) -> Option<(i32, bool)> {
    let d = HidDev::open(c)?;
    d.flush();
    let rid = if pid == 0x0552 { MACRONIX } else { YS_TECH };
    let mut g = Dongle { d, rid, frames: Frames::default() };
    if rid == MACRONIX && g.d.write(&[MACRONIX, 0x40]) {
        let end = Deadline::in_ms(500);
        while !end.passed() {
            if g.d.read(end.left_ms()).is_some_and(|r| r.len() >= 2 && r[0] == MACRONIX && r[1] == 0x40) {
                break;
            }
        }
    }
    if !g.remote(true) {
        return None;
    }
    let level = g.get(BATTERY).filter(|&l| l <= 100);
    let cable = g.get(CABLE) == Some(1);
    g.remote(false);
    level.map(|l| (l as i32, cable))
}

pub fn poll(ctx: &mut PollContext) {
    for pid in DONGLES {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.page() == 0xFF00 && c.out_len() as usize == LEN && c.in_len() as usize == LEN);
        let Some(first) = colls.first() else { continue };
        let base = DeviceStatus {
            key: format!("barracudax2022:{}", hex4(pid)),
            name: "Razer Barracuda X".into(),
            kind: DeviceKind::Headset,
            via: "2.4 GHz dongle".into(),
            hw: Some((VID, pid)),
            grace_sec: 600,
            ..Default::default()
        };
        let s = match read(first, pid) {
            Some((level, charging)) => DeviceStatus { percent: level, charging, ..base },
            None => DeviceStatus { connected: false, ..base },
        };
        ctx.add(s, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    }

    #[test]
    fn frames_per_bridge() {
        let cmd = [0x50, 0x41, 0x08, 0x01, 0x03, 0x21, 0x00, 0x00];
        let m = frame(MACRONIX, &cmd);
        assert_eq!(m[..11], [0x01, 0x80, 0x08, 0x50, 0x41, 0x08, 0x01, 0x03, 0x21, 0x00, 0x00]);
        let y = frame(YS_TECH, &cmd);
        assert_eq!(y[..3], [0x02, 0x80, 0x08]);
        assert_eq!(y[5..13], cmd);
        let mut z = y;
        z[3] = 0;
        z[4] = 0;
        assert_eq!(crc16_xmodem(&z), y[3] as u16 | (y[4] as u16) << 8);
    }

    #[test]
    fn battery_reply_split_across_reports() {
        // 50 49 08 fa b8 ce 17 01 04 00 21 01 01 26 = 38 %
        let full = [0x50, 0x49, 0x08, 0xFA, 0xB8, 0xCE, 0x17, 0x01, 0x04, 0x00, 0x21, 0x01, 0x01, 0x26];
        let mut fr = Frames::default();
        let mut r1 = vec![MACRONIX, 0x80, 6];
        r1.extend_from_slice(&full[..6]);
        assert!(fr.feed(chunk(&r1, MACRONIX)).is_empty());
        let mut r2 = vec![MACRONIX, 0x80, 8];
        r2.extend_from_slice(&full[6..]);
        let got = fr.feed(chunk(&r2, MACRONIX));
        assert_eq!(got.len(), 1);
        assert_eq!(get_value(&got[0], BATTERY), Some(38));
        assert_eq!(get_value(&got[0], CABLE), None);
        let ys = [&[YS_TECH, full.len() as u8][..], &full].concat();
        assert_eq!(Frames::default().feed(chunk(&ys, YS_TECH)).len(), 1);
    }
}
