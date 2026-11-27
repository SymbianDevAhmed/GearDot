//! JBL Quantum 800 on the 0ECB:203E 2.4 GHz dongle.
//!
//! JBL QuantumENGINE's Q800 HIDV1 implementation uses feature report 0x68 for
//! the wireless link and 0x49 for battery percentage. Both were read on hardware
//! via interface 5, collection FF00:0001. The value is byte 1 in each report.
//! The Q800 implementation does not query charging status.

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};

const VID: u16 = 0x0ECB;
const PID: u16 = 0x203E;
const LINK_REPORT: u8 = 0x68;
const BATTERY_REPORT: u8 = 0x49;

fn read_value(dev: &HidDev, report_id: u8) -> Option<u8> {
    let report = dev.get_feature(report_id)?;
    match report.as_slice() {
        [id, value, ..] if *id == report_id => Some(*value),
        _ => None,
    }
}

fn read(coll: &HidColl) -> Option<DeviceStatus> {
    let dev = HidDev::open_feature_only(coll)?;
    let (connected, percent) = match read_value(&dev, LINK_REPORT)? {
        0 => (false, -1),
        1 => (true, i32::from(read_value(&dev, BATTERY_REPORT).filter(|&level| level <= 100)?)),
        _ => return None,
    };

    Some(DeviceStatus {
        key: "jbl:203e".into(),
        name: "JBL Quantum 800".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz dongle".into(),
        hw: Some((VID, PID)),
        grace_sec: 60,
        connected,
        percent,
        ..Default::default()
    })
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.iface == 5 && c.page() == 0xFF00 && c.usage() == 1 && c.feat_len() == 64);
    if let Some(status) = colls.iter().find_map(read) {
        ctx.add(status, VID, PID);
    }
}
