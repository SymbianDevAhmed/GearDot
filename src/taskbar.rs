//! A compact native Windows taskbar indicator. No Explorer injection or hooks.
use std::{
    cell::RefCell,
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};
use wireless_battery_monitor::preferences::WidgetOptions;

#[derive(Clone, Copy, Debug)]
pub enum WidgetEvent {
    Details,
    Settings,
    Refresh,
    Exit,
    Suspend,
    Resume,
    Visibility(bool),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WidgetReading {
    pub level: Option<u8>,
    pub charging: bool,
    pub stale: bool,
    pub full: bool,
    pub low_threshold: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    dpi: u32,
    light: bool,
    cell: i32,
    scale: u16,
    options: WidgetOptions,
    phase: bool,
    readings: [WidgetReading; 2],
}

struct WidgetState {
    options: WidgetOptions,
    phase: bool,
    readings: [WidgetReading; 2],
    shown: bool,
    last_layout: Option<Layout>,
    emit: Box<dyn Fn(WidgetEvent)>,
}

/// Owns one non-activating, transparent window and its callback state.
pub struct TaskbarWidget {
    hwnd: HWND,
    state: Box<RefCell<WidgetState>>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
fn px(n: i32, dpi: u32) -> i32 {
    (n * dpi as i32 + 48) / 96
}
fn timer_ms(options: WidgetOptions) -> u32 {
    if options.power_saving {
        2000
    } else {
        750
    }
}

unsafe fn taskbar_for(slot: u8) -> HWND {
    let primary = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
    if slot == 0 {
        return primary;
    }
    let class = wide("Shell_SecondaryTrayWnd");
    let mut previous = null_mut();
    for _ in 0..slot {
        previous = FindWindowExW(null_mut(), previous, class.as_ptr(), null());
        if previous.is_null() {
            return primary;
        }
    }
    previous
}

pub fn secondary_count() -> u8 {
    let mut count = 0;
    let mut previous = null_mut();
    let class = wide("Shell_SecondaryTrayWnd");
    unsafe {
        while count < 8 {
            previous = FindWindowExW(null_mut(), previous, class.as_ptr(), null());
            if previous.is_null() {
                break;
            }
            count += 1;
        }
    }
    count
}

impl TaskbarWidget {
    pub fn new(options: WidgetOptions, emit: impl Fn(WidgetEvent) + 'static) -> Option<Self> {
        let mut state = Box::new(RefCell::new(WidgetState {
            options,
            phase: false,
            readings: [WidgetReading::default(); 2],
            shown: false,
            last_layout: None,
            emit: Box::new(emit),
        }));
        unsafe {
            let class = wide("G435BatteryTaskbarWidget");
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..zeroed()
            };
            if RegisterClassW(&wc) == 0 {
                return None;
            }
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class.as_ptr(),
                wide("Wireless Battery Monitor").as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                (&mut *state as *mut RefCell<WidgetState>).cast::<c_void>(),
            );
            if hwnd.is_null() {
                return None;
            }
            if SetTimer(hwnd, 1, timer_ms(options), None) == 0 {
                DestroyWindow(hwnd);
                return None;
            }
            refresh_layout(hwnd, &mut state.borrow_mut());
            Some(Self { hwnd, state })
        }
    }

    pub fn update(&mut self, readings: [WidgetReading; 2], options: WidgetOptions) {
        let mut state = self.state.borrow_mut();
        state.readings = readings;
        if state.options.power_saving != options.power_saving {
            unsafe {
                SetTimer(self.hwnd, 1, timer_ms(options), None);
            }
        }
        state.options = options;
        unsafe {
            refresh_layout(self.hwnd, &mut state);
        }
    }
}

