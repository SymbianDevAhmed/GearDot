//! The bundled catalogue names observed hardware and selects compiled readers.
use crate::preferences::InputKind;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub(crate) enum Reader {
    #[default]
    Inventory,
    G435,
    MadR,
    DualSenseUsb,
}

#[derive(Deserialize)]
pub(crate) struct Profile {
    pub id: String,
    pub name: String,
    pub kind: InputKind,
    #[serde(default)]
    pub receiver: bool,
    #[serde(default)]
    pub reader: Reader,
}

fn catalog() -> &'static [Profile] {
    static CATALOG: OnceLock<Vec<Profile>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/devices/catalog.json"))
            .expect("bundled device catalogue is valid")
    })
}

pub(crate) fn lookup(vid: u16, pid: u16) -> Option<&'static Profile> {
    let id = format!("{vid:04X}:{pid:04X}");
    catalog().iter().find(|profile| profile.id == id)
}

pub(crate) fn usb_ids(instance: &str) -> Option<(u16, u16)> {
    let ids = instance.strip_prefix(r"usb\vid_")?;
    let (vid, ids) = ids.split_once("&pid_")?;
    let pid = ids.get(..4)?;
    Some((
        u16::from_str_radix(vid, 16).ok()?,
        u16::from_str_radix(pid, 16).ok()?,
    ))
}

fn contains_model(name: &str, model: &[&str]) -> bool {
    let words: Vec<_> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    words.windows(model.len()).any(|words| {
        words
            .iter()
            .zip(model)
            .all(|(found, expected)| found.eq_ignore_ascii_case(expected))
    })
}

pub(crate) fn is_tac75_he(name: &str) -> bool {
    contains_model(name, &["TAC75", "HE"])
}

pub(crate) fn is_abkoncore_b780(name: &str) -> bool {
    contains_model(name, &["B780"])
}

pub(crate) fn is_wired_only(kind: InputKind, ids: (u16, u16), name: &str) -> bool {
    match (kind, ids) {
        (InputKind::Keyboard, (0x3151, 0x502d)) => is_tac75_he(name),
        // Other Xbox-compatible devices may share this USB ID.
        (InputKind::Gamepad, (0x045e, 0x02ff)) => contains_model(name, &["GameSir", "G7", "SE"]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_usb_ids_require_an_explicit_model_name() {
        let akko = (0x3151, 0x502d);
        assert!(lookup(akko.0, akko.1).is_none());
        assert!(is_wired_only(InputKind::Keyboard, akko, "Akko TAC75 HE"));
        assert!(!is_wired_only(InputKind::Keyboard, akko, "Akko Keyboard"));
        assert!(!is_wired_only(
            InputKind::Keyboard,
            akko,
            "MonsGeek FUN60 Pro"
        ));
        let xbox = (0x045e, 0x02ff);
        assert!(is_wired_only(
            InputKind::Gamepad,
            xbox,
            "GameSir-G7 SE Controller for Xbox"
        ));
        assert!(!is_wired_only(
            InputKind::Gamepad,
            xbox,
            "Controller (Xbox One For Windows)"
        ));
        assert!(!is_wired_only(InputKind::Gamepad, xbox, "GameSir G7 Pro"));
        assert!(is_abkoncore_b780("Speakers (ABKONCORE B780)"));
        assert!(!is_abkoncore_b780("USB Audio Device"));
        assert!(!is_abkoncore_b780("B7800"));
    }

    #[test]
    fn catalogue_uses_unique_exact_hardware_ids() {
        let mut ids = std::collections::BTreeSet::new();
        for profile in catalog() {
            let (vid, pid) = profile.id.split_once(':').unwrap();
            assert_eq!((vid.len(), pid.len()), (4, 4));
            let key = (
                u16::from_str_radix(vid, 16).unwrap(),
                u16::from_str_radix(pid, 16).unwrap(),
            );
            assert!(ids.insert(key));
            let record = lookup(key.0, key.1).unwrap();
            assert!(!record.name.is_empty());
            if record.reader == Reader::DualSenseUsb {
                assert_eq!(record.kind, InputKind::Gamepad);
                assert!(!record.receiver);
            }
        }
        assert!(lookup(0x046d, 0x0acb).is_none());
        assert_eq!(
            usb_ids(r"usb\vid_046d&pid_0adf\private"),
            Some((0x046d, 0x0adf))
        );
    }
}
