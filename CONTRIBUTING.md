# 🤝 Contributing

This is a private, owner-maintained project. Contributors need repository access.

## Development
- Use the Rust stable MSVC toolchain on Windows.
- Keep device-specific HID logic separate from the UI and domain model.
- Use the shared BatterySource trait and factory rather than duplicating protocol calls.
- Preserve the existing UI conventions, local-data design and device identity checks.
- Add tests for changes to report parsing, alert policy, settings or history calculations.

## Quality checks

```powershell
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets
```

If dependency changes are intentional, regenerate Cargo.lock and review the diff. Ordinary CI does not publish executables. The manually selected release workflow builds and packages the app only when the owner is ready.

## Reporting a bug

Include the app version, Windows version, device model/connection type, reproduction steps, expected and observed result. The Settings page can open local diagnostic logs. Review any material before sharing it; omit personal paths, USB serial numbers or unrelated system data.

## Hardware support

See [Device support](docs/DEVICE_SUPPORT.md) for the adapter contract and validation expectations.
