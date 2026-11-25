# 🔋 Wireless Battery Monitor – Rust

<p align="center">
  <img src="assets/icon.svg" width="112" alt="Wireless Battery Monitor original emblem"/>
</p>

<p align="center">
  A lightweight Windows taskbar battery monitor for Logitech G435 LIGHTSPEED and VXE MAD R, with live readings, charging status, battery history, and estimated remaining time.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-2021-CE422B?style=for-the-badge&logo=rust&logoColor=white" alt="Rust 2021"/>
  <img src="https://img.shields.io/badge/Windows-10%2F11-0078D4?style=for-the-badge&logo=windows&logoColor=white" alt="Windows"/>
  <img src="https://img.shields.io/badge/USB_HID-LIGHTSPEED_%26_2.4GHz-007ACC?style=for-the-badge" alt="USB HID"/>
  <img src="https://img.shields.io/badge/OOP-Object--Oriented_Design-3776AB?style=for-the-badge" alt="OOP"/>
</p>

---

## 📌 About the Project

The **Wireless Battery Monitor** is a personal native Windows application developed in Rust to monitor compatible wireless peripherals without keeping their vendor applications open.

The project started with the Logitech G435, which does not expose its exact battery level through Logitech G HUB. A successful USB diagnostic on the actual receiver provided the battery voltage, and the application converts that voltage into an approximate percentage. Support was then extended to the VXE MAD R mouse using its device-reported battery level.

Both readings appear side by side next to the Windows notification area, using a transparent custom taskbar widget. A compact details window provides independent battery history for each device.

---

## ✨ Key Features

> **Development update:** the options below are implemented on main for the next release. The latest downloadable stable build remains v1.0.0 until the owner approves publication.

- 🎧 **Headset Battery** — Read Logitech G435 voltage through its LIGHTSPEED receiver and display an approximate percentage.
- 🖱️ **Mouse Battery** — Query the VXE MAD R receiver or USB cable for the reported percentage, voltage and charging status.
- 📊 **Dual Taskbar Widget** — Show the headset and mouse symbols with individual battery percentages, without a coloured background.
- 🎨 **Appearance** — Choose System, Mint, Blue or Violet widget colours, resize the widget from 80–130%, and select System/Light/Dark for the details window. Status colours retain their meaning and the taskbar adapts to Windows DPI.
- 🔄 **Independent Automatic Refresh** — Poll each device independently, normally every 60 seconds (adjustable from 15–300 seconds); retry unavailable devices and refresh charging devices every 15 seconds. Manual refresh remains available.
- 🔌 **Charging & Full Status** — MAD R shows a green percentage with a charging symbol, then Full at a device-reported 100%. A completion alert can be enabled. G435 is voltage-based, so charging cannot be confirmed from the receiver.
- 🔴 **Low Battery Notification** — Configurable threshold (5–50%), Windows notification-area alerts, and recovery hysteresis to avoid repeating alerts on every poll.
- ⚙️ **Settings Panel** — Configure startup, polling interval, low-battery and charge-complete alerts; preferences are applied and saved locally.
- 🟢 **Start with Windows** — Optional current-user sign-in startup, disabled by default.
- 🧩 **Single Instance** — One monitor and one details/settings window per Windows session.
- 🎛️ **Device Management** — Independently show or hide G435 and MAD R on the taskbar; hidden devices still collect history and alerts.
- ✨ **Charging Animation** — Optional subtle green pulse while a supported device charges; Full remains steady.
- 💤 **Sleep Handling** — Briefly retain the mouse's last reading, marked with a tilde and muted colour, when it stops responding; show an unavailable value after the grace period.
- 📈 **Battery History** — Choose a 3/6/12/24/48-hour chart, view min/max/net change, and obtain a regression-based remaining-time estimate only when sufficient uninterrupted discharge data is available.
- 🛡️ **Privacy** — Keep readings locally; no telemetry, external service, or firmware changes.
- 🎨 **Original App Icon** — A custom headset, mouse and charge emblem, embedded in the Windows executable and details window.
- 📦 **Portable EXE** — Run without installing Rust, .NET or G HUB.

---

## 🎮 Supported Devices

| Device | Connection | Hardware ID | Battery measurement |
|---|---|---|---|
| Logitech G435 LIGHTSPEED | 2.4 GHz USB receiver | `046D:0ADF` | Voltage-derived estimate |
| VXE MAD R | 2.4 GHz USB receiver | `373B:1040` | Device-reported percentage and charging flag |
| VXE MAD R | USB cable | `373B:103F` | Device-reported percentage and charging flag |

