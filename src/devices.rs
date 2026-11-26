//! The device factory keeps protocol-specific construction out of the UI.
use crate::battery::{BatteryError, BatteryReading, BatterySource, DeviceKind};
use crate::{mouse::MadRReceiver, receiver::G435Receiver};

pub fn adapter(kind: DeviceKind) -> Box<dyn BatterySource> {
    match kind {
        DeviceKind::Headset => Box::new(G435Receiver::new()),
        DeviceKind::Mouse => Box::new(MadRReceiver),
    }
}

pub fn read(kind: DeviceKind) -> Result<BatteryReading, BatteryError> {
    adapter(kind).read_battery()
}
