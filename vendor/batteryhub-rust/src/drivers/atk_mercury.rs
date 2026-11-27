//! ATK Mercury I / M1 headset over its 2.4 GHz receiver (VID 373B, PID 107C).
//!
//! Protocol as documented by Joule (AGPL-3.0, github.com/DillanCidral/Joule, pull request #79:
//! worked out from captures and the firmware, validated on hardware for receiver 0005 with headset
//! firmware 0007) and as used by TaskbarVitals (GPL-3.0, github.com/symNistaken/TaskbarVitals,
//! src/drivers/atk_mercury.cpp); implemented here from those facts. The receiver's vendor collection
//! FF22:0001 (interface 0, 64-byte feature report AA) is a mailbox: each request is a SetFeature,
//! the answer is read with GetFeature about 300 ms later. Three reads, each checked before the next:
//!
//!   AA 01                     -> AA 01 00 00 00 07 ...      headset firmware 0007 (the validated one)
//!   AA 03 00 01 04 01 00 FE 94 -> AA 03 04 5C FE 00 01 ...  4 bytes at 0x0100FE94 (big-endian address):
//!                                                          the power structure pointer, must be 0x0100FE5C
//!   AA 03 00 01 04 01 00 FE 78 -> AA 03 04 PP 00 00 00 ...  4 bytes at 0x0100FE78: battery percent (LE)
//!
//! Only these reads are sent. Any other firmware answer or pointer stops the read, so a different
//! firmware never gets the memory reads; the receiver is then shown as disconnected. No charging
//! state is known.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::sleep_ms;

const VID: u16 = 0x373B;
const PID: u16 = 0x107C;
const REPORT: u8 = 0xAA;
const LEN: usize = 64;
const FIRMWARE: [u8; 6] = [0xAA, 0x01, 0x00, 0x00, 0x00, 0x07];
const POWER_POINTER_AT: u32 = 0x0100_FE94;
const POWER_POINTER: u32 = 0x0100_FE5C;
const CAPACITY_AT: u32 = 0x0100_FE78;

pub fn version_request() -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[..2].copy_from_slice(&[REPORT, 0x01]);
    r
}

pub fn memory_request(address: u32) -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[..5].copy_from_slice(&[REPORT, 0x03, 0x00, 0x01, 0x04]);
    r[5..9].copy_from_slice(&address.to_be_bytes());
    r
}

pub fn known_firmware(r: &[u8]) -> bool {
    r.len() == LEN && r[..FIRMWARE.len()] == FIRMWARE
}

/// Memory answer: the 4-byte value (little endian).
pub fn memory_value(r: &[u8]) -> Option<u32> {
    (r.len() == LEN && r[..3] == [REPORT, 0x03, 0x04]).then(|| u32::from_le_bytes([r[3], r[4], r[5], r[6]]))
}

fn ask(d: &HidDev, request: &[u8; LEN]) -> Option<Vec<u8>> {
    if !d.set_feature(request) {
        return None;
    }
    sleep_ms(300);
    d.get_feature(REPORT).filter(|r| r.len() == LEN)
}

fn read(c: &HidColl) -> Option<i32> {
    let d = HidDev::open_feature_only(c).or_else(|| HidDev::open(c))?;
    if !known_firmware(&ask(&d, &version_request())?) {
        return None;
    }
    if memory_value(&ask(&d, &memory_request(POWER_POINTER_AT))?)? != POWER_POINTER {
        return None;
    }
    let percent = memory_value(&ask(&d, &memory_request(CAPACITY_AT))?)?;
    (percent <= 100).then_some(percent as i32)
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.iface == 0 && c.page() == 0xFF22 && c.usage() == 0x0001 && c.feat_len() as usize == LEN);
    if colls.is_empty() {
        return;
    }
    let base = DeviceStatus {
        key: "atk_mercury:107c".into(),
        name: "ATK Mercury I / M1".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 600,
        ..Default::default()
    };
    let s = match colls.iter().find_map(read) {
        Some(percent) => DeviceStatus { percent, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(head: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; LEN];
        f[..head.len()].copy_from_slice(head);
        f
    }

    #[test]
    fn requests() {
        assert_eq!(version_request()[..3], [0xAA, 0x01, 0x00]);
        assert_eq!(memory_request(POWER_POINTER_AT)[..9], [0xAA, 0x03, 0x00, 0x01, 0x04, 0x01, 0x00, 0xFE, 0x94]);
        assert_eq!(memory_request(CAPACITY_AT)[5..9], [0x01, 0x00, 0xFE, 0x78]);
    }

    #[test]
    fn firmware_check() {
        assert!(known_firmware(&frame(&[0xAA, 0x01, 0x00, 0x00, 0x00, 0x07, 0x12])));
        assert!(!known_firmware(&frame(&[0xAA, 0x01, 0x00, 0x00, 0x00, 0x08])));
        assert!(!known_firmware(&frame(&[0xAA, 0x01, 0x00, 0x00, 0x01, 0x03, 0xFF, 0x25])));
        assert!(!known_firmware(&version_request())); // our own request read back
    }

    #[test]
    fn memory_answers() {
        assert_eq!(memory_value(&frame(&[0xAA, 0x03, 0x04, 0x5C, 0xFE, 0x00, 0x01])), Some(POWER_POINTER));
        assert_eq!(memory_value(&frame(&[0xAA, 0x03, 0x04, 72, 0, 0, 0])), Some(72));
        assert_eq!(memory_value(&frame(&[0xAA, 0x03, 0x02, 72])), None);
        assert_eq!(memory_value(&memory_request(CAPACITY_AT)), None); // the request has 00 at byte 2
        assert_eq!(memory_value(&[0xAA, 0x03, 0x04, 72]), None); // short
    }
}
