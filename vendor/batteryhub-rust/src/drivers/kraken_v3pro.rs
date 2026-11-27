//! Razer Kraken V3 Pro headset over its HyperSpeed dongle (VID 1532, PID 052C).
//!
//! Not the Razer HID protocol: the dongle's HID interface has media keys only; its settings channel is
//! the USB CDC serial port (interface 4, "USB Serial Device (COMn)"). Protocol as documented by
//! razer-peripherals-protocol (github.com/immortalbob/razer-peripherals-protocol,
//! razer-kraken-v3-pro.md: from USB captures of Synapse and observed on the headset on Windows with the
//! inbox usbser driver) and OmaKraken (MIT, github.com/Dankestrick/OmaKraken); implemented here from
//! those facts. HCI-style frames at 9600 8N1:
//!
//!   request  01 F3 FF 18 02 20 24 00 ... 00   28 bytes: vendor command FFF3, read (20), command 24
//!   reply    04 0E 1C 01 F3 FF 00 02 20 24 V0 V1 V2 V3 ...   31 bytes
//!            V0 bit 2 = charging, V3 = level % (in 10 % steps)
//!   events   04 FF 01 E8 / E9   headset link up / down
//!
//! A headset that is off gives no reply at all. Only this read is sent. The port can have one owner
//! only, so nothing is opened while Razer Synapse runs (it would lose the headset otherwise).
use super::atk_serial::{Serial, com_ports};
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::util::Deadline;

const VID: u16 = 0x1532;
const PID: u16 = 0x052C;
const CMD_STATUS: u8 = 0x24;
const REPLY_MS: u64 = 700;

pub fn request() -> [u8; 28] {
    let mut f = [0u8; 28];
    f[..7].copy_from_slice(&[0x01, 0xF3, 0xFF, 0x18, 0x02, 0x20, CMD_STATUS]);
    f
}

/// (level, charging) from the reply to the status read found in the byte stream `buf`.
pub fn find_status(buf: &[u8]) -> Option<(i32, bool)> {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] != 0x04 {
            i += 1;
            continue;
        }
        let &len = buf.get(i + 2)?;
        let end = i + 3 + len as usize;
        if end > buf.len() {
            return None;
        }
        let p = &buf[i..end];
        if p.len() >= 14 && p[1..6] == [0x0E, 0x1C, 0x01, 0xF3, 0xFF] && p[7] == 0x02 && p[8] == 0x20 && p[9] == CMD_STATUS && p[13] <= 100 {
            return Some((p[13] as i32, p[10] & 0x04 != 0));
        }
        i = end;
    }
    None
}

fn synapse_running() -> bool {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    let mut found = false;
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return true };
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snap, &mut e).is_ok();
        while more && !found {
            found = crate::util::from_wide(&e.szExeFile).to_lowercase().starts_with("razer synapse");
            more = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
    }
    found
}

pub fn poll(ctx: &mut PollContext) {
    // The dongle's HID interface shows it is plugged in; only then are serial ports looked up.
    if ctx.is_claimed(VID, PID) || !ctx.products(VID).contains(&(VID, PID)) || synapse_running() {
        return;
    }
    let base = DeviceStatus {
        key: "razer:kraken-v3-pro".into(),
        name: "Razer Kraken V3 Pro".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 300,
        ..Default::default()
    };
    for port in com_ports(VID, PID) {
        let Some(s) = Serial::open(&port) else { continue };
        if !s.write_all(&request()) {
            continue;
        }
        let mut buf = Vec::new();
        let end = Deadline::in_ms(REPLY_MS);
        let mut found = None;
        while !end.passed() && buf.len() < 4096 {
            let Some(chunk) = s.read_some() else { break };
            buf.extend_from_slice(&chunk);
            if let Some(v) = find_status(&buf) {
                found = Some(v);
                break;
            }
        }
        let st = match found {
            Some((level, charging)) => DeviceStatus { percent: level, charging, ..base },
            None => DeviceStatus { connected: false, ..base }, // no reply: the headset is off
        };
        ctx.add(st, VID, PID);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame() {
        let f = request();
        assert_eq!(f[..7], [0x01, 0xF3, 0xFF, 0x18, 0x02, 0x20, 0x24]);
        assert!(f[7..].iter().all(|&b| b == 0));
    }

    #[test]
    fn replies() {
        // The documented capture: on battery, 30 %.
        let mut r = vec![0x04, 0x0E, 0x1C, 0x01, 0xF3, 0xFF, 0x00, 0x02, 0x20, 0x24, 0x01, 0x0E, 0xB6, 0x1E];
        r.resize(31, 0);
        assert_eq!(find_status(&r), Some((30, false)));
        // A link event first, the reply split across reads.
        let mut s = vec![0x04, 0xFF, 0x01, 0xE8];
        s.extend_from_slice(&r);
        s[4 + 10] = 0x05;
        assert_eq!(find_status(&s), Some((30, true)));
        assert_eq!(find_status(&s[..20]), None);
        // Another command's reply is not it.
        let mut o = r.clone();
        o[9] = 0x25;
        assert_eq!(find_status(&o), None);
    }
}
