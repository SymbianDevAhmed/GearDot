//! A compact native Windows taskbar indicator. No Explorer injection or hooks.
use crate::{
    icon::DeviceGlyph,
    taskbar_anchor::{taskbar_slot_origin, TaskbarAnchor},
};
use eframe::egui;
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
    Graphics::{
        Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
        Gdi::*,
    },
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};
use wireless_battery_monitor::{
    battery::DeviceKind,
    palette::{Palette, Rgb},
    preferences::{
        device_order, EdgeSide, InputCharge, LiveSnapshot, Settings, Theme, WidgetOptions,
    },
};

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
    pub name: String,
    pub detail: String,
    pub connection: &'static str,
    pub glyph: crate::icon::DeviceGlyph,
    pub level: u8,
    pub unavailable: bool,
    pub charging: bool,
    pub full: bool,
    pub low_threshold: u8,
    pub coarse: Option<&'static str>,
    pub stale: bool,
    pub estimated: bool,
    pub(crate) pinned: bool,
}

impl WidgetReading {
    pub fn low(&self) -> bool {
        !self.stale
            && !self.charging
            && (matches!(self.coarse, Some("Low" | "Empty"))
                || (!self.unavailable && self.level <= self.low_threshold))
    }

    pub fn battery_summary(&self) -> String {
        format!("{}: {}", self.name, self.detail)
    }
}

pub fn widget_readings(
    settings: &Settings,
    snapshot: &LiveSnapshot,
    now: u64,
) -> Vec<WidgetReading> {
    let options = settings.widget_options();
    let mut readings = Vec::new();
    for (kind, visible) in DeviceKind::ALL.into_iter().zip(options.visible) {
        let reading = &snapshot.devices[kind.index()];
        let Some(level) = reading.level.filter(|_| visible && reading.online()) else {
            continue;
        };
        let prefix = if reading.estimated { "~" } else { "" };
        readings.push(WidgetReading {
            id: kind.profile_id().into(),
            name: settings.device_name(kind).to_owned(),
            detail: format!("{prefix}{level}% · {}", reading.state_label()),
            connection: reading.connection.label(),
            glyph: if kind == DeviceKind::Headset {
                DeviceGlyph::Headset
            } else {
                DeviceGlyph::Mouse
            },
            level,
            unavailable: false,
            charging: reading.charging == Some(true),
            full: reading.confirmed_full(),
            low_threshold: settings.profile(kind).low_threshold,
            estimated: reading.estimated,
            ..Default::default()
        });
    }
    for reading in snapshot.connected_bluetooth(now) {
        let id = reading.profile_id();
        if !settings.detected_visible(&id) {
            continue;
        }
        let name = settings.detected_name(&id, &reading.name).to_owned();
        readings.push(WidgetReading {
            id: id.into_owned(),
            name,
            detail: reading
                .percent
                .map_or_else(|| "Battery unavailable".into(), |level| format!("{level}%")),
            connection: "Bluetooth",
            glyph: DeviceGlyph::for_bluetooth(&reading.name, reading.kind),
            level: reading.percent.unwrap_or_default(),
            unavailable: reading.percent.is_none(),
            low_threshold: settings.low_threshold,
            ..Default::default()
        });
    }
    for input in snapshot.connected_inputs(now) {
        let id = input.profile_id();
        if !settings.detected_visible(&id) {
            continue;
        }
        let name = settings.detected_name(&id, &input.name).to_owned();
        let battery = input.battery;
        let coarse = input.status.level_label();
        readings.push(WidgetReading {
            id: id.into_owned(),
            name,
            glyph: DeviceGlyph::for_detected(&input.name, input.kind, input.connection()),
            connection: input.connection().label(),
            unavailable: battery.is_none(),
            coarse,
            low_threshold: settings.low_threshold,
            level: battery.map_or(0, |reading| reading.percent),
            full: battery
                .is_some_and(|reading| reading.charge == InputCharge::Full && !reading.stale),
            charging: battery
                .is_some_and(|reading| reading.charge == InputCharge::Charging && !reading.stale),
            stale: battery.is_some_and(|reading| reading.stale),
            estimated: battery.is_some_and(|reading| reading.estimated),
            pinned: false,
            detail: battery.map_or_else(
                || coarse.unwrap_or("Battery unavailable").to_owned(),
                |reading| {
                    let prefix = if reading.estimated { "~" } else { "" };
                    if reading.stale {
                        format!(
                            "{prefix}{}% · last read {}s ago",
                            reading.percent,
                            now.saturating_sub(reading.checked_at)
                        )
                    } else {
                        format!(
                            "{prefix}{}% · {}",
                            reading.percent,
                            input.connection().label()
                        )
                    }
                },
            ),
        });
    }
    readings.sort_by_key(|reading| {
        device_order(
            settings.placement(&reading.id).pinned,
            reading.low(),
            (!reading.unavailable).then_some(reading.level),
        )
    });
    readings
}

