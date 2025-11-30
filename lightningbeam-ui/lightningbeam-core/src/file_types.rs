//! File type detection and supported extension constants
//!
//! This module provides shared file extension constants that can be used
//! across the codebase for file dialogs, import detection, etc.

/// Supported image file extensions
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "gif", "avif", "jpg", "jpeg"];

/// Supported audio file extensions
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "aiff", "ogg", "flac"];

/// Supported video file extensions
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "avi", "mkv", "webm", "m4v"];

/// Supported MIDI file extensions
pub const MIDI_EXTENSIONS: &[&str] = &["mid", "midi"];

// Note: SVG import deferred to future task
// Note: .beam project files handled separately in file save/load feature

/// File type categories for import routing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Image,
    Audio,
    Video,
    Midi,
}

/// Detect file type from extension string
///
/// Returns `None` if the extension is not recognized.
///
/// # Example
/// ```
/// use lightningbeam_core::file_types::get_file_type;
///
/// assert_eq!(get_file_type("png"), Some(lightningbeam_core::file_types::FileType::Image));
/// assert_eq!(get_file_type("MP3"), Some(lightningbeam_core::file_types::FileType::Audio));
/// assert_eq!(get_file_type("unknown"), None);
/// ```
pub fn get_file_type(extension: &str) -> Option<FileType> {
    let ext = extension.to_lowercase();
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Image);
    }
    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Audio);
    }
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Video);
    }
    if MIDI_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Midi);
    }
    None
}

/// Get all supported extensions as a single flat list
///
/// Useful for "All Supported Files" filter in file dialogs.
pub fn all_supported_extensions() -> Vec<&'static str> {
    let mut all = Vec::new();
    all.extend_from_slice(IMAGE_EXTENSIONS);
    all.extend_from_slice(AUDIO_EXTENSIONS);
    all.extend_from_slice(VIDEO_EXTENSIONS);
    all.extend_from_slice(MIDI_EXTENSIONS);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_file_type() {
        assert_eq!(get_file_type("png"), Some(FileType::Image));
        assert_eq!(get_file_type("PNG"), Some(FileType::Image));
        assert_eq!(get_file_type("jpg"), Some(FileType::Image));
        assert_eq!(get_file_type("jpeg"), Some(FileType::Image));

        assert_eq!(get_file_type("mp3"), Some(FileType::Audio));
        assert_eq!(get_file_type("wav"), Some(FileType::Audio));
        assert_eq!(get_file_type("flac"), Some(FileType::Audio));

        assert_eq!(get_file_type("mp4"), Some(FileType::Video));
        assert_eq!(get_file_type("webm"), Some(FileType::Video));

        assert_eq!(get_file_type("mid"), Some(FileType::Midi));
        assert_eq!(get_file_type("midi"), Some(FileType::Midi));

        assert_eq!(get_file_type("unknown"), None);
        assert_eq!(get_file_type("svg"), None); // SVG deferred
    }

    #[test]
    fn test_all_supported_extensions() {
        let all = all_supported_extensions();
        assert!(all.contains(&"png"));
        assert!(all.contains(&"mp3"));
        assert!(all.contains(&"mp4"));
        assert!(all.contains(&"mid"));
    }
}
