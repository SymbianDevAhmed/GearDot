//! Logitech G321 LIGHTSPEED headset (VID 046D, PID 0B39) over its 2.4 GHz receiver.
//!
//! Contributed by Bacurovisk (github.com/Bacurovisk) in pull request #107, tested on hardware there
//! (on battery, charging, headset off); integrated with the request restricted to the collection the
//! protocol source uses.
//!
//! This receiver does not expose the HID++ collections the Logitech driver talks to (FF00:0001 /
//! FF00:0002, FF43:0202). Its interface 3 carries 000C:0001 (consumer controls), FF00:0212 and
//! FF0B:0101. The battery is read on the vendor collection FF00:0212 (MI_03 & COL02; numbered
//! reports, 67-byte feature / 61-byte input), which relays the request to the headset:
//!
//!   request   feature report  24 2B 00 ..                  (zero-padded to the report length)
//!   reply     input report    27 2B 00 0B A1..A6 V1..V4 PP ..
//!             A1..A6  peer (headset) address
//!             V1..V4  version
//!             PP      battery percent, 0..100
//!
//! No reply within the timeout means the headset is switched off or out of range (the receiver
//! stays enumerated). The reply carries no charging flag: on the cable it is identical apart from
//! the percent, so only the level is shown. Nothing is sent to the other vendor collection (FF0B).
//!
//! Protocol from G321 Studio (github.com/gleyssonsc/g321-studio, LightspeedConnection.cs, MIT).
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x046D;
const G321: u16 = 0x0B39;

/// Receivers that speak this protocol.
const PIDS: &[u16] = &[G321];

/// The receiver interface and vendor collection that carry the request.
const IFACE: i32 = 3;
const PAGE: u16 = 0xFF00;
const USAGE: u16 = 0x0212;

const REQUEST: [u8; 3] = [0x24, 0x2B, 0x00];
const REPLY_HEADER: [u8; 4] = [0x27, 0x2B, 0x00, 0x0B];
const PERCENT_AT: usize = 14;
const TIMEOUT_MS: u64 = 800;

/// Battery percent from a reply, or None if `r` is not a battery reply. `r` is the input report as
/// read (r[0] == report ID 0x27).
pub fn battery_percent(r: &[u8]) -> Option<i32> {
    if r.len() <= PERCENT_AT || r[..REPLY_HEADER.len()] != REPLY_HEADER || r[PERCENT_AT] > 100 {
        return None;
    }
    Some(r[PERCENT_AT] as i32)
}

fn status(pid: u16) -> DeviceStatus {
    DeviceStatus {
        key: format!("logi-g321:{}", hex4(pid)),
        name: "Logitech G321 LIGHTSPEED".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz receiver".into(),
        grace_sec: 60,
        vid: VID,
        hw: Some((VID, pid)),
        ..Default::default()
    }
}

/// Decodes a battery reply into a device status.
pub fn decode(pid: u16, r: &[u8]) -> Option<DeviceStatus> {
    let percent = battery_percent(r)?;
    Some(DeviceStatus { percent, ..status(pid) })
}

/// The collection the request is sent on (FF00:0212 on interface 3).
fn is_battery_collection(c: &HidColl) -> bool {
    c.iface == IFACE && c.page() == PAGE && c.usage() == USAGE && c.feat_len() as usize >= REQUEST.len() && c.in_len() as usize > PERCENT_AT
}

fn query(d: &HidDev) -> Option<Vec<u8>> {
    d.flush();
    if !d.set_feature(&REQUEST) {
        return None;
    }
    // The receiver answers on interrupt IN; other reports may arrive first.
    let end = Deadline::in_ms(TIMEOUT_MS);
    loop {
        let left = end.left_ms();
        if left == 0 {
            return None;
        }
        let r = d.read(left)?;
        if battery_percent(&r).is_some() {
            return Some(r);
        }
    }
}

pub fn poll(ctx: &mut PollContext) {
    for (vid, pid) in ctx.products(VID) {
        if !PIDS.contains(&pid) {
            continue;
        }
        let colls = ctx.find(vid, pid, is_battery_collection);
        if colls.is_empty() {
            continue;
        }
        let reply = colls.iter().filter_map(HidDev::open).find_map(|d| query(&d));
        // No reply: receiver present, headset off or out of range.
        let s = reply.and_then(|r| decode(pid, &r)).unwrap_or(DeviceStatus { connected: false, ..status(pid) });
        ctx.add(s, vid, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reply recorded from a G321 LIGHTSPEED (046D:0B39) on battery at 77%, headset address
    /// zeroed; the rest of the 61-byte report is zero (PR #107).
    const RECORDED: [u8; 15] = [0x27, 0x2B, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x4D];

    /// The recorded reply with another percent.
    fn reply(percent: u8) -> Vec<u8> {
        let mut r = RECORDED.to_vec();
        r.resize(61, 0);
        r[PERCENT_AT] = percent;
        r
    }

    #[test]
    fn reads_the_recorded_reply() {
        let mut r = RECORDED.to_vec();
        r.resize(61, 0);
        let s = decode(G321, &r).unwrap();
        assert_eq!(s.percent, 77);
        assert_eq!(s.status_text(), "77%");
    }

    #[test]
    fn reads_the_percent() {
        let s = decode(G321, &reply(79)).unwrap();
        assert_eq!(s.percent, 79);
        assert_eq!(s.kind, DeviceKind::Headset);
        assert_eq!(s.hw, Some((VID, G321)));
        assert!(s.connected && !s.charging);
        assert_eq!(decode(G321, &reply(0)).unwrap().percent, 0);
        assert_eq!(decode(G321, &reply(100)).unwrap().percent, 100);
    }

    #[test]
    fn ignores_other_reports() {
        assert_eq!(battery_percent(&reply(101)), None);
        let mut other = reply(50);
        other[1] = 0x2C;
        assert_eq!(battery_percent(&other), None);
        let mut other = reply(50);
        other[0] = 0x24;
        assert_eq!(battery_percent(&other), None);
        assert_eq!(battery_percent(&reply(50)[..PERCENT_AT]), None);
        assert_eq!(battery_percent(&[]), None);
    }

    #[test]
    fn request_is_report_24() {
        assert_eq!(REQUEST, [0x24, 0x2B, 0x00]);
    }

    #[test]
    fn only_the_ff00_0212_collection() {
        let coll =
            |iface: i32, page: u16, usage: u16| HidColl { iface, usage_page: page, usage_id: usage, input_len: 61, feature_len: 67, ..Default::default() };
        assert!(is_battery_collection(&coll(3, 0xFF00, 0x0212)));
        assert!(!is_battery_collection(&coll(3, 0xFF0B, 0x0101))); // the other vendor channel
        assert!(!is_battery_collection(&coll(3, 0x000C, 0x0001)));
        assert!(!is_battery_collection(&coll(0, 0xFF00, 0x0212)));
    }
}
