//! Read Windows' cached device properties. No connection or HID requests.
use std::{collections::BTreeMap, mem::size_of, ptr::{null, null_mut}};
use windows_sys::{
    core::GUID,
    Win32::{
        Devices::{DeviceAndDriverInstallation::*, Properties::*},
    },
};

const BATTERY_LEVEL: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

use windows::{
    core::HSTRING,
    Devices::{
        Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice},
        Enumeration::{DeviceInformation, DeviceInformationKind},
    },
    Foundation::Collections::IIterable,
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};
use wireless_battery_monitor::preferences::BluetoothReading;

struct WinRt;

impl Drop for WinRt {
    fn drop(&mut self) {
        unsafe { RoUninitialize(); }
    }
}

fn connected_names() -> windows::core::Result<BTreeMap<String, String>> {
    unsafe { RoInitialize(RO_INIT_MULTITHREADED)?; }
    let _win_rt = WinRt;
    let connected = BluetoothConnectionStatus::Connected;
    let mut names = BTreeMap::new();
    for selector in [
        BluetoothDevice::GetDeviceSelectorFromConnectionStatus(connected)?,
        BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(connected)?,
    ] {
        let items = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
            &selector, None::<&IIterable<HSTRING>>, DeviceInformationKind::AssociationEndpoint,
        )?.get()?;
        for index in 0..items.Size()? {
            let name = items.GetAt(index)?.Name()?.to_string_lossy();
            if !name.trim().is_empty() {
                names.entry(name.trim().to_lowercase()).or_insert(name);
            }
        }
    }
    Ok(names)
}

struct DeviceSet(HDEVINFO);

impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.0); }
    }
}

fn friendly_name(set: HDEVINFO, dev: &SP_DEVINFO_DATA) -> Option<String> {
    let mut buffer = [0_u16; 256];
    for property in [SPDRP_FRIENDLYNAME, SPDRP_DEVICEDESC] {
        let ok = unsafe {
            SetupDiGetDeviceRegistryPropertyW(set, dev, property, null_mut(),
                buffer.as_mut_ptr().cast(), size_of::<[u16; 256]>() as u32, null_mut())
        };
        if ok != 0 {
            let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            let name = String::from_utf16_lossy(&buffer[..end]);
            if !name.trim().is_empty() { return Some(name); }
        }
    }
    None
}

pub fn connected_readings() -> windows::core::Result<Vec<BluetoothReading>> {
    let connected = connected_names()?;
    if connected.is_empty() { return Ok(Vec::new()); }
    let mut devices = BTreeMap::new();
    for enumerator in ["BTHENUM", "BTHLE", "BTHLEDEVICE"] {
        let wide: Vec<_> = enumerator.encode_utf16().chain([0]).collect();
        let handle = unsafe {
            SetupDiGetClassDevsW(null(), wide.as_ptr(), null_mut(), DIGCF_PRESENT | DIGCF_ALLCLASSES)
        };
        if handle as isize == -1 { continue; }
        let set = DeviceSet(handle);
        for index in 0..1024 {
            let mut dev: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
            dev.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
            if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut dev) } == 0 { break; }
            let mut value = 0_u8;
            let mut kind = 0;
            let mut size = 0;
            let ok = unsafe {
                SetupDiGetDevicePropertyW(set.0, &dev, &BATTERY_LEVEL, &mut kind,
                    &mut value, 1, &mut size, 0)
            };
            if ok == 0 || kind != DEVPROP_TYPE_BYTE || size != 1 || value > 100 { continue; }
            if let Some(name) = friendly_name(set.0, &dev) {
                if let Some(name) = connected.get(&name.trim().to_lowercase()) {
                    // Windows can expose the same device through multiple profiles.
                    devices.entry(name.clone()).and_modify(|level: &mut u8| *level = (*level).min(value)).or_insert(value);
                }
            }
        }
    }
    Ok(devices.into_iter().map(|(name, percent)| BluetoothReading { name, percent }).collect())
}
