use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::keymap::KeybindingConfig;
use lightningbeam_core::file_io::LargeMediaMode;

/// Application configuration (persistent)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Recent files list (newest first, max 10 items)
    #[serde(default)]
    pub recent_files: Vec<PathBuf>,

    // User Preferences
    /// Default BPM for new projects
    #[serde(default = "defaults::bpm")]
    pub bpm: u32,

    /// Default framerate for new projects
    #[serde(default = "defaults::framerate")]
    pub framerate: u32,

    /// Default file width in pixels
    #[serde(default = "defaults::file_width")]
    pub file_width: u32,

    /// Default file height in pixels
    #[serde(default = "defaults::file_height")]
    pub file_height: u32,

    /// Scroll speed multiplier
    #[serde(default = "defaults::scroll_speed")]
    pub scroll_speed: f64,

    /// Audio buffer size in samples (128, 256, 512, 1024, 2048, 4096)
    #[serde(default = "defaults::audio_buffer_size")]
    pub audio_buffer_size: u32,

    /// Reopen last session on startup
    #[serde(default = "defaults::reopen_last_session")]
    pub reopen_last_session: bool,

    /// Restore layout when opening files
    #[serde(default = "defaults::restore_layout_from_file")]
    pub restore_layout_from_file: bool,

    /// Enable debug mode
    #[serde(default = "defaults::debug")]
    pub debug: bool,

    /// Show waveforms as stacked stereo instead of combined mono
    #[serde(default = "defaults::waveform_stereo")]
    pub waveform_stereo: bool,

    /// Theme mode ("light", "dark", or "system")
    #[serde(default = "defaults::theme_mode")]
    pub theme_mode: String,

    /// Custom keyboard shortcut overrides (sparse — only non-default bindings stored)
    #[serde(default)]
    pub keybindings: KeybindingConfig,

    /// How to store media files at/above the large-media threshold (~2GB).
    /// `Ask` (default) prompts the first time such a file is imported, then the
    /// chosen mode is persisted here. Reset to `Ask` to be prompted again.
    #[serde(default)]
    pub large_media_default: LargeMediaMode,

    /// Finest-level resolution of the waveform LOD pyramid: source frames per
    /// floor texel (`B`). Smaller = larger on-disk pyramid but zoom-in re-decodes
    /// sooner; larger = smaller pyramid, wider re-decode span. Default 256.
    #[serde(default = "defaults::waveform_floor_samples_per_texel")]
    pub waveform_floor_samples_per_texel: u32,

    /// Last-used audio-export "Artist" tag, remembered so it prefills next time.
    #[serde(default)]
    pub last_audio_artist: String,

    /// Last-used audio-export "Album" tag, remembered so it prefills next time.
    #[serde(default)]
    pub last_audio_album: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            recent_files: Vec::new(),
            bpm: defaults::bpm(),
            framerate: defaults::framerate(),
            file_width: defaults::file_width(),
            file_height: defaults::file_height(),
            scroll_speed: defaults::scroll_speed(),
            audio_buffer_size: defaults::audio_buffer_size(),
            reopen_last_session: defaults::reopen_last_session(),
            restore_layout_from_file: defaults::restore_layout_from_file(),
            debug: defaults::debug(),
            waveform_stereo: defaults::waveform_stereo(),
            theme_mode: defaults::theme_mode(),
            keybindings: KeybindingConfig::default(),
            large_media_default: LargeMediaMode::default(),
            waveform_floor_samples_per_texel: defaults::waveform_floor_samples_per_texel(),
            last_audio_artist: String::new(),
            last_audio_album: String::new(),
        }
    }
}