pub fn readings_for_display(
    readings: &[WidgetReading],
    settings: &Settings,
    edge: bool,
) -> Vec<WidgetReading> {
    readings
        .iter()
        .filter_map(|reading| {
            let placement = settings.placement(&reading.id);
            let visible = (if edge {
                placement.edge
            } else {
                placement.taskbar
            })
            .unwrap_or(edge || !reading.unavailable || reading.coarse.is_some());
            visible.then(|| WidgetReading {
                pinned: placement.pinned,
                ..reading.clone()
            })
        })
        .collect()
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
    system_light: bool,
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
    hover: crate::edge_hover::Hover,
    taskbar_geometry: Option<(HWND, [i32; 4], u32)>,
    taskbar_hidden_since: Option<Instant>,
    strip: HWND,
    strip_parent: HWND,
    anchor: TaskbarAnchor,
    emit: Box<dyn Fn(WidgetEvent)>,
}

/// Owns the edge window or a taskbar controller and its Explorer child strip.
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
            system_light: if edge {
                crate::runtime::system_light_theme()
            } else {
                light_taskbar()
            },
            options,
            phase: false,
            readings: Vec::new(),
            motion: BTreeMap::new(),
            shown: false,
            render_failed: false,
            last_layout: None,
            hover: Default::default(),
            taskbar_geometry: None,
            taskbar_hidden_since: None,
            strip: null_mut(),
            strip_parent: null_mut(),
            anchor: TaskbarAnchor::default(),
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
                WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | if edge {
                        WS_EX_LAYERED | WS_EX_TOPMOST
                    } else {
                        0
                    },
                class.as_ptr(),
                wide("GearDot").as_ptr(),
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
            if SetTimer(hwnd, 1, if edge { 40 } else { 125 }, None) == 0 {
                DestroyWindow(hwnd);
                return None;
            }
            refresh_safely(hwnd, &mut state.borrow_mut());
            Some(Self { hwnd, state })
        }
    }

    pub fn update(&mut self, mut readings: Vec<WidgetReading>, options: WidgetOptions) {
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
        if !state.edge {
            // A battery percentage change must not reorder existing taskbar icons.
            readings.sort_by_key(|reading| {
                (
                    !reading.pinned,
                    state
                        .readings
                        .iter()
                        .position(|old| old.id == reading.id)
                        .unwrap_or(usize::MAX),
                )
            });
        }
        state.readings = readings;
        state.options = options;
        unsafe {
            refresh_safely(self.hwnd, &mut state);
        }
    }
}

