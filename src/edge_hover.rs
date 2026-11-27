//! Non-activating companion card for the device under the cursor.
use std::{
    mem::zeroed,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};
use wireless_battery_monitor::palette::Rgb;

#[derive(PartialEq, Eq)]
pub struct Card {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
    pub name: String,
    pub status: String,
    pub surface: Rgb,
    pub border: Rgb,
    pub title: Rgb,
    pub text: Rgb,
}

pub struct Hover {
    hwnd: HWND,
    card: Box<Option<Card>>,
}

impl Default for Hover {
    fn default() -> Self {
        Self {
            hwnd: null_mut(),
            card: Box::new(None),
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn colour((r, g, b): Rgb) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}

impl Hover {
    pub fn hide(&mut self) {
        if self.card.take().is_some() {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }

    pub fn show(&mut self, owner: HWND, card: Card) {
        if self.card.as_ref().as_ref() == Some(&card) {
            return;
        }
        unsafe {
            if self.hwnd.is_null() {
                let class = wide("LvlEdgeHoverCard");
                let instance = GetModuleHandleW(null());
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance,
                    lpszClassName: class.as_ptr(),
                    ..zeroed()
                };
                RegisterClassW(&wc);
                self.hwnd = CreateWindowExW(
                    WS_EX_TOOLWINDOW
                        | WS_EX_NOACTIVATE
                        | WS_EX_TOPMOST
                        | WS_EX_LAYERED
                        | WS_EX_TRANSPARENT,
                    class.as_ptr(),
                    null(),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    owner,
                    null_mut(),
                    instance,
                    (&mut *self.card as *mut Option<Card>).cast(),
                );
                if self.hwnd.is_null() {
                    return;
                }
                SetLayeredWindowAttributes(self.hwnd, 0, 255, LWA_ALPHA);
            }
            let (x, y, width, height, dpi) = (card.x, card.y, card.width, card.height, card.dpi);
            if self
                .card
                .as_ref()
                .as_ref()
                .is_none_or(|old| old.dpi != dpi || old.width != width || old.height != height)
            {
                let radius = (16 * dpi as i32 / 96).max(2);
                let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
                if !region.is_null() && SetWindowRgn(self.hwnd, region, 0) == 0 {
                    DeleteObject(region);
                }
            }
            *self.card = Some(card);
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.hwnd, null(), 0);
        }
    }
}

impl Drop for Hover {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    if msg == WM_NCHITTEST {
        return HTTRANSPARENT as LRESULT;
    }
    if msg == WM_MOUSEACTIVATE {
        return MA_NOACTIVATE as LRESULT;
    }
    if msg == WM_PAINT {
        let mut paint: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut paint);
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Option<Card>;
        if let Some(card) = pointer.as_ref().and_then(Option::as_ref) {
            draw(dc, card);
        }
        EndPaint(hwnd, &paint);
        return 0;
    }
    if msg == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }
    DefWindowProcW(hwnd, msg, w, l)
}

unsafe fn draw(dc: HDC, card: &Card) {
    let px = |value: i32| (value * card.dpi as i32 + 48) / 96;
    let bounds = RECT {
        left: 0,
        top: 0,
        right: card.width,
        bottom: card.height,
    };
    let background = CreateSolidBrush(colour(card.surface));
    let border = CreateSolidBrush(colour(card.border));
    FillRect(dc, &bounds, background);
    FrameRect(dc, &bounds, border);
    DeleteObject(background);
    DeleteObject(border);
    SetBkMode(dc, TRANSPARENT as i32);
    for (text, y, size, weight, ink) in [
        (&card.name, 7, 16, FW_SEMIBOLD, card.title),
        (&card.status, 31, 13, FW_NORMAL, card.text),
    ] {
        let font = CreateFontW(
            -px(size),
            0,
            0,
            0,
            weight as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            ANTIALIASED_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        if font.is_null() {
            continue;
        }
        let previous = SelectObject(dc, font);
        SetTextColor(dc, colour(ink));
        let text = wide(text);
        let mut rect = RECT {
            left: px(12),
            top: px(y),
            right: card.width - px(12),
            bottom: px(y + 22),
        };
        DrawTextW(
            dc,
            text.as_ptr(),
            (text.len() - 1) as i32,
            &mut rect,
            DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX | DT_VCENTER,
        );
        SelectObject(dc, previous);
        DeleteObject(font);
    }
}
