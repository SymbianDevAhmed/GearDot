//! AJAZZ AK820 MAX, AULA F87 Pro V2C and other keyboards on the SONiX 2.4 GHz receiver 0C45:FEFC.
//!
//! Protocol as documented by HaloBattery (MIT, github.com/HeyOkay/HaloBattery, providers/aula.py:
//! recovered from AULA HUB and confirmed on an F87 Pro V2C on this receiver); implemented here from
//! those facts. Vendor collection FF80:0001, 33-byte reports (report id 0 + 32 bytes):
//!
//!   request  AA 10 00 ...
//!   reply    55 10 .. ... LL ...     LL (byte 25 of the 32) = level 1..100
//!
//! Only this read is sent; no charging state. A keyboard that is off or asleep does not answer. The AK820
//! MAX of issue #160 has the same collection on the same receiver; not tested on it.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{self, HidColl, HidDev};

const VID: u16 = 0x0C45;
const PID: u16 = 0xFEFC;
/// Report id 0 first.
const REQUEST: [u8; 3] = [0x00, 0xAA, 0x10];
const LEN: u16 = 33;
const REPLY_MS: u64 = 750;
const LEVEL_AT: usize = 25;

/// The level in a reply (report id 0 already stripped by the reader).
pub fn parse(r: &[u8]) -> Option<i32> {
    (r.len() > LEVEL_AT && r[..2] == [0x55, 0x10] && (1..=100).contains(&r[LEVEL_AT])).then(|| r[LEVEL_AT] as i32)
}

fn read(c: &HidColl) -> Option<i32> {
    let d = HidDev::open(c)?;
    d.ask(&REQUEST, REPLY_MS, parse).or_else(|| d.ask(&REQUEST, REPLY_MS, parse))
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF80 && c.usage() == 0x0001 && c.in_len() == LEN && c.out_len() == LEN);
    // Not listed while silent: a keyboard that answered before is shown asleep by the app.
    let Some((c, percent)) = colls.iter().find_map(|c| read(c).map(|l| (c, l))) else { return };
    let name = hid::product_string(c);
    let name = name.trim();
    let generic = name.is_empty() || ["2.4g", "dongle", "receiver"].iter().any(|w| name.to_ascii_lowercase().contains(w));
    let s = DeviceStatus {
        key: "aula-hub:fefc".into(),
        name: if generic { "Wireless keyboard".into() } else { name.to_string() },
        kind: DeviceKind::Keyboard,
        via: "2.4 GHz receiver".into(),
        percent,
        hw: Some((VID, PID)),
        grace_sec: 1800,
        ..Default::default()
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        let mut r = vec![0u8; 32];
        r[..3].copy_from_slice(&[0x55, 0x10, 0x18]);
        r[LEVEL_AT] = 96;
        assert_eq!(parse(&r), Some(96));
        r[2] = 0x00; // incremental frame
        assert_eq!(parse(&r), Some(96));
        r[LEVEL_AT] = 0;
        assert_eq!(parse(&r), None);
        r[LEVEL_AT] = 50;
        r[1] = 0x11;
        assert_eq!(parse(&r), None);
        assert_eq!(parse(&[0x55, 0x10]), None);
    }
}
