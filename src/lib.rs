pub mod battery;
pub mod protocol;

#[cfg(windows)]
pub mod receiver;

pub mod mouse;
pub mod palette;

pub mod preferences;

#[cfg(windows)]
pub mod devices;

pub mod diagnostics;

#[cfg(any(windows, test))]
mod usb_inventory;

#[cfg(any(windows, test))]
mod device_catalog;
#[cfg(any(windows, test))]
mod dualsense;
