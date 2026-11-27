//! Passive USB input-report decoding. Battery buckets are ten points wide.
//! Layout reference: https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_ps5.c
use crate::preferences::{InputBattery, InputCharge};

pub(crate) fn usb_battery(report: &[u8]) -> Option<InputBattery> {
    if report.len() != 64 || report[0] != 1 {
        return None;
    }
    let (state, bucket) = (report[53] >> 4, report[53] & 0xf);
    let (percent, charge, estimated) = match (state, bucket) {
        (2, 0..=10) => (100, InputCharge::Full, false),
        (0 | 1, 0..=10) => (
            (bucket * 10 + 5).min(100),
            if state == 1 {
                InputCharge::Charging
            } else {
                InputCharge::Discharging
            },
            true,
        ),
        _ => return None,
    };
    Some(InputBattery {
        percent,
        charge,
        estimated,
        checked_at: 0,
        stale: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_report_distinguishes_buckets_full_and_invalid_readings() {
        let mut report = [0; 64];
        report[0] = 1;
        report[53] = 0x13;
        let battery = usb_battery(&report).unwrap();
        assert_eq!(
            (battery.percent, battery.charge, battery.estimated),
            (35, InputCharge::Charging, true)
        );
        report[53] = 0x2a;
        assert_eq!(usb_battery(&report).unwrap().charge, InputCharge::Full);
        report[53] = 0x3a;
        assert!(usb_battery(&report).is_none());
        assert!(usb_battery(&report[..63]).is_none());
    }
}
