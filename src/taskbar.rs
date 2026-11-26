//! A compact native Windows taskbar indicator. No Explorer injection or hooks.
use wireless_battery_monitor::preferences::WidgetOptions;
use std::{cell::RefCell, ffi::c_void, mem::{size_of, zeroed}, ptr::{null, null_mut}, time::{Duration, Instant}};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

#[derive(Clone, Copy, Debug)]
pub enum WidgetEvent {
    Details, Settings, Refresh, Exit, Suspend, Resume,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WidgetReading {
    pub glyph: crate::icon::DeviceGlyph,
    pub level: Option<u8>,
    pub charging: bool,
    pub stale: bool,
    pub full: bool,
    pub low_threshold: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Layout {
    x: i32, y: i32, w: i32, h: i32, dpi: u32, light: bool,
    cell: i32, scale: u16, options: WidgetOptions, phase: bool, readings: Vec<WidgetReading>,
    edge: bool, panel_width: i32, reveal: i32,
}

struct WidgetState {
    options: WidgetOptions,
    phase: bool,
    readings: Vec<WidgetReading>,
    edge: bool,
    reveal: f32,
    last_frame: Instant,
    created_at: Instant,
    keep_open_until: Instant,
    shown: bool,
    last_layout: Option<Layout>,
    emit: Box<dyn Fn(WidgetEvent)>,
}

/// Owns one non-activating, transparent window and its callback state.
pub struct TaskbarWidget {
    hwnd: HWND,
    state: Box<RefCell<WidgetState>>,
}

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }
fn px(n: i32, dpi: u32) -> i32 { (n * dpi as i32 + 48) / 96 }
fn timer_ms(state: &WidgetState) -> u32 {
    if state.edge { 40 } else if state.options.power_saving { 2000 } else { 750 }
}

unsafe fn taskbar_for(slot: u8) -> HWND {
    let primary = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
    if slot == 0 { return primary; }
    let class = wide("Shell_SecondaryTrayWnd");
    let mut previous = null_mut();
    for _ in 0..slot {
        previous = FindWindowExW(null_mut(), previous, class.as_ptr(), null());
        if previous.is_null() { return primary; }
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
            if previous.is_null() { break; }
            count += 1;
        }
    }
    count
}

pub fn overview_position(slot: u8, width: i32, height: i32) -> Option<[f32; 2]> {
    unsafe {
        let taskbar = taskbar_for(slot);
        if taskbar.is_null() { return None; }
        let mut monitor: MONITORINFO = zeroed();
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromWindow(taskbar, MONITOR_DEFAULTTONEAREST), &mut monitor) == 0 {
            return None;
        }
        let work = monitor.rcWork;
        let dpi = GetDpiForWindow(taskbar).max(96);
        let scale = 96.0 / dpi as f32;
        let x = (work.right - px(width + 16, dpi)).max(work.left);
        let y = (work.bottom - px(height + 48, dpi)).max(work.top);
        Some([x as f32 * scale, y as f32 * scale])
    }
}

