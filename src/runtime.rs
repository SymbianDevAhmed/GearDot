//! Current-user Windows integration. Startup is explicitly controlled by Settings.
use std::{
    collections::VecDeque,
    ffi::OsStr,
    io,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, HANDLE, HWND,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::*,
        Threading::{CreateMutexW, OpenMutexW, MUTEX_MODIFY_STATE},
    },
    UI::{Shell::*, WindowsAndMessaging::*},
};

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain([0]).collect()
}

pub const DETAILS_TITLE: &str = "GearDot — Details & Settings";
const MONITOR_MUTEX: &str = r"Local\WirelessBatteryMonitor.Tray";

pub struct InstanceGuard(HANDLE);
impl InstanceGuard {
    pub fn acquire(window: bool) -> io::Result<Option<Self>> {
        Self::acquire_named(if window {
            r"Local\WirelessBatteryMonitor.Details"
        } else {
            MONITOR_MUTEX
        })
    }

    fn acquire_named(name: &str) -> io::Result<Option<Self>> {
        let name = wide(name);
        unsafe {
            let handle = CreateMutexW(null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return Ok(None);
            }
            Ok(Some(Self(handle)))
        }
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn monitor_running() -> bool {
    unsafe {
        let handle = OpenMutexW(MUTEX_MODIFY_STATE, 0, wide(MONITOR_MUTEX).as_ptr());
        if handle.is_null() {
            return false;
        }
        CloseHandle(handle);
        true
    }
}

pub fn system_light_theme() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value = 0u32;
    let mut length = size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut length,
        ) == 0
            && value != 0
    }
}

pub fn focus_details() -> bool {
    unsafe {
        let title = wide(DETAILS_TITLE);
        let hwnd = FindWindowW(null(), title.as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
            true
        } else {
            false
        }
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "WirelessBatteryMonitor";

fn startup_command(background: bool) -> io::Result<Vec<u16>> {
    let exe = std::env::current_exe()?;
    let mut command = vec![b'"' as u16];
    command.extend(exe.as_os_str().encode_wide());
    command.push(b'"' as u16);
    if background {
        command.extend(" --background".encode_utf16());
    }
    command.push(0);
    if command.len() > 260 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Move the EXE to a shorter path before enabling startup.",
        ));
    }
    Ok(command)
}

pub fn startup_enabled() -> io::Result<bool> {
    if wireless_battery_monitor::preferences::is_portable() {
        return Ok(false);
    }
    let key = wide(RUN_KEY);
    let name = wide(RUN_NAME);
    let mut bytes = 0u32;
    unsafe {
        let result = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut bytes,
        );
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        if bytes > 4096 || bytes % 2 != 0 {
            return Ok(false);
        }
        let mut data = vec![0u16; bytes as usize / 2];
        let result = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            data.as_mut_ptr().cast(),
            &mut bytes,
        );
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        Ok(data == startup_command(true)? || data == startup_command(false)?)
    }
}

