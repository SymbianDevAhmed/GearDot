# 🔋 GearDot – Windows Battery Monitor

<p align="center">
  <img src="assets/geardot.svg" width="96" alt="GearDot logo">
</p>

<p align="center">
  A Rust desktop application for checking the battery level of connected mice, keyboards, headsets, and controllers on Windows.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4?style=for-the-badge" alt="Windows 10 and 11 x64">
  <img src="https://img.shields.io/badge/Rust-Desktop_App-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <a href="https://github.com/SymbianDevAhmed/GearDot/releases/latest"><img src="https://img.shields.io/badge/GitHub-Download-181717?style=for-the-badge&logo=github&logoColor=white" alt="Download from GitHub"></a>
</p>

---

## 📌 About the Project

**GearDot** brings battery information for compatible peripherals together in the Windows notification area. Click the tray icon to see connected devices, or enable the screen-edge slider to keep supported readings close at hand.

Battery information is read locally. The available percentage and charging details depend on the device and how it connects.

---

## ✨ Key Features

- 🔌 **Connected Devices** — View peripherals that Windows currently detects.
- 🔋 **Battery Status** — See percentages, level estimates, or charging status where the device provides them.
- 📍 **Screen-Edge Slider** — See battery rings and hover for device details.
- 🔔 **Battery Alerts** — Set a low-battery threshold and temporarily mute alerts from **Controls**.
- 🧾 **Diagnostics** — Copy a report of detected devices and the battery information available for each one.

---

## 🎮 Device Support

| Device or connection | Available reading |
|---|---|
| **VXE MAD R** on supported USB connections | Device-reported percentage and charging status |
| **Logitech G435** with a supported LIGHTSPEED receiver | Percentage estimated from voltage |
| **DualSense and DualSense Edge** over USB | Estimated percentage from reported battery levels |
| **Windows Bluetooth** and **XInput** | Battery information when Windows or the controller provides it |
| **Other USB and audio devices** | Device detection and connection information when available |

---

## ▶️ How to Run

Open [GearDot Releases](https://github.com/SymbianDevAhmed/GearDot/releases/latest). Choose the Installer package if you want a Start menu shortcut, or choose Portable to run GearDot from an extracted folder.

For Portable, extract the ZIP into a writable folder and open `GearDot.exe`. Keep `portable.flag` beside the executable. Click the GearDot icon in the Windows notification area to open the app.

---

## 🛠️ Technologies & Tools

<p align="center">
  <img src="https://img.shields.io/badge/Cargo-Build_Tool-8B4513?style=for-the-badge&logo=rust&logoColor=white" alt="Cargo">
  <img src="https://img.shields.io/badge/Windows-Device_APIs-0078D4?style=for-the-badge" alt="Windows device APIs">
  <img src="https://img.shields.io/badge/GitHub_Actions-CI-2088FF?style=for-the-badge&logo=githubactions&logoColor=white" alt="GitHub Actions">
</p>

---

> [!NOTE]
> GearDot is an unofficial project and is not affiliated with Logitech, VXE, or ATK. See [Third-party notices](THIRD-PARTY-NOTICES.txt).

---

## 👨‍💻 Author

**Ahmed Gaber**
