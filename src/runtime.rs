//! Current-user Windows integration. Startup is explicitly controlled by Settings.
use std::{collections::VecDeque, ffi::OsStr, io, mem::{size_of, zeroed}, os::windows::ffi::OsStrExt, ptr::{null, null_mut}, time::{Duration, Instant}};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, HWND, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND},
    System::{LibraryLoader::GetModuleHandleW, Registry::*, Threading::CreateMutexW},
    UI::{Shell::*, WindowsAndMessaging::*},
};

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> { s.as_ref().encode_wide().chain([0]).collect() }

pub struct InstanceGuard(HANDLE);
impl InstanceGuard {
    pub fn acquire(window: bool) -> io::Result<Option<Self>> {
        let name = wide(if window { r"Local\WirelessBatteryMonitor.Details" } else { r"Local\WirelessBatteryMonitor.Tray" });
        unsafe {
            let handle = CreateMutexW(null(), 0, name.as_ptr());
            if handle.is_null() { return Err(io::Error::last_os_error()); }
            if GetLastError() == ERROR_ALREADY_EXISTS { CloseHandle(handle); return Ok(None); }
            Ok(Some(Self(handle)))
        }
    }
}
impl Drop for InstanceGuard { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }

pub fn system_light_theme() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value = 0u32;
    let mut length = size_of::<u32>() as u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD,
        null_mut(), (&mut value as *mut u32).cast(), &mut length) == 0 && value != 0 }
}

pub fn focus_details() {
    unsafe {
        let title = wide("Wireless Battery Monitor — Details & Settings");
        let hwnd = FindWindowW(null(), title.as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "WirelessBatteryMonitor";

fn startup_command() -> io::Result<Vec<u16>> {
    let exe = std::env::current_exe()?;
    let mut command = vec![b'"' as u16];
    command.extend(exe.as_os_str().encode_wide());
    command.extend([b'"' as u16, 0]);
    if command.len() > 260 { return Err(io::Error::new(io::ErrorKind::InvalidInput, "Move the EXE to a shorter path before enabling startup.")); }
    Ok(command)
}

pub fn startup_enabled() -> io::Result<bool> {
    let key = wide(RUN_KEY);
    let name = wide(RUN_NAME);
    let mut bytes = 0u32;
    unsafe {
        let result = RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ,
            null_mut(), null_mut(), &mut bytes);
        if result == ERROR_FILE_NOT_FOUND { return Ok(false); }
        if result != 0 { return Err(io::Error::from_raw_os_error(result as i32)); }
        if bytes > 4096 || bytes % 2 != 0 { return Ok(false); }
        let mut data = vec![0u16; bytes as usize / 2];
        let result = RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ,
            null_mut(), data.as_mut_ptr().cast(), &mut bytes);
        if result != 0 { return Err(io::Error::from_raw_os_error(result as i32)); }
        Ok(data == startup_command()?)
    }
}

pub fn set_startup(enabled: bool) -> io::Result<()> {
    let key = wide(RUN_KEY);
    let name = wide(RUN_NAME);
    let result = unsafe {
        if enabled {
            let command = startup_command()?;
            RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ,
                command.as_ptr().cast(), (command.len() * 2) as u32)
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr())
        }
    };
    if result == 0 || (!enabled && result == ERROR_FILE_NOT_FOUND) { Ok(()) }
    else { Err(io::Error::from_raw_os_error(result as i32)) }
}

/// Native notification-area balloon; respects Windows notification preferences.
pub struct Notifier { hwnd: HWND, until: Option<Instant>, pending: VecDeque<(String, String, bool)> }
impl Notifier {
    pub fn new() -> Self {
        let hwnd = unsafe {
            CreateWindowExW(WS_EX_TOOLWINDOW, wide("STATIC").as_ptr(), wide("Wireless Battery Monitor").as_ptr(),
                WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), GetModuleHandleW(null()), null())
        };
        Self { hwnd, until: None, pending: VecDeque::new() }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        unsafe { NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..zeroed() } }
    }
    pub fn show(&mut self, title: &str, body: &str, full: bool) {
        if self.hwnd.is_null() { return; }
        self.pending.push_back((title.to_owned(), body.to_owned(), full));
        self.tick();
    }
    fn display(&mut self, title: &str, body: &str, full: bool) {
        unsafe {
            let mut data = self.data();
            data.uFlags = NIF_ICON | NIF_TIP;
            data.hIcon = LoadIconW(GetModuleHandleW(null()), 1usize as *const u16);
            fill_wide(&mut data.szTip, "Wireless Battery Monitor");
            // Re-register if Explorer has restarted or the previous notice expired.
            if Shell_NotifyIconW(NIM_MODIFY, &data) == 0 && Shell_NotifyIconW(NIM_ADD, &data) == 0 { return; }
            data.uFlags = NIF_INFO;
            data.dwInfoFlags = if full { NIIF_INFO } else { NIIF_WARNING };
            fill_wide(&mut data.szInfoTitle, title);
            fill_wide(&mut data.szInfo, body);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            self.until = Some(Instant::now() + Duration::from_secs(25));
        }
    }
    pub fn tick(&mut self) {
        if self.until.is_some_and(|until| Instant::now() >= until) {
            self.remove();
        }
        if self.until.is_none() {
            if let Some((title, body, full)) = self.pending.pop_front() {
                self.display(&title, &body, full);
            }
        }
    }
    fn remove(&mut self) {
        if !self.hwnd.is_null() { unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data()); } }
        self.until = None;
    }
}
impl Drop for Notifier {
    fn drop(&mut self) {
        self.remove();
        if !self.hwnd.is_null() { unsafe { DestroyWindow(self.hwnd); } }
    }
}
fn fill_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    target.fill(0);
    for (dst, value) in target.iter_mut().take(N.saturating_sub(1)).zip(value.encode_utf16()) { *dst = value; }
}