impl TaskbarWidget {
    pub fn new(options: WidgetOptions, edge: bool, emit: impl Fn(WidgetEvent) + 'static) -> Option<Self> {
        let mut state = Box::new(RefCell::new(WidgetState {
            options, phase: false, readings: Vec::new(), shown: false, last_layout: None,
            edge, reveal: 0.0, last_frame: Instant::now(), created_at: Instant::now(), keep_open_until: Instant::now(), emit: Box::new(emit),
        }));
        unsafe {
            let class = wide(if edge { "WbmEdgeSlider" } else { "G435BatteryTaskbarWidget" });
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..zeroed()
            };
            if RegisterClassW(&wc) == 0 { return None; }
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class.as_ptr(), wide("Wireless Battery Monitor").as_ptr(), WS_POPUP,
                0, 0, 0, 0, null_mut(), null_mut(), instance,
                (&mut *state as *mut RefCell<WidgetState>).cast::<c_void>(),
            );
            if hwnd.is_null() { return None; }
            if SetTimer(hwnd, 1, timer_ms(&state.borrow()), None) == 0 {
                DestroyWindow(hwnd);
                return None;
            }
            refresh_layout(hwnd, &mut state.borrow_mut());
            Some(Self { hwnd, state })
        }
    }

    pub fn update(&mut self, readings: Vec<WidgetReading>, options: WidgetOptions) {
        let mut state = self.state.borrow_mut();
        state.readings = readings;
        let timer = timer_ms(&state);
        state.options = options;
        if timer != timer_ms(&state) {
            unsafe { SetTimer(self.hwnd, 1, timer_ms(&state), None); }
        }
        unsafe { refresh_layout(self.hwnd, &mut state); }
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
    if pointer.is_null() { return DefWindowProcW(hwnd, msg, w, l); }
    if msg == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, w, l);
    }
    if msg == WM_MOUSEACTIVATE { return MA_NOACTIVATE as LRESULT; }
    // WinUser.h PBT_APMSUSPEND=4 and PBT_APMRESUMEAUTOMATIC=18.
    if msg == WM_POWERBROADCAST {
        if let Ok(state) = (*pointer).try_borrow() {
            if state.edge { return 1; }
            match w {
                4 => (state.emit)(WidgetEvent::Suspend),
                18 => (state.emit)(WidgetEvent::Resume),
                _ => {}
            }
        }
        return 1;
    }
    if !matches!(msg, WM_TIMER | WM_LBUTTONUP | WM_RBUTTONUP | WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED) {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let Ok(mut state) = (*pointer).try_borrow_mut() else { return 0; };
    match msg {
        WM_TIMER => {
            state.phase = state.options.animate && state.readings.iter().any(|r| r.charging && !r.full && !r.stale)
                && (state.created_at.elapsed().as_millis() / 700) & 1 == 0;
            refresh_layout(hwnd, &mut state);
            0
        }
        WM_LBUTTONUP => {
            if state.edge && state.reveal < 0.95 {
                state.keep_open_until = Instant::now() + Duration::from_millis(800);
                refresh_layout(hwnd, &mut state);
            } else {
                (state.emit)(WidgetEvent::Details);
            }
            0
        }
        WM_RBUTTONUP => { context_menu(hwnd, &state); 0 }
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
    if menu.is_null() { return; }
    AppendMenuW(menu, MF_STRING, 1, wide("Battery details").as_ptr());
    AppendMenuW(menu, MF_STRING, 4, wide("Settings").as_ptr());
    AppendMenuW(menu, MF_STRING, 2, wide("Refresh now").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, 3, wide("Exit").as_ptr());
    let mut cursor: POINT = zeroed();
    GetCursorPos(&mut cursor);
    SetForegroundWindow(hwnd);
    let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
        cursor.x, cursor.y, 0, hwnd, null());
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
    RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD,
        null_mut(), (&mut data as *mut u32).cast(), &mut length) == 0 && data != 0
}