Bluetooth and the older G435 receiver (`046D:0ACB`) are not supported. Other MAD R receiver revisions may have different IDs; the initial implementation intentionally targets the known IDs and vendor HID collection rather than sending commands to unrelated devices.

---

## 🧠 OOP Concepts Applied

| Concept | Description |
|---|---|
| **Encapsulation** | Private fields in `BatteryReading`, `BatteryHistory` and `TaskbarWidget` hide internal state behind methods. |
| **Abstraction** | The `BatterySource` trait defines a common battery-reading interface. |
| **Polymorphism** | `G435Receiver` and `MadRReceiver` implement the same trait with device-specific protocols. |
| **Composition** | `TrayApp` coordinates the hardware adapters, independent history stores and UI components. |
| **Enum** | `DeviceKind`, `BatteryError` and `WidgetEvent` model supported devices, failures and UI actions. |
| **Error Handling** | `Result` and `Option` separate valid readings from disconnected or sleeping devices. |
| **Ownership & RAII** | Rust owns HID handles and native widget resources, with explicit cleanup for Windows resources. |

Rust uses structs, traits and composition in place of traditional class inheritance.

---

## 🛠️ Technologies & Tools

<p align="center">
  <img src="https://img.shields.io/badge/Cargo-Build_Tool-CE422B?style=for-the-badge&logo=rust&logoColor=white" alt="Cargo"/>
  <img src="https://img.shields.io/badge/Win32-Native_Taskbar-0078D4?style=for-the-badge" alt="Win32"/>
  <img src="https://img.shields.io/badge/HIDAPI-USB_Device_IO-3949AB?style=for-the-badge" alt="HIDAPI"/>
  <img src="https://img.shields.io/badge/egui-Desktop_UI-455A64?style=for-the-badge" alt="egui"/>
  <img src="https://img.shields.io/badge/GitHub_Actions-CI%2FCD-2088FF?style=for-the-badge&logo=githubactions&logoColor=white" alt="GitHub Actions"/>
</p>

The application uses Rust, Cargo, `hidapi`, `windows-sys`, `winit`, `tray-icon`, and `eframe/egui`. The native layered widget draws directly with Win32 GDI and does not inject code into Windows Explorer. The monitor itself runs without network access.

---

## 📂 Project Structure

```text
Wireless-Battery-Monitor/
├── .github/
│   └── workflows/
│       ├── build.yml
│       └── publish-v0.1.0.yml
├── src/
│   ├── app.rs
│   ├── battery.rs
│   ├── details.rs
│   ├── history.rs
│   ├── icon.rs
│   ├── lib.rs
│   ├── main.rs
│   ├── mouse.rs
│   ├── preferences.rs
│   ├── protocol.rs
│   ├── receiver.rs
│   ├── runtime.rs
│   ├── taskbar.rs
│   └── bin/
│       └── diagnostic.rs
├── tests/
│   └── protocol_capture.rs
├── BUILD.cmd
├── assets/
│   └── icon.svg
├── build.rs
├── Cargo.toml
├── THIRD-PARTY-NOTICES.txt
└── README.md
```

| Module | Responsibility |
|---|---|
| `battery.rs` | Shared battery model, device type, errors and source trait |
| `protocol.rs` | G435 HID framing, CRC and voltage parser |
| `receiver.rs` | G435 LIGHTSPEED hardware adapter |
| `mouse.rs` | MAD R battery protocol, hardware adapter and protocol tests |
| `history.rs` | Local per-device storage and discharge trend estimates |
| `taskbar.rs` | Native transparent two-device display |
| `app.rs` | Application lifecycle, independent polling and actions |
| `preferences.rs` | Validated settings, live device state and notification policy |
| `runtime.rs` | Windows startup preference, single-instance guard and native notifications |
| `details.rs` | Device selector, battery chart and details card |
| `icon.rs` | System tray fallback icon |
| `diagnostic.rs` | One-shot console reading for both supported devices |

---

## ▶️ How to Run

### Download

1. Open the [Releases](https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor/releases) page.
2. Download `wireless-battery-monitor.exe` from the [latest stable release](https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor/releases/latest).
3. Connect the supported USB receivers and switch on the peripherals.
4. Run the executable. The percentages appear on the taskbar when the devices respond.

Click the widget to open battery details. Right-click to refresh or exit. When the taskbar layout cannot be located, the standard notification-area icon remains available with both device statuses. The fallback may appear under the Windows hidden-icons arrow.

### Settings and Charging

Right-click the taskbar widget and select **Settings**, or open the details card and choose the Settings tab. Save changes to apply them within two seconds. Start with Windows is opt-in; keep the EXE in a permanent location and re-enable the setting after moving it. Settings are written to `%LOCALAPPDATA%\G435Battery\settings.json`.

