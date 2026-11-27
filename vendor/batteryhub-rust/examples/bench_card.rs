//! Benchmark of one frame of the hover card's animation: v1.5's code (copied below, unchanged
//! apart from taking its colours as arguments) against the current one in src/cardfx.rs.
//!
//!     cargo run --release --example bench_card
//!
//! A card of 350 x 284 px at 150 % scaling (525 x 426) with text-like content. "Switch" is a
//! frame of the move from one device's card to the next (two layers, both blurred), "show" a
//! frame of the fade-in (one blurred layer). Allocations are counted with a wrapping allocator.
use std::alloc::{GlobalAlloc, Layout, System};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use batteryhub::cardfx::{BlurScratch, CardBg, CardContent, compose_card};
use batteryhub::ui::{rgb, split};
use windows::Win32::Foundation::COLORREF;

struct Counting;
static BYTES: AtomicUsize = AtomicUsize::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        BYTES.fetch_add(l.size(), Ordering::Relaxed);
        COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static A: Counting = Counting;

const W: i32 = 525;
const H: i32 = 426;

/// Text-like content: short runs of opaque pixels on about 15 % of the card, transparent elsewhere.
fn content(seed: u32) -> Vec<u32> {
    let mut v = vec![0u32; (W * H) as usize];
    let mut x = seed.wrapping_mul(2_654_435_761);
    for (i, p) in v.iter_mut().enumerate() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let row = (i as i32 / W) % 40;
        if row < 18 && x.is_multiple_of(7) {
            *p = 0xFFFF_FFFF;
        }
    }
    v
}

// ---- v1.5 ---------------------------------------------------------------------------------------

mod old {
    use super::*;

    pub struct Canvas {
        pub w: i32,
        pub h: i32,
        pub px: Vec<u32>,
    }

    impl Canvas {
        #[allow(clippy::too_many_arguments)]
        pub fn fill_rrect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, radius: i32, c: COLORREF, a: u32) {
            let r = radius.max(0).min((x1 - x0) / 2).min((y1 - y0) / 2);
            for y in y0..y1 {
                for x in x0..x1 {
                    let dx = if x < x0 + r {
                        x0 + r - x
                    } else if x >= x1 - r {
                        x - (x1 - r - 1)
                    } else {
                        0
                    };
                    let dy = if y < y0 + r {
                        y0 + r - y
                    } else if y >= y1 - r {
                        y - (y1 - r - 1)
                    } else {
                        0
                    };
                    if dx * dx + dy * dy <= r * r {
                        self.blend(x, y, c, a);
                    }
                }
            }
        }

        fn blend(&mut self, x: i32, y: i32, c: COLORREF, a: u32) {
            if x < 0 || y < 0 || x >= self.w || y >= self.h || a == 0 {
                return;
            }
            let (r, g, b) = split(c);
            let d = &mut self.px[(y * self.w + x) as usize];
            let inv = 255 - a;
            let ch = |src: u8, shift: u32| ((src as u32 * a + ((*d >> shift) & 0xFF) * inv) / 255) & 0xFF;
            let na = (a + ((*d >> 24) & 0xFF) * inv / 255).min(255);
            *d = na << 24 | ch(r, 16) << 16 | ch(g, 8) << 8 | ch(b, 0);
        }
    }

    pub fn blur(src: &[u32], w: i32, h: i32, r: i32) -> Vec<u32> {
        if r <= 0 || w <= 0 || h <= 0 {
            return src.to_vec();
        }
        let (w, h) = (w as usize, h as usize);
        let mut ch: [Vec<u32>; 4] = std::array::from_fn(|k| src.iter().map(|p| (p >> (k * 8)) & 0xFF).collect());
        let mut tmp = vec![0u32; w * h];
        let r = r as usize;
        let div = (2 * r + 1) as u32;
        for c in ch.iter_mut() {
            for _ in 0..2 {
                for y in 0..h {
                    let row = &c[y * w..y * w + w];
                    let mut sum: u32 = (0..=2 * r).map(|i| row[i.saturating_sub(r).min(w - 1)]).sum();
                    for x in 0..w {
                        tmp[y * w + x] = sum / div;
                        sum += row[(x + r + 1).min(w - 1)];
                        sum -= row[x.saturating_sub(r)];
                    }
                }
                for x in 0..w {
                    let at = |y: usize| tmp[y.min(h - 1) * w + x];
                    let mut sum: u32 = (0..=2 * r).map(|i| at(i.saturating_sub(r))).sum();
                    for y in 0..h {
                        c[y * w + x] = sum / div;
                        sum += at(y + r + 1);
                        sum -= at(y.saturating_sub(r));
                    }
                }
            }
        }
        (0..w * h).map(|i| ch[3][i] << 24 | ch[2][i] << 16 | ch[1][i] << 8 | ch[0][i]).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn compose_card(w: i32, h: i32, bx: i32, bw: i32, radius: i32, bg: COLORREF, border: COLORREF, layers: &[(&[u32], i32, f32, i32)]) -> Vec<u32> {
        let mut cv = Canvas { w, h, px: vec![0; (w * h) as usize] };
        cv.fill_rrect(bx, 0, bx + bw, h, radius, border, 255);
        cv.fill_rrect(bx + 1, 1, bx + bw - 1, h - 1, radius - 1, bg, 250);
        for &(src, sw, f, r) in layers {
            let f = (f.clamp(0.0, 1.0) * 255.0) as u32;
            if f == 0 {
                continue;
            }
            let blurred;
            let src: &[u32] = if r > 0 {
                blurred = blur(src, sw, src.len() as i32 / sw.max(1), r);
                &blurred
            } else {
                src
            };
            for y in 0..h.min(src.len() as i32 / sw.max(1)) {
                for x in 0..sw.min(bw) {
                    let c = src[(y * sw + x) as usize];
                    let ca = (c >> 24) * f / 255;
                    if ca == 0 {
                        continue;
                    }
                    let d = &mut cv.px[(y * w + bx + x) as usize];
                    let inv = 255 - ca;
                    let ch = |sh: u32| ((((c >> sh) & 0xFF) * f / 255) + ((*d >> sh) & 0xFF) * inv / 255).min(255);
                    *d = ch(24) << 24 | ch(16) << 16 | ch(8) << 8 | ch(0);
                }
            }
        }
        cv.px
    }
}

