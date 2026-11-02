// Page processing, following KCC's pipeline for a grayscale Kindle:
// crop -> gamma -> autocontrast -> spread split/rotate -> resize -> JPEG.

use crate::convert::Options;
use image::imageops::{self, FilterType};
use image::{GrayImage, ImageReader};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

// Kindle 11 screen
pub const DW: u32 = 1072;
pub const DH: u32 = 1448;
const JPEG_QUALITY: u8 = 85;

pub struct Page {
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub size: u64,
}

pub fn process(src: &Path, o: &Options, idx: usize, dir: &Path) -> Result<Vec<Page>, String> {
    let img = ImageReader::open(src)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let mut g = img.to_luma8();
    drop(img);

    if o.cropping > 0 {
        g = crop(g, o);
    }
    apply_gamma(&mut g, o.gamma);
    autocontrast(&mut g);

    let mut pages = Vec::new();
    for (tag, part) in split(g, o) {
        let out = resize(part, o);
        let path = dir.join(format!("{:05}_{}.jpg", idx, tag));
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(BufWriter::new(file), JPEG_QUALITY);
        enc.encode_image(&out).map_err(|e| e.to_string())?;
        drop(enc);
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        pages.push(Page { path, w: out.width(), h: out.height(), size });
    }
    Ok(pages)
}

// ---------- spread splitter ----------

// Tags sort in reading order: a = rotated (first), b/c = halves, d = rotated (last), x = normal.
fn split(g: GrayImage, o: &Options) -> Vec<(char, GrayImage)> {
    let (w, h) = g.dimensions();
    let landscape = w > h;
    let ratio = w as f32 / h as f32;
    let rot_tag = if o.rotate_first { 'a' } else { 'd' };

    if landscape && w <= DH && h <= DW && o.splitter == 1 {
        return vec![(rot_tag, imageops::rotate270(&g))];
    }
    if landscape && ratio > 1.16 {
        let do_split = o.splitter != 1 && ratio < 1.8;
        let do_rotate = o.splitter > 0 || ratio >= 1.8;
        let mut out = Vec::new();
        if do_rotate && o.rotate_first {
            out.push(('a', imageops::rotate270(&g)));
        }
        if do_split {
            let half = w / 2;
            let left = imageops::crop_imm(&g, 0, 0, half, h).to_image();
            let right = imageops::crop_imm(&g, half, 0, w - half, h).to_image();
            if o.right_to_left {
                out.push(('b', right));
                out.push(('c', left));
            } else {
                out.push(('b', left));
                out.push(('c', right));
            }
        }
        if do_rotate && !o.rotate_first {
            out.push(('d', imageops::rotate270(&g)));
        }
        return out;
    }
    vec![('x', g)]
}

// ---------- resize ----------

fn resize(g: GrayImage, o: &Options) -> GrayImage {
    let (w, h) = g.dimensions();
    let small = w <= DW && h <= DH;
    let filter = if small { FilterType::CatmullRom } else { FilterType::Lanczos3 };

    if o.scale == "stretch" {
        return imageops::resize(&g, DW, DH, filter);
    }
    if small && o.scale != "upscale" {
        return g;
    }
    let ar_img = w as f32 / h as f32;
    let ar_dev = DW as f32 / DH as f32;
    if (ar_img - ar_dev).abs() < 0.015 {
        // nearly the screen's shape: fill the screen, trim the overflow
        let s = f32::max(DW as f32 / w as f32, DH as f32 / h as f32);
        let nw = ((w as f32 * s).ceil() as u32).max(DW);
        let nh = ((h as f32 * s).ceil() as u32).max(DH);
        let r = imageops::resize(&g, nw, nh, filter);
        imageops::crop_imm(&r, (nw - DW) / 2, (nh - DH) / 2, DW, DH).to_image()
    } else {
        let s = f32::min(DW as f32 / w as f32, DH as f32 / h as f32);
        let nw = ((w as f32 * s).round() as u32).clamp(1, DW);
        let nh = ((h as f32 * s).round() as u32).clamp(1, DH);
        imageops::resize(&g, nw, nh, filter)
    }
}

// ---------- tone ----------

fn apply_gamma(g: &mut GrayImage, gamma: f32) {
    if gamma < 0.1 || (gamma - 1.0).abs() < 1e-3 {
        return; // Auto = profile gamma 1.0
    }
    let lut: Vec<u8> = (0..256)
        .map(|v| (255.0 * (v as f32 / 255.0).powf(gamma)) as u8)
        .collect();
    for p in g.iter_mut() {
        *p = lut[*p as usize];
    }
}

