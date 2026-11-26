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
echo Build complete: target\release\wireless-battery-monitor.exe
pause
exit /b 0
:failed
echo Build failed. Please share the terminal output.
pause
exit /b 1
