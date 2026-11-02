use crate::imaging::{self, Page};
use crate::devices::{self, Device};
use crate::{epub, mobi, CANCEL};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering as CmpOrdering;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub device: String,
    pub format: String,      // "mobi" | "azw3"
    pub folder_mode: String, // "volumes" | "single"
    pub right_to_left: bool,
    pub splitter: u8, // 0 split, 1 rotate, 2 split + rotate
    pub rotate_first: bool,
    pub scale: String, // "off" | "stretch" | "upscale"
    pub gamma: f32,    // 0 = auto
    pub cropping: u8,  // 0 off, 1 margins, 2 margins + page numbers
    pub cropping_power: f32,
    pub preserve_margin: f32,
    pub chunk_mb: u64,
    pub kindlegen: Option<String>,
}

#[derive(Serialize, Clone)]
struct LogLine<'a> {
    level: &'a str,
    text: String,
}

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
}

fn log(app: &AppHandle, level: &str, text: impl Into<String>) {
    let _ = app.emit("log", LogLine { level, text: text.into() });
}

fn cancelled() -> bool {
    CANCEL.load(Ordering::SeqCst)
}

// ---------- KindleGen ----------

pub fn find_kindlegen(custom: Option<&str>) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(c) = custom.filter(|c| !c.trim().is_empty()) {
        cands.push(PathBuf::from(c));
    }
    let tail = r"Amazon\Kindle Previewer 3\lib\fc\bin\kindlegen.exe";
    for var in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Ok(base) = std::env::var(var) {
            cands.push(Path::new(&base).join(tail));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("kindlegen.exe"));
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            cands.push(dir.join("kindlegen.exe"));
        }
    }
    cands.into_iter().find(|p| p.is_file())
}

fn run_kindlegen(kg: &Path, epub: &Path, work: &Path) -> Result<PathBuf, String> {
    let log_path = work.join("kindlegen.log");
    let log_file = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(kg);
    cmd.arg("-dont_append_source")
        .arg("-locale")
        .arg("en")
        .arg(epub)
        .current_dir(work)
        .stdout(log_file.try_clone().map_err(|e| e.to_string())?)
        .stderr(log_file);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start KindleGen: {e}"))?;
    loop {
        if let Some(_status) = child.try_wait().map_err(|e| e.to_string())? {
            break;
        }
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Stopped.".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let mobi = epub.with_extension("mobi");
    if mobi.is_file() {
        return Ok(mobi);
    }
    let text = std::fs::read_to_string(&log_path).unwrap_or_default();
    let errors: Vec<&str> = text.lines().filter(|l| l.contains("Error")).collect();
    let detail = if errors.is_empty() {
        text.lines().rev().take(3).collect::<Vec<_>>().join(" | ")
    } else {
        errors.iter().rev().take(3).rev().cloned().collect::<Vec<_>>().join(" | ")
    };
    Err(format!("KindleGen failed: {detail}"))
}

// ---------- input folders ----------

struct Job {
    title: String,
    images: Vec<PathBuf>,
    out_dir: PathBuf,
}

const IMAGE_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "webp", "gif", "bmp"];

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn hidden(p: &Path) -> bool {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    n.starts_with('.') || n == "__MACOSX"
}

fn list_dir(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut files, mut dirs) = (Vec::new(), Vec::new());
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if hidden(&p) {
                continue;
            }
            if p.is_dir() {
                dirs.push(p);
            } else if is_image(&p) {
                files.push(p);
            }
        }
    }
    files.sort_by(|a, b| natural_path_cmp(a, b));
    dirs.sort_by(|a, b| natural_path_cmp(a, b));
    (files, dirs)
}

fn images_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let (files, dirs) = list_dir(dir);
    out.extend(files);
    for d in dirs {
        images_recursive(&d, out);
    }
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn collect_jobs(root: &Path, o: &Options, jobs: &mut Vec<Job>) -> Result<(), String> {
    if !root.is_dir() {
        return Err(format!("Not a folder: {}", root.display()));
    }
    let name = name_of(root);
    let parent = root.parent().unwrap_or(root).to_path_buf();
    let (direct, dirs) = list_dir(root);

    if o.folder_mode == "single" || dirs.is_empty() {
        let mut images = Vec::new();
        images_recursive(root, &mut images);
        if !images.is_empty() {
            jobs.push(Job { title: name, images, out_dir: parent });
        }
        return Ok(());
    }
    // each subfolder is a volume; its book is saved next to it (inside the series folder)
    if !direct.is_empty() {
        jobs.push(Job { title: name.clone(), images: direct, out_dir: parent });
    }
    for d in dirs {
        let mut images = Vec::new();
        images_recursive(&d, &mut images);
        if images.is_empty() {
            continue;
        }
        let vol = name_of(&d);
        let title = if vol.to_lowercase().contains(&name.to_lowercase()) {
            vol
        } else {
            format!("{name} - {vol}")
        };
        jobs.push(Job { title, images, out_dir: root.to_path_buf() });
    }
    Ok(())
}

fn natural_cmp(a: &str, b: &str) -> CmpOrdering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ia, mut ib) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return CmpOrdering::Equal,
            (None, Some(_)) => return CmpOrdering::Less,
            (Some(_), None) => return CmpOrdering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ia.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ia.next();
                }
                let mut nb = String::new();
                while let Some(c) = ib.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    ib.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != CmpOrdering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ia.next();
                ib.next();
            }
        }
    }
}

fn natural_path_cmp(a: &Path, b: &Path) -> CmpOrdering {
    natural_cmp(&name_of(a), &name_of(b))
}

