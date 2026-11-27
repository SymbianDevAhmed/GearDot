//! GameSir G7 Pro 8K PC (Aimlabs), in XInput mode. The Xbox battery report can
//! say 100% while the vendor HID stream carries the actual percentage.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x3537;
const CABLE: u16 = 0x10C5;
const RECEIVER: u16 = 0x10C6;

pub fn supports(vid: u16, pid: u16) -> bool {
    vid == VID && matches!(pid, CABLE | RECEIVER)
}

fn battery(report: &[u8]) -> Option<(i32, bool)> {
    if report.len() != 64 || report[0] != 0x12 || report[36] > 100 {
        return None;
    }
    // An unpaired receiver can stream the report ID followed entirely by zeros.
    if report[1..].iter().all(|&b| b == 0) {
        return None;
    }
    Some((i32::from(report[36]), report[35] & 1 != 0))
}

fn query(c: &HidColl) -> Option<(i32, bool)> {
    let device = HidDev::open(c)?;
    device.flush();
    if !device.write(&[0x0F, 0xF2]) {
        return None;
    }
    let end = Deadline::in_ms(600);
    while !end.passed() {
        let report = device.read(end.left_ms().max(1))?;
        if let Some(level) = battery(&report) {
            return Some(level);
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    for (pid, via) in [(CABLE, "USB cable"), (RECEIVER, "2.4 GHz receiver")] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.page() == 0xFFF0 && c.usage() == 0x0040 && c.in_len() >= 64 && c.out_len() >= 64);
        if colls.is_empty() {
            continue;
        }
        let mut status = DeviceStatus {
            key: format!("gamesir:{pid:04X}"),
            name: "GameSir G7 Pro 8K".into(),
            kind: DeviceKind::Gamepad,
            via: via.into(),
            hw: Some((VID, pid)),
            ..Default::default()
        };
        if let Some((percent, charging)) = colls.iter().find_map(query) {
            status.percent = percent;
            status.charging = charging;
        } else {
            status.connected = false;
        }
        ctx.add(status, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_charge_from_a_populated_status_report() {
        // Captured from the 10C6 receiver on battery power.
        let mut report = [
            0x12, 0x80, 0x80, 0x80, 0x80, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x6f, 0x4a, 0x00, 0x03, 0x00, 0x06, 0x00, 0x06, 0x00, 0x41, 0x00, 0xb6, 0x20, 0xc4,
            0xfe, 0x00, 0x5a, 0x36, 0x66, 0x6a, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4d, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x84, 0x83, 0x81, 0x7f, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(battery(&report), Some((77, false)));
        report[35] = 1;
        for percent in [0, 79, 100] {
            report[36] = percent;
            assert_eq!(battery(&report), Some((i32::from(percent), true)));
        }
    }

    #[test]
    fn rejects_empty_receiver_and_invalid_reports() {
        let mut report = [0; 64];
        report[0] = 0x12;
        assert_eq!(battery(&report), None);
        report[36] = 101;
        assert_eq!(battery(&report), None);
        report[36] = 78;
        assert_eq!(battery(&report[..36]), None);
        report[0] = 0x10;
        assert_eq!(battery(&report), None);
    }
}
