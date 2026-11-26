//! Read Windows' cached device properties. No connection or HID requests.
use std::{
    collections::BTreeMap,
    mem::size_of,
    ptr::{null, null_mut},
};
use windows_sys::{
    core::GUID,
    Win32::Devices::{DeviceAndDriverInstallation::*, Properties::*},
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
        unsafe {
            RoUninitialize();
        }
    }
}

fn connected_names() -> windows::core::Result<BTreeMap<String, String>> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let _win_rt = WinRt;
    let connected = BluetoothConnectionStatus::Connected;
    let mut names = BTreeMap::new();
    for selector in [
        BluetoothDevice::GetDeviceSelectorFromConnectionStatus(connected)?,
        BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(connected)?,
    ] {
        let items = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
            &selector,
            None::<&IIterable<HSTRING>>,
            DeviceInformationKind::AssociationEndpoint,
        )?
        .get()?;
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
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

fn friendly_name(set: HDEVINFO, dev: &SP_DEVINFO_DATA) -> Option<String> {
    let mut buffer = [0_u16; 256];
    for property in [SPDRP_FRIENDLYNAME, SPDRP_DEVICEDESC] {
        let ok = unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                set,
                dev,
                property,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                size_of::<[u16; 256]>() as u32,
                null_mut(),
            )
        };
        if ok != 0 {
            let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            let name = String::from_utf16_lossy(&buffer[..end]);
            if !name.trim().is_empty() {
                return Some(name);
            }
        }
    }
    None
}

pub fn connected_readings() -> windows::core::Result<Vec<BluetoothReading>> {
    let connected = connected_names()?;
    if connected.is_empty() {
        return Ok(Vec::new());
    }
    let mut devices = BTreeMap::new();
    for enumerator in ["BTHENUM", "BTHLE", "BTHLEDEVICE"] {
        let wide: Vec<_> = enumerator.encode_utf16().chain([0]).collect();
        let handle = unsafe {
            SetupDiGetClassDevsW(
                null(),
                wide.as_ptr(),
                null_mut(),
                DIGCF_PRESENT | DIGCF_ALLCLASSES,
            )
        };
        if handle as isize == -1 {
            continue;
        }
        let set = DeviceSet(handle);
        for index in 0..1024 {
            let mut dev: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
            dev.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
            if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut dev) } == 0 {
                break;
            }
            let mut value = 0_u8;
            let mut kind = 0;
            let mut size = 0;
            let ok = unsafe {
                SetupDiGetDevicePropertyW(
                    set.0,
                    &dev,
                    &BATTERY_LEVEL,
                    &mut kind,
                    &mut value,
                    1,
                    &mut size,
                    0,
                )
            };
            if ok == 0 || kind != DEVPROP_TYPE_BYTE || size != 1 || value > 100 {
                continue;
            }
            if let Some(name) = friendly_name(set.0, &dev) {
                if let Some(name) = connected.get(&name.trim().to_lowercase()) {
                    // Windows can expose the same device through multiple profiles.
                    devices
                        .entry(name.clone())
                        .and_modify(|level: &mut u8| *level = (*level).min(value))
                        .or_insert(value);
                }
            }
        }
    }
    Ok(devices
        .into_iter()
        .map(|(name, percent)| BluetoothReading { name, percent })
        .collect())
}

// Keep the Windows Bluetooth APIs in a disposable process, outside the tray event loop.
pub fn probe() -> std::io::Result<Vec<BluetoothReading>> {
    use std::{
        io::{self, Read},
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    const MAX_RESPONSE: u64 = 64 * 1024;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--bluetooth-probe")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let result = thread::scope(|scope| {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Missing Bluetooth response pipe"))?;
        let response = scope.spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(MAX_RESPONSE + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_RESPONSE {
                return Err(io::Error::other(
                    "Bluetooth response exceeded the size limit",
                ));
            }
            Ok(bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                Ok(None) => {
                    break Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Bluetooth probe timed out",
                    ))
                }
                Err(error) => break Err(error),
            }
        };
        if status.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let bytes = response
            .join()
            .map_err(|_| io::Error::other("Bluetooth response reader panicked"))??;
        let status = status?;
        if !status.success() {
            let code = status.code().unwrap_or(-1) as u32;
            return Err(io::Error::other(format!(
                "Bluetooth probe exited with 0x{code:08X}"
            )));
        }
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    });
    // Missing stdout is impossible with Stdio::piped, but still clean up if it occurs.
    if result.is_err() && matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

pub fn write_probe_response() -> Result<(), Box<dyn std::error::Error>> {
    let readings = connected_readings()?;
    let output = std::io::stdout();
    serde_json::to_writer(output.lock(), &readings)?;
    Ok(())
}
