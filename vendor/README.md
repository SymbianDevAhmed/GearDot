# Battery readers

GearDot is distributed under GPL-3.0-or-later. The vendored projects retain their own licence notices.

| Directory | Upstream revision |
|---|---|
| `batteryhub-rust` | [ferloom/Wireless-Device-Battery-Monitor at 49c9063](https://github.com/ferloom/Wireless-Device-Battery-Monitor/tree/49c9063b10ebbc19f3313d12bff402814303a6a7) |
| `wdbm-cpp` | [SymbianDevAhmed/W.D.B.M at fd2267d](https://github.com/SymbianDevAhmed/W.D.B.M/tree/fd2267d770c9dafc46047f715e79da525e3fecf4) |

GearDot invokes the Rust library in a bounded probe mode of its own executable. The C++ project is built from the included source and installed alongside it as `GearDot-DeviceProbe.exe`. Readers run sequentially on a background worker. GearDot's built-in MAD R reader owns its USB IDs. For the G435 receiver `046D:0ADF`, GearDot tries its built-in reader first, then runs the vendored Rust and C++ G435 readers as isolated fallbacks. The older `046D:0ACB` bootloader-reading path is kept out of the polling flow because it interrupts audio. The C++ adapter emits hardware IDs where its driver records both IDs. GearDot runs HeadsetControl separately so that it can bound its execution and merge by USB ID.

Both pinned upstream driver directories are present in full (89 files from BatteryHub and 113 from W.D.B.M, including shared protocol files and headers). The C++ probe exposes a full JSON scan and a G435-only JSON scan. Its Phone Link reader contributes a cache entry to Diagnostics, labelled as connection unconfirmed. That entry never enters the live device list or generates battery alerts.

The upstream support tables are in the [BatteryHub README](batteryhub-rust/README.md) and the [W.D.B.M README](wdbm-cpp/README.md). They include models that require a specific firmware, connection mode, or vendor application. GearDot limits AirPods readings to devices that Windows reports as connected. GearDot currently shows one reading per device, including the combined reading for earbuds.

## HeadsetControl

GearDot uses the [HeadsetControl 4.3.0](https://github.com/Sapd/HeadsetControl/releases/tag/4.3.0) Windows x64 executable as a battery fallback. The packaging script pins SHA-256 digests for the binary and the corresponding source. GearDot starts the bundled executable by absolute path with the battery and JSON options. Some models only report whether they are charging; GearDot requires a valid percentage before displaying a fallback reading.
