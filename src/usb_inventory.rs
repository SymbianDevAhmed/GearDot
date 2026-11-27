use crate::preferences::{local_id, InputBattery, InputKind, InputPeripheral, InputStatus};
use std::collections::BTreeMap;

pub(crate) struct UsbInterface {
    pub identity: String,
    pub kind: InputKind,
    pub name: String,
    pub product_name: bool,
    pub receiver: bool,
    pub wired: bool,
    pub primary: bool,
    pub usb_ids: Option<(u16, u16)>,
    pub battery: Option<InputBattery>,
}

fn priority(interface: &UsbInterface) -> (bool, bool, bool, bool, bool, bool, u8) {
    let name = interface.name.to_ascii_lowercase();
    let confirmed_kind = interface.product_name
        && match interface.kind {
            InputKind::Keyboard => name.contains("keyboard"),
            InputKind::Mouse => name.contains("mouse"),
            InputKind::Gamepad => ["controller", "gamepad", "joystick"]
                .iter()
                .any(|word| name.contains(word)),
            InputKind::Headset => name.contains("headset") || name.contains("headphone"),
            InputKind::Other => false,
        };
    let fallback = match interface.kind {
        InputKind::Keyboard => 0,
        InputKind::Mouse => 1,
        InputKind::Gamepad => 2,
        InputKind::Headset => 3,
        InputKind::Other => 4,
    };
    (
        interface.battery.is_some(),
        interface.wired,
        interface.product_name && !generic_composite(&name),
        confirmed_kind,
        interface.product_name,
        interface.primary,
        fallback,
    )
}

pub(crate) fn generic_composite(name: &str) -> bool {
    name.contains("composite")
        && (name.contains("2.4g") || name.contains("2.4 g") || name.starts_with("usb composite"))
}

pub(crate) fn is_receiver(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("dongle")
        || name.contains("receiver")
        || name.contains("2.4g")
        || name.contains("2.4 g")
}

pub(crate) fn collapse_interfaces(interfaces: Vec<UsbInterface>) -> Vec<InputPeripheral> {
    let mut devices = BTreeMap::<String, UsbInterface>::new();
    for interface in interfaces {
        if devices
            .get(&interface.identity)
            .is_none_or(|existing| priority(&interface) > priority(existing))
        {
            devices.insert(interface.identity.clone(), interface);
        }
    }
    // A generic composite interface describes the USB transport. It cannot
    // identify the peripheral attached to it and would appear as another card.
    devices
        .into_values()
        .filter(|device| !generic_composite(&device.name.to_ascii_lowercase()))
        .map(|device| InputPeripheral {
            // Preserve the existing profile key for the selected interface.
            id: local_id(
                "hid-",
                format!("{}:{:?}", device.identity, device.kind).as_bytes(),
            ),
            status: if device.wired {
                InputStatus::WiredOnly
            } else if device.receiver || is_receiver(&device.name) {
                InputStatus::Receiver
            } else {
                InputStatus::Usb
            },
            connection_override: None,
            name: device.name,
            kind: device.kind,
            usb_ids: device.usb_ids,
            battery: device.battery,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn akko(identity: &str, kind: InputKind) -> UsbInterface {
        UsbInterface {
            identity: identity.into(),
            kind,
            name: "Akko Keyboard".into(),
            product_name: true,
            receiver: false,
            wired: false,
            primary: false,
            usb_ids: None,
            battery: None,
        }
    }

    #[test]
    fn keyboard_mouse_collection_does_not_create_a_duplicate_card() {
        let devices = collapse_interfaces(vec![
            akko("same", InputKind::Mouse),
            akko("same", InputKind::Keyboard),
        ]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].kind, InputKind::Keyboard);
        assert_eq!(devices[0].name, "Akko Keyboard");
    }

    #[test]
    fn distinct_physical_devices_can_share_a_model_name() {
        let devices = collapse_interfaces(vec![
            akko("first", InputKind::Keyboard),
            akko("second", InputKind::Keyboard),
        ]);
        assert_eq!(devices.len(), 2);
        assert_ne!(devices[0].id, devices[1].id);
    }

    #[test]
    fn usb_receiver_is_classified_separately_from_a_wired_input() {
        let receiver = UsbInterface {
            name: "USB Receiver".into(),
            ..akko("receiver", InputKind::Mouse)
        };
        let devices = collapse_interfaces(vec![receiver, akko("keyboard", InputKind::Keyboard)]);
        assert_eq!(
            devices
                .iter()
                .find(|device| device.name == "USB Receiver")
                .unwrap()
                .status,
            InputStatus::Receiver
        );
        assert_eq!(
            devices
                .iter()
                .find(|device| device.name == "Akko Keyboard")
                .unwrap()
                .status,
            InputStatus::Usb
        );
    }

    #[test]
    fn generic_composite_entries_do_not_create_extra_cards() {
        let generic = |identity| UsbInterface {
            name: "2.4G Composite Devic".into(),
            ..akko(identity, InputKind::Keyboard)
        };
        let devices = collapse_interfaces(vec![
            generic("same"),
            generic("unmatched"),
            akko("same", InputKind::Keyboard),
        ]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Akko Keyboard");
    }
}