unsafe fn visibility(hwnd: HWND, state: &mut WidgetState, visible: bool) {
    if state.shown != visible {
        state.shown = visible;
        ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
    if !visible {
        state.last_layout = None;
        state.reveal = 0.0;
        state.keep_open_until = Instant::now();
    }
}

unsafe fn refresh_layout(hwnd: HWND, state: &mut WidgetState) {
    if state.edge && !state.options.edge { visibility(hwnd, state, false); return; }
    let taskbar = taskbar_for(state.options.monitor_slot);
    let tray = if taskbar.is_null() { null_mut() } else {
        FindWindowExW(taskbar, null_mut(), wide("TrayNotifyWnd").as_ptr(), null())
    };
    let mut bar: RECT = zeroed();
    let mut notify: RECT = zeroed();
    if taskbar.is_null() || IsWindowVisible(taskbar) == 0 ||
        GetWindowRect(taskbar, &mut bar) == 0 {
        visibility(hwnd, state, false); return;
    }
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(MonitorFromWindow(taskbar, MONITOR_DEFAULTTONEAREST), &mut monitor) == 0 {
        visibility(hwnd, state, false); return;
    }
    // Suppress the overlay when a foreground fullscreen app covers the monitor.
    let foreground = GetForegroundWindow();
    if !foreground.is_null() && foreground != hwnd && foreground != taskbar && foreground != GetShellWindow() {
        let mut full: RECT = zeroed();
        let screen = monitor.rcMonitor;
        if GetWindowRect(foreground, &mut full) != 0 && full.left <= screen.left &&
            full.top <= screen.top && full.right >= screen.right && full.bottom >= screen.bottom {
            visibility(hwnd, state, false); return;
        }
    }
    let dpi = GetDpiForWindow(taskbar).max(96);
    let visible_count = state.readings.len() as i32;
    if visible_count == 0 { visibility(hwnd, state, false); return; }
    let (x, y, w, h, cell, scale, panel_width, reveal) = if state.edge {
        let work = monitor.rcWork;
        let scale = state.options.scale;
        let cell = px((65 * i32::from(scale) + 50) / 100, dpi);
        let panel_width = px((64 * i32::from(scale) + 50) / 100, dpi);
        let available = ((work.bottom - work.top) / cell).min(visible_count);
        if available == 0 || work.right - work.left < panel_width {
            visibility(hwnd, state, false); return;
        }
        let panel_height = cell * available;
        let grip_width = px(7, dpi);
        let grip_height = px(60, dpi);
        let middle = work.top + (work.bottom - work.top) / 2;
        let mut cursor: POINT = zeroed();
        let over_panel = GetCursorPos(&mut cursor) != 0
            && cursor.x >= work.right - grip_width.max(
                state.last_layout.as_ref().map_or(0, |layout| layout.w))
            && cursor.x < work.right
            && (cursor.y - middle).abs() <= grip_height.max(
                state.last_layout.as_ref().map_or(0, |layout| layout.h)) / 2;
        let now = Instant::now();
        if over_panel { state.keep_open_until = now + Duration::from_millis(380); }
        let moving = (now.duration_since(state.last_frame).as_secs_f32() / 0.16).min(1.0);
        state.last_frame = now;
        if now < state.keep_open_until {
            state.reveal = (state.reveal + moving).min(1.0);
        } else {
            state.reveal = (state.reveal - moving).max(0.0);
        }
        let ease = state.reveal * state.reveal * (3.0 - 2.0 * state.reveal);
        let reveal = ((panel_width - grip_width) as f32 * ease).round() as i32;
        let w = grip_width + reveal;
        let h = if reveal == 0 { grip_height } else { panel_height };
        (work.right - w, middle - h / 2, w, h, cell, scale, panel_width, reveal)
    } else {
        let h = bar.bottom - bar.top;
        let primary = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        let anchor = if !tray.is_null() && GetWindowRect(tray, &mut notify) != 0 && notify.right > notify.left {
            notify.left
        } else if taskbar != primary {
            bar.right - px(150, dpi)
        } else {
            bar.right - px(250, dpi)
        };
        let scale = state.options.scale.min(((h - px(4, dpi)).max(1) * 100 / px(18, dpi).max(1)) as u16).max(70);
        let cell = px((72 * i32::from(scale) + 50) / 100, dpi);
        let w = cell * visible_count;
        let shown_height = bar.bottom.min(monitor.rcMonitor.bottom) - bar.top.max(monitor.rcMonitor.top);
        if h < px(20, dpi) || bar.right - bar.left < h * 4 ||
            shown_height < h / 2 || anchor < bar.left + w + px(12, dpi) || anchor > bar.right {
            visibility(hwnd, state, false); return;
        }
        ((anchor - w - px(i32::from(state.options.offset), dpi)).max(bar.left + px(12, dpi)), bar.top, w, h, cell, scale, w, 0)
    };
    let light = !state.edge && light_taskbar();
    let layout = Layout { x, y, w, h, dpi, light, cell, scale, options: state.options,
        phase: state.phase && state.options.animate && (!state.edge || reveal > 0),
        readings: if state.edge { state.readings.iter().copied().take((h / cell) as usize).collect() } else { state.readings.clone() },
        edge: state.edge, panel_width, reveal };
    if state.last_layout.as_ref() != Some(&layout) {
        if !paint(hwnd, &layout) {
            visibility(hwnd, state, false); return;
        }
        state.last_layout = Some(layout);
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
    visibility(hwnd, state, true);
}

unsafe fn paint(hwnd: HWND, layout: &Layout) -> bool {
    let Layout { x, y, w: width, h: height, dpi, light, cell, scale, options, phase, edge, panel_width, reveal, .. } = *layout;
    let dc = CreateCompatibleDC(null_mut());
    if dc.is_null() { return false; }
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
        if !bitmap.is_null() { DeleteObject(bitmap); }
        DeleteDC(dc); return false;
    }
    let old_bitmap = SelectObject(dc, bitmap);
    let count = (width * height) as usize;
    std::slice::from_raw_parts_mut(bits.cast::<u32>(), count).fill(0);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, 0x00ff_ffff);
    let pen = CreatePen(PS_SOLID, px((i32::from(scale) + 50) / 100, dpi).max(1), 0x00ff_ffff);
    let old_pen = SelectObject(dc, pen);
    let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
    let s = |v: i32| px((v * i32::from(scale) + 50) / 100, dpi);
    let font = CreateFontW(-s(13), 0, 0, 0, FW_NORMAL as i32, 0, 0, 0,
        DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32,
        ANTIALIASED_QUALITY as u32, DEFAULT_PITCH as u32, wide("Segoe UI").as_ptr());
    let old_font = SelectObject(dc, font);
    if edge && reveal > 0 {
        let brush = CreateSolidBrush(0x0027_2727);
        let previous = SelectObject(dc, brush);
        let outline = CreatePen(PS_SOLID, 1, 0x003d_3d3d);
        let previous_pen = SelectObject(dc, outline);
        RoundRect(dc, 0, 0, width + s(9), height, s(15), s(15));
        SelectObject(dc, previous_pen);
        SelectObject(dc, previous);
        DeleteObject(outline);
        DeleteObject(brush);
    }
    for (slot, reading) in layout.readings.iter().enumerate().filter(|_| !edge || reveal > 0) {
        let origin = cell * slot as i32;
        let (left, top) = if edge {
            let center = panel_width / 2;
            let ring_y = origin + s(26);
            let radius = s(20);
            let track = CreatePen(PS_SOLID, s(3).max(1), 0x0055_5555);
            if !track.is_null() {
                let previous = SelectObject(dc, track);
                Ellipse(dc, center-radius, ring_y-radius, center+radius, ring_y+radius);
                SelectObject(dc, previous);
                DeleteObject(track);
            }
            if let Some(level) = reading.level {
                // The blue byte is a mask for accent-coloured strokes.
                let ring = CreatePen(PS_SOLID, s(3).max(1), 0x00ff_0000);
                let previous = SelectObject(dc, ring);
                let span = std::f32::consts::TAU * f32::from(level) / 100.0;
                for step in 0..=48 {
                    let angle = span * step as f32 / 48.0 - std::f32::consts::FRAC_PI_2;
                    let x = center + (angle.cos() * radius as f32).round() as i32;
                    let y = ring_y + (angle.sin() * radius as f32).round() as i32;
                    if step == 0 { MoveToEx(dc, x, y, null_mut()); } else { LineTo(dc, x, y); }
                }
                SelectObject(dc, previous);
                DeleteObject(ring);
            }
            (center - s(9), ring_y - s(9))
        } else { (origin + s(4), (height - s(18)) / 2) };
        draw_glyph(dc, left, top, s, reading.glyph);
        let text = wide(&if reading.full { "Full".into() }
            else if reading.charging { reading.level.map(|v| format!("{v}%⚡")).unwrap_or_else(|| "--%".into()) }
            else { reading.level.map(|v| format!("{}{}%", if reading.stale { "~" } else { "" }, v)).unwrap_or_else(|| "--%".into()) });
        let (mut rect, align) = if edge {
            (RECT { left: 0, top: origin+s(46), right: panel_width, bottom: origin+cell }, DT_CENTER)
        } else {
            (RECT { left: origin+s(25), top: 0, right: origin+cell-s(2), bottom: height }, DT_LEFT)
        };
        DrawTextW(dc, text.as_ptr(), (text.len()-1) as i32, &mut rect, align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    }
    SelectObject(dc, old_font);
    DeleteObject(font);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(pen);
    GdiFlush();
    let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), count);
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let x = (index % width as usize) as i32;
        let y = (index / width as usize) as i32;
        if edge && reveal == 0 {
            let center = height / 2;
            let rounded = (y - center).abs() < height / 2 - px(3, dpi)
                || x >= px(3, dpi);
            *pixel = if rounded { 0xff_68c9c3 } else { 0 };
            continue;
        }
        if *pixel == 0 { continue; }
        let axis = if edge { y } else { x };
        let Some(reading) = layout.readings.get((axis / cell) as usize) else { continue; };
        let colour = if reading.stale || reading.level.is_none() {
            if light { (110, 110, 110) } else { (155, 155, 155) }
        } else if reading.charging || reading.full {
            if reading.charging && !reading.full && phase {
                if light { (0, 85, 45) } else { (154, 245, 184) }
            } else if light { (0, 115, 60) } else { (89, 210, 145) }
        } else if reading.level.is_some_and(|v| v <= reading.low_threshold) {
            if light { (170, 25, 38) } else { (255, 126, 137) }
        } else if edge && options.accent == wireless_battery_monitor::preferences::Accent::System {
            (104, 201, 195)
        } else { options.accent.rgb(light) };
        if edge {
            let ring = (*pixel & 0x00ff_ffff) == 0x00ff_0000;
            let rgb = if ring {
                (colour.0 << 16) | (colour.1 << 8) | colour.2
            } else { *pixel & 0x00ff_ffff };
            *pixel = 0xff00_0000 | rgb;
        } else {
            let a = *pixel & 0xff;
            let r = colour.0 * a / 255;
            let g = colour.1 * a / 255;
            let b = colour.2 * a / 255;
            *pixel = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }
    let pos = POINT { x, y };
    let src = POINT { x: 0, y: 0 };
    let size = SIZE { cx: width, cy: height };
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0,
        SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    let okay = UpdateLayeredWindow(hwnd, null_mut(), &pos, &size, dc, &src, 0, &blend, ULW_ALPHA) != 0;
    SelectObject(dc, old_bitmap);
    DeleteObject(bitmap);
    DeleteDC(dc);
    okay
}

