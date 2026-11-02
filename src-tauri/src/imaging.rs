// Page processing, following KCC's pipeline:
// crop -> gamma -> autocontrast -> spread split/rotate -> resize to the device -> JPEG.
// Pages are grayscale, except colour pages on a colour Kindle.

use crate::convert::Options;
use crate::devices::Device;
use image::imageops::{self, FilterType};
use image::{DynamicImage, GrayImage, ImageReader};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub struct Page {
    /// pictures stacked top to bottom (two on a Scribe when the page is taller than 1920 px)
    pub parts: Vec<(PathBuf, u32, u32)>,
    pub size: u64,
}

impl Page {
    pub fn width(&self) -> u32 {
        self.parts[0].1
    }

    pub fn height(&self) -> u32 {
        self.parts.iter().map(|p| p.2).sum()
    }
}

pub fn process(src: &Path, o: &Options, dev: &Device, idx: usize, dir: &Path) -> Result<Vec<Page>, String> {
    let img = ImageReader::open(src)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let color = dev.color && is_color(&img);
    let mut im = if color {
        DynamicImage::ImageRgb8(img.to_rgb8())
    } else {
        DynamicImage::ImageLuma8(img.to_luma8())
    };
    drop(img);

    if o.cropping > 0 {
        let rect = match &im {
            DynamicImage::ImageLuma8(g) => crop_rect(g, o),
            other => crop_rect(&other.to_luma8(), o),
        };
        if let Some((x, y, w, h)) = rect {
            im = im.crop_imm(x, y, w, h);
        }
    }
    apply_gamma(&mut im, o.gamma);
    if let DynamicImage::ImageLuma8(g) = &mut im {
        autocontrast(g); // colour pages are left as they are, like KCC
    }

    let q = dev.jpeg_quality();
    let mut pages = Vec::new();
    for (tag, part) in split(im, o, dev) {
        let out = resize(part, o, dev);
        let (w, h) = (out.width(), out.height());
        let mut page = Page { parts: Vec::new(), size: 0 };
        if dev.scribe() && h > 1920 {
            let above = out.crop_imm(0, 0, w, 1920);
            let below = out.crop_imm(0, 1920, w, h - 1920);
            for (name, pic) in [("above", above), ("below", below)] {
                let path = dir.join(format!("{idx:05}_{tag}_{name}.jpg"));
                page.size += save_jpeg(&pic, &path, q)?;
                page.parts.push((path, pic.width(), pic.height()));
            }
        } else {
            let path = dir.join(format!("{idx:05}_{tag}.jpg"));
            page.size += save_jpeg(&out, &path, q)?;
            page.parts.push((path, w, h));
        }
        pages.push(page);
    }
    Ok(pages)
}

fn save_jpeg(im: &DynamicImage, path: &Path, quality: u8) -> Result<u64, String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(BufWriter::new(file), quality);
    enc.encode_image(im).map_err(|e| e.to_string())?;
    drop(enc);
    Ok(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0))
}

/// Cover picture: the page itself, or for a Scribe the whole page shrunk to fit 1920 x 1920.
pub fn cover_jpeg(p: &Page, dev: &Device) -> Result<Vec<u8>, String> {
    if p.parts.len() == 1 && !(dev.scribe() && (p.width() > 1920 || p.height() > 1920)) {
        return std::fs::read(&p.parts[0].0).map_err(|e| e.to_string());
    }
    let mut parts = Vec::new();
    for (path, _, _) in &p.parts {
        parts.push(image::open(path).map_err(|e| e.to_string())?);
    }
    let mut whole = DynamicImage::new(p.width(), p.height(), parts[0].color());
    let mut y = 0i64;
    for part in &parts {
        imageops::overlay(&mut whole, part, 0, y);
        y += part.height() as i64;
    }
    let whole = whole.resize(1920, 1920, FilterType::Lanczos3);
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, dev.jpeg_quality())
        .encode_image(&whole)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

// ---------- colour detection (KCC's colour-mode check) ----------

fn is_color(img: &DynamicImage) -> bool {
    if !img.color().has_color() {
        return false;
    }
    let rgb = img.to_rgb8();
    let (mut cb, mut cr) = ([0u64; 256], [0u64; 256]);
    for p in rgb.pixels() {
        let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
        let vb = 128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b;
        let vr = 128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b;
        cb[vb.round().clamp(0.0, 255.0) as usize] += 1;
        cr[vr.round().clamp(0.0, 255.0) as usize] += 1;
    }
    for (cutoff, diff) in [(0.0f32, 22usize), (0.2, 10), (3.0, 4)] {
        let (b0, b1) = hist_ends(&cb, cutoff);
        let (r0, r1) = hist_ends(&cr, cutoff);
        if b0 > 128 || r0 > 128 || b1 < 128 || r1 < 128 {
            return true;
        }
        if b0 <= 128 - diff || r0 <= 128 - diff || b1 >= 128 + diff || r1 >= 128 + diff {
            return true;
        }
    }
    false
}

