//! Join Windows Bluetooth battery and connection properties by radio address.
use std::{collections::BTreeMap, io, mem::size_of, ptr::null_mut};
use windows_sys::{
    core::GUID,
    Win32::Devices::{DeviceAndDriverInstallation::*, Properties::*},
};
use wireless_battery_monitor::preferences::BluetoothReading;

const fn key(id: u128, pid: u32) -> DEVPROPKEY {
    DEVPROPKEY { fmtid: GUID::from_u128(id), pid }
}

const BATTERY: DEVPROPKEY = key(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5, 2);
const BATTERY_UPDATED: DEVPROPKEY = key(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5, 7);
const ADDRESS: DEVPROPKEY = key(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a, 1);
const CONNECTED: DEVPROPKEY = key(0x83da6326_97a6_4088_9453_a1923f573b29, 15);

struct DeviceSet(HDEVINFO);

impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.0); }
    }
}

impl DeviceSet {
    fn property<T: Copy + Default>(&self, node: &SP_DEVINFO_DATA, key: &DEVPROPKEY, kind: u32) -> Option<T> {
        let mut value = T::default();
        let mut actual_kind = 0;
        let mut actual_size = 0;
        let size = size_of::<T>() as u32;
        let ok = unsafe {
            SetupDiGetDevicePropertyW(self.0, node, key, &mut actual_kind,
                (&mut value as *mut T).cast(), size, &mut actual_size, 0)
        };
        (ok != 0 && actual_kind == kind && actual_size == size).then_some(value)
    }

    fn string(&self, node: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<String> {
        let mut buffer = [0_u16; 256];
        let mut kind = 0;
        let ok = unsafe {
            SetupDiGetDevicePropertyW(self.0, node, key, &mut kind, buffer.as_mut_ptr().cast(),
                size_of::<[u16; 256]>() as u32, null_mut(), 0)
        };
        (ok != 0 && kind == DEVPROP_TYPE_STRING).then_some(text(&buffer)).filter(|value| !value.is_empty())
    }

    fn battery(&self, node: &SP_DEVINFO_DATA) -> Option<u8> {
        self.property::<u8>(node, &BATTERY, DEVPROP_TYPE_BYTE)
            .or_else(|| self.property::<i8>(node, &BATTERY, DEVPROP_TYPE_SBYTE).and_then(|level| u8::try_from(level).ok()))
            .or_else(|| self.property::<u16>(node, &BATTERY, DEVPROP_TYPE_UINT16).and_then(|level| u8::try_from(level).ok()))
            .or_else(|| self.property::<u32>(node, &BATTERY, DEVPROP_TYPE_UINT32).and_then(|level| u8::try_from(level).ok()))
            .or_else(|| self.property::<i32>(node, &BATTERY, DEVPROP_TYPE_INT32).and_then(|level| u8::try_from(level).ok()))
            .filter(|level| *level <= 100)
    }
}

fn text(buffer: &[u16]) -> String {
    let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length]).trim().to_owned()
}

fn radio_address(value: &str) -> Option<u64> {
    let digits: String = value.chars().filter(|c| !matches!(c, ':' | '-' | ' ')).collect();
    if digits.len() != 12 { return None; }
    u64::from_str_radix(&digits, 16).ok().filter(|value| *value != 0)
}

#[derive(Default)]
struct Device {
    connected: bool,
    root_name: Option<String>,
    battery: Option<(u64, u8, Option<String>)>,
}

impl Device {
    fn observe(&mut self, is_root: bool, connected: bool, name: Option<String>, battery: Option<(u64, u8)>) {
        self.connected |= connected;
        if is_root && self.root_name.is_none() {
            self.root_name = name.clone();
        }
        if let Some((time, level)) = battery {
            if self.battery.as_ref().is_none_or(|previous| time > previous.0) {
                self.battery = Some((time, level, name));
            }
        }
    }