pub fn set_startup(enabled: bool) -> io::Result<()> {
    if wireless_battery_monitor::preferences::is_portable() {
        return if enabled {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Startup is disabled in portable mode.",
            ))
        } else {
            Ok(())
        };
    }
    let key = wide(RUN_KEY);
    let name = wide(RUN_NAME);
    let result = unsafe {
        if enabled {
            let command = startup_command(true)?;
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr())
        }
    };
    if result == 0 || (!enabled && result == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

/// Native notification-area balloon; respects Windows notification preferences.
pub struct Notifier {
    hwnd: HWND,
    until: Option<Instant>,
    pending: VecDeque<(String, String, bool, bool)>,
}
impl Notifier {
    pub fn new() -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                wide("GearDot").as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        Self {
            hwnd,
            until: None,
            pending: VecDeque::new(),
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        unsafe {
            NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                ..zeroed()
            }
        }
    }
    pub fn show(&mut self, title: &str, body: &str, full: bool, sound: bool) {
        if self.hwnd.is_null() {
            return;
        }
        self.pending
            .push_back((title.to_owned(), body.to_owned(), full, sound));
        self.tick();
    }
    fn display(&mut self, title: &str, body: &str, full: bool, sound: bool) {
        unsafe {
            let mut data = self.data();
            data.uFlags = NIF_ICON | NIF_TIP;
            data.hIcon = LoadIconW(GetModuleHandleW(null()), 1usize as *const u16);
            fill_wide(&mut data.szTip, "GearDot");
            // Re-register if Explorer has restarted or the previous notice expired.
            if Shell_NotifyIconW(NIM_MODIFY, &data) == 0 && Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                return;
            }
            data.uFlags = NIF_INFO;
            data.dwInfoFlags = (if full { NIIF_INFO } else { NIIF_WARNING })
                | if sound { 0 } else { NIIF_NOSOUND };
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
            if let Some((title, body, full, sound)) = self.pending.pop_front() {
                self.display(&title, &body, full, sound);
            }
        }
    }
    pub fn clear(&mut self) {
        self.pending.clear();
        self.remove();
    }
    fn remove(&mut self) {
        if !self.hwnd.is_null() {
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
        }
        self.until = None;
    }
}
impl Drop for Notifier {
    fn drop(&mut self) {
        self.remove();
        if !self.hwnd.is_null() {
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }
}
fn fill_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    target.fill(0);
    for (dst, value) in target
        .iter_mut()
        .take(N.saturating_sub(1))
        .zip(value.encode_utf16())
    {
        *dst = value;
    }
}

pub fn animations_enabled() -> bool {
    let mut enabled = 1i32;
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    read == 0 || enabled != 0
}

pub fn transparency_enabled() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("EnableTransparency");
    let mut value = 1u32;
    let mut length = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut length,
        )
    };
    result != 0 || value != 0
}

pub fn details_hwnd(frame: &eframe::Frame) -> Option<HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::Win32(handle) = frame.window_handle().ok()?.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get() as HWND)
}

pub fn keep_details_frameless(hwnd: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let caption = (WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize;
        if style & caption != 0 {
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !caption);
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

pub fn set_composition(hwnd: HWND, glass: bool, light: bool, transparency: u8) -> bool {
    use windows_sys::Win32::Graphics::Dwm::*;

    let set = |attribute: DWMWINDOWATTRIBUTE, value: i32| unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute as u32,
            (&value as *const i32).cast(),
            size_of::<i32>() as u32,
        ) >= 0
    };
    // The client draws its own rounded silhouette.
    let _ = set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND);
    let _ = set(DWMWA_BORDER_COLOR, -2);
    let _ = set(DWMWA_USE_IMMERSIVE_DARK_MODE, i32::from(!light));
    if !glass {
        set_compositor_acrylic(hwnd, None);
        return false;
    }

    let alpha = ((100 - u16::from(transparency.min(85))) * 255 / 100) as u8;
    let tint = if light {
        [225, 235, 247, alpha]
    } else {
        [22, 29, 43, alpha]
    };
    set_compositor_acrylic(hwnd, Some(tint))
}

/// Optional compatibility path for Windows builds without the system backdrop
/// attribute, or for windows on which it does not produce a visible material.
fn set_compositor_acrylic(hwnd: HWND, tint: Option<[u8; 4]>) -> bool {
    use std::ffi::c_void;
    use windows_sys::Win32::System::LibraryLoader::GetProcAddress;

    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        tint: u32,
        animation: u32,
    }

    #[repr(C)]
    struct CompositionAttribute {
        id: u32,
        data: *mut c_void,
        bytes: usize,
    }

    type Setter = unsafe extern "system" fn(HWND, *mut CompositionAttribute) -> i32;
    let user32 = wide("user32.dll");
    let module = unsafe { GetModuleHandleW(user32.as_ptr()) };
    if module.is_null() {
        return false;
    }
    let Some(symbol) =
        (unsafe { GetProcAddress(module, b"SetWindowCompositionAttribute\0".as_ptr()) })
    else {
        return false;
    };
    let setter: Setter = unsafe { std::mem::transmute(symbol) };

    let mut policy = AccentPolicy {
        state: if tint.is_some() { 4 } else { 0 },
        flags: 0,
        tint: tint.map(u32::from_le_bytes).unwrap_or_default(),
        animation: 0,
    };
    let mut attribute = CompositionAttribute {
        id: 19,
        data: (&mut policy as *mut AccentPolicy).cast(),
        bytes: size_of::<AccentPolicy>(),
    };
    (unsafe { setter(hwnd, &mut attribute) }) != 0
}

