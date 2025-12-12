//! Desktop notifications for export completion

use notify_rust::Notification;
use std::path::Path;

/// Send a desktop notification for successful export
///
/// # Arguments
/// * `output_path` - Path to the exported file
///
/// # Returns
/// Ok(()) if notification sent successfully, Err with log message if failed
pub fn notify_export_complete(output_path: &Path) -> Result<(), String> {
    let filename = output_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    Notification::new()
        .summary("Export Complete")
        .body(&format!("Successfully exported: {}", filename))
        .icon("dialog-information")  // Standard icon name (freedesktop.org)
        .timeout(5000)  // 5 seconds
        .show()
        .map_err(|e| format!("Failed to show notification: {}", e))?;

    Ok(())
}

/// Send a desktop notification for export error
///
/// # Arguments
/// * `error_message` - The error message to display
///
/// # Returns
/// Ok(()) if notification sent successfully, Err with log message if failed
pub fn notify_export_error(error_message: &str) -> Result<(), String> {
    // Truncate very long error messages
    let truncated = if error_message.len() > 100 {
        format!("{}...", &error_message[..97])
    } else {
        error_message.to_string()
    };

    Notification::new()
        .summary("Export Failed")
        .body(&truncated)
        .icon("dialog-error")  // Standard error icon
        .timeout(10000)  // 10 seconds for errors (longer to read)
        .show()
        .map_err(|e| format!("Failed to show notification: {}", e))?;

    Ok(())
}
