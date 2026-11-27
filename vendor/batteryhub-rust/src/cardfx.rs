//! The hover card's pixel work: blending card contents over the rounded background, with a blur
//! that runs at half resolution on four 16-bit channels packed in one u64. Kept apart from the
//! window code so it can be tested and benchmarked (examples/bench_card.rs).
use std::rc::Rc;

use windows::Win32::Foundation::COLORREF;

use crate::ui::split;

/// A card's rendered content (premultiplied BGRA, transparent around the text and chart).
pub struct CardContent {
    pub sig: u64,
    pub key: String,
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
    /// Half-resolution copy for the blur, made the first time the content is blurred.
    pub half: std::cell::OnceCell<Half>,
}

/// Pixels at half the width and height, 16 bits per channel (b, g, r, a) in one u64, so the blur
/// adds and subtracts all four channels at once.
pub struct Half {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u64>,
}

pub const LANES: u64 = 0x00FF_00FF_00FF_00FF;

pub fn widen(p: u32) -> u64 {
    (p & 0xFF) as u64 | (((p >> 8) & 0xFF) as u64) << 16 | (((p >> 16) & 0xFF) as u64) << 32 | (((p >> 24) & 0xFF) as u64) << 48
}

pub fn narrow(v: u64) -> u32 {
    (v & 0xFF) as u32 | (((v >> 16) & 0xFF) as u32) << 8 | (((v >> 32) & 0xFF) as u32) << 16 | (((v >> 48) & 0xFF) as u32) << 24
}

impl CardContent {
    pub fn half(&self) -> &Half {
        self.half.get_or_init(|| half_of(&self.px, self.w.max(1) as usize, self.h.max(1) as usize))
    }
}

/// 2 x 2 average (edges repeat the last row / column).
pub fn half_of(px: &[u32], w: usize, h: usize) -> Half {
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = Vec::with_capacity(hw * hh);
    for y in 0..hh {
        let (y0, y1) = (2 * y, (2 * y + 1).min(h - 1));
        for x in 0..hw {
            let (x0, x1) = (2 * x, (2 * x + 1).min(w - 1));
            let s = widen(px[y0 * w + x0]) + widen(px[y0 * w + x1]) + widen(px[y1 * w + x0]) + widen(px[y1 * w + x1]);
            out.push((s >> 2) & LANES);
        }
    }
    Half { w: hw, h: hh, px: out }
}

/// Buffers the blur reuses from frame to frame.
#[derive(Default)]
pub struct BlurScratch {
    pub a: Vec<u64>,
    pub b: Vec<u64>,
    pub sums: Vec<u64>,
    /// Blurred layers blended at half size, and where each of its rows has content.
    pub m: Vec<u64>,
    pub spans: Vec<(usize, usize)>,
}

/// Each lane divided by `div` (lanes hold sums of at most `div` values of 0..=255).
pub fn lanes_div(v: u64, inv: u64) -> u64 {
    // Two lanes per multiplication: lanes 0 and 2 (and 1 and 3) are 32 bits apart, and each
    // product (at most 255 * 19 * 65536 / 3) stays under 2^32, so they do not mix.
    const EVEN: u64 = 0x0000_FFFF_0000_FFFF;
    let a = (((v & EVEN) * inv) >> 16) & 0x0000_00FF_0000_00FF;
    let b = ((((v >> 16) & EVEN) * inv) >> 16) & 0x0000_00FF_0000_00FF;
    a | b << 16
}

/// Box blur of radius `r`, twice per axis (close to a Gaussian), of `src` (w x h) into
/// `s.b`. Every pass runs along rows, so memory is read in order.
pub fn blur_lanes(src: &[u64], w: usize, h: usize, r: usize, s: &mut BlurScratch) {
    let n = w * h;
    s.a.resize(n, 0);
    s.b.resize(n, 0);
    s.sums.resize(w, 0);
    let div = (2 * r + 1) as u64;
    let inv = 65536_u64.div_ceil(div);
    let horizontal = |from: &[u64], to: &mut [u64]| {
        for y in 0..h {
            let row = &from[y * w..y * w + w];
            let out = &mut to[y * w..y * w + w];
            let mut sum: u64 = (0..=2 * r).map(|i| row[i.saturating_sub(r).min(w - 1)]).sum();
            for x in 0..w {
                out[x] = lanes_div(sum, inv);
                sum += row[(x + r + 1).min(w - 1)];
                sum -= row[x.saturating_sub(r)];
            }
        }
    };
    let vertical = |from: &[u64], to: &mut [u64], sums: &mut [u64]| {
        for x in 0..w {
            sums[x] = (0..=2 * r).map(|i| from[i.saturating_sub(r).min(h - 1) * w + x]).sum();
        }
        for y in 0..h {
            let (add, sub) = ((y + r + 1).min(h - 1) * w, y.saturating_sub(r) * w);
            for x in 0..w {
                to[y * w + x] = lanes_div(sums[x], inv);
                sums[x] += from[add + x];
                sums[x] -= from[sub + x];
            }
        }
    };
    let (a, b, sums) = (&mut s.a, &mut s.b, &mut s.sums);
    horizontal(src, a);
    vertical(a, b, sums);
    horizontal(b, a);
    vertical(a, b, sums);
}