impl Drop for TaskbarWidget {
    fn drop(&mut self) {
        unsafe {
            KillTimer(self.hwnd, 1);
            let state = self.state.get_mut();
            destroy_strip(self.hwnd, state);
            drop(std::mem::take(&mut state.hover));
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
            state.system_light = if state.edge {
                crate::runtime::system_light_theme()
            } else {
                light_taskbar()
            };
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

pub(crate) fn light_taskbar() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("SystemUsesLightTheme");
    let mut data: u32 = 0;
    let mut length = size_of::<u32>() as u32;
    unsafe {
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
}

unsafe fn visibility(hwnd: HWND, state: &mut WidgetState, visible: bool) {
    if state.shown != visible {
        state.shown = visible;
        let surface = if state.edge { hwnd } else { state.strip };
        if !surface.is_null() {
            ShowWindow(surface, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }
    if !visible {
        state.hover.hide();
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

unsafe fn foreground_covers_monitor(hwnd: HWND, taskbar: HWND, screen: RECT) -> bool {
    let foreground = GetForegroundWindow();
    if foreground.is_null()
        || foreground == hwnd
        || foreground == taskbar
        || foreground == GetShellWindow()
    {
        return false;
    }

    // The notification overflow and other Explorer flyouts are shell UI.
    let mut foreground_process = 0;
    let mut taskbar_process = 0;
    GetWindowThreadProcessId(foreground, &mut foreground_process);
    GetWindowThreadProcessId(taskbar, &mut taskbar_process);
    if foreground_process != 0 && foreground_process == taskbar_process {
        return false;
    }

    // GetWindowRect can extend under the taskbar for an ordinary maximised window.
    let mut bounds: RECT = zeroed();
    if DwmGetWindowAttribute(
        foreground,
        DWMWA_EXTENDED_FRAME_BOUNDS as u32,
        (&mut bounds as *mut RECT).cast(),
        size_of::<RECT>() as u32,
    ) < 0
        && (IsZoomed(foreground) != 0 || GetWindowRect(foreground, &mut bounds) == 0)
    {
        return false;
    }
    bounds.left <= screen.left
        && bounds.top <= screen.top
        && bounds.right >= screen.right
        && bounds.bottom >= screen.bottom
}

unsafe fn strip_is_ours(controller: HWND, strip: HWND) -> bool {
    !strip.is_null()
        && GetWindowLongPtrW(strip, GWLP_USERDATA) == GetWindowLongPtrW(controller, GWLP_USERDATA)
        && IsWindow(strip) != 0
}

unsafe fn destroy_strip(controller: HWND, state: &mut WidgetState) {
    if strip_is_ours(controller, state.strip) {
        DestroyWindow(state.strip);
    }
    state.strip = null_mut();
    state.strip_parent = null_mut();
    state.shown = false;
    state.last_layout = None;
}

unsafe fn taskbar_strip(controller: HWND, taskbar: HWND, state: &mut WidgetState) -> HWND {
    if state.strip_parent == taskbar && strip_is_ours(controller, state.strip) {
        return state.strip;
    }
    destroy_strip(controller, state);
    state.strip = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_NOACTIVATE,
        wide("G435BatteryTaskbarWidget").as_ptr(),
        wide("GearDot").as_ptr(),
        WS_CHILD | WS_CLIPSIBLINGS,
        0,
        0,
        1,
        1,
        taskbar,
        null_mut(),
        GetModuleHandleW(null()),
        GetWindowLongPtrW(controller, GWLP_USERDATA) as *const c_void,
    );
    state.strip_parent = taskbar;
    state.strip
}

unsafe fn taskbar_unavailable(hwnd: HWND, state: &mut WidgetState) {
    let now = Instant::now();
    let since = *state.taskbar_hidden_since.get_or_insert(now);
    if state.edge || now.duration_since(since) >= Duration::from_millis(750) {
        visibility(hwnd, state, false);
    }
}

unsafe fn refresh_layout(hwnd: HWND, state: &mut WidgetState) {
    let enabled = if state.edge {
        state.options.edge
    } else {
        state.options.taskbar
    };
    if !enabled {
        visibility(hwnd, state, false);
        return;
    }
    let visible_count = state.readings.len() as i32;
    if visible_count == 0 {
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
        taskbar_unavailable(hwnd, state);
        return;
    }
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(
        MonitorFromWindow(taskbar, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    ) == 0
    {
        taskbar_unavailable(hwnd, state);
        return;
    }
    if foreground_covers_monitor(hwnd, taskbar, monitor.rcMonitor) {
        visibility(hwnd, state, false);
        return;
    }
    let dpi = GetDpiForWindow(taskbar).max(96);
    let (x, y, w, h, cell, scale, panel_width, reveal) = if state.edge {
        let work = monitor.rcWork;
        let scale = state.options.scale;
        let cell = px((56 * i32::from(scale) + 50) / 100, dpi);
        let panel_width = px((56 * i32::from(scale) + 50) / 100, dpi);
        let work_height = work.bottom - work.top;
        let available = ((work_height - px(16, dpi)) / cell).min(visible_count);
        if available <= 0 || work.right - work.left < panel_width {
            visibility(hwnd, state, false);
            return;
        }
        let panel_height = cell * available + px(10, dpi);
        let grip_width = px(6, dpi);
        let grip_height = px(50, dpi);
        let half_height = (panel_height.max(grip_height) + 1) / 2;
        let middle = work.top
            + half_height
            + (work_height - 2 * half_height) * i32::from(state.options.edge_height_percent) / 100;
        let left = state.options.edge_side == EdgeSide::Left;
        let hit_width = grip_width.max(state.last_layout.as_ref().map_or(0, |layout| layout.w));
        let hit_left = if left {
            work.left
        } else {
            work.right - hit_width
        };
        let mut cursor: POINT = zeroed();
        let over_panel = GetCursorPos(&mut cursor) != 0
            && (hit_left..hit_left + hit_width).contains(&cursor.x)
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
        let x = if left { work.left } else { work.right - w };
        (x, middle - h / 2, w, h, cell, scale, panel_width, reveal)
    } else {
        let h = bar.bottom - bar.top;
        let shown_height =
            bar.bottom.min(monitor.rcMonitor.bottom) - bar.top.max(monitor.rcMonitor.top);
        if h < px(20, dpi) || bar.right - bar.left < h * 4 || shown_height < h / 2 {
            taskbar_unavailable(hwnd, state);
            return;
        }
        let geometry = (taskbar, [bar.left, bar.top, bar.right, bar.bottom], dpi);
        if state.taskbar_geometry != Some(geometry) {
            state.taskbar_geometry = Some(geometry);
            state.anchor = TaskbarAnchor::default();
        }
        let primary = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        let fallback = if taskbar != primary {
            bar.right - px(150, dpi)
        } else {
            bar.right - px(250, dpi)
        };
        let measured = (!tray.is_null()
            && GetWindowRect(tray, &mut notify) != 0
            && notify.right > notify.left
            && notify.left > bar.left
            && notify.left <= bar.right
            && notify.top < bar.bottom
            && notify.bottom > bar.top)
            .then_some(notify.left);
        let anchor = state.anchor.position(measured, fallback, Instant::now());
        let scale = state
            .options
            .scale
            .min(((h - px(4, dpi)).max(1) * 100 / px(18, dpi).max(1)) as u16)
            .max(70);
        let cell = px((57 * i32::from(scale) + 50) / 100, dpi);
        let right = anchor - px(i32::from(state.options.offset), dpi);
        let slots = ((bar.right - bar.left) / 3 / cell)
            .min((right - bar.left - px(12, dpi)) / cell)
            .min(visible_count);
        if slots <= 0 {
            taskbar_unavailable(hwnd, state);
            return;
        }
        state.taskbar_hidden_since = None;
        let w = cell * slots;
        (right - w, bar.top, w, h, cell, scale, w, 0)
    };
    let light = if state.edge {
        state
            .options
            .visual_theme
            .is_light(match state.options.theme {
                Theme::System => state.system_light,
                Theme::Light => true,
                Theme::Dark => false,
            })
    } else {
        state.system_light
    };
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
                let level = state
                    .motion
                    .get(&reading.id)
                    .map_or(f32::from(reading.level), |motion| motion.current(now));
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
    let surface = if state.edge {
        hwnd
    } else {
        taskbar_strip(hwnd, taskbar, state)
    };
    if surface.is_null() {
        return;
    }
    let layout_changed = state.last_layout.as_ref() != Some(&layout);
    if layout_changed {
        if !paint(surface, &layout) {
            return;
        }
    }
    if !state.edge && (layout_changed || !GetWindow(surface, GW_HWNDPREV).is_null()) {
        let mut position = POINT {
            x: layout.x,
            y: layout.y,
        };
        if ScreenToClient(taskbar, &mut position) == 0
            || SetWindowPos(
                surface,
                HWND_TOP,
                position.x,
                position.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE,
            ) == 0
        {
            return;
        }
    }
    if layout_changed {
        state.last_layout = Some(layout);
    }
    visibility(hwnd, state, true);
    if state.edge {
        if let Some(layout) = state.last_layout.as_ref() {
            update_hover(hwnd, &mut state.hover, layout, monitor.rcWork);
        }
    }
}

fn update_hover(owner: HWND, hover: &mut crate::edge_hover::Hover, layout: &Layout, work: RECT) {
    let mut cursor = POINT { x: 0, y: 0 };
    let inside = layout.w == layout.panel_width
        && layout.reveal != 0
        && unsafe { GetCursorPos(&mut cursor) } != 0
        && (layout.x..layout.x + layout.w).contains(&cursor.x)
        && (layout.y..layout.y + layout.h).contains(&cursor.y);
    let slot = (cursor.y.saturating_sub(layout.y) / layout.cell) as usize;
    let Some(reading) = layout.readings.get(slot).filter(|_| inside) else {
        hover.hide();
        return;
    };
    let dpi = layout.dpi;
    let width = px(258, dpi);
    let height = px(59, dpi);
    let gap = px(8, dpi);
    if work.right - work.left < width + layout.w + gap || work.bottom - work.top < height {
        hover.hide();
        return;
    }
    let x = if layout.options.edge_side == EdgeSide::Left {
        layout.x + layout.w + gap
    } else {
        layout.x - width - gap
    };
    let y = (layout.y + slot as i32 * layout.cell + (layout.cell - height) / 2)
        .clamp(work.top, work.bottom - height);
    let palette = layout.options.visual_theme.palette(layout.light);
    let value = reading
        .detail
        .split(" · ")
        .next()
        .unwrap_or(&reading.detail);
    let connection = match reading.connection {
        "" | "Unknown" => "Connected",
        value => value,
    };
    let status = if reading.stale {
        reading.detail.clone()
    } else if reading.low() {
        format!("{value} · Low battery")
    } else if reading.full {
        "Fully charged · 100%".into()
    } else if reading.charging {
        format!("Charging · {value}")
    } else if reading.unavailable && reading.coarse.is_none() && connection == "Wired" {
        "Wired".into()
    } else {
        format!("{connection} · {value}")
    };
    hover.show(
        owner,
        crate::edge_hover::Card {
            x,
            y,
            width,
            height,
            dpi,
            name: reading.name.clone(),
            status,
            surface: palette.surface,
            border: palette.border,
            title: palette.text,
            text: if reading.unavailable && reading.coarse.is_none() || reading.stale {
                palette.muted
            } else {
                reading_colour(layout, palette, reading)
            },
        },
    );
}

unsafe fn paint(hwnd: HWND, layout: &Layout) -> bool {
    render_bitmap(layout, |dc, _| {
        let pos = POINT {
            x: layout.x,
            y: layout.y,
        };
        let src = POINT { x: 0, y: 0 };
        let size = SIZE {
            cx: layout.w,
            cy: layout.h,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(
            hwnd,
            null_mut(),
            if layout.edge { &pos } else { null() },
            &size,
            dc,
            &src,
            0,
            &blend,
            ULW_ALPHA,
        ) != 0
    })
    .unwrap_or(false)
}

unsafe fn render_bitmap<T>(
    layout: &Layout,
    use_pixels: impl FnOnce(HDC, &[u32]) -> T,
) -> Option<T> {
    let Layout {
        w: width,
        h: height,
        dpi,
        cell,
        scale,
        options,
        edge,
        panel_width,
        ..
    } = *layout;
    let dc = CreateCompatibleDC(null_mut());
    if dc.is_null() {
        return None;
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
        return None;
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
    let edge_content_offset = if edge && options.edge_side == EdgeSide::Left {
        panel_width - width
    } else {
        0
    };
    for (slot, reading) in layout.readings.iter().enumerate() {
        let origin = if edge {
            cell * slot as i32
        } else {
            taskbar_slot_origin(width, cell, slot)
        };
        let (left, top) = if edge {
            (panel_width / 2 - s(7) - edge_content_offset, origin + s(17))
        } else {
            (origin + s(2), (height - s(17)) / 2)
        };
        let glyph_size = |unit| if edge { s(unit * 4 / 5) } else { s(unit) };
        draw_glyph(dc, left, top, glyph_size, reading.glyph);
        let prefix = if reading.estimated { "~" } else { "" };
        let label = if reading.unavailable {
            reading.coarse.unwrap_or("—").to_owned()
        } else if reading.full {
            "Full".to_owned()
        } else if reading.charging {
            format!("{prefix}{}%⚡", reading.level)
        } else {
            format!("{prefix}{}%", reading.level)
        };
        let text = wide(&label);
        let (mut rect, align) = if edge {
            (
                RECT {
                    left: -edge_content_offset,
                    top: origin + s(39),
                    right: panel_width - edge_content_offset,
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
    let palette = if layout.edge {
        layout.options.visual_theme.palette(layout.light)
    } else {
        Palette::for_theme(layout.light)
    };
    let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), count);
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let x = (index % width as usize) as i32;
        let y = (index / width as usize) as i32;
        let coverage = (*pixel & 0xff) as f32 / 255.0;
        if edge {
            *pixel = edge_pixel(layout, palette, x, y, coverage);
        } else if let Some(reading) = layout.readings.get((x / cell) as usize) {
            let label_start = px((21 * i32::from(layout.scale) + 50) / 100, layout.dpi);
            let ink = if x % cell < label_start {
                reading.glyph.tint(layout.light)
            } else {
                reading_colour(layout, palette, reading)
            };
            *pixel = blend_pixel(0, ink, coverage);
        }
    }
    let result = use_pixels(dc, pixels);
    SelectObject(dc, old_bitmap);
    DeleteObject(bitmap);
    DeleteDC(dc);
    Some(result)
}

#[derive(Default)]
pub struct WidgetPreview(Option<(Layout, egui::TextureHandle)>);

impl WidgetPreview {
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        options: WidgetOptions,
        readings: Vec<WidgetReading>,
        edge: bool,
        expanded: bool,
        taskbar_light: bool,
    ) -> Option<&egui::TextureHandle> {
        let scale = options.scale;
        let cell = ((if edge { 56 } else { 57 }) * i32::from(scale) + 50) / 100;
        let count = readings.len() as i32;
        let mut layout = Layout {
            x: 0,
            y: 0,
            w: cell * count,
            h: 32,
            dpi: 96,
            light: taskbar_light,
            cell,
            scale,
            options,
            phase: false,
            readings,
            edge,
            panel_width: cell,
            reveal: 0,
            progress: Vec::new(),
        };
        if edge {
            layout.light = options.visual_theme.is_light(match options.theme {
                Theme::System => taskbar_light,
                Theme::Light => true,
                Theme::Dark => false,
            });
            layout.w = if expanded { cell } else { 6 };
            layout.h = if expanded { cell * count + 10 } else { 50 };
            layout.reveal = if expanded { cell - 6 } else { 0 };
            layout.progress = layout
                .readings
                .iter()
                .map(|r| u16::from(r.level) * 10)
                .collect();
        }
        if self
            .0
            .as_ref()
            .is_none_or(|(previous, _)| previous != &layout)
        {
            let width = layout.w as usize;
            let height = layout.h as usize;
            let image = unsafe {
                render_bitmap(&layout, |_, pixels| {
                    let rgba: Vec<_> = pixels
                        .iter()
                        .flat_map(|pixel| {
                            let [b, g, r, a] = pixel.to_le_bytes();
                            [r, g, b, a]
                        })
                        .collect();
                    egui::ColorImage::from_rgba_premultiplied([width, height], &rgba)
                })
            };
            self.0 = image.map(|image| {
                (
                    layout,
                    ctx.load_texture("widget-preview", image, egui::TextureOptions::LINEAR),
                )
            });
        }
        self.0.as_ref().map(|(_, texture)| texture)
    }
}

fn reading_colour(layout: &Layout, palette: &Palette, reading: &WidgetReading) -> Rgb {
    if reading.stale {
        return palette.muted;
    }
    if reading.unavailable {
        return match reading.coarse {
            Some("Empty" | "Low") => palette.low,
            Some("Medium") => palette.battery_level(60, reading.low_threshold),
            Some("Full") => palette.full,
            _ => palette.muted,
        };
    }
    let base = palette.battery_colour(
        reading.level,
        reading.low_threshold,
        reading.charging,
        reading.full,
    );
    if !reading.charging || reading.full || !layout.phase {
        return base;
    }
    let pulse = |channel: u8, text: u8| ((3 * u16::from(channel) + u16::from(text)) / 4) as u8;
    (
        pulse(base.0, palette.text.0),
        pulse(base.1, palette.text.1),
        pulse(base.2, palette.text.2),
    )
}

/// Premultiplied BGRA for UpdateLayeredWindow, using a monochrome GDI text mask.
fn blend_pixel(dst: u32, colour: Rgb, alpha: f32) -> u32 {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    let inverse = 255 - a;
    let over = |value: u32, shift: u32| {
        ((value * a + ((dst >> shift) & 0xff) * inverse + 127) / 255) << shift
    };
    over(255, 24)
        | over(u32::from(colour.0), 16)
        | over(u32::from(colour.1), 8)
        | over(u32::from(colour.2), 0)
}

fn rounded_rect_distance(x: f32, y: f32, half_width: f32, half_height: f32, radius: f32) -> f32 {
    let dx = x.abs() - half_width + radius;
    let dy = y.abs() - half_height + radius;
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius
}

fn edge_pixel(layout: &Layout, palette: &Palette, x: i32, y: i32, mask: f32) -> u32 {
    use std::f32::consts::TAU;

    let scale = f32::from(layout.scale) / 100.0 * layout.dpi as f32 / 96.0;
    let left = layout.options.edge_side == EdgeSide::Left;
    let x = x as f32 + 0.5;
    let py = y as f32 + 0.5;
    let half_h = layout.h as f32 / 2.0;
    if layout.reveal == 0 {
        let x = if left { layout.w as f32 - x } else { x };
        let half_w = layout.w as f32 / 2.0;
        let dist =
            rounded_rect_distance(x - half_w - 1.0, py - half_h, half_w + 1.0, half_h, half_w);
        let accent =
            if layout.options.accent == wireless_battery_monitor::preferences::Accent::System {
                palette.accent
            } else {
                layout.options.accent.rgb(layout.light)
            };
        return blend_pixel(0, accent, 0.90 * (0.5 - dist).clamp(0.0, 1.0));
    }

    let x = if left {
        x + (layout.panel_width - layout.w) as f32
    } else {
        x
    };
    let half_w = layout.panel_width as f32 / 2.0;
    let dist = rounded_rect_distance(x - half_w, py - half_h, half_w, half_h, 10.0 * scale);
    let outer = (0.5 - dist).clamp(0.0, 1.0);
    if outer == 0.0 {
        return 0;
    }
    let mut out = blend_pixel(0, palette.surface, 0.98 * outer);
    out = blend_pixel(
        out,
        palette.border,
        0.85 * outer * (1.0 + dist).clamp(0.0, 1.0),
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
    let base = reading_colour(layout, palette, reading);
    if radial_distance < thickness + 3.0 * scale {
        out = blend_pixel(
            out,
            palette.border,
            (thickness + 0.5 - radial_distance).clamp(0.0, 1.0),
        );
        let angle = dx.atan2(-dy).rem_euclid(TAU);
        if let Some(level) = reading.coarse {
            let filled_segments = match level {
                "Empty" => 0,
                "Low" => 1,
                "Medium" => 2,
                _ => 3,
            };
            let segment = angle * 3.0 / TAU;
            if (segment as u8) < filled_segments && segment.fract() < 0.84 {
                out = blend_pixel(
                    out,
                    base,
                    (thickness + 0.5 - radial_distance).clamp(0.0, 1.0),
                );
            }
        }
        let fraction = f32::from(layout.progress[slot]) / 1000.0;
        if fraction > 0.0 {
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
    let label = if y % layout.cell >= (39.0 * scale).floor() as i32 {
        base
    } else {
        reading.glyph.tint(layout.light)
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
        DeviceGlyph::Keyboard => {
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            RoundRect(
                dc,
                left + s(1),
                top + s(4),
                left + s(18),
                top + s(14),
                s(4),
                s(4),
            );
            for x in [4, 7, 10, 13] {
                line(x, 7, x + 1, 7);
            }
            line(5, 10, 13, 10);
            SelectObject(dc, old_brush);
        }
        DeviceGlyph::Gamepad => {
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let outline = [
                (1, 14),
                (3, 7),
                (6, 4),
                (12, 4),
                (15, 7),
                (17, 14),
                (15, 15),
                (11, 11),
                (7, 11),
                (3, 15),
                (1, 14),
            ]
            .map(|(x, y)| POINT {
                x: left + s(x),
                y: top + s(y),
            });
            Polyline(dc, outline.as_ptr(), outline.len() as i32);
            line(4, 8, 8, 8);
            line(6, 6, 6, 10);
            line(11, 7, 12, 7);
            line(13, 9, 14, 9);
            SelectObject(dc, old_brush);
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
    fn medium_ring_uses_amber() {
        let pixel = edge_pixel(&example(50), Palette::for_theme(false), 43, 24, 0.0);
        let red = (pixel >> 16) & 255;
        let blue = pixel & 255;
        assert!(red > 100 && red > blue);
    }

    #[test]
    fn zero_percent_retains_the_track_without_a_filled_arc() {
        let pixel = edge_pixel(&example(0), Palette::for_theme(false), 43, 24, 0.0);
        assert!(((pixel >> 16) & 255) < 100);
    }
}