unsafe fn draw_glyph(dc: HDC, left: i32, top: i32, s: impl Fn(i32) -> i32, glyph: crate::icon::DeviceGlyph) {
    use crate::icon::DeviceGlyph;
    let line = |x1, y1, x2, y2| {
        MoveToEx(dc, left + s(x1), top + s(y1), null_mut());
        LineTo(dc, left + s(x2), top + s(y2));
    };
    let solid = GetStockObject(WHITE_BRUSH);
    let old = SelectObject(dc, solid);
    match glyph {
        DeviceGlyph::Headset => {
            Arc(dc, left+s(1), top+s(1), left+s(17), top+s(17),
                left+s(17), top+s(9), left+s(1), top+s(9));
            RoundRect(dc, left+s(1), top+s(8), left+s(6), top+s(17), s(4), s(4));
            RoundRect(dc, left+s(12), top+s(8), left+s(17), top+s(17), s(4), s(4));
        }
        DeviceGlyph::Mouse => {
            RoundRect(dc, left+s(4), top, left+s(14), top+s(18), s(9), s(9));
            let wheel = CreatePen(PS_SOLID, s(2).max(1), 0x0070_7070);
            let previous = SelectObject(dc, wheel);
            line(9, 4, 9, 8);
            SelectObject(dc, previous);
            DeleteObject(wheel);
        }
        DeviceGlyph::Earbuds => {
            for x in [2, 10] {
                Ellipse(dc, left+s(x), top+s(3), left+s(x+6), top+s(10));
                RoundRect(dc, left+s(x+1), top+s(7), left+s(x+4), top+s(15), s(2), s(2));
            }
        }
        DeviceGlyph::Bluetooth => {
            line(9, 1, 9, 17);
            line(9, 1, 14, 5);
            line(14, 5, 4, 13);
            line(4, 5, 14, 13);
            line(14, 13, 9, 17);
        }
    }
    SelectObject(dc, old);
}
