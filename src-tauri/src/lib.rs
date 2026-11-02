mod convert;
mod epub;
mod imaging;
mod mobi;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

pub static CANCEL: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);

#[tauri::command]
fn find_kindlegen(custom: Option<String>) -> Option<String> {
    convert::find_kindlegen(custom.as_deref()).map(|p| p.display().to_string())
}

#[tauri::command]
fn start_convert(app: AppHandle, inputs: Vec<String>, opts: convert::Options) -> Result<(), String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("A conversion is already running.".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        convert::run(&app, inputs, opts);
        RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("finished", ());
    });
    Ok(())
}

#[tauri::command]
fn stop_convert() {
    CANCEL.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn open_folder(path: String) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            find_kindlegen,
            start_convert,
            stop_convert,
            open_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
