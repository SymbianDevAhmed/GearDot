//! ATK N9 Pro (VID 373B: dongle 11CB, cable 11CA) and first-generation N9 Ultra (dongle 1204, cable
//! 1203) headsets. They are not read over HID but over the USB virtual COM port the dongle also
//! exposes (9600 baud, 8N1, no flow control), with 64-byte frames:
//!
//!   request  02 CC SS 00 ...            reply  LL CC SS D0 D1 ...   (data = bytes 3..63)
//!   00 04    link status: D1 == 1 when the headset is connected to the dongle
//!   00 03    battery:     D1 bits 0..6 = level %, bit 7 = charging
//!
//! Facts as documented by myfinder-device-battery (github.com/wushuai0802/myfinder-device-battery,
//! engine/upstream-protocols.md, read off ATK's own configuration software "ATK HUB", model
//! "n9-pro") and as used by the original C++ BatteryHub (GPL-3.0, src/drivers/atk_serial.cpp);
//! implemented here from those facts, no code taken. Only these two reads are sent. The headset also
//! pushes unsolicited frames, which are skipped. While ATK's own app holds the port, the headset is
//! not read.
use windows::Win32::Devices::Communication::{
    COMMTIMEOUTS, DCB, GetCommState, NOPARITY, ONESTOPBIT, PURGE_RXCLEAR, PURGE_TXCLEAR, PurgeComm, SetCommState, SetCommTimeouts,
};
use windows::Win32::Devices::Properties::DEVPKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, OPEN_EXISTING, ReadFile, WriteFile};
use windows::core::HSTRING;

use super::sdi::DevSet;
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x373B;
const FRAME: usize = 64;

struct Model {
    pid: u16,
    name: &'static str,
    cable: bool,
}

const MODELS: [Model; 4] = [
    Model { pid: 0x11CB, name: "ATK N9 Pro", cable: false },
    Model { pid: 0x11CA, name: "ATK N9 Pro", cable: true },
    Model { pid: 0x1204, name: "ATK N9 Ultra", cable: false },
    Model { pid: 0x1203, name: "ATK N9 Ultra", cable: true },
];

pub const CMD_BATTERY: (u8, u8) = (0x00, 0x03);
pub const CMD_STATUS: (u8, u8) = (0x00, 0x04);

pub fn request((cmd, sub): (u8, u8)) -> [u8; FRAME] {
    let mut r = [0u8; FRAME];
    r[..3].copy_from_slice(&[0x02, cmd, sub]);
    r
}

/// The data bytes of the reply to `cmd` found in `buf` (whole 64-byte frames; others skipped).
pub fn find_reply(buf: &[u8], (cmd, sub): (u8, u8)) -> Option<&[u8]> {
    buf.as_chunks::<FRAME>().0.iter().find(|f| f[1] == cmd && f[2] == sub).map(|f| &f[3..])
}

/// (level %, charging) from battery data.
pub fn battery(data: &[u8]) -> Option<(i32, bool)> {
    let b = *data.get(1)?;
    let level = (b & 0x7F) as i32;
    (level <= 100).then_some((level, b & 0x80 != 0))
}

/// "USB Serial Device (COM7)" -> "COM7".
pub fn port_from_name(name: &str) -> Option<String> {
    let start = name.rfind("(COM")? + 1;
    let end = start + name[start..].find(')')?;
    let port = &name[start..end];
    port[3..].chars().all(|c| c.is_ascii_digit()).then(|| port.to_string())
}

/// COM ports of the given product, from the USB device nodes' friendly names.
fn ports(pid: u16) -> Vec<String> {
    com_ports(VID, pid)
}

/// COM ports of any USB product (`vid`, `pid`), from the device nodes' friendly names.
pub(crate) fn com_ports(vid: u16, pid: u16) -> Vec<String> {
    let Some(set) = DevSet::present(Some("USB")) else { return Vec::new() };
    let id = format!("VID_{}&PID_{}", hex4(vid), hex4(pid));
    set.nodes()
        .iter()
        .filter(|n| n.instance_id.to_ascii_uppercase().contains(&id))
        .filter_map(|n| n.string(&DEVPKEY_Device_FriendlyName).and_then(|f| port_from_name(&f)))
        .collect()
}

pub(crate) struct Serial(HANDLE);

