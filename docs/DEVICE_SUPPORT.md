# 🎮 Device support and architecture

| Device | USB ID | Battery source |
|---|---|---|
| Logitech G435 LIGHTSPEED | 046D:0ADF | Voltage-derived estimate |
| VXE MAD R receiver | 373B:1040 | Reported percentage and charging flag |
| VXE MAD R cable | 373B:103F | Reported percentage and charging flag |

## Adapter contract

The domain-level BatterySource trait returns a BatteryReading or structured BatteryError. A device adapter owns hardware matching and report parsing, without knowledge of the taskbar or preferences. The factory in devices.rs selects the adapter used by both monitor and diagnostic. DeviceKind identifies the supported model; a custom display name never changes hardware matching.

## Adding a supported peripheral

1. Establish the model and connection mode, including the exact hardware ID.
2. Identify a documented or independently observed read-only battery-report interface.
3. Build an isolated adapter implementing BatterySource, validating report identity, lengths and ranges; do not change firmware, pairing, DPI, lighting or other device settings.
4. Add the adapter to the factory and extend the device model, preferences and UI where required.
5. Add deterministic parser tests and compare the reading against the actual device or its vendor UI.
6. Document the distinction between reported data, estimates, unavailable charging status and connection failures.

## Validation

A failed read is not 0% and stale samples do not trigger alerts. Charging completion requires a confirmed level and an observed charging session. Never infer an exact charge state from G435 voltage alone.

## Connected-device catalogue

[`assets/devices/catalog.json`](../assets/devices/catalog.json) maps an exact USB
VID:PID to a display name, kind, receiver flag and optional built-in reader. A
catalogue row is applied only when Windows enumerates matching hardware. A row
without a reader improves naming and connection classification only. A detected
USB receiver can remain visible even when Windows cannot confirm the state of
its paired peripheral.

DualSense and DualSense Edge use passive USB reports. GearDot does not configure
the controller. It labels percentages derived from the reported charge buckets
as estimates. The XInput path keeps its discrete levels.

For a missing device, copy the report in Diagnostics and note the connection
mode. The report includes USB vendor and product IDs when available and leaves
out serial numbers and device paths. Confirm a reader against the hardware
before calling its battery behaviour tested. Do not extend a vendor reader to
a shared ID without checking how to distinguish the models.

References for matching and protocol research:

- [W.D.B.M supported devices](https://github.com/SymbianDevAhmed/W.D.B.M#supported-devices)
- [Wireless-Device-Battery-Monitor supported devices](https://github.com/SymbianDevAhmed/Wireless-Device-Battery-Monitor#supported-devices)
- [Joule device verification notes](https://github.com/SymbianDevAhmed/Joule/blob/HEAD/docs/supported-devices.md)
- [SDL DualSense report layout](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_ps5.c)