/// The rounded background of one frame: [bx, bx + bw) x [0, h), border and fill colour.
pub struct CardBg {
    pub bx: i32,
    pub bw: i32,
    pub h: i32,
    pub radius: i32,
    pub bg: COLORREF,
    pub border: COLORREF,
}

/// Premultiplied BGRA of `c` at `a`.
pub fn premul(c: COLORREF, a: u32) -> u32 {
    let (r, g, b) = split(c);
    a << 24 | (r as u32 * a / 255) << 16 | (g as u32 * a / 255) << 8 | (b as u32 * a / 255)
}

/// Columns left out at each end of row `y` of a rounded rectangle of height `h` and radius `r`.
pub fn corner_inset(y: i32, h: i32, r: i32) -> i32 {
    let dy = if y < r {
        r - y
    } else if y >= h - r {
        y - (h - r - 1)
    } else {
        0
    };
    if dy == 0 {
        return 0;
    }
    let dx = ((r * r - dy * dy).max(0) as f32).sqrt() as i32;
    r - dx
}

/// One content layer of a frame: the content, its opacity (0..1) and its blur radius in px.
pub type Layer = (Rc<CardContent>, f32, i32);

/// `src` (16-bit lanes, premultiplied) at opacity `f` (0..=256) over `dst`.
#[inline]
fn over_lanes(dst: u64, src: u64, f: u64) -> u64 {
    let p = ((src * f) >> 8) & LANES;
    let pa = p >> 48;
    if pa == 0 {
        return dst;
    }
    p + (((dst * (256 - pa.min(256))) >> 8) & LANES)
}

