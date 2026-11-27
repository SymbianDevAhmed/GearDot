//! The device factory keeps protocol-specific construction out of the UI.
use crate::battery::{BatteryError, BatteryReading, BatterySource, Connection, DeviceKind};
use crate::{
    device_catalog::{self, Reader},
    mouse::MadRReceiver,
    receiver::G435Receiver,
};
use std::mem::size_of;
use windows_sys::{
    core::GUID,
    Win32::Devices::{DeviceAndDriverInstallation::*, Properties::*},
};

mod audio;

pub fn adapter(kind: DeviceKind) -> Box<dyn BatterySource> {
    match kind {
        DeviceKind::Headset => Box::new(G435Receiver::new()),
        DeviceKind::Mouse => Box::new(MadRReceiver),
    }
}

pub fn read(kind: DeviceKind) -> Result<BatteryReading, BatteryError> {
    adapter(kind).read_battery().map(|reading| match kind {
        DeviceKind::Headset => reading.with_connection(Connection::Dongle),
        DeviceKind::Mouse => reading,
    })
}

pub struct InputScan {
    pub devices: Vec<crate::preferences::InputPeripheral>,
    pub hid_error: Option<String>,
    pub audio_error: Option<String>,
}

/// Enumerate connected inputs using Windows capabilities and HID descriptors.
pub fn connected_inputs() -> InputScan {
    use crate::preferences::{InputKind, InputPeripheral, InputStatus};
    use crate::usb_inventory::{collapse_interfaces, UsbInterface};
    use hidapi::{BusType, HidApi};
    use windows_sys::Win32::UI::Input::XboxController::*;

    let (mut devices, audio_error) = match audio::connected_headsets() {
        Ok(headsets) => (headsets, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let controller_start = devices.len();
    for slot in 0..4 {
        let mut capabilities: XINPUT_CAPABILITIES = unsafe { std::mem::zeroed() };
        let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
        if unsafe { XInputGetState(slot, &mut state) } != 0
            || unsafe { XInputGetCapabilities(slot, XINPUT_FLAG_GAMEPAD, &mut capabilities) } != 0
        {
            continue;
        }
        let mut battery: XINPUT_BATTERY_INFORMATION = unsafe { std::mem::zeroed() };
        let status =
            if unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut battery) }
                != 0
            {
                InputStatus::Unknown
            } else {
                match battery.BatteryType {
                    BATTERY_TYPE_WIRED => InputStatus::Wired,
                    BATTERY_TYPE_DISCONNECTED => continue,
                    BATTERY_TYPE_ALKALINE | BATTERY_TYPE_NIMH => match battery.BatteryLevel {
                        BATTERY_LEVEL_EMPTY => InputStatus::Empty,
                        BATTERY_LEVEL_LOW => InputStatus::Low,
                        BATTERY_LEVEL_MEDIUM => InputStatus::Medium,
                        BATTERY_LEVEL_FULL => InputStatus::Full,
                        _ => InputStatus::Unknown,
                    },
                    _ => InputStatus::Unknown,
                }
            };
        devices.push(InputPeripheral {
            id: format!("xinput-{slot}"),
            name: format!("XInput controller {}", slot + 1),
            kind: InputKind::Gamepad,
            status,
            connection_override: None,
            usb_ids: None,
            battery: None,
        });
    }
    let controller_end = devices.len();

    let api = match HidApi::new() {
        Ok(api) => api,
        Err(error) => {
            return InputScan {
                devices,
                hid_error: Some(error.to_string()),
                audio_error,
            }
        }
    };
    let mut interfaces = Vec::new();
    let controller_names = raw_controller_names();
    let mut passive_reads = 0;
    let mut xinput_devices = std::collections::BTreeMap::new();
    for device in api.device_list() {
        if !matches!(device.bus_type(), BusType::Usb) || device.usage_page() != 1 {
            continue;
        }
        let (kind, fallback) = match device.usage() {
            2 => (InputKind::Mouse, "USB mouse"),
            6 => (InputKind::Keyboard, "USB keyboard"),
            4 | 5 => (InputKind::Gamepad, "USB controller"),
            _ => continue,
        };
        let path = device.path().to_string_lossy().to_ascii_lowercase();
        let is_xinput_interface = kind == InputKind::Gamepad && path.contains("&ig_");
        let product_name = device
            .product_string()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let ids = (device.vendor_id(), device.product_id());
        let profile = device_catalog::lookup(ids.0, ids.1);
        let reader = profile.map_or(Reader::Inventory, |profile| profile.reader);
        // The monitor publishes these devices only when their existing reader confirms presence.
        if matches!(reader, Reader::G435 | Reader::MadR) {
            continue;
        }
        let usb = hid_instance_id(&path).and_then(|instance| usb_parent(&instance));
        if usb.as_ref().is_some_and(|(_, id)| {
            let audio_id = crate::preferences::local_id("audio-", id.as_bytes());
            devices.iter().any(|device| device.id == audio_id)
        }) {
            continue;
        }
        let bus_name = usb
            .as_ref()
            .and_then(|(node, _)| node_string(*node, &DEVPKEY_Device_BusReportedDeviceDesc));
        let product_name = product_name
            .filter(|name| !crate::usb_inventory::generic_composite(&name.to_ascii_lowercase()))
            .or(bus_name.as_deref())
            .or(product_name);
        let profile = profile.filter(|profile| profile.kind == kind);
        let windows_name = (kind == InputKind::Gamepad)
            .then(|| {
                let instance = hid_instance_id(&path)?;
                let nodes = ancestors(&instance)
                    .take_while(|(_, id)| id.starts_with(r"hid\") || id.starts_with(r"usb\vid_"));
                nodes
                    .flat_map(|(node, _)| {
                        [
                            node_string(node, &DEVPKEY_Device_FriendlyName),
                            node_string(node, &DEVPKEY_Device_BusReportedDeviceDesc),
                            node_string(node, &DEVPKEY_Device_DeviceDesc),
                        ]
                    })
                    .flatten()
                    .find(|name| !generic_gamepad_name(name))
            })
            .flatten();
        let raw_name = (kind == InputKind::Gamepad)
            .then(|| controller_names.get(&ids))
            .flatten()
            .and_then(Option::as_deref);
        let tac75_name = (kind == InputKind::Keyboard && ids == (0x3151, 0x502d))
            .then(|| {
                [product_name, bus_name.as_deref()]
                    .into_iter()
                    .flatten()
                    .find(|name| device_catalog::is_tac75_he(name))
            })
            .flatten()
            .map(|_| "Akko TAC75 HE");
        let name = profile
            .map(|profile| profile.name.as_str())
            .or(tac75_name)
            .or(raw_name.filter(|name| !generic_gamepad_name(name)))
            .or(windows_name.as_deref())
            .or(product_name.filter(|name| !generic_gamepad_name(name)))
            .or(raw_name)
            .or(product_name)
            .unwrap_or(fallback);
        let receiver = profile.is_some_and(|profile| profile.receiver)
            || bus_name
                .as_deref()
                .is_some_and(crate::usb_inventory::is_receiver);
        let wired = device_catalog::is_wired_only(kind, ids, name);
        let battery =
            if reader == Reader::DualSenseUsb && kind == InputKind::Gamepad && passive_reads < 16 {
                passive_reads += 1;
                let mut report = [0; 64];
                api.open_path(device.path())
                    .ok()
                    .and_then(|handle| handle.read_timeout(&mut report, 25).ok())
                    .and_then(|size| report.get(..size))
                    .and_then(crate::dualsense::usb_battery)
            } else {
                None
            };
        let identity = usb
            .map(|(_, id)| id)
            .or_else(|| container_identity(&path))
            .unwrap_or(path.clone());
        if is_xinput_interface {
            xinput_devices
                .entry(identity.clone())
                .or_insert((name.to_owned(), ids, wired));
            if controller_end > controller_start {
                continue;
            }
        }
        interfaces.push(UsbInterface {
            identity,
            kind,
            name: name.chars().take(100).collect(),
            product_name: profile.is_some() || product_name.is_some(),
            usb_ids: Some(ids),
            battery,
            receiver,
            wired,
            primary: path.contains("&mi_00") || !path.contains("&mi_"),
        });
    }
    // XInput slot indices have no documented mapping to physical device paths.
    // Only transfer the name when there is a single candidate and a single slot.
    if controller_end == controller_start + 1 && xinput_devices.len() == 1 {
        if let Some((_, (name, ids, wired))) = xinput_devices.into_iter().next() {
            let controller = &mut devices[controller_start];
            controller.name = name.chars().take(100).collect();
            controller.usb_ids = Some(ids);
            if wired {
                controller.status = InputStatus::WiredOnly;
            }
        }
    }
    devices.extend(collapse_interfaces(interfaces));
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    InputScan {
        devices,
        hid_error: None,
        audio_error,
    }
}

fn raw_controller_names() -> std::collections::BTreeMap<(u16, u16), Option<String>> {
    use windows::{
        Gaming::Input::RawGameController,
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }

    let mut names = std::collections::BTreeMap::new();
    if unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_err() {
        return names;
    }
    let _apartment = Apartment;
    let Ok(controllers) = RawGameController::RawGameControllers() else {
        return names;
    };
    for index in 0..controllers.Size().unwrap_or(0).min(64) {
        let Ok(controller) = controllers.GetAt(index) else {
            continue;
        };
        let (Ok(vid), Ok(pid), Ok(name)) = (
            controller.HardwareVendorId(),
            controller.HardwareProductId(),
            controller.DisplayName(),
        ) else {
            continue;
        };
        let name = name.to_string_lossy();
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        names
            .entry((vid, pid))
            .and_modify(|existing: &mut Option<String>| {
                if existing.as_deref() != Some(name) {
                    *existing = None;
                }
            })
            .or_insert_with(|| Some(name.to_owned()));
    }
    names
}

fn generic_gamepad_name(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    [
        "hid-compliant",
        "usb input device",
        "usb composite",
        "usb controller",
        "controller (xbox",
        "xinput compatible",
        "xinput controller",
        "xbox gaming device",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
        || matches!(
            name.as_str(),
            "controller" | "game controller" | "gamepad" | "usb gamepad"
        )
}

fn hid_instance_id(path: &str) -> Option<String> {
    let mut segments = path.strip_prefix(r"\\?\")?.split('#');
    Some(format!(
        r"{}\{}\{}",
        segments.next()?,
        segments.next()?,
        segments.next()?
    ))
}

fn locate(instance: &str) -> Option<u32> {
    let instance: Vec<u16> = instance.encode_utf16().chain([0]).collect();
    let mut node = 0;
    (unsafe { CM_Locate_DevNodeW(&mut node, instance.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) }
        == CR_SUCCESS)
        .then_some(node)
}

fn ancestors(instance: &str) -> impl Iterator<Item = (u32, String)> {
    let mut next = locate(instance);
    std::iter::from_fn(move || {
        let node = next?;
        let mut parent = 0;
        next = (unsafe { CM_Get_Parent(&mut parent, node, 0) } == CR_SUCCESS).then_some(parent);
        let mut buffer = [0u16; 512];
        let result =
            unsafe { CM_Get_Device_IDW(node, buffer.as_mut_ptr(), buffer.len() as u32, 0) };
        let id = if result == CR_SUCCESS {
            utf16_text(&buffer).to_ascii_lowercase()
        } else {
            String::new()
        };
        Some((node, id))
    })
    .take(16)
}

fn usb_parent(instance: &str) -> Option<(u32, String)> {
    ancestors(instance).find(|(_, id)| id.starts_with(r"usb\vid_") && !id.contains("&mi_"))
}

fn node_string(node: u32, property: &DEVPROPKEY) -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut length = size_of_val(&buffer) as u32;
    let mut kind = 0;
    let result = unsafe {
        CM_Get_DevNode_PropertyW(
            node,
            property,
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut length,
            0,
        )
    };
    (result == CR_SUCCESS && kind == DEVPROP_TYPE_STRING)
        .then(|| utf16_text(&buffer))
        .filter(|text| !text.is_empty())
}

fn utf16_text(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&value| value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end]).trim().to_owned()
}

fn container_identity(path: &str) -> Option<String> {
    let node = locate(&hid_instance_id(path)?)?;
    let mut guid = GUID::from_u128(0);
    let mut length = size_of::<GUID>() as u32;
    let mut kind = 0;
    let result = unsafe {
        CM_Get_DevNode_PropertyW(
            node,
            &DEVPKEY_Device_ContainerId,
            &mut kind,
            (&mut guid as *mut GUID).cast(),
            &mut length,
            0,
        )
    };
    if result != CR_SUCCESS
        || kind != DEVPROP_TYPE_GUID
        || length != size_of::<GUID>() as u32
        || (guid.data1 == 0 && guid.data2 == 0 && guid.data3 == 0)
    {
        return None;
    }
    Some(format!(
        "{:08x}-{:04x}-{:04x}-{:02x?}",
        guid.data1, guid.data2, guid.data3, guid.data4
    ))
}
