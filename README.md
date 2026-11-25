# G435 Battery Monitor — Rust

A fresh Windows-focused project for the Logitech G435 LIGHTSPEED USB receiver
`046D:0ADF`. It does not require G HUB, a different GitHub account, or a clone of
another battery-monitor repository. Standard Rust crates are used for hardware
interoperability and desktop UI; Cargo fetches them during a build.

## Features

- Native taskbar indicator with a headset outline and adjacent battery percentage.
  It anchors near the notification area, follows the taskbar theme and DPI,
  hides for fullscreen apps, and falls back to the normal tray icon when the
  taskbar layout cannot be located.
- Click the icon (or choose Battery details) for a dark, compact details card
  with a charge bar and a real 12-hour history chart.
- Refresh every 60 seconds, manual refresh, and an Exit menu.
- Grey icon when no current reading is available; red when below 20%.
- Readings stored locally in `%LOCALAPPDATA%\G435Battery\history.csv`.
- Approximate remaining time only after sufficient uninterrupted discharge data.
- No telemetry, no G HUB, no firmware flashing. The HID reader uses the tested
  temporary remote battery-query session and closes it after each query.
- Below 20%, the percentage changes colour. No auto-start or notification balloon yet.
  The overlay is a separate non-activating Win32 window, not Explorer injection.

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
| `icon.rs` | BatteryIcon renders a numeric RGBA image (fallback) |
| `taskbar.rs` | Native layered widget renders a headset outline and percentage |
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
The v0.1.0 build passed all four Windows CI tests. The new native taskbar widget
is introduced in v0.2.0 and requires UI validation on real Windows hardware.

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
