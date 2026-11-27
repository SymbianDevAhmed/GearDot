//! Mice on the Incott 8K receiver (VID 093A, PID 522C), e.g. the Incott mice and the Lunacy Another
//! One.
//!
//! Nothing is sent: while it is used, the mouse reports by itself with input report 09 on the vendor
//! collection FF05:0001, `09 <battery> ..`; battery <= 100 is the level %, above 100 it is charging
//! and the level is battery - 128. Format from OpenMouse (AGPL-3.0, src/incott/index.ts, checked
//! there over a full charge cycle: 09 5F = 95 %, 09 E1 = charging 97 %) as also used by Joule (MIT,
//! github.com/DillanCidral/Joule, IncottProvider.cs); implemented here from those facts. The query
//! the official app sends returns a frozen value, so it is not used.
//!
//! The receiver is listened to for a short while on each poll; a mouse that is not used sends
//! nothing, and the last level stays shown (dimmed after the grace time).
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x093A;
const PID: u16 = 0x522C;
const REPORT: u8 = 0x09;
const LISTEN_MS: u64 = 1500;

/// (level, charging) from one input report, if it is the status report.
pub fn decode(r: &[u8]) -> Option<(i32, bool)> {
    if r.len() < 3 || r[0] != REPORT {
        return None;
    }
    match r[1] {
        b @ 1..=100 => Some((b as i32, false)),
        b @ 129..=228 => Some((b as i32 - 128, true)),
        _ => None,
    }
}

fn listen(c: &HidColl) -> Option<(i32, bool)> {
    let d = HidDev::open(c)?;
    let end = Deadline::in_ms(LISTEN_MS);
    let mut last = None;
    while !end.passed() {
        match d.read(end.left_ms().min(300)) {
            Some(r) => {
                if let Some(v) = decode(&r) {
                    last = Some(v);
                }
            }
            None if last.is_some() => break,
            None => {}
        }
    }
    last
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF05 && c.usage() == 0x0001 && c.in_len() >= 3);
    let Some((level, charging)) = colls.iter().find_map(listen) else { return };
    ctx.add(
        DeviceStatus {
            key: "incott:522c".into(),
            name: "Incott 8K mouse".into(),
            kind: DeviceKind::Mouse,
            via: "2.4 GHz receiver".into(),
            hw: Some((VID, PID)),
            grace_sec: 1800,
            percent: level,
            charging,
            ..Default::default()
        },
        VID,
        PID,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports() {
        assert_eq!(decode(&[0x09, 0x5F, 0x02]), Some((95, false)));
        assert_eq!(decode(&[0x09, 0xE1, 0x02]), Some((97, true)));
        assert_eq!(decode(&[0x09, 0x00, 0x02]), None);
        assert_eq!(decode(&[0x08, 0x5F, 0x02]), None);
        assert_eq!(decode(&[0x09, 0xF0, 0x02]), None);
    }
}