    fn reading(self) -> Option<BluetoothReading> {
        let (_, percent, profile_name) = self.battery?;
        if !self.connected { return None; }
        let name = self.root_name.or(profile_name).unwrap_or_else(|| "Bluetooth device".into());
        Some(BluetoothReading { name, percent })
    }
}

fn connected_readings() -> io::Result<Vec<BluetoothReading>> {
    let handle = unsafe {
        SetupDiGetClassDevsW(std::ptr::null(), std::ptr::null(), null_mut(), DIGCF_PRESENT | DIGCF_ALLCLASSES)
    };
    if handle as isize == -1 { return Err(io::Error::last_os_error()); }

    let set = DeviceSet(handle);
    let mut devices = BTreeMap::<u64, Device>::new();
    for index in 0.. {
        let mut node: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
        node.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut node) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(windows_sys::Win32::Foundation::ERROR_NO_MORE_ITEMS as i32) {
                return Err(error);
            }
            break;
        }
        let mut instance_id = [0_u16; 512];
        let ok = unsafe {
            SetupDiGetDeviceInstanceIdW(set.0, &node, instance_id.as_mut_ptr(), instance_id.len() as u32, null_mut())
        };
        if ok == 0 { continue; }
        let id = text(&instance_id).to_ascii_uppercase();
        if !id.starts_with("BTH") { continue; }
        let root_address = id.split('\\').nth(1).and_then(|part| part.strip_prefix("DEV_"));
        let address = set.string(&node, &ADDRESS).as_deref().and_then(radio_address)
            .or_else(|| root_address.and_then(radio_address));
        let Some(address) = address else { continue; };
        let connected = set.property::<u8>(&node, &CONNECTED, DEVPROP_TYPE_BOOLEAN).is_some_and(|value| value != 0);
        let name = set.string(&node, &DEVPKEY_Device_FriendlyName).or_else(|| set.string(&node, &DEVPKEY_Device_DeviceDesc));
        let battery = set.battery(&node).map(|level| {
            let time = set.property::<u64>(&node, &BATTERY_UPDATED, DEVPROP_TYPE_FILETIME).unwrap_or(0);
            (time, level)
        });
        devices.entry(address).or_default().observe(root_address.is_some(), connected, name, battery);
    }

    let mut readings: Vec<_> = devices.into_values().filter_map(Device::reading).collect();
    readings.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(readings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_headset_uses_connection_from_root_and_battery_from_profile() {
        assert_eq!(radio_address("aa:bb:cc:dd:ee:ff"), radio_address("AABBCCDDEEFF"));
        let mut headset = Device::default();
        headset.observe(false, false, Some("Headset Hands-Free AG".into()), Some((22, 70)));
        headset.observe(true, true, Some("Headset".into()), None);
        headset.observe(false, false, None, Some((12, 95)));
        let reading = headset.reading().unwrap();
        assert_eq!(reading.name, "Headset");
        assert_eq!(reading.percent, 70);
    }

    #[test]
    fn offline_battery_is_hidden_and_online_zero_percent_is_valid() {
        let mut device = Device::default();
        device.observe(true, false, None, Some((10, 100)));
        assert!(device.reading().is_none());
        let mut device = Device::default();
        device.observe(true, true, None, Some((10, 0)));
        assert_eq!(device.reading().unwrap().percent, 0);
    }
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
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("Missing Bluetooth response pipe"))?;
        let response = scope.spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(MAX_RESPONSE + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_RESPONSE {
                return Err(io::Error::other("Bluetooth response exceeded the size limit"));
            }
            Ok(bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                Ok(None) => break Err(io::Error::new(io::ErrorKind::TimedOut, "Bluetooth probe timed out")),
                Err(error) => break Err(error),
            }
        };
        if status.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let bytes = response.join().map_err(|_| io::Error::other("Bluetooth response reader panicked"))??;
        let status = status?;
        if !status.success() {
            let code = status.code().unwrap_or(-1) as u32;
            return Err(io::Error::other(format!("Bluetooth probe exited with 0x{code:08X}")));
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