/// First and last used histogram bins after trimming `cutoff` % of pixels from each end.
fn hist_ends(h: &[u64; 256], cutoff: f32) -> (usize, usize) {
    let n: u64 = h.iter().sum();
    let cut = (n as f64 * cutoff as f64 / 100.0) as u64;
    let (mut acc, mut lo) = (0u64, 0usize);
    for (v, &c) in h.iter().enumerate() {
        acc += c;
        if acc > cut {
            lo = v;
            break;
        }
    }
    let (mut acc, mut hi) = (0u64, 255usize);
    for v in (0..256).rev() {
        acc += h[v];
        if acc > cut {
            hi = v;
            break;
        }
    }
    (lo, hi)
}

// ---------- spread splitter ----------

// Tags sort in reading order: a = rotated (first), b/c = halves, d = rotated (last), x = normal.
fn split(im: DynamicImage, o: &Options, dev: &Device) -> Vec<(char, DynamicImage)> {
    let (dw, dh) = dev.size();
    let (w, h) = (im.width(), im.height());
    let landscape = w > h;
    let ratio = w as f32 / h as f32;
    let rot_tag = if o.rotate_first { 'a' } else { 'd' };

    if landscape && w <= dh && h <= dw && o.splitter == 1 {
        return vec![(rot_tag, im.rotate270())];
    }
    if landscape && ratio > 1.16 {
        let do_split = o.splitter != 1 && ratio < 1.8;
        let do_rotate = o.splitter > 0 || ratio >= 1.8;
        let mut out = Vec::new();
        if do_rotate && o.rotate_first {
            out.push(('a', im.rotate270()));
        }
        if do_split {
            let half = w / 2;
            let left = im.crop_imm(0, 0, half, h);
            let right = im.crop_imm(half, 0, w - half, h);
            if o.right_to_left {
                out.push(('b', right));
                out.push(('c', left));
            } else {
                out.push(('b', left));
                out.push(('c', right));
            }
        }
        if do_rotate && !o.rotate_first {
            out.push(('d', im.rotate270()));
        }
        return out;
    }
    vec![('x', im)]
}

// ---------- resize ----------

fn resize(im: DynamicImage, o: &Options, dev: &Device) -> DynamicImage {
    let (dw, dh) = dev.size();
    let (w, h) = (im.width(), im.height());
    let small = w <= dw && h <= dh;
    let filter = if small { FilterType::CatmullRom } else { FilterType::Lanczos3 };

    if o.scale == "stretch" {
        return im.resize_exact(dw, dh, filter);
    }
    if small && o.scale != "upscale" {
        return im;
    }
    let ar_img = w as f32 / h as f32;
    let ar_dev = dw as f32 / dh as f32;
    if (ar_img - ar_dev).abs() < 0.015 {
        // nearly the screen's shape: fill the screen, trim the overflow
        let s = f32::max(dw as f32 / w as f32, dh as f32 / h as f32);
        let nw = ((w as f32 * s).ceil() as u32).max(dw);
        let nh = ((h as f32 * s).ceil() as u32).max(dh);
        im.resize_exact(nw, nh, filter).crop_imm((nw - dw) / 2, (nh - dh) / 2, dw, dh)
    } else {
        let s = f32::min(dw as f32 / w as f32, dh as f32 / h as f32);
        let nw = ((w as f32 * s).round() as u32).clamp(1, dw);
        let nh = ((h as f32 * s).round() as u32).clamp(1, dh);
        im.resize_exact(nw, nh, filter)
    }
}

// ---------- tone ----------

fn apply_gamma(im: &mut DynamicImage, gamma: f32) {
    if gamma < 0.1 || (gamma - 1.0).abs() < 1e-3 {
        return; // Auto = profile gamma 1.0
    }
    let lut: Vec<u8> = (0..256)
        .map(|v| (255.0 * (v as f32 / 255.0).powf(gamma)) as u8)
        .collect();
    let bytes: &mut [u8] = match im {
        DynamicImage::ImageLuma8(g) => &mut **g,
        DynamicImage::ImageRgb8(c) => &mut **c,
        _ => return,
    };
    for p in bytes.iter_mut() {
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

/// Crop rectangle (x, y, w, h), or None to keep the page as it is.
fn crop_rect(g: &GrayImage, o: &Options) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = g.dimensions();
    let (mut l, mut t, mut r, mut b) = content_box(g, o.cropping_power, o.cropping == 2)?;
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
        return None;
    }
    Some((l, t, r - l, b - t))
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