fn measure(name: &str, frames: usize, mut f: impl FnMut(usize)) -> (f64, f64, f64) {
    f(0); // warm-up
    BYTES.store(0, Ordering::Relaxed);
    COUNT.store(0, Ordering::Relaxed);
    let t = Instant::now();
    for i in 0..frames {
        f(i);
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    let kb = BYTES.load(Ordering::Relaxed) as f64 / 1024.0 / frames as f64;
    let n = COUNT.load(Ordering::Relaxed) as f64 / frames as f64;
    println!("{name:<34} {ms:>8.3} ms/frame  {kb:>9.1} KB allocated/frame  {n:>6.1} allocations/frame");
    (ms, kb, n)
}

fn main() {
    let (a, b) = (content(1), content(2));
    let (bg, border) = (rgb(44, 44, 44), rgb(70, 70, 70));
    let frames = 120;
    println!("Card {W} x {H} px (350 x 284 at 150 %), {frames} frames each\n");

    // Blur radius of a frame in the middle of the animation: BLUR_MAX 6 px x 1.5 x ~0.5.
    let r = 5;
    let old_switch = measure("v1.5  switch (2 layers, blurred)", frames, |i| {
        let e = (i % 14) as f32 / 14.0;
        let px = old::compose_card(W + 60, H, 30, W, 12, bg, border, &[(&a, W, 1.0 - e, r), (&b, W, e, r)]);
        std::hint::black_box(px);
    });
    let old_show = measure("v1.5  show (1 layer, blurred)", frames, |_| {
        std::hint::black_box(old::compose_card(W, H, 0, W, 12, bg, border, &[(&a, W, 1.0, r)]));
    });

    let ca = Rc::new(CardContent { sig: 1, key: "a".into(), w: W, h: H, px: a.clone(), half: Default::default() });
    let cb = Rc::new(CardContent { sig: 2, key: "b".into(), w: W, h: H, px: b.clone(), half: Default::default() });
    let mut dst = vec![0u32; ((W + 60) * H) as usize];
    let mut scratch = BlurScratch::default();
    let stride = (W + 60) as usize;
    let new_switch = measure("now   switch (2 layers, blurred)", frames, |i| {
        let e = (i % 14) as f32 / 14.0;
        let bgc = CardBg { bx: 30, bw: W, h: H, radius: 12, bg, border };
        compose_card(&mut dst, stride, W + 60, H, &bgc, &[(ca.clone(), 1.0 - e, r), (cb.clone(), e, r)], &mut scratch);
        std::hint::black_box(&dst);
    });
    let new_show = measure("now   show (1 layer, blurred)", frames, |_| {
        let bgc = CardBg { bx: 0, bw: W, h: H, radius: 12, bg, border };
        compose_card(&mut dst, stride, W, H, &bgc, &[(ca.clone(), 1.0, r)], &mut scratch);
        std::hint::black_box(&dst);
    });
    println!();
    println!("switch: {:.1}x faster, {:.0} KB -> {:.0} KB allocated per frame", old_switch.0 / new_switch.0, old_switch.1, new_switch.1);
    println!("show:   {:.1}x faster, {:.0} KB -> {:.0} KB allocated per frame", old_show.0 / new_show.0, old_show.1, new_show.1);
    // At ~64 frames per second (the 15 ms timer), as a share of one of the CPU's threads.
    println!("CPU at 64 frames/s while switching: v1.5 {:.1}% of one thread, now {:.1}%", old_switch.0 * 64.0 / 10.0, new_switch.0 * 64.0 / 10.0);
}