impl Serial {
    /// Opens `port` exclusively at 9600 8N1, DTR and RTS on, no flow control, input purged.
    pub(crate) fn open(port: &str) -> Option<Self> {
        let path = HSTRING::from(format!("\\\\.\\{port}"));
        // Exclusive open: fails while ATK's app (or anything else) uses the port.
        let h = unsafe { CreateFileW(&path, GENERIC_READ.0 | GENERIC_WRITE.0, FILE_SHARE_MODE(0), None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }
            .ok()
            .filter(|h| !h.is_invalid())?;
        let s = Serial(h);
        let mut dcb = DCB { DCBlength: size_of::<DCB>() as u32, ..Default::default() };
        unsafe { GetCommState(h, &mut dcb) }.ok()?;
        dcb.BaudRate = 9600;
        dcb.ByteSize = 8;
        dcb.Parity = NOPARITY;
        dcb.StopBits = ONESTOPBIT;
        // fBinary, DTR and RTS on, no flow control or XON/XOFF.
        dcb._bitfield = 0x0001 | (1 << 4) | (1 << 12);
        let to = COMMTIMEOUTS { ReadIntervalTimeout: 50, ReadTotalTimeoutConstant: 300, WriteTotalTimeoutConstant: 300, ..Default::default() };
        unsafe {
            SetCommState(h, &dcb).ok()?;
            SetCommTimeouts(h, &to).ok()?;
            let _ = PurgeComm(h, PURGE_RXCLEAR | PURGE_TXCLEAR);
        }
        Some(s)
    }

    pub(crate) fn write_all(&self, data: &[u8]) -> bool {
        let mut n = 0u32;
        unsafe { WriteFile(self.0, Some(data), Some(&mut n), None) }.is_ok() && n as usize == data.len()
    }

    /// Whatever arrived (waits up to the read timeouts set in `open`).
    pub(crate) fn read_some(&self) -> Option<Vec<u8>> {
        let mut chunk = [0u8; 128];
        let mut got = 0u32;
        unsafe { ReadFile(self.0, Some(&mut chunk), Some(&mut got), None) }.ok()?;
        Some(chunk[..got as usize].to_vec())
    }

    fn command(&self, cmd: (u8, u8)) -> Option<Vec<u8>> {
        let mut n = 0u32;
        unsafe { WriteFile(self.0, Some(&request(cmd)), Some(&mut n), None) }.ok()?;
        if n as usize != FRAME {
            return None;
        }
        let mut buf = Vec::new();
        let end = Deadline::in_ms(800);
        while !end.passed() {
            let mut chunk = [0u8; 128];
            let mut got = 0u32;
            unsafe { ReadFile(self.0, Some(&mut chunk), Some(&mut got), None) }.ok()?;
            buf.extend_from_slice(&chunk[..got as usize]);
            if let Some(d) = find_reply(&buf, cmd) {
                return Some(d.to_vec());
            }
        }
        None
    }
}

impl Drop for Serial {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn poll(ctx: &mut PollContext) {
    let present = ctx.products(VID);
    for m in &MODELS {
        // The dongle's HID interfaces show it is plugged in; only then are serial ports looked up.
        if ctx.is_claimed(VID, m.pid) || !present.contains(&(VID, m.pid)) {
            continue;
        }
        for port in ports(m.pid) {
            let Some(s) = Serial::open(&port) else { continue };
            let Some(status) = s.command(CMD_STATUS) else { continue };
            let base = DeviceStatus {
                key: format!("atk-serial:{}", hex4(m.pid)),
                name: m.name.into(),
                kind: DeviceKind::Headset,
                via: if m.cable { "USB cable".into() } else { "2.4 GHz receiver".into() },
                hw: Some((VID, m.pid)),
                grace_sec: 60,
                ..Default::default()
            };
            let st = if status.get(1) != Some(&1) {
                DeviceStatus { connected: false, ..base }
            } else {
                match s.command(CMD_BATTERY).as_deref().and_then(battery) {
                    Some((level, charging)) => DeviceStatus { percent: level, charging, ..base },
                    None => DeviceStatus { percent: -1, ..base },
                }
            };
            ctx.add(st, VID, m.pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(cmd: u8, sub: u8, d1: u8) -> Vec<u8> {
        let mut f = vec![0u8; FRAME];
        f[..5].copy_from_slice(&[0x03, cmd, sub, 0x00, d1]);
        f
    }

    #[test]
    fn request_frames() {
        let r = request(CMD_BATTERY);
        assert_eq!(&r[..3], &[0x02, 0x00, 0x03]);
        assert!(r[3..].iter().all(|&b| b == 0));
        assert_eq!(&request(CMD_STATUS)[..3], &[0x02, 0x00, 0x04]);
    }

    #[test]
    fn replies() {
        let mut buf = frame(0x01, 0x07, 0x55); // an unsolicited frame first
        buf.extend(frame(0x00, 0x03, 0x80 | 64));
        let d = find_reply(&buf, CMD_BATTERY).unwrap();
        assert_eq!(battery(d), Some((64, true)));
        assert_eq!(battery(find_reply(&frame(0x00, 0x03, 90), CMD_BATTERY).unwrap()), Some((90, false)));
        assert_eq!(find_reply(&frame(0x00, 0x04, 1), CMD_BATTERY), None);
        assert_eq!(find_reply(&frame(0x00, 0x03, 50)[..40], CMD_BATTERY), None); // incomplete frame
        assert_eq!(battery(&[0x00, 0x7F]), None); // 127 %
    }

    #[test]
    fn port_names() {
        assert_eq!(port_from_name("USB Serial Device (COM7)").as_deref(), Some("COM7"));
        assert_eq!(port_from_name("Silicon Labs CP210x (COM12)").as_deref(), Some("COM12"));
        assert_eq!(port_from_name("USB Serial Device"), None);
        assert_eq!(port_from_name("Thing (COMX)"), None);
    }
}
