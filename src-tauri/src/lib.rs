use std::sync::{Mutex};

use tauri_plugin_log::{Target, TargetKind};
use log::{trace, info, debug, warn, error};
use tracing_subscriber::EnvFilter;
use chrono::Local;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};


#[derive(Default)]
struct AppState {
  counter: u32,
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn trace(msg: String) {
  trace!("{}",msg);
}
#[tauri::command]
fn info(msg: String) {
  info!("{}",msg);
}
#[tauri::command]
fn debug(msg: String) {
  debug!("{}",msg);
}
#[tauri::command]
fn warn(msg: String) {
  warn!("{}",msg);
}
#[tauri::command]
fn error(msg: String) {
  error!("{}",msg);
}

#[tauri::command]
async fn create_window(app: tauri::AppHandle) {
    let state = app.state::<Mutex<AppState>>();

    // Lock the mutex to get mutable access:
    let mut state = state.lock().unwrap();
    
    // Increment the counter and generate a unique window label
    let window_label = format!("window{}", state.counter);
    state.counter += 1;
    
    // Build the new window with the unique label
    let _webview_window = WebviewWindowBuilder::new(&app, &window_label, WebviewUrl::App("index.html".into()))
        .build()
        .unwrap();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let pkg_name = env!("CARGO_PKG_NAME").to_string();
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .format(|out, message, record| {
                    let date = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                    out.finish(format_args!(
                        "{}[{}] {}",
                        date,
                        record.level(),
                        message
                      ))
                  })
                .targets([
                    Target::new(TargetKind::Stdout),
                    // LogDir locations:
                    // Linux: /home/user/.local/share/org.lightningbeam.core/logs
                    // macOS: /Users/user/Library/Logs/org.lightningbeam.core/logs
                    // Windows: C:\Users\user\AppData\Local\org.lightningbeam.core\logs
                    Target::new(TargetKind::LogDir { file_name: Some("logs".to_string()) }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![greet, trace, debug, info, warn, error, create_window])
        // .manage(window_counter)
        .setup(|app| {
          #[cfg(debug_assertions)] // only include this code on debug builds
          {
            let window = app.get_webview_window("main").unwrap();
            window.open_devtools();
            window.close_devtools();
          }
          {
            app.manage(Mutex::new(AppState::default()));
          }
          Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    tracing_subscriber::fmt().with_env_filter(EnvFilter::new(format!("{}=trace", pkg_name))).init();
}
