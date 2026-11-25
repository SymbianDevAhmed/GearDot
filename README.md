# G435 Battery Monitor — Rust

A fresh Windows-focused project for the Logitech G435 LIGHTSPEED USB receiver
`046D:0ADF`. It does not require G HUB, a different GitHub account, or a clone of
another battery-monitor repository. Standard Rust crates are used for hardware
interoperability and desktop UI; Cargo fetches them during a build.

## Features

- Numeric battery icon in the Windows system tray; hover for the estimated
  percentage and actual measured millivolts.
- Click the icon (or choose Battery details) for a dark, compact details card
  with a charge bar and a real 12-hour history chart.
- Refresh every 60 seconds, manual refresh, and an Exit menu.
- Grey icon when no current reading is available; red when below 20%.
- Readings stored locally in `%LOCALAPPDATA%\G435Battery\history.csv`.
- Approximate remaining time only after sufficient uninterrupted discharge data.
- No telemetry, no G HUB, no firmware flashing. The HID reader uses the tested
  temporary remote battery-query session and closes it after each query.
- No auto-start or notification balloon in this first version. The icon is a
  standard system-tray icon, not an external overlay attached to the taskbar.

The percentage is an estimate from actual cell voltage, and is not an exact
fuel-gauge reading. Charging state is not detected; voltage during charging or
high load can distort the estimate. Remaining runtime is a trend estimate, not
a device-reported number.

## Rust OOP architecture

| Module | Responsibility |
| --- | --- |
| `battery.rs` | Encapsulated domain model, error enum and BatterySource trait |
| `protocol.rs` | G435Protocol struct + associated methods, CRC and parsing |
| `receiver.rs` | G435Receiver struct implementing BatterySource; HID I/O |
| `history.rs` | BatteryHistory owns samples, storage and trend calculation |
| `app.rs` | TrayApp struct implements winit ApplicationHandler; background query |
| `icon.rs` | BatteryIcon renders a numeric RGBA image |
| `details.rs` | DetailsCard implements eframe App for the dashboard |
| `bin/diagnostic.rs` | Console one-shot test utility |

The design uses composition, private fields, traits, enums, Result and
ownership rather than class inheritance. All hardware-dependent code is
Windows-gated; the protocol/domain tests can run on other platforms.

## Build on Windows

Install the Rust stable MSVC toolchain from https://rustup.rs/ and the Visual
Studio C++ build tools if prompted. In the extracted project directory:

```powershell
cargo test --all-targets
cargo build --release --bins
```

The GUI program is `target\release\g435-battery.exe`. The independent one-shot
reader is `target\release\diagnostic.exe`. You can also run BUILD.cmd.

## Build on GitHub

Upload the contents of this directory (including .github) to your own
repository. The included GitHub Actions workflow runs tests and builds both
executables on Windows. Download the artifact G435Battery-windows-x64 from
the completed Actions run. Cargo.lock is generated when Cargo is first run;
commit it after the first successful build to freeze dependency resolution.

## Validation status

The supplied hardware diagnostic capture shows 3861 mV and ~64%. All eight
request reports and the battery response were independently checked against
that capture. The Rust integration tests encode these exact reports.
Rust compilation and Windows UI/hardware execution have not yet been performed
in the development environment (no Rust compiler or physical USB receiver);
the GitHub Actions workflow is the next verification step.

## Technical references

- https://doc.rust-lang.org/book/ch17-01-what-is-oo.html
- https://docs.rs/hidapi/
- https://docs.rs/tray-icon/
- https://docs.rs/winit/
- https://docs.rs/eframe/
- G435 voltage curve and query reference: https://github.com/portyporty/g435-battery-percentage
  (MIT; see THIRD-PARTY-NOTICES.txt). No existing application source is imported
  or built; hardware frame literals were independently verified against the
  actual capture.
