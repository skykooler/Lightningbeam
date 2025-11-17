/// Theme system for Lightningbeam Editor
///
/// Parses CSS rules from assets/styles.css at runtime
/// and provides type-safe access to styles via selectors.

use eframe::egui;
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::ToCss;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
    System, // Follow system preference
}

/// Style properties that can be applied to UI elements
#[derive(Debug, Clone, Default)]
pub struct Style {
    pub background_color: Option<egui::Color32>,
    pub border_color: Option<egui::Color32>,
    pub text_color: Option<egui::Color32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    // Add more properties as needed
}

impl Style {
    /// Merge another style into this one (other's properties override if present)
    pub fn merge(&mut self, other: &Style) {
        if other.background_color.is_some() {
            self.background_color = other.background_color;
        }
        if other.border_color.is_some() {
            self.border_color = other.border_color;
        }
        if other.text_color.is_some() {
            self.text_color = other.text_color;
        }
        if other.width.is_some() {
            self.width = other.width;
        }
        if other.height.is_some() {
            self.height = other.height;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    light_variables: HashMap<String, String>,
    dark_variables: HashMap<String, String>,
    light_styles: HashMap<String, Style>,
    dark_styles: HashMap<String, Style>,
    current_mode: ThemeMode,
}

impl Theme {
    /// Load theme from CSS file
    pub fn from_css(css: &str) -> Result<Self, String> {
        let stylesheet = StyleSheet::parse(
            css,
            ParserOptions::default(),
        ).map_err(|e| format!("Failed to parse CSS: {:?}", e))?;

        let mut light_variables = HashMap::new();
        let mut dark_variables = HashMap::new();
        let mut light_styles = HashMap::new();
        let mut dark_styles = HashMap::new();

        // First pass: Extract CSS custom properties from :root
        for rule in &stylesheet.rules.0 {
            match rule {
                lightningcss::rules::CssRule::Style(style_rule) => {
                    let selectors = style_rule.selectors.0.iter()
                        .filter_map(|s| s.to_css_string(PrinterOptions::default()).ok())
                        .collect::<Vec<_>>();

                    // Check if this is :root
                    if selectors.iter().any(|s| s.contains(":root")) {
                        extract_css_variables(&style_rule.declarations, &mut light_variables)?;
                    }
                }
                lightningcss::rules::CssRule::Media(media_rule) => {
                    let media_str = media_rule.query.to_css_string(PrinterOptions::default())
                        .unwrap_or_default();

                    if media_str.contains("prefers-color-scheme") && media_str.contains("dark") {
                        for inner_rule in &media_rule.rules.0 {
                            if let lightningcss::rules::CssRule::Style(style_rule) = inner_rule {
                                let selectors = style_rule.selectors.0.iter()
                                    .filter_map(|s| s.to_css_string(PrinterOptions::default()).ok())
                                    .collect::<Vec<_>>();

                                if selectors.iter().any(|s| s.contains(":root")) {
                                    extract_css_variables(&style_rule.declarations, &mut dark_variables)?;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Second pass: Parse style rules and resolve var() references
        // We need to parse selectors TWICE - once with light variables, once with dark variables
        for rule in &stylesheet.rules.0 {
            match rule {
                lightningcss::rules::CssRule::Style(style_rule) => {
                    let selectors = style_rule.selectors.0.iter()
                        .filter_map(|s| s.to_css_string(PrinterOptions::default()).ok())
                        .collect::<Vec<_>>();

                    for selector in selectors {
                        let selector = selector.trim();
                        // Only process class and ID selectors
                        if selector.starts_with('.') || selector.starts_with('#') {
                            // Parse with light variables
                            let light_style = parse_style_properties(&style_rule.declarations, &light_variables)?;
                            light_styles.insert(selector.to_string(), light_style);

                            // Also parse with dark variables (merge dark over light)
                            let mut dark_vars = light_variables.clone();
                            dark_vars.extend(dark_variables.clone());
                            let dark_style = parse_style_properties(&style_rule.declarations, &dark_vars)?;
                            dark_styles.insert(selector.to_string(), dark_style);
                        }
                    }
                }
                lightningcss::rules::CssRule::Media(media_rule) => {
                    let media_str = media_rule.query.to_css_string(PrinterOptions::default())
                        .unwrap_or_default();

                    eprintln!("🔍 Found media query: {}", media_str);
                    eprintln!("   Contains {} rules", media_rule.rules.0.len());

                    if media_str.contains("prefers-color-scheme") && media_str.contains("dark") {
                        eprintln!("   ✓ This is a dark mode media query!");
                        for (i, inner_rule) in media_rule.rules.0.iter().enumerate() {
                            eprintln!("   Rule {}: {:?}", i, std::mem::discriminant(inner_rule));
                            if let lightningcss::rules::CssRule::Style(style_rule) = inner_rule {
                                let selectors = style_rule.selectors.0.iter()
                                    .filter_map(|s| s.to_css_string(PrinterOptions::default()).ok())
                                    .collect::<Vec<_>>();

                                eprintln!("   Found selectors: {:?}", selectors);

                                for selector in selectors {
                                    let selector = selector.trim();
                                    if selector.starts_with('.') || selector.starts_with('#') {
                                        // Merge dark and light variables (dark overrides light)
                                        let mut vars = light_variables.clone();
                                        vars.extend(dark_variables.clone());
                                        let style = parse_style_properties(&style_rule.declarations, &vars)?;
                                        dark_styles.insert(selector.to_string(), style);
                                        eprintln!("     Added dark style for: {}", selector);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(Self {
            light_variables,
            dark_variables,
            light_styles,
            dark_styles,
            current_mode: ThemeMode::System,
        })
    }

    /// Load theme from embedded CSS file
    pub fn load_default() -> Result<Self, String> {
        let css = include_str!("../assets/styles.css");
        Self::from_css(css)
    }

    /// Set the current theme mode
    pub fn set_mode(&mut self, mode: ThemeMode) {
        self.current_mode = mode;
    }

    /// Get the current theme mode
    pub fn mode(&self) -> ThemeMode {
        self.current_mode
    }

    /// Get style for a selector (e.g., ".panel" or "#timeline-header")
    pub fn style(&self, selector: &str, ctx: &egui::Context) -> Style {
        let is_dark = match self.current_mode {
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
            ThemeMode::System => ctx.style().visuals.dark_mode,
        };

        if is_dark {
            // Try dark style first, fall back to light style
            self.dark_styles.get(selector).cloned()
                .or_else(|| self.light_styles.get(selector).cloned())
                .unwrap_or_default()
        } else {
            self.light_styles.get(selector).cloned().unwrap_or_default()
        }
    }

    /// Get a CSS variable value and parse as color (backward compatibility helper)
    /// This allows old code using theme.color("variable-name") to work
    pub fn color(&self, var_name: &str) -> Option<egui::Color32> {
        // Try light variables first, then dark variables
        let value = self.light_variables.get(var_name)
            .or_else(|| self.dark_variables.get(var_name))?;
        parse_hex_color(value)
    }

    /// Get the number of loaded selectors
    pub fn len(&self) -> usize {
        self.light_styles.len()
    }

    /// Check if theme has no styles
    pub fn is_empty(&self) -> bool {
        self.light_styles.is_empty()
    }

    /// Debug: print loaded theme info
    pub fn debug_print(&self) {
        println!("📊 Theme Debug Info:");
        println!("  Light variables: {}", self.light_variables.len());
        for (k, v) in self.light_variables.iter().take(5) {
            println!("    --{}: {}", k, v);
        }
        println!("  Dark variables: {}", self.dark_variables.len());
        for (k, v) in self.dark_variables.iter().take(5) {
            println!("    --{}: {}", k, v);
        }
        println!("  Light styles: {}", self.light_styles.len());
        for k in self.light_styles.keys().take(5) {
            println!("    {}", k);
        }
        println!("  Dark styles: {}", self.dark_styles.len());
        for k in self.dark_styles.keys().take(5) {
            println!("    {}", k);
        }
    }
}

/// Extract CSS custom properties (--variables) from declarations
fn extract_css_variables(
    declarations: &lightningcss::declaration::DeclarationBlock,
    variables: &mut HashMap<String, String>,
) -> Result<(), String> {
    for property in &declarations.declarations {
        if let lightningcss::properties::Property::Custom(_) = property {
            let property_css = property.to_css_string(false, PrinterOptions::default())
                .map_err(|e| format!("Failed to serialize property: {:?}", e))?;

            if let Some((name, value)) = property_css.split_once(':') {
                let name = name.trim().strip_prefix("--").unwrap_or(name.trim()).to_string();
                let value = value.trim().to_string();
                variables.insert(name, value);
            }
        }
    }
    Ok(())
}

/// Parse style properties from CSS declarations into a Style struct, resolving var() references
fn parse_style_properties(
    declarations: &lightningcss::declaration::DeclarationBlock,
    variables: &HashMap<String, String>,
) -> Result<Style, String> {
    let mut style = Style::default();

    for property in &declarations.declarations {
        // Convert property to CSS string and parse
        let prop_str = property.to_css_string(false, PrinterOptions::default())
            .map_err(|e| format!("Failed to serialize property: {:?}", e))?;

        // Parse property name and value
        if let Some((name, value)) = prop_str.split_once(':') {
            let name = name.trim();
            let value = value.trim().trim_end_matches(';');

            match name {
                "background-color" => {
                    style.background_color = parse_color_value(value, variables);
                }
                "border-color" | "border-top-color" => {
                    style.border_color = parse_color_value(value, variables);
                }
                "color" => {
                    style.text_color = parse_color_value(value, variables);
                }
                "width" => {
                    style.width = parse_dimension_value(value, variables);
                }
                "height" => {
                    style.height = parse_dimension_value(value, variables);
                }
                _ => {}
            }
        }
    }

    Ok(style)
}

/// Parse a CSS color value (hex or var())
fn parse_color_value(value: &str, variables: &HashMap<String, String>) -> Option<egui::Color32> {
    let value = value.trim();

    // Check if it's a var() reference
    if let Some(var_name) = parse_var_reference(value) {
        let resolved = variables.get(&var_name)?;
        return parse_hex_color(resolved);
    }

    // Try to parse as direct hex color
    parse_hex_color(value)
}

/// Parse a CSS dimension value (px or var())
fn parse_dimension_value(value: &str, variables: &HashMap<String, String>) -> Option<f32> {
    let value = value.trim();

    // Check if it's a var() reference
    if let Some(var_name) = parse_var_reference(value) {
        let resolved = variables.get(&var_name)?;
        return parse_dimension_string(resolved);
    }

    // Try to parse as direct dimension
    parse_dimension_string(value)
}

/// Parse a var() reference to get the variable name
fn parse_var_reference(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.starts_with("var(") && trimmed.ends_with(')') {
        let inner = trimmed.strip_prefix("var(")?.strip_suffix(')')?;
        let var_name = inner.trim().strip_prefix("--")?;
        Some(var_name.to_string())
    } else {
        None
    }
}

/// Parse hex color string to egui::Color32
fn parse_hex_color(value: &str) -> Option<egui::Color32> {
    let value = value.trim();
    if !value.starts_with('#') {
        return None;
    }

    let hex = value.trim_start_matches('#');
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some(egui::Color32::from_rgb(r, g, b))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(egui::Color32::from_rgb(r, g, b))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(egui::Color32::from_rgba_unmultiplied(r, g, b, a))
        }
        _ => None,
    }
}

/// Parse dimension string (e.g., "50px" or "25")
fn parse_dimension_string(value: &str) -> Option<f32> {
    let value = value.trim();
    if let Some(stripped) = value.strip_suffix("px") {
        stripped.trim().parse::<f32>().ok()
    } else {
        value.parse::<f32>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_theme() {
        let theme = Theme::load_default().expect("Failed to load default theme");
        assert!(!theme.is_empty(), "Theme should have styles loaded");
    }
}
