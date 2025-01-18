use std::{path::PathBuf, sync::Mutex};

use tauri_plugin_log::{Target, TargetKind};
use log::{trace, info, debug, warn, error};
use tracing_subscriber::EnvFilter;
use chrono::Local;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};


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

use tauri::PhysicalSize;

#[tauri::command]
async fn create_window(app: tauri::AppHandle, path: Option<String>) {
    let state = app.state::<Mutex<AppState>>();

    // Lock the mutex to get mutable access:
    let mut state = state.lock().unwrap();

    // Increment the counter and generate a unique window label
    let window_label = format!("window{}", state.counter);
    state.counter += 1;

    // Build the new window with the unique label
    let webview_window = WebviewWindowBuilder::new(&app, &window_label, WebviewUrl::App("index.html".into()))
        .title("Lightningbeam")
        .build()
        .unwrap();

    // Get the current monitor's screen size from the new window
    if let Ok(Some(monitor)) = webview_window.current_monitor() {
        let screen_size = monitor.size(); // Get the size of the monitor
        let width = 4096;
        let height = 4096;

        // Set the window size to be the smaller of the specified size or the screen size
        let new_width = width.min(screen_size.width as u32);
        let new_height = height.min(screen_size.height as u32 - 100);

        // Set the size using PhysicalSize
        webview_window.set_size(tauri::Size::Physical(PhysicalSize::new(new_width, new_height)))
            .expect("Failed to set window size");
    } else {
        eprintln!("Could not detect the current monitor.");
    }

    // Set the opened file if provided
    if let Some(val) = path {
        // Pass path data to the window via JavaScript
        webview_window.eval(&format!("window.openedFiles = [\"{val}\"]")).unwrap();

        // Set the window title if provided
        webview_window.set_title(&val).expect("Failed to set window title");
    }
}


fn handle_file_associations(app: AppHandle, files: Vec<PathBuf>) {
  // -- Scope handling start --

  // You can remove this block if you only want to know about the paths, but not actually "use" them in the frontend.

  // This requires the `fs` tauri plugin and is required to make the plugin's frontend work:
  use tauri_plugin_fs::FsExt;
  let fs_scope = app.fs_scope();

  // This is for the `asset:` protocol to work:
  let asset_protocol_scope = app.asset_protocol_scope();

  for file in &files {
    // This requires the `fs` plugin:
    let _ = fs_scope.allow_file(file);

    // This is for the `asset:` protocol:
    let _ = asset_protocol_scope.allow_file(file);
  }

  // -- Scope handling end --

  let files = files
    .into_iter()
    .map(|f| {
      let file = f.to_string_lossy().replace('\\', "\\\\"); // escape backslash
      format!("\"{file}\"",) // wrap in quotes for JS array
    })
    .collect::<Vec<_>>()
    .join(",");
  warn!("{}",files);

  let window = app.get_webview_window("main").unwrap();
  window.eval(&format!("window.openedFiles = [{files}]")).unwrap();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let pkg_name = env!("CARGO_PKG_NAME").to_string();
    tauri::Builder::default()
        .setup(|app| {
          {
            app.manage(Mutex::new(AppState::default()));
          }
          #[cfg(any(windows, target_os = "linux"))] // Windows/Linux needs different handling from macOS
          {
            let mut files = Vec::new();

            // NOTICE: `args` may include URL protocol (`your-app-protocol://`)
            // or arguments (`--`) if your app supports them.
            // files may aslo be passed as `file://path/to/file`
            for maybe_file in std::env::args().skip(1) {
              // skip flags like -f or --flag
              if maybe_file.starts_with('-') {
                continue;
              }

              // handle `file://` path urls and skip other urls
              if let Ok(url) = Url::parse(&maybe_file) {
              // if let Ok(url) = url::Url::parse(&maybe_file) {
                if let Ok(path) = url.to_file_path() {
                  files.push(path);
                }
              } else {
                files.push(PathBuf::from(maybe_file))
              }
            }

            handle_file_associations(app.handle().clone(), files);
          }
          #[cfg(debug_assertions)] // only include this code on debug builds
          {
            let window = app.get_webview_window("main").unwrap();
            window.open_devtools();
            window.close_devtools();
          }
          Ok(())
        })
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
                .build()
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![greet, trace, debug, info, warn, error, create_window])
        // .manage(window_counter)
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(
          #[allow(unused_variables)]
          |app, event| {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let tauri::RunEvent::Opened { urls } = event {
              let files = urls
                .into_iter()
                .filter_map(|url| url.to_file_path().ok())
                .collect::<Vec<_>>();
    
              handle_file_associations(app.clone(), files);
            }
          },
        );
    tracing_subscriber::fmt().with_env_filter(EnvFilter::new(format!("{}=trace", pkg_name))).init();
}
