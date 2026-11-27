//! Headphone and headset playback endpoints currently active in Windows.
use super::{ancestors, hid_instance_id, node_string, DEVPKEY_Device_BusReportedDeviceDesc};
use crate::{
    device_catalog::is_abkoncore_b780,
    preferences::{local_id, InputKind, InputPeripheral, InputStatus},
    usb_inventory::is_receiver,
};
use std::collections::BTreeMap;
use windows::{
    core::{Interface, BSTR, PWSTR},
    Win32::{
        Devices::FunctionDiscovery::{PKEY_Device_FriendlyName, PKEY_Device_InstanceId},
        Media::{
            Audio::*,
            KernelStreaming::{
                IKsJackDescription, IKsJackDescription2, JACKDESC2_PRESENCE_DETECT_CAPABILITY,
                KSJACK_DESCRIPTION,
            },
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
            COINIT_MULTITHREADED, STGM_READ,
        },
        UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY},
    },
};

pub(super) fn connected_headsets() -> windows::core::Result<Vec<InputPeripheral>> {
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let _apartment = Apartment;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let endpoints = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
    let mut headsets = BTreeMap::new();

    for index in 0..unsafe { endpoints.GetCount()? } {
        let Ok(endpoint) = (unsafe { endpoints.Item(index) }) else {
            continue;
        };
        let Ok(properties) = (unsafe { endpoint.OpenPropertyStore(STGM_READ) }) else {
            continue;
        };
        let form = unsafe { properties.GetValue(&PKEY_AudioEndpoint_FormFactor) }
            .ok()
            .and_then(|value| u32::try_from(&value).ok());
        let name = property_text(&properties, &PKEY_Device_FriendlyName)
            .unwrap_or_else(|| "Headphones".into());
        let lower_name = name.to_lowercase();
        let headset =
            form.is_some_and(|form| form == Headphones.0 as u32 || form == Headset.0 as u32);
        let headset = headset
            || ["headphone", "headset", "earbud"]
                .iter()
                .any(|label| lower_name.contains(label));
        let mut b780 = is_abkoncore_b780(&name);

        let Ok(raw_id) = (unsafe { endpoint.GetId() }) else {
            continue;
        };
        let Some(endpoint_id) = owned_string(raw_id) else {
            continue;
        };
        let endpoint_instance = format!(r"SWD\MMDEVAPI\{endpoint_id}");
        let device_instance = property_text(&properties, &PKEY_Device_InstanceId);
        let connector = unsafe { endpoint.Activate::<IDeviceTopology>(CLSCTX_ALL, None) }
            .and_then(|topology| unsafe { topology.GetConnector(0) })
            .ok();
        let adapter_instance = connector
            .as_ref()
            .and_then(|connector| unsafe { connector.GetDeviceIdConnectedTo() }.ok())
            .and_then(owned_string)
            .and_then(|id| hid_instance_id(&id.to_ascii_lowercase()).or(Some(id)));
        let nodes: Vec<_> = [
            device_instance.as_deref(),
            Some(endpoint_instance.as_str()),
            adapter_instance.as_deref(),
        ]
        .into_iter()
        .flatten()
        .flat_map(ancestors)
        .collect();
        if nodes.iter().any(|(_, id)| id.starts_with("bth")) || lower_name.contains("bluetooth") {
            continue;
        }

        let (identity, status, usb_ids) = if let Some((node, id)) = nodes
            .iter()
            .find(|(_, id)| id.starts_with(r"usb\vid_") && !id.contains("&mi_"))
        {
            // The G435 card is driven by its own receiver, including radio presence.
            let ids = crate::device_catalog::usb_ids(id);
            let profile = ids
                .and_then(|(vid, pid)| crate::device_catalog::lookup(vid, pid))
                .filter(|profile| profile.kind == InputKind::Headset);
            if profile.is_some_and(|profile| profile.reader == crate::device_catalog::Reader::G435)
            {
                continue;
            }
            let from_bus = node_string(*node, &DEVPKEY_Device_BusReportedDeviceDesc);
            b780 |= from_bus.as_deref().is_some_and(is_abkoncore_b780);
            let receiver = profile.is_some_and(|profile| profile.receiver)
                || is_receiver(&name)
                || from_bus.as_deref().is_some_and(is_receiver);
            let status = if b780 {
                InputStatus::WiredOnly
            } else if receiver {
                InputStatus::Receiver
            } else {
                InputStatus::Usb
            };
            (id.as_str(), status, ids)
        } else if nodes
            .iter()
            .any(|(_, id)| id.starts_with("hdaudio\\") || id.starts_with("intelaudio\\"))
        {
            if connector.as_ref().and_then(jack_connected) == Some(false) {
                continue;
            }
            (endpoint_id.as_str(), InputStatus::Wired, None)
        } else {
            continue;
        };

        if !headset && !b780 {
            continue;
        }
        let name = usb_ids
            .and_then(|(vid, pid)| crate::device_catalog::lookup(vid, pid))
            .filter(|profile| profile.kind == InputKind::Headset)
            .map_or(name, |profile| profile.name.clone());
        let name = if b780 && status == InputStatus::WiredOnly {
            "ABKONCORE Gaming Headset B780".into()
        } else {
            name
        };
        let id = local_id("audio-", identity.as_bytes());
        headsets.entry(id.clone()).or_insert(InputPeripheral {
            id,
            name,
            kind: InputKind::Headset,
            status,
            connection_override: None,
            usb_ids,
            battery: None,
        });
    }
    Ok(headsets.into_values().collect())
}

fn property_text(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let value = unsafe { store.GetValue(key) }.ok()?;
    let value = BSTR::try_from(&value).ok()?.to_string();
    let text = value.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn owned_string(raw: PWSTR) -> Option<String> {
    let value = if raw.is_null() {
        None
    } else {
        unsafe { raw.to_string() }.ok()
    };
    unsafe {
        CoTaskMemFree(Some(raw.0.cast()));
    }
    value
}

fn jack_connected(connector: &IConnector) -> Option<bool> {
    let part: IPart = unsafe { connector.GetConnectedTo() }.ok()?.cast().ok()?;
    let capabilities: IKsJackDescription2 = activate_part(&part)?;
    let descriptions: IKsJackDescription = activate_part(&part)?;
    let count = unsafe { capabilities.GetJackCount() }.ok()?;
    let mut observed = false;
    for index in 0..count {
        let info = unsafe { capabilities.GetJackDescription2(index) }.ok()?;
        if info.JackCapabilities & JACKDESC2_PRESENCE_DETECT_CAPABILITY == 0 {
            continue;
        }
        let mut description = KSJACK_DESCRIPTION::default();
        unsafe { descriptions.GetJackDescription(index, &mut description) }.ok()?;
        if description.IsConnected.as_bool() {
            return Some(true);
        }
        observed = true;
    }
    observed.then_some(false)
}

fn activate_part<T: Interface>(part: &IPart) -> Option<T> {
    let mut pointer = std::ptr::null_mut();
    unsafe { part.Activate(CLSCTX_ALL.0, &T::IID, Some(&mut pointer)) }.ok()?;
    (!pointer.is_null()).then(|| unsafe { T::from_raw(pointer) })
}
