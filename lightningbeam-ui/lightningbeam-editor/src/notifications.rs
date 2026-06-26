//! Desktop notifications for export completion

use notify_rust::Notification;
use std::path::Path;

// `notify_rust`'s `Notification::show()` is a SYNCHRONOUS D-Bus call. When no
// notification daemon is running it can block for the service-activation timeout
// (~25s) before failing, so these are fire-and-forget on a detached thread — the
// UI must never wait on them (e.g. they're triggered from the export-complete
// handler on the UI thread).

/// Show a desktop notification for a successful export (fire-and-forget).
pub fn notify_export_complete(output_path: &Path) {
    let filename = output_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    std::thread::spawn(move || {
        if let Err(e) = Notification::new()
            .summary("Export Complete")
            .body(&format!("Successfully exported: {}", filename))
            .icon("dialog-information")  // Standard icon name (freedesktop.org)
            .timeout(5000)  // 5 seconds
            .show()
        {
            eprintln!("⚠️  Could not send desktop notification: {}", e);
        }
    });
}

/// Show a desktop notification for an export error (fire-and-forget).
pub fn notify_export_error(error_message: &str) {
    notify_error("Export Failed", error_message);
}

/// Show a desktop error notification with a custom title (fire-and-forget).
pub fn notify_error(title: &'static str, error_message: &str) {
    // Truncate very long error messages (on a char boundary).
    let truncated = if error_message.chars().count() > 100 {
        let prefix: String = error_message.chars().take(97).collect();
        format!("{}...", prefix)
    } else {
        error_message.to_string()
    };

    std::thread::spawn(move || {
        if let Err(e) = Notification::new()
            .summary(title)
            .body(&truncated)
            .icon("dialog-error")  // Standard error icon
            .timeout(10000)  // 10 seconds for errors (longer to read)
            .show()
        {
            eprintln!("⚠️  Could not send desktop notification: {}", e);
        }
    });
}
