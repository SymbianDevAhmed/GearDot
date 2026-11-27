<div align="center">
  <img src="assets/geardot.svg" width="96" alt="GearDot symbol">
  <h1>GearDot</h1>
  <p><strong>See what needs charging, right from Windows.</strong></p>
  <a href="https://github.com/SymbianDevAhmed/GearDot/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/SymbianDevAhmed/GearDot?label=Release&color=9656C7"></a>
  <img alt="Windows x64" src="https://img.shields.io/badge/Platform-Windows%20x64-30313D">
  <img alt="Built with Rust" src="https://img.shields.io/badge/Built%20with-Rust-30313D">
</div>

## Download

Get **Setup** or **Portable** from the [latest release](https://github.com/SymbianDevAhmed/GearDot/releases/latest). Setup adds a Start menu shortcut. For Portable, extract the ZIP and keep `portable.flag` beside `GearDot.exe`.

## What GearDot does

- Shows connected mice, keyboards, headsets and game controllers.
- Opens the device list from the Windows notification area.
- Shows battery rings and device details in an optional screen-edge widget.
- Offers battery alerts, temporary mute and a copyable diagnostics report.

## Battery support

| Device or connection | Available reading |
|---|---|
| VXE MAD R on supported USB connections | Reported percentage and charging status |
| Logitech G435 on a supported LIGHTSPEED receiver | Voltage-based estimate |
| DualSense and DualSense Edge over USB | Estimate from reported battery levels |
| Windows Bluetooth and XInput | Battery information when Windows or the controller exposes it |
| Other wired USB and audio devices | Detection and device information when available |

GearDot monitors supported devices locally. Available battery data depends on the model and connection.

> Independent project by Ahmed Gaber. Unaffiliated with Logitech, VXE, ATK, or other device makers. See [third-party notices](THIRD-PARTY-NOTICES.txt).
