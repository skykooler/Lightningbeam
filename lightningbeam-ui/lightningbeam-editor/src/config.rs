use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application configuration (persistent)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Recent files list (newest first, max 10 items)
    #[serde(default)]
    pub recent_files: Vec<PathBuf>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            recent_files: Vec::new(),
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
}