/// One frame of the card into `dst` (row stride `stride`, size w x h): the rounded background,
/// then each content layer (content, opacity, blur radius in px) drawn from the background's left
/// edge and clipped to it.
///
/// Blurred layers are blurred at half resolution, and consecutive blurred layers (the two cards
/// of a switch) are blended into one half-resolution image first, so the full-resolution pass
/// that scales it back up smoothly runs once per frame, and only over the part of each row that
/// has content. Sharp layers are blended at full resolution.
pub fn compose_card(dst: &mut [u32], stride: usize, w: i32, h: i32, bg: &CardBg, layers: &[Layer], s: &mut BlurScratch) {
    let (border, fill) = (premul(bg.border, 255), premul(bg.bg, 250));
    let r = bg.radius.max(1).min(bg.bw / 2).min(bg.h / 2);
    for y in 0..h.min(bg.h) {
        let row = &mut dst[y as usize * stride..y as usize * stride + w as usize];
        row.fill(0);
        let outer = corner_inset(y, bg.h, r);
        let (x0, x1) = ((bg.bx + outer).max(0), (bg.bx + bg.bw - outer).min(w));
        if x0 < x1 {
            row[x0 as usize..x1 as usize].fill(border);
        }
        if y > 0 && y < bg.h - 1 {
            let inner = corner_inset(y - 1, bg.h - 2, r - 1);
            let (i0, i1) = ((bg.bx + 1 + inner).max(0), (bg.bx + bg.bw - 1 - inner).min(w));
            if i0 < i1 {
                row[i0 as usize..i1 as usize].fill(fill);
            }
        }
    }
    let opacity = |o: f32| (o.clamp(0.0, 1.0) * 256.0) as u64;
    let clip_w = |c: &CardContent| c.w.min(bg.bw).min(w - bg.bx.max(0)).max(0);
    let mut i = 0;
    while i < layers.len() {
        let (c, o, radius) = &layers[i];
        let f = opacity(*o);
        if f == 0 {
            i += 1;
            continue;
        }
        if *radius <= 1 {
            // Sharp: straight at full resolution.
            let (cw, ch) = (clip_w(c), c.h.min(h));
            for y in 0..ch {
                let src = &c.px[(y * c.w) as usize..];
                let row = &mut dst[y as usize * stride..];
                for x in 0..cw {
                    let p = src[x as usize];
                    if p >> 24 != 0 {
                        let d = &mut row[(bg.bx + x) as usize];
                        *d = narrow(over_lanes(widen(*d), widen(p), f));
                    }
                }
            }
            i += 1;
            continue;
        }
        // A run of blurred layers, blended into one half-size image `m`.
        let mut j = i;
        let (mut mw, mut mh, mut cw, mut ch) = (0usize, 0usize, 0i32, 0i32);
        while j < layers.len() && layers[j].2 > 1 {
            let hf = layers[j].0.half();
            (mw, mh) = (mw.max(hf.w), mh.max(hf.h));
            (cw, ch) = (cw.max(clip_w(&layers[j].0)), ch.max(layers[j].0.h.min(h)));
            j += 1;
        }
        s.m.clear();
        s.m.resize(mw * mh, 0);
        for (c, o, radius) in &layers[i..j] {
            let f = opacity(*o);
            if f == 0 {
                continue;
            }
            let hf = c.half();
            blur_lanes(&hf.px, hf.w, hf.h, (*radius as usize).div_ceil(2), s);
            let (b, m) = (&s.b, &mut s.m);
            for y in 0..hf.h {
                let (src, out) = (&b[y * hf.w..(y + 1) * hf.w], &mut m[y * mw..y * mw + hf.w]);
                for (d, &p) in out.iter_mut().zip(src) {
                    if p != 0 {
                        *d = over_lanes(*d, p, f);
                    }
                }
            }
        }
        // Where each half-size row has content.
        s.spans.clear();
        s.spans.extend((0..mh).map(|y| {
            let row = &s.m[y * mw..(y + 1) * mw];
            match row.iter().position(|&p| p != 0) {
                Some(a) => (a, row.iter().rposition(|&p| p != 0).unwrap_or(a) + 1),
                None => (0, 0),
            }
        }));
        // Scaled back up 2x: full-size pixel x sits at half-size (x - 0.5) / 2, so the weights
        // are always 3/4 and 1/4 (exact bilinear, without multiplications).
        let lerp = |p: u64, q: u64| ((3 * p + q) >> 2) & LANES;
        let m = &s.m;
        let row_buf = &mut s.a;
        row_buf.resize(mw, 0);
        for y in 0..ch {
            let (ya, yb) = if y == 0 { (0, 0) } else { (((y - 1) / 2) as usize, ((y + 1) / 2).min(mh as i32 - 1) as usize) };
            // Odd rows lean to the upper half row, even rows to the lower one.
            let (near, far) = if y % 2 == 1 { (ya, yb) } else { (yb, ya) };
            let (sa, sb) = (s.spans[near], s.spans[far]);
            let (lo, hi) = match (sa.0 < sa.1, sb.0 < sb.1) {
                (false, false) => continue,
                (true, false) => sa,
                (false, true) => sb,
                (true, true) => (sa.0.min(sb.0), sa.1.max(sb.1)),
            };
            let (rn, rf) = (&m[near * mw..(near + 1) * mw], &m[far * mw..(far + 1) * mw]);
            // Half-size columns lo-2 .. hi+2 are read by full-size pixels 2(lo-1) .. 2(hi+1).
            let (f0, f1) = (lo.saturating_sub(2), (hi + 2).min(mw));
            for k in f0..f1 {
                row_buf[k] = lerp(rn[k], rf[k]);
            }
            let (x_from, x_to) = ((lo.saturating_sub(1) * 2) as i32, ((hi + 1) * 2).min(2 * f1) as i32);
            let x_to = x_to.min(cw);
            let row = &mut dst[y as usize * stride..];
            for x in x_from..x_to {
                let k = (x / 2) as usize;
                let p = if x == 0 {
                    row_buf[0]
                } else if x % 2 == 1 {
                    lerp(row_buf[k], row_buf[(k + 1).min(mw - 1)])
                } else {
                    lerp(row_buf[k], row_buf[k - 1])
                };
                if p == 0 {
                    continue;
                }
                let d = &mut row[(bg.bx + x) as usize];
                *d = narrow(over_lanes(widen(*d), p, 256));
            }
        }
        i = j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::rgb;

    #[test]
    fn blur_keeps_flat_areas_and_spreads_a_dot() {
        let mut sc = BlurScratch::default();
        let flat = vec![widen(0x8040_2010); 30 * 10];
        blur_lanes(&flat, 30, 10, 3, &mut sc);
        assert!(sc.b.iter().all(|&v| narrow(v) == 0x8040_2010));
        let mut dot = vec![0u64; 21 * 21];
        dot[10 * 21 + 10] = widen(0xFF00_00FF);
        blur_lanes(&dot, 21, 21, 2, &mut sc);
        let centre = narrow(sc.b[10 * 21 + 10]) >> 24;
        assert!(centre > 0 && centre < 255);
        assert!(narrow(sc.b[10 * 21 + 12]) >> 24 > 0);
        assert_eq!(narrow(sc.b[0]), 0);
    }

    #[test]
    fn half_size_averages_and_lane_helpers() {
        let px = vec![0xFF00_0000u32, 0x0000_0000, 0xFF00_0000, 0x0000_0000];
        let h = half_of(&px, 2, 2);
        assert_eq!((h.w, h.h), (1, 1));
        assert_eq!(narrow(h.px[0]) >> 24, 127);
        assert_eq!(narrow(widen(0x1234_5678)), 0x1234_5678);
        assert_eq!(narrow(lanes_div(widen(0x0A0A_0A0A) * 3, 65536_u64.div_ceil(3))), 0x0A0A_0A0A);
    }

    #[test]
    fn card_frame_composes_background_and_content() {
        let content = Rc::new(CardContent { sig: 0, key: "k".into(), w: 20, h: 10, px: vec![0xFFFF_FFFF; 200], half: std::cell::OnceCell::new() });
        let bg = CardBg { bx: 0, bw: 20, h: 10, radius: 3, bg: rgb(10, 10, 10), border: rgb(50, 50, 50) };
        let mut dst = vec![0u32; 30 * 10];
        let mut sc = BlurScratch::default();
        compose_card(&mut dst, 30, 30, 10, &bg, &[(content.clone(), 1.0, 0)], &mut sc);
        assert_eq!(dst[5 * 30 + 10], 0xFFFF_FFFF); // opaque white content over the card
        assert_eq!(dst[5 * 30 + 25], 0); // right of the card: transparent
        compose_card(&mut dst, 30, 30, 10, &bg, &[], &mut sc);
        assert_eq!(dst[0], 0); // rounded corner of the background
        assert_eq!(dst[5 * 30 + 10], premul(rgb(10, 10, 10), 250));
        compose_card(&mut dst, 30, 30, 10, &bg, &[(content, 0.5, 4)], &mut sc);
        let mid = dst[5 * 30 + 10];
        assert!(mid >> 24 >= 250 && (mid & 0xFF) > 10 && (mid & 0xFF) < 255); // half-faded, blurred
    }

    #[test]
    fn blurred_layer_is_scaled_up_symmetrically() {
        // A square in the middle of a 20 x 20 card: the blurred result must be mirror-symmetric
        // (a wrong half-pixel offset or a stale column would break this).
        let mut px = vec![0u32; 400];
        for y in 8..12 {
            for x in 8..12 {
                px[y * 20 + x] = 0xFFFF_FFFF;
            }
        }
        let c = Rc::new(CardContent { sig: 0, key: "k".into(), w: 20, h: 20, px, half: std::cell::OnceCell::new() });
        let bg = CardBg { bx: 0, bw: 20, h: 20, radius: 2, bg: rgb(0, 0, 0), border: rgb(0, 0, 0) };
        let mut dst = vec![0u32; 400];
        compose_card(&mut dst, 20, 20, 20, &bg, &[(c, 1.0, 4)], &mut BlurScratch::default());
        for y in 1..19 {
            for x in 1..19 {
                let (a, b) = (dst[y * 20 + x] & 0xFF, dst[y * 20 + 19 - x] & 0xFF);
                assert!(a.abs_diff(b) <= 1, "({x}, {y}): {a} vs {b}");
                let (a, b) = (dst[y * 20 + x] & 0xFF, dst[(19 - y) * 20 + x] & 0xFF);
                assert!(a.abs_diff(b) <= 1, "({x}, {y}) vertical: {a} vs {b}");
            }
        }
        // Brightest in the middle, spread beyond the square.
        assert!(dst[9 * 20 + 9] & 0xFF > dst[9 * 20 + 5] & 0xFF);
        assert!(dst[9 * 20 + 6] & 0xFF > 0);
    }
}