impl Drop for TaskbarWidget {
    fn drop(&mut self) {
        unsafe {
            KillTimer(self.hwnd, 1);
            DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<WidgetState>;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    if msg == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, w, l);
    }
    if msg == WM_MOUSEACTIVATE {
        return MA_NOACTIVATE as LRESULT;
    }
    // WinUser.h PBT_APMSUSPEND=4 and PBT_APMRESUMEAUTOMATIC=18.
    if msg == WM_POWERBROADCAST {
        if let Ok(state) = (*pointer).try_borrow() {
            match w {
                4 => (state.emit)(WidgetEvent::Suspend),
                18 => (state.emit)(WidgetEvent::Resume),
                _ => {}
            }
        }
        return 1;
    }
    if !matches!(
        msg,
        WM_TIMER
            | WM_LBUTTONUP
            | WM_RBUTTONUP
            | WM_SETTINGCHANGE
            | WM_DISPLAYCHANGE
            | WM_DPICHANGED
    ) {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let Ok(mut state) = (*pointer).try_borrow_mut() else {
        return 0;
    };
    match msg {
        WM_TIMER => {
            if state.options.animate
                && state
                    .readings
                    .iter()
                    .zip(state.options.visible)
                    .any(|(r, v)| v && r.charging && !r.full && !r.stale)
            {
                state.phase = !state.phase;
            } else {
                state.phase = false;
            }
            refresh_layout(hwnd, &mut state);
            0
        }
        WM_LBUTTONUP => {
            (state.emit)(WidgetEvent::Details);
            0
        }
        WM_RBUTTONUP => {
            context_menu(hwnd, &state);
            0
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            state.last_layout = None;
            refresh_layout(hwnd, &mut state);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

unsafe fn context_menu(hwnd: HWND, state: &WidgetState) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    AppendMenuW(menu, MF_STRING, 1, wide("Battery details").as_ptr());
    AppendMenuW(menu, MF_STRING, 4, wide("Settings").as_ptr());
    AppendMenuW(menu, MF_STRING, 2, wide("Refresh now").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, 3, wide("Exit").as_ptr());
    let mut cursor: POINT = zeroed();
    GetCursorPos(&mut cursor);
    SetForegroundWindow(hwnd);
    let chosen = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
        cursor.x,
        cursor.y,
        0,
        hwnd,
        null(),
    );
    DestroyMenu(menu);
    PostMessageW(hwnd, WM_NULL, 0, 0);
    match chosen {
        1 => (state.emit)(WidgetEvent::Details),
        2 => (state.emit)(WidgetEvent::Refresh),
        3 => (state.emit)(WidgetEvent::Exit),
        4 => (state.emit)(WidgetEvent::Settings),
        _ => {}
    }
}

unsafe fn light_taskbar() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("SystemUsesLightTheme");
    let mut data: u32 = 0;
    let mut length = size_of::<u32>() as u32;
    RegGetValueW(
        HKEY_CURRENT_USER,
        key.as_ptr(),
        value.as_ptr(),
        RRF_RT_REG_DWORD,
        null_mut(),
        (&mut data as *mut u32).cast(),
        &mut length,
    ) == 0
        && data != 0
}

unsafe fn visibility(hwnd: HWND, state: &mut WidgetState, visible: bool) {
    if state.shown != visible {
        state.shown = visible;
        ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        (state.emit)(WidgetEvent::Visibility(visible));
    }
    if !visible {
        state.last_layout = None;
    }
}

unsafe fn refresh_layout(hwnd: HWND, state: &mut WidgetState) {
    let taskbar = taskbar_for(state.options.monitor_slot);
    let tray = if taskbar.is_null() {
        null_mut()
    } else {
        FindWindowExW(taskbar, null_mut(), wide("TrayNotifyWnd").as_ptr(), null())
    };
    let mut bar: RECT = zeroed();
    let mut notify: RECT = zeroed();
    if taskbar.is_null() || IsWindowVisible(taskbar) == 0 || GetWindowRect(taskbar, &mut bar) == 0 {
        visibility(hwnd, state, false);
        return;
    }
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(
        MonitorFromWindow(taskbar, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    ) == 0
    {
        visibility(hwnd, state, false);
        return;
    }
    // Suppress the overlay when a foreground fullscreen app covers the monitor.
    let foreground = GetForegroundWindow();
    if !foreground.is_null()
        && foreground != hwnd
        && foreground != taskbar
        && foreground != GetShellWindow()
    {
        let mut full: RECT = zeroed();
        let screen = monitor.rcMonitor;
        if GetWindowRect(foreground, &mut full) != 0
            && full.left <= screen.left
            && full.top <= screen.top
            && full.right >= screen.right
            && full.bottom >= screen.bottom
        {
            visibility(hwnd, state, false);
            return;
        }
    }
    let dpi = GetDpiForWindow(taskbar).max(96);
    let h = bar.bottom - bar.top;
    let primary = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
    let anchor =
        if !tray.is_null() && GetWindowRect(tray, &mut notify) != 0 && notify.right > notify.left {
            notify.left
        } else if taskbar != primary {
            // Secondary taskbars do not always expose a notification-area child.
            // Reserve the rightmost clock area rather than overlaying it.
            bar.right - px(150, dpi)
        } else {
            visibility(hwnd, state, false);
            return;
        };
    let visible_count = state.options.visible.iter().filter(|v| **v).count() as i32;
    if visible_count == 0 {
        visibility(hwnd, state, false);
        return;
    }
    let scale = state
        .options
        .scale
        .min(((h - px(4, dpi)).max(1) * 100 / px(18, dpi).max(1)) as u16)
        .max(70);
    let cell = px((72 * i32::from(scale) + 50) / 100, dpi);
    let w = cell * visible_count;
    // Do not cover a hidden/vertical taskbar or a tray that cannot be located.
    let shown_height =
        bar.bottom.min(monitor.rcMonitor.bottom) - bar.top.max(monitor.rcMonitor.top);
    if h < px(20, dpi)
        || bar.right - bar.left < h * 4
        || shown_height < h / 2
        || anchor < bar.left + w + px(12, dpi)
        || anchor > bar.right
    {
        visibility(hwnd, state, false);
        return;
    }
    let x = (anchor - w - px(i32::from(state.options.offset), dpi)).max(bar.left + px(12, dpi));
    let y = bar.top;
    let light = light_taskbar();
    let layout = Layout {
        x,
        y,
        w,
        h,
        dpi,
        light,
        cell,
        scale,
        options: state.options,
        phase: state.phase && state.options.animate,
        readings: state.readings,
    };
    if state.last_layout != Some(layout) {
        if !paint(hwnd, layout) {
            visibility(hwnd, state, false);
            return;
        }
        state.last_layout = Some(layout);
    }
    visibility(hwnd, state, true);
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
    );
}

unsafe fn paint(hwnd: HWND, layout: Layout) -> bool {
    let Layout {
        x,
        y,
        w: width,
        h: height,
        dpi,
        light,
        cell,
        scale,
        options,
        phase,
        readings,
    } = layout;
    let visible: Vec<usize> = (0..2).filter(|&i| options.visible[i]).collect();
    let dc = CreateCompatibleDC(null_mut());
    if dc.is_null() {
        return false;
    }
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits: *mut c_void = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        DeleteDC(dc);
        return false;
    }
    let old_bitmap = SelectObject(dc, bitmap);
    let count = (width * height) as usize;
    std::slice::from_raw_parts_mut(bits.cast::<u32>(), count).fill(0);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, 0x00ff_ffff);
    let pen = CreatePen(
        PS_SOLID,
        px((i32::from(scale) + 50) / 100, dpi).max(1),
        0x00ff_ffff,
    );
    let old_pen = SelectObject(dc, pen);
    let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
    let s = |v: i32| px((v * i32::from(scale) + 50) / 100, dpi);
    let top = (height - s(18)) / 2;
    let font = CreateFontW(
        -s(13),
        0,
        0,
        0,
        FW_NORMAL as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        ANTIALIASED_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide("Segoe UI").as_ptr(),
    );
    let old_font = SelectObject(dc, font);
    for (slot, &index) in visible.iter().enumerate() {
        let reading = &readings[index];
        let origin = cell * slot as i32;
        let left = origin + s(4);
        if index == 0 {
            // Headphone outline.
            Arc(
                dc,
                left + s(1),
                top + s(1),
                left + s(17),
                top + s(17),
                left + s(17),
                top + s(10),
                left + s(1),
                top + s(10),
            );
            MoveToEx(dc, left + s(1), top + s(9), null_mut());
            LineTo(dc, left + s(1), top + s(14));
            MoveToEx(dc, left + s(17), top + s(9), null_mut());
            LineTo(dc, left + s(17), top + s(14));
            RoundRect(
                dc,
                left + s(1),
                top + s(10),
                left + s(5),
                top + s(17),
                s(2),
                s(2),
            );
            RoundRect(
                dc,
                left + s(13),
                top + s(10),
                left + s(17),
                top + s(17),
                s(2),
                s(2),
            );
        } else {
            // Original mouse outline: rounded body, split buttons and wheel.
            RoundRect(
                dc,
                left + s(3),
                top,
                left + s(15),
                top + s(18),
                s(10),
                s(10),
            );
            MoveToEx(dc, left + s(4), top + s(7), null_mut());
            LineTo(dc, left + s(14), top + s(7));
            MoveToEx(dc, left + s(9), top + s(1), null_mut());
            LineTo(dc, left + s(9), top + s(7));
        }
        let text = wide(&if reading.full {
            "Full".into()
        } else if reading.charging {
            reading
                .level
                .map(|v| format!("{v}%⚡"))
                .unwrap_or_else(|| "--%".into())
        } else {
            reading
                .level
                .map(|v| format!("{}{}%", if reading.stale { "~" } else { "" }, v))
                .unwrap_or_else(|| "--%".into())
        });
        let mut rect = RECT {
            left: origin + s(25),
            top: 0,
            right: origin + cell - s(2),
            bottom: height,
        };
        DrawTextW(
            dc,
            text.as_ptr(),
            (text.len() - 1) as i32,
            &mut rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
    }
    SelectObject(dc, old_font);
    DeleteObject(font);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(pen);
    GdiFlush();
    // Per-device foreground colour so the headphones and mouse stay independent.
    let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), count);
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let slot = ((index % width as usize) / cell as usize).min(visible.len() - 1);
        let reading = readings[visible[slot]];
        let rgb = if reading.stale || reading.level.is_none() {
            if light {
                (110u32, 110u32, 110u32)
            } else {
                (155, 155, 155)
            }
        } else if reading.charging || reading.full {
            if reading.charging && !reading.full && phase {
                if light {
                    (0, 85, 45)
                } else {
                    (154, 245, 184)
                }
            } else if light {
                (0, 115, 60)
            } else {
                (89, 210, 145)
            }
        } else if reading.level.is_some_and(|v| v <= reading.low_threshold) {
            if light {
                (170, 25, 38)
            } else {
                (255, 126, 137)
            }
        } else {
            options.accent.rgb(light)
        };
        let a = (*pixel & 0xff).max(1);
        let r = rgb.0 * a / 255;
        let g = rgb.1 * a / 255;
        let b = rgb.2 * a / 255;
        *pixel = (a << 24) | (r << 16) | (g << 8) | b;
    }
    let pos = POINT { x, y };
    let src = POINT { x: 0, y: 0 };
    let size = SIZE {
        cx: width,
        cy: height,
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let okay = UpdateLayeredWindow(
        hwnd,
        null_mut(),
        &pos,
        &size,
        dc,
        &src,
        0,
        &blend,
        ULW_ALPHA,
    ) != 0;
    SelectObject(dc, old_bitmap);
    DeleteObject(bitmap);
    DeleteDC(dc);
    okay
}
