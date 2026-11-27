# 🔋 GearDot – Windows Battery Monitor

<p align="center">
  <img src="assets/geardot.svg" width="96" alt="GearDot logo">
</p>

<p align="center">
  Battery readings for connected mice, keyboards, headsets, earbuds, and controllers on Windows.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4?style=for-the-badge" alt="Windows 10 and 11 x64">
  <img src="https://img.shields.io/badge/Rust-Desktop_App-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/License-GPL--3.0--or--later-blue?style=for-the-badge" alt="GPL 3.0 or later">
  <a href="https://github.com/SymbianDevAhmed/GearDot/releases/latest"><img src="https://img.shields.io/badge/GitHub-Download-181717?style=for-the-badge&logo=github&logoColor=white" alt="Download from GitHub"></a>
</p>

---

## 📌 About the Project

**GearDot** is a Windows desktop utility that keeps battery information for compatible peripherals in one place. Open the device list from the notification area, show battery percentages next to the taskbar, or check them in the screen-edge slider. GearDot reads device information locally through Windows APIs and bundled device readers.

---

## ✨ Key Features

- 🔌 **Connected devices** — See which devices GearDot has detected and how they are connected. The list updates as devices connect and disconnect.
- 🔋 **Battery information** — View a reported percentage, an estimate, or a coarse level depending on the device. GearDot shows charging and full-charge states when it can determine them.
- 📍 **Taskbar and edge slider** — Choose where each device appears. Hover over the slider to see the device name and battery details.
- 🔔 **Battery alerts** — Configure low-battery and full-charge notifications, or mute alerts temporarily from **Controls**.
- ⚙️ **Device profiles** — Rename a device in GearDot, pin it to the top, and control where it appears.
- 🧾 **Diagnostics** — Refresh the device list and copy a report when a device is missing or a reading looks wrong.

---

## 🎮 Device Support

| Category | Examples and connection modes |
|---|---|
| **Mice** | VXE MAD R over a compatible USB cable or receiver, plus models handled by the BatteryHub and W.D.B.M readers |
| **Keyboards** | USB, 2.4 GHz receiver, and Bluetooth models that expose battery information to GearDot |
| **Headsets and earbuds** | Logitech G435 with the `046D:0ADF` LIGHTSPEED receiver, supported Bluetooth audio devices, and supported USB headsets via the bundled readers |
| **Game controllers** | DualSense and DualSense Edge over USB, XInput controllers with battery reporting, and models handled by the bundled readers |

Device support depends on the exact model, firmware, and connection. A device can appear without a battery reading when it uses wired power or does not expose battery information through a supported interface.

| Display | Meaning |
|---|---|
| **Percentage** | The device, Windows, or a bundled reader supplied a percentage |
| **Estimated percentage** | A voltage reading or a device battery level was converted to a percentage, as with the supported G435 receiver |
| **Level only** | The device supplied a coarse value such as Low, Medium, or Full |
| **Battery unavailable** | GearDot detected the device and could not obtain a supported battery reading |

The complete driver source trees are included under [`vendor/`](vendor/README.md). The upstream projects document model support and prerequisites. [BatteryHub](https://github.com/ferloom/Wireless-Device-Battery-Monitor) · [W.D.B.M](https://github.com/SymbianDevAhmed/W.D.B.M) · [HeadsetControl](https://github.com/Sapd/HeadsetControl#supported-headsets)

---

## 📥 Download and Run

Download a Windows x64 package from [GitHub Releases](https://github.com/SymbianDevAhmed/GearDot/releases/latest).

| Package | How to use it |
|---|---|
| **EXE installer** | Run `GearDot-Setup-x64.exe` and open **GearDot** from the Start menu |
| **MSI installer** | Run `GearDot-Setup-x64.msi` if you prefer Windows Installer |
| **Portable ZIP** | Extract the full ZIP to a writable folder and run `GearDot.exe` |

Choose one installer. For Portable, keep `portable.flag`, `GearDot-DeviceProbe.exe`, and `HeadsetControl.exe` alongside `GearDot.exe`. If an earlier test build is running, exit it from the notification area first. Windows Installer may require uninstalling a higher-numbered test MSI before installing the first stable release.

---

## ▶️ Using GearDot

1. Connect your device over USB, its wireless receiver, or Bluetooth.
2. Click the **GearDot** icon in the notification area to open **Devices**. Use the refresh control to scan again.
3. Open **Controls** for taskbar, edge slider, and alert options. Open **Settings → Device profiles** to choose where each device appears.
4. For a missing device or incorrect reading, open **Diagnostics → Copy diagnostic report**. Include the model, connection type, and expected battery reading when [opening an issue](https://github.com/SymbianDevAhmed/GearDot/issues).

---

## 🛠️ Technologies & Tools

<p align="center">
  <img src="https://img.shields.io/badge/Rust-Cargo-8B4513?style=for-the-badge&logo=rust&logoColor=white" alt="Rust and Cargo">
  <img src="https://img.shields.io/badge/C%2B%2B-Device_Readers-00599C?style=for-the-badge&logo=cplusplus&logoColor=white" alt="C++ device readers">
  <img src="https://img.shields.io/badge/Windows-PnP%20%2F%20HID%20%2F%20Bluetooth-0078D4?style=for-the-badge" alt="Windows PnP, HID, and Bluetooth">
  <img src="https://img.shields.io/badge/GitHub_Actions-CI-2088FF?style=for-the-badge&logo=githubactions&logoColor=white" alt="GitHub Actions">
</p>

| Path | Contents |
|---|---|
| [`src/`](src/) | Windows app, device detection, reader integration, and diagnostics |
| [`vendor/`](vendor/) | BatteryHub and W.D.B.M sources and integration notes |
| [`installer/`](installer/) | EXE, MSI, and Portable packaging files |
| [`tests/`](tests/) | Protocol fixtures and automated checks |

To build from source on Windows, install Rust 1.99 or newer, Visual Studio Build Tools with the C++ workload, and MinGW-w64 on your PATH. Run `BUILD.cmd` from the repository root. See [contributing notes](CONTRIBUTING.md), [packaging instructions](installer/README.md), and the [changelog](CHANGELOG.md).

---

## 📄 Licence and Credits

GearDot is released under the [GNU General Public License v3 or later](LICENSE). See the [third-party notices](THIRD-PARTY-NOTICES.txt) and [vendored source revisions](vendor/README.md) for BatteryHub, W.D.B.M, and HeadsetControl.

> [!NOTE]
> Unofficial project. GearDot is not affiliated with Logitech, VXE, or ATK.

---

## 👨‍💻 Author

**Ahmed Gaber**
