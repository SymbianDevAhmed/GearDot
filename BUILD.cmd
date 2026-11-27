@echo off
setlocal
cd /d "%~dp0"
where cargo >nul 2>&1
if errorlevel 1 (
  echo Install Rust from https://rustup.rs/ with the Windows MSVC build tools.
  pause
  exit /b 1
)
cargo test --locked --all-targets
if errorlevel 1 goto failed
cargo build --locked --release --bins
if errorlevel 1 goto failed
call vendor\wdbm-cpp\build.bat
if errorlevel 1 goto failed
copy /Y vendor\wdbm-cpp\BatteryHub.exe target\release\GearDot-DeviceProbe.exe >nul
if errorlevel 1 goto failed
powershell.exe -NoProfile -File installer\get-headsetcontrol.ps1
if errorlevel 1 goto failed
echo Build complete: target\release\wireless-battery-monitor.exe and GearDot-DeviceProbe.exe
pause
exit /b 0
:failed
echo Build failed. Please share the terminal output.
pause
exit /b 1