fn autocontrast(g: &mut GrayImage) {
    let (mut lo, mut hi) = (255u8, 0u8);
    for &p in g.iter() {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    // low contrast is treated as intentional
    if (hi as i32 - lo as i32) < 255 - 32 * 3 {
        return;
    }
    let scale = 255.0 / (hi - lo) as f32;
    let lut: Vec<u8> = (0..256)
        .map(|v| ((v as f32 - lo as f32) * scale).round().clamp(0.0, 255.0) as u8)
        .collect();
    for p in g.iter_mut() {
        *p = lut[*p as usize];
    }
}

// ---------- cropping ----------

fn crop(g: GrayImage, o: &Options) -> GrayImage {
    let (w, h) = g.dimensions();
    let Some((mut l, mut t, mut r, mut b)) = content_box(&g, o.cropping_power, o.cropping == 2) else {
        return g;
    };
    // never remove more than 10% per side
    l = l.min(w / 10);
    t = t.min(h / 10);
    r = r.max(w * 9 / 10);
    b = b.max(h * 9 / 10);
    if o.preserve_margin > 0.0 {
        let k = 1.0 - o.preserve_margin / 100.0;
        l = (l as f32 * k) as u32;
        t = (t as f32 * k) as u32;
        r = w - ((w - r) as f32 * k) as u32;
        b = h - ((h - b) as f32 * k) as u32;
    }
    if r <= l || b <= t || (l == 0 && t == 0 && r == w && b == h) {
        return g;
    }
    imageops::crop_imm(&g, l, t, r - l, b - t).to_image()
}

/// Content bounding box (x0, y0, x1, y1), x1/y1 exclusive.
fn content_box(g: &GrayImage, power: f32, page_numbers: bool) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (g.width() as usize, g.height() as usize);
    if w < 20 || h < 20 {
        return None;
    }
    let mut px = g.as_raw().clone();
    if dark_background(&px, w, h) {
        for p in px.iter_mut() {
            *p = 255 - *p;
        }
    }
    autocontrast_cutoff(&mut px, 1.0);
    let px = box_blur(&px, w, h);
    let threshold = (240.0 - power * 64.0).clamp(0.0, 255.0) as u8;
    let mut mask: Vec<bool> = px.iter().map(|&p| p <= threshold).collect();
    ignore_near_edge(&mut mask, w, h);

    let (l, t, r, mut b) = bbox(&mask, w, 0, h)?;
    if page_numbers {
        if let Some(cut) = page_number_cut(&mask, w, h, (l, t, r, b)) {
            if let Some(bb) = bbox(&mask, w, 0, cut) {
                return Some((bb.0 as u32, bb.1 as u32, bb.2 as u32, bb.3 as u32));
            }
        }
    }
    b = b.max(t + 1);
    Some((l as u32, t as u32, r as u32, b as u32))
}

fn dark_background(px: &[u8], w: usize, h: usize) -> bool {
    let bw = (w / 50).max(1);
    let bh = (h / 50).max(1);
    let (mut sum, mut n) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            if y < bh || y >= h - bh || x < bw || x >= w - bw {
                sum += px[y * w + x] as u64;
                n += 1;
            }
        }
    }
    n > 0 && sum / n < 100
}

fn autocontrast_cutoff(px: &mut [u8], cutoff_pct: f32) {
    let mut hist = [0u64; 256];
    for &p in px.iter() {
        hist[p as usize] += 1;
    }
    let cut = (px.len() as f32 * cutoff_pct / 100.0) as u64;
    let mut acc = 0;
    let mut lo = 0usize;
    for v in 0..256 {
        acc += hist[v];
        if acc > cut {
            lo = v;
            break;
        }
    }
    acc = 0;
    let mut hi = 255usize;
    for v in (0..256).rev() {
        acc += hist[v];
        if acc > cut {
            hi = v;
            break;
        }
    }
    if hi <= lo {
        return;
    }
    let scale = 255.0 / (hi - lo) as f32;
    let lut: Vec<u8> = (0..256)
        .map(|v| ((v as f32 - lo as f32) * scale).clamp(0.0, 255.0) as u8)
        .collect();
    for p in px.iter_mut() {
        *p = lut[*p as usize];
    }
}