The MAD R widget indicates charging with a green percentage and charging symbol, then displays **Full** when the mouse reports 100%. A single completion alert is emitted after a known charging session; Windows notification settings may suppress the notification. G435 still only reports battery voltage: an estimated 100% is explicitly labelled as an estimate in the details card, not verified charge completion.

Both devices reconnect automatically, with independent 15-second retries while unavailable. Device visibility, widget size and colour, charging animation, history period and window theme are adjustable from Settings. Old readings are marked stale and do not trigger alerts. The monitor prevents duplicate monitor instances in the same session.

### Build from Source

Prerequisites: Windows 10/11 x64, the [Rust stable toolchain](https://rustup.rs/), and the Microsoft Visual C++ build tools.

```powershell
cargo test --all-targets
cargo build --release --bins
```

The executable is `target\release\wireless-battery-monitor.exe`. Run `target\release\diagnostic.exe` for one-shot readings. Alternatively, double-click `BUILD.cmd`.

---

## 🧪 Testing

### Automated Protocol Tests

```bash
cargo test --all-targets
```

The tests check:

- All eight G435 requests against the captured exchange from the actual receiver.
- The captured 3861 mV reply, converted to an estimated 64%.
- Voltage limits and invalid or unrelated packets.
- MAD R request checksum, device-reported percentage, charging flag and rejection of asleep or unrelated reports.
- Remaining-time calculations using sustained discharge samples.
- Settings defaults, migration, appearance limits and JSON round-trip.
- History statistics and conservative regression-based discharge forecast.
- Low-battery alert hysteresis, disabled alerts, confirmed charging completion, disconnect handling and estimated-vs-reported full status.

### Hardware Validation

The original G435 diagnostic was performed on the actual `046D:0ADF` receiver. The v0.2.0 Windows release passed four automated tests, and the headset widget was confirmed by the user.

MAD R support is based on the documented battery-report format; the owner reported that v0.3.0 worked on their setup. The next version retains the same battery protocols and adds settings, independent reconnection, native alerts and charging/full status. Automated tests validate the logic; actual Windows notification delivery, startup and widget layout need real-device confirmation. Run `diagnostic.exe` to compare its reported percentage against ATK Gear. A missing response is not treated as a zero-percent battery.

---

## 📈 Battery Interpretation & Data

- Settings and the most recent live reading are stored locally. Start with Windows uses the standard current-user startup setting only when enabled by the user.

- The G435 percentage is an estimate from cell voltage and can vary with charging and load.
- MAD R reports its own percentage and charging flag; this is not calculated from the voltage curve.
- Data is saved locally in `%LOCALAPPDATA%\G435Battery\history.csv` and `mad-r-history.csv`, with up to 48 hours retained. Old G435 history is preserved.
- The dashboard charts 3, 6, 12, 24 or 48 hours; remaining time uses a fitted linear discharge trend only when recent samples support a sustained decline. Estimated runtime is not guaranteed.

---

## 🎯 Learning Objectives

- Apply idiomatic Rust OOP-style design through structs, traits and composition.
- Implement USB HID hardware interoperability and validate binary reports.
- Separate device-specific logic from domain models and UI components.
- Build a native Windows taskbar interface with per-device status indicators.
- Store local time-series readings and calculate conservative battery estimates.
- Automate testing, builds and versioned releases with GitHub Actions.

---

## 🔗 Technical References

- [Windows notification area API](https://learn.microsoft.com/windows/win32/api/shellapi/ns-shellapi-notifyicondataw) and [Windows sign-in startup](https://learn.microsoft.com/windows/win32/setupapi/run-and-runonce-registry-keys).

- [Windows resource embedding](https://docs.rs/winresource/) and [SVG rendering](https://docs.rs/resvg/) — the original vector icon is rendered during the build.

- [The Rust Programming Language — Object-Oriented Features](https://doc.rust-lang.org/book/ch17-01-what-is-oo.html)
- [HIDAPI Rust Documentation](https://docs.rs/hidapi/)
- [Microsoft Win32 Layered Windows](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features)
- [G435 battery protocol reference](https://github.com/portyporty/g435-battery-percentage) (see third-party notices).
- [MAD R protocol reference](https://github.com/dkbednarczyk/madr) and the provided protocol documentation.

This is an independent Rust implementation, not a clone or repackaging of another battery-monitor application.

---

## 👤 Developer

**Ahmed Gaber** — [SymbianDevAhmed](https://github.com/SymbianDevAhmed)

Unofficial project; not affiliated with Logitech, VXE or ATK.
