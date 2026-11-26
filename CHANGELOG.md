# 📋 Changelog

All notable changes to Wireless Battery Monitor are documented here.

## [Unreleased] — next version

### Added
- Persistent settings, optional sign-in startup, per-device polling and reconnection.
- Windows low-battery and charge-complete notifications with configurable threshold and test action.
- Confirmed MAD R Charging and Full indication, optional charging animation.
- Device visibility, custom names, widget scaling and colour presets.
- System, dark and light details-window themes.
- History ranges, min/max/net change and regression-based runtime estimation.
- About page, browser link to releases, local diagnostic logs and settings backup/restore.
- Per-user Windows installer source with Start menu shortcuts and uninstaller.
- Original executable version metadata, architecture and contributor documentation.
- Portable local-data mode, private package builds, independent device profiles, sleep/wake recovery, power saving, taskbar monitor and offset preferences, sound preference and CSV history export.
- Source formatting, Clippy checks and dependency locking in CI.

### Changed
- Separated device construction from UI and diagnostic entry point using a shared device factory.
- Push workflows test and check source; portable and installer packages are produced on explicitly requested private-package or release runs.

### Limitations
- G435 reports voltage, not a confirmed charge-state flag.
- Windows notification delivery, startup, installer and layout require real-device smoke tests.

## [1.0.0] — 2026-11-26

- First stable Windows x64 release, original embedded icon, dual-device widget, independent battery history, CI and SHA-256 checksums.

## [0.3.0] — 2026-11-26

- Added MAD R support, device-specific status and an expanded widget.

## [0.2.0] — 2026-11-26

- Introduced the transparent G435 taskbar widget and battery details history.

## [0.1.0] — 2026-11-25

- Initial Rust protocol and voltage-derived battery monitor for the G435.

[Unreleased]: https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor/compare/v1.0.0...main
[1.0.0]: https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor/releases/tag/v1.0.0
[0.3.0]: https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor/releases/tag/v0.3.0
[0.2.0]: https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor/releases/tag/v0.2.0
[0.1.0]: https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor/releases/tag/v0.1.0