/// Keep both native window icons at the system's DPI-specific icon metrics.
#[derive(Default)]
pub struct WindowIcons {
    hwnd: HWND,
    metrics: [i32; 4],
    icons: [HICON; 2],
    original: [HICON; 2],
}

impl WindowIcons {
    pub fn refresh(&mut self, frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};

        let Ok(handle) = frame.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let hwnd = handle.hwnd.get() as HWND;
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let metrics = [SM_CXSMICON, SM_CYSMICON, SM_CXICON, SM_CYICON]
            .map(|metric| unsafe { GetSystemMetricsForDpi(metric, dpi) });
        if self.hwnd == hwnd && self.metrics == metrics {
            return;
        }
        if metrics.iter().any(|&size| size <= 0) {
            return;
        }

        let module = unsafe { GetModuleHandleW(null()) };
        if module.is_null() {
            return;
        }
        let load = |width, height| unsafe {
            // Resource ID 1 is the executable's multi-resolution application icon.
            LoadImageW(module, 1usize as *const u16, IMAGE_ICON, width, height, 0) as HICON
        };
        let icons = [load(metrics[0], metrics[1]), load(metrics[2], metrics[3])];
        if icons.iter().any(|icon| icon.is_null()) {
            destroy_icons(icons);
            return;
        }

        if self.hwnd != hwnd {
            self.restore();
            self.hwnd = hwnd;
            self.original = [null_mut(); 2];
        }
        for (index, kind) in [ICON_SMALL, ICON_BIG].into_iter().enumerate() {
            let previous =
                unsafe { SendMessageW(hwnd, WM_SETICON, kind as usize, icons[index] as isize) }
                    as HICON;
            if self.icons[index].is_null() {
                self.original[index] = previous;
            }
        }
        destroy_icons(std::mem::replace(&mut self.icons, icons));
        self.metrics = metrics;
    }

    fn restore(&mut self) {
        if self.hwnd.is_null() {
            return;
        }
        if unsafe { IsWindow(self.hwnd) } != 0 {
            for (index, kind) in [ICON_SMALL, ICON_BIG].into_iter().enumerate() {
                unsafe {
                    SendMessageW(
                        self.hwnd,
                        WM_SETICON,
                        kind as usize,
                        self.original[index] as isize,
                    );
                }
            }
        }
        destroy_icons(std::mem::replace(&mut self.icons, [null_mut(); 2]));
        self.hwnd = null_mut();
    }
}

impl Drop for WindowIcons {
    fn drop(&mut self) {
        self.restore();
    }
}

fn destroy_icons(icons: [HICON; 2]) {
    for icon in icons {
        if !icon.is_null() {
            unsafe {
                DestroyIcon(icon);
            }
        }
    }
}

pub fn choose_file(frame: &eframe::Frame, filter: &str) -> io::Result<Option<std::path::PathBuf>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::mem::{size_of, zeroed};
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
    use windows_sys::Win32::{Foundation::HWND, UI::Controls::Dialogs::*};

    let RawWindowHandle::Win32(handle) = frame.window_handle().map_err(io::Error::other)?.as_raw()
    else {
        return Err(io::Error::other("The window is unavailable."));
    };
    let mut file = vec![0u16; 32768];
    let filter: Vec<u16> = filter.encode_utf16().collect();
    let mut dialog: OPENFILENAMEW = unsafe { zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = handle.hwnd.get() as HWND;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.nFilterIndex = 1;
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(None)
        } else {
            Err(io::Error::other(format!("File picker error {error:#x}.")))
        };
    }
    let end = file
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(file.len());
    Ok(Some(PathBuf::from(OsString::from_wide(&file[..end]))))
}
