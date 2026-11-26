# 🎮 Device support and architecture

| Device | USB ID | Battery source |
|---|---|---|
| Logitech G435 LIGHTSPEED | 046D:0ADF | Voltage-derived estimate |
| VXE MAD R receiver | 373B:1040 | Reported percentage and charging flag |
| VXE MAD R cable | 373B:103F | Reported percentage and charging flag |

## Adapter contract

The domain-level BatterySource trait returns a BatteryReading or structured BatteryError. A device adapter owns hardware matching and report parsing, without knowledge of the taskbar, preferences or history. The factory in devices.rs selects the adapter used by both monitor and diagnostic. DeviceKind identifies the supported model; a custom display name never changes hardware matching.

## Adding a supported peripheral

1. Establish the model and connection mode, including the exact hardware ID.
2. Identify a documented or independently observed read-only battery-report interface.
3. Build an isolated adapter implementing BatterySource, validating report identity, lengths and ranges; do not change firmware, pairing, DPI, lighting or other device settings.
4. Add the adapter to the factory and extend the device model, preferences and UI where required.
5. Add deterministic parser tests and compare the reading against the actual device or its vendor UI.
6. Document the distinction between reported data, estimates, unavailable charging status and connection failures.

## Validation

A failed read is not 0% and stale samples do not trigger alerts. Battery remaining time is only a trend estimate, not a measure of cell health. Charging completion requires a confirmed level and an observed charging session. Never infer an exact charge state from G435 voltage alone.