fn box_blur(px: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut tmp = vec![0u16; w * h];
    for y in 0..h {
        let row = &px[y * w..(y + 1) * w];
        for x in 0..w {
            let a = row[x.saturating_sub(1)] as u16;
            let c = row[(x + 1).min(w - 1)] as u16;
            tmp[y * w + x] = a + row[x] as u16 + c;
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let ya = y.saturating_sub(1);
        let yc = (y + 1).min(h - 1);
        for x in 0..w {
            let s = tmp[ya * w + x] + tmp[y * w + x] + tmp[yc * w + x];
            out[y * w + x] = ((s + 4) / 9) as u8;
        }
    }
    out
}

/// Remove thin scanner/border junk: if the band 2-2.5% in from an edge is (almost) empty,
/// anything between it and the edge is not content.
fn ignore_near_edge(mask: &mut [bool], w: usize, h: usize) {
    let count = |mask: &[bool], x0: usize, x1: usize, y0: usize, y1: usize| -> usize {
        let mut n = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                if mask[y * w + x] {
                    n += 1;
                }
            }
        }
        n
    };
    let clear = |mask: &mut [bool], x0: usize, x1: usize, y0: usize, y1: usize| {
        for y in y0..y1 {
            for x in x0..x1 {
                mask[y * w + x] = false;
            }
        }
    };
    let (h2, h25) = ((h as f32 * 0.02) as usize, (h as f32 * 0.025) as usize);
    let (w2, w25) = ((w as f32 * 0.02) as usize, (w as f32 * 0.025) as usize);
    // (inner band, outer band) per side as x0,x1,y0,y1
    let mut sides = Vec::new();
    if h25 > h2 {
        sides.push(((0, w, h2, h25), (0, w, 0, h2)));
        sides.push(((0, w, h - h25, h - h2), (0, w, h - h2, h)));
    }
    if w25 > w2 {
        sides.push(((w2, w25, 0, h), (0, w2, 0, h)));
        sides.push(((w - w25, w - w2, 0, h), (w - w2, w, 0, h)));
    }
    for (inner, outer) in sides {
        let area = ((inner.1 - inner.0) * (inner.3 - inner.2)).max(1);
        let n = count(mask, inner.0, inner.1, inner.2, inner.3);
        if (n as f32 / area as f32) < 0.001 {
            if n > 0 {
                clear(mask, inner.0, inner.1, inner.2, inner.3);
            }
            clear(mask, outer.0, outer.1, outer.2, outer.3);
        }
    }
}

fn bbox(mask: &[bool], w: usize, y0: usize, y1: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut l, mut t, mut r, mut b) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in y0..y1 {
        let row = &mask[y * w..(y + 1) * w];
        let Some(first) = row.iter().position(|&m| m) else { continue };
        let last = row.iter().rposition(|&m| m).unwrap();
        l = l.min(first);
        r = r.max(last + 1);
        t = t.min(y);
        b = y + 1;
    }
    (l != usize::MAX).then_some((l, t, r, b))
}

/// If a lone small mark (a page number) sits at the bottom of the content, return the row
/// above it so the crop can stop there.
fn page_number_cut(mask: &[bool], w: usize, h: usize, bb: (usize, usize, usize, usize)) -> Option<usize> {
    let (l, t, r, b) = bb;
    let win_h = (h as f32 * 0.025) as usize;
    if win_h == 0 || b <= t {
        return None;
    }
    let y0 = b.saturating_sub(win_h).max(t);
    let gap_x = (w as f32 * 0.01) as usize;
    let gap_y = (h as f32 * 0.002) as usize;

    // boxes as [x0, y0, x1, y1] inclusive
    let mut boxes: Vec<[usize; 4]> = Vec::new();
    for y in y0..b {
        let row = &mask[y * w..(y + 1) * w];
        let mut x = l;
        while x < r {
            if !row[x] {
                x += 1;
                continue;
            }
            let start = x;
            let mut end = x;
            x += 1;
            while x < r {
                if row[x] {
                    end = x;
                } else if x - end > gap_x {
                    break;
                }
                x += 1;
            }
            boxes.push([start, y, end, y]);
            if boxes.len() > 1500 {
                return None; // dense art at the bottom, not a page number
            }
        }
    }
    let close = |a: &[usize; 4], c: &[usize; 4]| {
        let dx = (a[0].max(c[0]) as isize - a[2].min(c[2]) as isize).max(0) as usize;
        let dy = (a[1].max(c[1]) as isize - a[3].min(c[3]) as isize).max(0) as usize;
        dx <= gap_x && dy <= gap_y
    };
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < boxes.len() {
            let mut j = i + 1;
            while j < boxes.len() {
                if close(&boxes[i], &boxes[j]) {
                    let c = boxes.swap_remove(j);
                    let a = &mut boxes[i];
                    a[0] = a[0].min(c[0]);
                    a[1] = a[1].min(c[1]);
                    a[2] = a[2].max(c[2]);
                    a[3] = a[3].max(c[3]);
                    merged = true;
                    j = i + 1; // box i grew: check the rest again
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            break;
        }
    }
    let min_w = (w as f32 * 0.003) as usize;
    let min_h = (h as f32 * 0.006) as usize;
    boxes.retain(|bx| bx[2] - bx[0] + 1 >= min_w && bx[3] - bx[1] + 1 >= min_h);
    let lowest_top = boxes.iter().filter(|bx| bx[3] == b - 1).map(|bx| bx[1]).min()?;
    let keep: Vec<_> = boxes.iter().filter(|bx| bx[3] >= lowest_top).collect();
    if keep.len() != 1 {
        return None;
    }
    let bx = keep[0];
    let max_w = (w as f32 * 0.045) as usize;
    let max_h = ((h as f32 * 0.02) as usize).max(3);
    if bx[2] - bx[0] + 1 <= max_w && bx[3] - bx[1] + 1 <= max_h && bx[1] > t {
        Some(bx[1])
    } else {
        None
    }
}