impl AppConfig {
    /// Load config from standard location
    /// Returns default config if file doesn't exist or is malformed
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(config) => config,
            Err(e) => {
                eprintln!("⚠️  Failed to load config: {}", e);
                eprintln!("   Using default configuration");
                Self::default()
            }
        }
    }

    /// Try to load config, returning error if something goes wrong
    fn try_load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_path = Self::config_path()?;

        if !config_path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(&config_path)?;
        let config: AppConfig = serde_json::from_str(&contents)?;
        Ok(config)
    }

    /// Save config to standard location
    /// Logs error but doesn't block if save fails
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            eprintln!("⚠️  Failed to save config: {}", e);
        }
    }

    /// Try to save config atomically (write to temp, then rename)
    fn try_save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config_path = Self::config_path()?;

        // Ensure parent directory exists
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Serialize to JSON with pretty formatting
        let json = serde_json::to_string_pretty(self)?;

        // Atomic write: write to temp file, then rename
        let temp_path = config_path.with_extension("json.tmp");
        std::fs::write(&temp_path, json)?;
        std::fs::rename(temp_path, config_path)?;

        Ok(())
    }

    /// Get cross-platform config file path
    fn config_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        use directories::ProjectDirs;

        let proj_dirs = ProjectDirs::from("", "", "lightningbeam")
            .ok_or("Failed to determine config directory")?;

        Ok(proj_dirs.config_dir().join("config.json"))
    }

    /// Add a file to recent files list
    /// - Canonicalize path (resolve relative paths and symlinks)
    /// - Move to front if already in list (remove duplicates)
    /// - Enforce 10-item limit (LRU eviction)
    /// - Auto-save config
    pub fn add_recent_file(&mut self, path: PathBuf) {
        // Try to canonicalize path (absolute, resolve symlinks)
        let canonical = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                // Canonicalize can fail for unsaved files or deleted files
                eprintln!("⚠️  Could not canonicalize path {:?}: {}", path, e);
                return; // Don't add non-existent paths
            }
        };

        // Remove if already present (we'll add to front)
        self.recent_files.retain(|p| p != &canonical);

        // Add to front
        self.recent_files.insert(0, canonical);

        // Enforce 10-item limit
        self.recent_files.truncate(10);

        // Auto-save
        self.save();
    }

    /// Get recent files list, filtering out files that no longer exist
    /// Returns newest first
    pub fn get_recent_files(&self) -> Vec<PathBuf> {
        self.recent_files
            .iter()
            .filter(|p| p.exists())
            .cloned()
            .collect()
    }

    /// Clear all recent files
    pub fn clear_recent_files(&mut self) {
        self.recent_files.clear();
        self.save();
    }

    /// Validate BPM range (20-300)
    pub fn validate_bpm(&self) -> Result<(), String> {
        if self.bpm >= 20 && self.bpm <= 300 {
            Ok(())
        } else {
            Err(format!("BPM must be between 20 and 300 (got {})", self.bpm))
        }
    }

    /// Validate framerate range (1-120)
    pub fn validate_framerate(&self) -> Result<(), String> {
        if self.framerate >= 1 && self.framerate <= 120 {
            Ok(())
        } else {
            Err(format!("Framerate must be between 1 and 120 (got {})", self.framerate))
        }
    }

    /// Validate file width range (100-10000)
    pub fn validate_file_width(&self) -> Result<(), String> {
        if self.file_width >= 100 && self.file_width <= 10000 {
            Ok(())
        } else {
            Err(format!("File width must be between 100 and 10000 (got {})", self.file_width))
        }
    }

    /// Validate file height range (100-10000)
    pub fn validate_file_height(&self) -> Result<(), String> {
        if self.file_height >= 100 && self.file_height <= 10000 {
            Ok(())
        } else {
            Err(format!("File height must be between 100 and 10000 (got {})", self.file_height))
        }
    }

    /// Validate scroll speed range (0.1-10.0)
    pub fn validate_scroll_speed(&self) -> Result<(), String> {
        if self.scroll_speed >= 0.1 && self.scroll_speed <= 10.0 {
            Ok(())
        } else {
            Err(format!("Scroll speed must be between 0.1 and 10.0 (got {})", self.scroll_speed))
        }
    }

    /// Validate audio buffer size (must be 128, 256, 512, 1024, 2048, or 4096)
    pub fn validate_audio_buffer_size(&self) -> Result<(), String> {
        match self.audio_buffer_size {
            128 | 256 | 512 | 1024 | 2048 | 4096 => Ok(()),
            _ => Err(format!("Audio buffer size must be 128, 256, 512, 1024, 2048, or 4096 (got {})", self.audio_buffer_size))
        }
    }

    /// Validate theme mode (must be "light", "dark", or "system")
    pub fn validate_theme_mode(&self) -> Result<(), String> {
        match self.theme_mode.to_lowercase().as_str() {
            "light" | "dark" | "system" => Ok(()),
            _ => Err(format!("Theme mode must be 'light', 'dark', or 'system' (got '{}')", self.theme_mode))
        }
    }

    /// Validate all preferences
    pub fn validate(&self) -> Result<(), String> {
        self.validate_bpm()?;
        self.validate_framerate()?;
        self.validate_file_width()?;
        self.validate_file_height()?;
        self.validate_scroll_speed()?;
        self.validate_audio_buffer_size()?;
        self.validate_theme_mode()?;
        Ok(())
    }
}

/// Default values for preferences (matches JS implementation)
mod defaults {
    pub fn bpm() -> u32 { 120 }
    pub fn framerate() -> u32 { 24 }
    pub fn file_width() -> u32 { 800 }
    pub fn file_height() -> u32 { 600 }
    pub fn scroll_speed() -> f64 { 1.0 }
    pub fn audio_buffer_size() -> u32 { 256 }
    pub fn reopen_last_session() -> bool { false }
    pub fn restore_layout_from_file() -> bool { true }
    pub fn debug() -> bool { false }
    pub fn waveform_stereo() -> bool { false }
    pub fn theme_mode() -> String { "system".to_string() }
    pub fn waveform_floor_samples_per_texel() -> u32 { 256 }
}