fn safe_file_name(s: &str) -> String {
    s.chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c })
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .to_string()
}

// ---------- conversion ----------

struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn run(app: &AppHandle, inputs: Vec<String>, mut o: Options) {
    let Some(kg) = find_kindlegen(o.kindlegen.as_deref()) else {
        log(app, "error", "KindleGen was not found. Install Kindle Previewer 3, or pick kindlegen.exe with \"Browse…\".");
        return;
    };
    let dev = devices::get(&o.device);
    let (dw, dh) = dev.size();
    log(app, "info", format!("Device: {} ({dw} × {dh}{})", dev.name, if dev.color { ", colour" } else { "" }));
    if dev.kf7 && o.format == "azw3" {
        log(app, "warn", format!("{} can't open AZW3, making MOBI instead.", dev.name));
        o.format = "mobi".into();
    }

    let mut jobs = Vec::new();
    for input in &inputs {
        if let Err(e) = collect_jobs(Path::new(input), &o, &mut jobs) {
            log(app, "error", e);
        }
    }
    if jobs.is_empty() {
        log(app, "error", "No pictures found in the selected folders.");
        return;
    }
    let total: usize = jobs.iter().map(|j| j.images.len()).sum();
    let done = AtomicUsize::new(0);
    let _ = app.emit("progress", Progress { done: 0, total });

    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads((threads.saturating_sub(1)).clamp(1, 4))
        .build()
        .expect("thread pool");

    let n = jobs.len();
    let (mut ok, mut failed) = (0, 0);
    for (i, job) in jobs.iter().enumerate() {
        if cancelled() {
            break;
        }
        log(app, "info", format!("[{}/{}] {}  ({} pictures)", i + 1, n, job.title, job.images.len()));
        match convert_job(app, &pool, job, &o, dev, &kg, &done, total) {
            Ok(outs) => {
                ok += 1;
                for p in outs {
                    log(app, "ok", format!("Saved: {}", p.display()));
                }
            }
            Err(e) => {
                failed += 1;
                log(app, "error", e);
            }
        }
    }
    if cancelled() {
        log(app, "warn", "Stopped.");
    } else {
        log(app, if failed > 0 { "warn" } else { "ok" }, format!("Finished: {ok} done, {failed} failed."));
    }
}

fn convert_job(
    app: &AppHandle,
    pool: &rayon::ThreadPool,
    job: &Job,
    o: &Options,
    dev: &Device,
    kg: &Path,
    done: &AtomicUsize,
    total: usize,
) -> Result<Vec<PathBuf>, String> {
    let work = WorkDir(std::env::temp_dir().join(format!("kmc-{}", uuid::Uuid::new_v4())));
    let pages_dir = work.0.join("pages");
    std::fs::create_dir_all(&pages_dir).map_err(|e| format!("Cannot create temp folder: {e}"))?;

    log(app, "info", "Processing pictures…");
    let results: Vec<(usize, Result<Vec<Page>, String>)> = pool.install(|| {
        job.images
            .par_iter()
            .enumerate()
            .map(|(idx, src)| {
                if cancelled() {
                    return (idx, Ok(Vec::new()));
                }
                let r = imaging::process(src, o, dev, idx, &pages_dir);
                let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app.emit("progress", Progress { done: d, total });
                (idx, r)
            })
            .collect()
    });
    if cancelled() {
        return Err("Stopped.".into());
    }
    let mut pages = Vec::new();
    for (idx, r) in results {
        match r {
            Ok(p) => pages.extend(p),
            Err(e) => log(app, "warn", format!("Skipped {}: {e}", job.images[idx].display())),
        }
    }
    if pages.is_empty() {
        return Err("No picture could be read.".into());
    }

    // chunk by size
    let limit = o.chunk_mb.max(1) * 1024 * 1024;
    let mut parts: Vec<Vec<Page>> = vec![Vec::new()];
    let mut size = 0u64;
    for p in pages {
        if size + p.size > limit && !parts.last().unwrap().is_empty() {
            parts.push(Vec::new());
            size = 0;
        }
        size += p.size;
        parts.last_mut().unwrap().push(p);
    }

    let ext = if o.format == "azw3" { "azw3" } else { "mobi" };
    let np = parts.len();
    if np > 1 {
        log(app, "info", format!("Bigger than {} MB: making {np} files.", o.chunk_mb));
    }
    let mut outs = Vec::new();
    for (pi, part) in parts.iter().enumerate() {
        if cancelled() {
            return Err("Stopped.".into());
        }
        let (title, stem) = if np > 1 {
            (
                format!("{} [{:02}/{:02}]", job.title, pi + 1, np),
                format!("{} - Part {:02}", job.title, pi + 1),
            )
        } else {
            (job.title.clone(), job.title.clone())
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let epub_path = work.0.join(format!("book{pi}.epub"));
        let cover = imaging::cover_jpeg(&part[0], dev)?;
        epub::build(&epub_path, &title, &uuid, part, &cover, o.right_to_left, dev)?;
        log(app, "info", format!("Running KindleGen{}… (can take a few minutes)", if np > 1 { format!(" for part {}/{np}", pi + 1) } else { String::new() }));
        let mobi_path = run_kindlegen(kg, &epub_path, &work.0)?;
        let data = std::fs::read(&mobi_path).map_err(|e| e.to_string())?;
        let out = mobi::finish(&data, &uuid, o.format == "azw3")?;
        let out_path = job.out_dir.join(format!("{}.{ext}", safe_file_name(&stem)));
        std::fs::write(&out_path, out).map_err(|e| format!("Cannot write {}: {e}", out_path.display()))?;
        outs.push(out_path);
    }
    Ok(outs)
}
