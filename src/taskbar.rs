//! A compact native Windows taskbar indicator. No Explorer injection or hooks.
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    time::{Duration, Instant},
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
    DevicesChanged,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WidgetReading {
    pub id: String,
    pub glyph: crate::icon::DeviceGlyph,
    pub level: u8,
    pub charging: bool,
    pub full: bool,
    pub low_threshold: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    readings: Vec<WidgetReading>,
    edge: bool,
    panel_width: i32,
    reveal: i32,
    progress: Vec<u16>,
}

struct RingMotion {
    start: Instant,
    from: f32,
    target: u8,
}

impl RingMotion {
    fn current(&self, now: Instant) -> f32 {
        let t = (now.duration_since(self.start).as_secs_f32() / 0.55).min(1.0);
        let ease = 1.0 - (1.0 - t).powi(3);
        self.from + (f32::from(self.target) - self.from) * ease
    }
}

struct WidgetState {
    options: WidgetOptions,
    phase: bool,
    readings: Vec<WidgetReading>,
    motion: BTreeMap<String, RingMotion>,
    edge: bool,
    reveal: f32,
    last_frame: Instant,
    created_at: Instant,
    keep_open_until: Instant,
    shown: bool,
    render_failed: bool,
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
fn timer_ms(state: &WidgetState) -> u32 {
    if state.edge {
        40
    } else if state.options.power_saving {
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

pub fn overview_position(slot: u8, width: i32, height: i32) -> Option<[f32; 2]> {
    unsafe {
        let taskbar = taskbar_for(slot);
        if taskbar.is_null() {
            return None;
        }
        let mut monitor: MONITORINFO = zeroed();
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(
            MonitorFromWindow(taskbar, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        ) == 0
        {
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
    pub fn new(
        options: WidgetOptions,
        edge: bool,
        emit: impl Fn(WidgetEvent) + 'static,
    ) -> Option<Self> {
        let mut state = Box::new(RefCell::new(WidgetState {
            options,
            phase: false,
            readings: Vec::new(),
            motion: BTreeMap::new(),
            shown: false,
            render_failed: false,
            last_layout: None,
            edge,
            reveal: 0.0,
            last_frame: Instant::now(),
            created_at: Instant::now(),
            keep_open_until: Instant::now(),
            emit: Box::new(emit),
        }));
        unsafe {
            let class = wide(if edge {
                "WbmEdgeSlider"
            } else {
                "G435BatteryTaskbarWidget"
            });
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
            if SetTimer(hwnd, 1, timer_ms(&state.borrow()), None) == 0 {
                DestroyWindow(hwnd);
                return None;
            }
            refresh_safely(hwnd, &mut state.borrow_mut());
            Some(Self { hwnd, state })
        }
    }

    pub fn update(&mut self, readings: Vec<WidgetReading>, options: WidgetOptions) {
        let mut state = self.state.borrow_mut();
        if state.edge {
            let now = Instant::now();
            state
                .motion
                .retain(|id, _| readings.iter().any(|reading| reading.id == *id));
            for reading in &readings {
                let motion = state
                    .motion
                    .entry(reading.id.clone())
                    .or_insert(RingMotion {
                        start: now,
                        from: 0.0,
                        target: reading.level,
                    });
                if motion.target != reading.level {
                    *motion = RingMotion {
                        start: now,
                        from: motion.current(now),
                        target: reading.level,
                    };
                }
            }
        }
        state.readings = readings;
        let timer = timer_ms(&state);
        state.options = options;
        if timer != timer_ms(&state) {
            unsafe {
                SetTimer(self.hwnd, 1, timer_ms(&state), None);
            }
        }
        unsafe {
            refresh_safely(self.hwnd, &mut state);
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
            if state.edge {
                return 1;
            }
            match w {
                4 => (state.emit)(WidgetEvent::Suspend),
                18 => (state.emit)(WidgetEvent::Resume),
                _ => {}
            }
        }
        return 1;
    }
    // DBT_DEVNODES_CHANGED, DBT_DEVICEARRIVAL and DBT_DEVICEREMOVECOMPLETE.
    if msg == WM_DEVICECHANGE && matches!(w, 7 | 0x8000 | 0x8004) {
        if let Ok(state) = (*pointer).try_borrow() {
            if !state.edge {
                (state.emit)(WidgetEvent::DevicesChanged);
            }
        }
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
            state.phase = state.options.animate
                && state.readings.iter().any(|r| r.charging && !r.full)
                && (state.created_at.elapsed().as_millis() / 700) & 1 == 0;
            refresh_safely(hwnd, &mut state);
            0
        }
        WM_LBUTTONUP => {
            if state.edge && state.reveal < 0.95 {
                state.keep_open_until = Instant::now() + Duration::from_millis(800);
                refresh_safely(hwnd, &mut state);
            } else {
                (state.emit)(WidgetEvent::Details);
            }
            0
        }
        WM_RBUTTONUP => {
            context_menu(hwnd, &state);
            0
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            state.last_layout = None;
            refresh_safely(hwnd, &mut state);
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
    }
    if !visible {
        state.last_layout = None;
        state.reveal = 0.0;
        state.keep_open_until = Instant::now();
    }
}

unsafe fn refresh_safely(hwnd: HWND, state: &mut WidgetState) {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use wireless_battery_monitor::diagnostics::Log;

    if state.render_failed {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| refresh_layout(hwnd, state))).is_err() {
        state.render_failed = true;
        KillTimer(hwnd, 1);
        visibility(hwnd, state, false);
        let surface = if state.edge {
            "Screen-edge slider"
        } else {
            "Taskbar indicator"
        };
        let _ = Log::Monitor.event(
            "widget",
            &format!("{surface} disabled after a rendering panic"),
        );
    }
}

unsafe fn refresh_layout(hwnd: HWND, state: &mut WidgetState) {
    if state.edge && !state.options.edge {
        visibility(hwnd, state, false);
        return;
    }
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
    let visible_count = state.readings.len() as i32;
    if visible_count == 0 {
        visibility(hwnd, state, false);
        return;
    }
    let (x, y, w, h, cell, scale, panel_width, reveal) = if state.edge {
        let work = monitor.rcWork;
        let scale = state.options.scale;
        let cell = px((56 * i32::from(scale) + 50) / 100, dpi);
        let panel_width = px((56 * i32::from(scale) + 50) / 100, dpi);
        let available = ((work.bottom - work.top - px(16, dpi)) / cell).min(visible_count);
        if available == 0 || work.right - work.left < panel_width {
            visibility(hwnd, state, false);
            return;
        }
        let panel_height = cell * available + px(10, dpi);
        let grip_width = px(6, dpi);
        let grip_height = px(50, dpi);
        let middle = work.top + (work.bottom - work.top) / 2;
        let mut cursor: POINT = zeroed();
        let over_panel = GetCursorPos(&mut cursor) != 0
            && cursor.x
                >= work.right
                    - grip_width.max(state.last_layout.as_ref().map_or(0, |layout| layout.w))
            && cursor.x < work.right
            && (cursor.y - middle).abs()
                <= grip_height.max(state.last_layout.as_ref().map_or(0, |layout| layout.h)) / 2;
        let now = Instant::now();
        if over_panel {
            state.keep_open_until = now + Duration::from_millis(380);
        }
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
        let h = if reveal == 0 {
            grip_height
        } else {
            panel_height
        };
        (
            work.right - w,
            middle - h / 2,
            w,
            h,
            cell,
            scale,
            panel_width,
            reveal,
        )
    } else {
        let h = bar.bottom - bar.top;
        let primary = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        let anchor = if !tray.is_null()
            && GetWindowRect(tray, &mut notify) != 0
            && notify.right > notify.left
        {
            notify.left
        } else if taskbar != primary {
            bar.right - px(150, dpi)
        } else {
            bar.right - px(250, dpi)
        };
        let scale = state
            .options
            .scale
            .min(((h - px(4, dpi)).max(1) * 100 / px(18, dpi).max(1)) as u16)
            .max(70);
        let cell = px((57 * i32::from(scale) + 50) / 100, dpi);
        let slots = ((bar.right - bar.left) / 3 / cell).min(visible_count);
        if slots == 0 {
            visibility(hwnd, state, false);
            return;
        }
        let w = cell * slots;
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
        (
            (anchor - w - px(i32::from(state.options.offset), dpi)).max(bar.left + px(12, dpi)),
            bar.top,
            w,
            h,
            cell,
            scale,
            w,
            0,
        )
    };
    let light = !state.edge && light_taskbar();
    let slots = if state.edge {
        if reveal > 0 {
            h / cell
        } else {
            0
        }
    } else {
        w / cell
    };
    let readings: Vec<_> = state
        .readings
        .iter()
        .take(slots as usize)
        .cloned()
        .collect();
    let now = Instant::now();
    let progress = if state.edge {
        readings
            .iter()
            .map(|reading| {
                let level = if state.options.power_saving {
                    f32::from(reading.level)
                } else {
                    state
                        .motion
                        .get(&reading.id)
                        .map_or(f32::from(reading.level), |motion| motion.current(now))
                };
                (level * 10.0).round() as u16
            })
            .collect()
    } else {
        Vec::new()
    };
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
        phase: state.phase && state.options.animate && (!state.edge || reveal > 0),
        readings,
        edge: state.edge,
        panel_width,
        reveal,
        progress,
    };
    if state.last_layout.as_ref() != Some(&layout) {
        if !paint(hwnd, &layout) {
            visibility(hwnd, state, false);
            return;
        }
        state.last_layout = Some(layout);
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
    visibility(hwnd, state, true);
}

unsafe fn paint(hwnd: HWND, layout: &Layout) -> bool {
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
        edge,
        panel_width,
        reveal,
        ..
    } = *layout;
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
    for (slot, reading) in layout.readings.iter().enumerate() {
        let origin = cell * slot as i32;
        let (left, top) = if edge {
            (panel_width / 2 - s(7), origin + s(17))
        } else {
            (origin + s(2), (height - s(17)) / 2)
        };
        let glyph_size = |unit| if edge { s(unit * 4 / 5) } else { s(unit) };
        draw_glyph(dc, left, top, glyph_size, reading.glyph);
        let label = if reading.full {
            "Full".to_owned()
        } else if reading.charging {
            format!("{}%⚡", reading.level)
        } else {
            format!("{}%", reading.level)
        };
        let text = wide(&label);
        let (mut rect, align) = if edge {
            (
                RECT {
                    left: 0,
                    top: origin + s(39),
                    right: panel_width,
                    bottom: origin + cell + s(2),
                },
                DT_CENTER,
            )
        } else {
            (
                RECT {
                    left: origin + s(21),
                    top: 0,
                    right: origin + cell - s(1),
                    bottom: height,
                },
                DT_LEFT,
            )
        };
        DrawTextW(
            dc,
            text.as_ptr(),
            (text.len() - 1) as i32,
            &mut rect,
            align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
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
        let coverage = (*pixel & 0xff) as f32 / 255.0;
        if edge {
            *pixel = edge_pixel(layout, x, y, coverage);
        } else if let Some(reading) = layout.readings.get((x / cell) as usize) {
            *pixel = blend_pixel(0, reading_colour(layout, reading), coverage);
        }
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

type Rgb = (u32, u32, u32);

fn reading_colour(layout: &Layout, reading: &WidgetReading) -> Rgb {
    if reading.full {
        return if layout.light {
            (0, 115, 60)
        } else {
            (105, 227, 169)
        };
    }
    if reading.charging {
        return if layout.light {
            (135, 83, 0)
        } else if layout.phase {
            (255, 224, 146)
        } else {
            (242, 193, 78)
        };
    }
    if reading.level <= reading.low_threshold {
        return if layout.light {
            (170, 25, 38)
        } else {
            (255, 130, 151)
        };
    }
    if layout.edge && layout.options.accent == wireless_battery_monitor::preferences::Accent::System
    {
        (244, 193, 70)
    } else {
        layout.options.accent.rgb(layout.light)
    }
}

/// Premultiplied BGRA for UpdateLayeredWindow, using a monochrome GDI text mask.
fn blend_pixel(dst: u32, colour: Rgb, alpha: f32) -> u32 {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    let inverse = 255 - a;
    let over = |value: u32, shift: u32| {
        ((value * a + ((dst >> shift) & 0xff) * inverse + 127) / 255) << shift
    };
    over(255, 24) | over(colour.0, 16) | over(colour.1, 8) | over(colour.2, 0)
}

fn rounded_rect_distance(x: f32, y: f32, half_width: f32, half_height: f32, radius: f32) -> f32 {
    let dx = x.abs() - half_width + radius;
    let dy = y.abs() - half_height + radius;
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius
}

fn edge_pixel(layout: &Layout, x: i32, y: i32, mask: f32) -> u32 {
    use std::f32::consts::TAU;

    let scale = f32::from(layout.scale) / 100.0 * layout.dpi as f32 / 96.0;
    let x = x as f32 + 0.5;
    let py = y as f32 + 0.5;
    let half_h = layout.h as f32 / 2.0;
    if layout.reveal == 0 {
        let half_w = layout.w as f32 / 2.0;
        let dist =
            rounded_rect_distance(x - half_w - 1.0, py - half_h, half_w + 1.0, half_h, half_w);
        return blend_pixel(0, (238, 181, 56), 0.90 * (0.5 - dist).clamp(0.0, 1.0));
    }

    let half_w = layout.panel_width as f32 / 2.0;
    let dist = rounded_rect_distance(x - half_w, py - half_h, half_w, half_h, 10.0 * scale);
    let outer = (0.5 - dist).clamp(0.0, 1.0);
    if outer == 0.0 {
        return 0;
    }
    let tint = (1.0 - x / layout.panel_width as f32).clamp(0.0, 1.0);
    let body = 23 + (9.0 * tint) as u32;
    let mut out = blend_pixel(0, (body + 2, body + 1, body), 0.96 * outer);
    out = blend_pixel(
        out,
        (98, 91, 77),
        0.35 * outer * (1.0 + dist).clamp(0.0, 1.0),
    );

    let slot = (y / layout.cell) as usize;
    let Some(reading) = layout.readings.get(slot) else {
        return out;
    };
    let dx = x - half_w;
    let dy = py - (slot as i32 * layout.cell) as f32 - 24.0 * scale;
    let radius = 15.0 * scale;
    let radial_distance = (dx.hypot(dy) - radius).abs();
    let thickness = 1.55 * scale;
    let base = reading_colour(layout, reading);
    if radial_distance < thickness + 3.0 * scale {
        out = blend_pixel(
            out,
            (67, 64, 62),
            (thickness + 0.5 - radial_distance).clamp(0.0, 1.0),
        );
        let fraction = f32::from(layout.progress[slot]) / 1000.0;
        if fraction > 0.0 {
            let angle = dx.atan2(-dy).rem_euclid(TAU);
            let sweep = TAU * fraction;
            let arc_distance = if angle <= sweep {
                radial_distance
            } else {
                let start = dx.hypot(dy + radius);
                let end = (dx - radius * sweep.sin()).hypot(dy + radius * sweep.cos());
                start.min(end)
            };
            let intensity = if fraction >= 0.999 {
                0.94 + 0.06 * (angle / TAU).cos()
            } else {
                0.40 + 0.60 * (angle / sweep).min(1.0)
            };
            let glow = (1.0 - arc_distance / (thickness + 2.0 * scale)).clamp(0.0, 1.0);
            out = blend_pixel(out, base, glow * glow * 0.26);
            out = blend_pixel(
                out,
                base,
                intensity * (thickness + 0.5 - arc_distance).clamp(0.0, 1.0),
            );
        }
    }
    let label = if y % layout.cell >= (39.0 * scale).floor() as i32
        && (reading.full || reading.level <= reading.low_threshold)
    {
        base
    } else {
        (247, 245, 242)
    };
    blend_pixel(out, label, mask * outer)
}

unsafe fn draw_glyph(
    dc: HDC,
    left: i32,
    top: i32,
    s: impl Fn(i32) -> i32,
    glyph: crate::icon::DeviceGlyph,
) {
    use crate::icon::DeviceGlyph;
    let line = |x1, y1, x2, y2| {
        MoveToEx(dc, left + s(x1), top + s(y1), null_mut());
        LineTo(dc, left + s(x2), top + s(y2));
    };
    let solid = GetStockObject(WHITE_BRUSH);
    let old = SelectObject(dc, solid);
    match glyph {
        DeviceGlyph::Headset => {
            Arc(
                dc,
                left + s(1),
                top + s(1),
                left + s(17),
                top + s(17),
                left + s(17),
                top + s(9),
                left + s(1),
                top + s(9),
            );
            RoundRect(
                dc,
                left + s(1),
                top + s(8),
                left + s(6),
                top + s(17),
                s(4),
                s(4),
            );
            RoundRect(
                dc,
                left + s(12),
                top + s(8),
                left + s(17),
                top + s(17),
                s(4),
                s(4),
            );
        }
        DeviceGlyph::Mouse => {
            RoundRect(dc, left + s(4), top, left + s(14), top + s(18), s(9), s(9));
            let wheel = CreatePen(PS_SOLID, s(2).max(1), 0x0070_7070);
            let previous = SelectObject(dc, wheel);
            line(9, 4, 9, 8);
            SelectObject(dc, previous);
            DeleteObject(wheel);
        }
        DeviceGlyph::Earbuds => {
            for x in [2, 10] {
                Ellipse(dc, left + s(x), top + s(3), left + s(x + 6), top + s(10));
                RoundRect(
                    dc,
                    left + s(x + 1),
                    top + s(7),
                    left + s(x + 4),
                    top + s(15),
                    s(2),
                    s(2),
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn example(level: u8) -> Layout {
        Layout {
            x: 0,
            y: 0,
            w: 56,
            h: 66,
            dpi: 96,
            light: false,
            cell: 56,
            scale: 100,
            options: WidgetOptions::default(),
            phase: false,
            readings: vec![WidgetReading {
                level,
                low_threshold: 20,
                ..Default::default()
            }],
            edge: true,
            panel_width: 56,
            reveal: 50,
            progress: vec![u16::from(level) * 10],
        }
    }

    #[test]
    fn normal_ring_has_a_warm_stroke() {
        let pixel = edge_pixel(&example(50), 43, 24, 0.0);
        let red = (pixel >> 16) & 255;
        let blue = pixel & 255;
        assert!(red > 100 && red > blue * 2);
    }

    #[test]
    fn zero_percent_retains_the_track_without_a_filled_arc() {
        let pixel = edge_pixel(&example(0), 43, 24, 0.0);
        assert!(((pixel >> 16) & 255) < 100);
    }
}
