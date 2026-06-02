//! Default effect definitions registry
//!
//! Provides default effect definitions with embedded WGSL shaders.
//! These are copied into documents when used - no runtime dependency on registry.
//!
//! Built-in effects use stable UUIDs so they can be reliably looked up by ID.

use crate::effect::{EffectCategory, EffectDefinition, EffectParameterDef};
use uuid::Uuid;

// Stable UUIDs for built-in effects (randomly generated, never change)
const GRAYSCALE_ID: Uuid = Uuid::from_u128(0xac2cd8ce_4ea3_4c84_8c70_5cfc4dae22fb);
const INVERT_ID: Uuid = Uuid::from_u128(0x9ff36aef_5f40_45b2_bf42_cbe7fa52bd3a);
const BRIGHTNESS_CONTRAST_ID: Uuid = Uuid::from_u128(0x6cd772c9_ea8a_4b1e_93fb_2aa1d3306f62);
const HUE_SATURATION_ID: Uuid = Uuid::from_u128(0x3f210ac2_4eb5_436a_8337_c583d19dcbe1);
const COLOR_TINT_ID: Uuid = Uuid::from_u128(0x7b85ea51_22d6_4506_8689_85bdcd9ca6db);
const GAUSSIAN_BLUR_ID: Uuid = Uuid::from_u128(0x3e36bc88_3495_4f8b_ad07_8a5cdcc4c05b);
const VIGNETTE_ID: Uuid = Uuid::from_u128(0xf21873da_df9e_4ba2_ba5d_46a276e6485c);
const SHARPEN_ID: Uuid = Uuid::from_u128(0x217f644a_c4a1_46ed_b9b7_86b820792b29);

/// Registry of default built-in effects
pub struct EffectRegistry;

impl EffectRegistry {
    /// Get all available default effect definitions
    pub fn get_all() -> Vec<EffectDefinition> {
        vec![
            Self::grayscale(),
            Self::invert(),
            Self::brightness_contrast(),
            Self::hue_saturation(),
            Self::color_tint(),
            Self::gaussian_blur(),
            Self::vignette(),
            Self::sharpen(),
        ]
    }

    /// Get a specific effect by name
    pub fn get_by_name(name: &str) -> Option<EffectDefinition> {
        match name.to_lowercase().as_str() {
            "grayscale" => Some(Self::grayscale()),
            "invert" => Some(Self::invert()),
            "brightness/contrast" | "brightness_contrast" => Some(Self::brightness_contrast()),
            "hue/saturation" | "hue_saturation" => Some(Self::hue_saturation()),
            "color tint" | "color_tint" => Some(Self::color_tint()),
            "gaussian blur" | "gaussian_blur" => Some(Self::gaussian_blur()),
            "vignette" => Some(Self::vignette()),
            "sharpen" => Some(Self::sharpen()),
            _ => None,
        }
    }

    /// Get a specific effect by its UUID
    pub fn get_by_id(id: &Uuid) -> Option<EffectDefinition> {
        Self::get_all().into_iter().find(|def| def.id == *id)
    }

    /// Grayscale effect - converts to black and white
    pub fn grayscale() -> EffectDefinition {
        EffectDefinition::with_id(
            GRAYSCALE_ID,
            "Grayscale",
            EffectCategory::Color,
            include_str!("shaders/effect_grayscale.wgsl"),
            vec![
                EffectParameterDef::float_range("amount", "Amount", 1.0, 0.0, 1.0),
            ],
        ).with_description("Convert image to grayscale")
    }

    /// Invert effect - inverts colors
    pub fn invert() -> EffectDefinition {
        EffectDefinition::with_id(
            INVERT_ID,
            "Invert",
            EffectCategory::Color,
            include_str!("shaders/effect_invert.wgsl"),
            vec![
                EffectParameterDef::float_range("amount", "Amount", 1.0, 0.0, 1.0),
            ],
        ).with_description("Invert image colors")
    }

    /// Brightness/Contrast adjustment
    pub fn brightness_contrast() -> EffectDefinition {
        EffectDefinition::with_id(
            BRIGHTNESS_CONTRAST_ID,
            "Brightness/Contrast",
            EffectCategory::Color,
            include_str!("shaders/effect_brightness_contrast.wgsl"),
            vec![
                EffectParameterDef::float_range("brightness", "Brightness", 0.0, -1.0, 1.0),
                EffectParameterDef::float_range("contrast", "Contrast", 1.0, 0.0, 3.0),
            ],
        ).with_description("Adjust brightness and contrast")
    }

    /// Hue/Saturation adjustment
    pub fn hue_saturation() -> EffectDefinition {
        EffectDefinition::with_id(
            HUE_SATURATION_ID,
            "Hue/Saturation",
            EffectCategory::Color,
            include_str!("shaders/effect_hue_saturation.wgsl"),
            vec![
                EffectParameterDef::angle("hue", "Hue Shift", 0.0),
                EffectParameterDef::float_range("saturation", "Saturation", 1.0, 0.0, 3.0),
                EffectParameterDef::float_range("lightness", "Lightness", 0.0, -1.0, 1.0),
            ],
        ).with_description("Adjust hue, saturation, and lightness")
    }

    /// Color tint effect
    pub fn color_tint() -> EffectDefinition {
        EffectDefinition::with_id(
            COLOR_TINT_ID,
            "Color Tint",
            EffectCategory::Color,
            include_str!("shaders/effect_color_tint.wgsl"),
            vec![
                EffectParameterDef::color("tint_color", "Tint Color", 1.0, 0.5, 0.0, 1.0),
                EffectParameterDef::float_range("amount", "Amount", 0.5, 0.0, 1.0),
            ],
        ).with_description("Apply a color tint overlay")
    }

    /// Gaussian blur effect
    pub fn gaussian_blur() -> EffectDefinition {
        EffectDefinition::with_id(
            GAUSSIAN_BLUR_ID,
            "Gaussian Blur",
            EffectCategory::Blur,
            include_str!("shaders/effect_blur.wgsl"),
            vec![
                EffectParameterDef::float_range("radius", "Radius", 5.0, 0.0, 50.0),
                EffectParameterDef::float_range("quality", "Quality", 1.0, 0.0, 1.0),
            ],
        ).with_description("Gaussian blur effect")
    }

    /// Vignette effect - darkens edges
    pub fn vignette() -> EffectDefinition {
        EffectDefinition::with_id(
            VIGNETTE_ID,
            "Vignette",
            EffectCategory::Stylize,
            include_str!("shaders/effect_vignette.wgsl"),
            vec![
                EffectParameterDef::float_range("radius", "Radius", 0.5, 0.0, 1.5),
                EffectParameterDef::float_range("softness", "Softness", 0.5, 0.0, 1.0),
                EffectParameterDef::float_range("amount", "Amount", 0.5, 0.0, 1.0),
            ],
        ).with_description("Add a vignette darkening effect to edges")
    }

    /// Sharpen effect
    pub fn sharpen() -> EffectDefinition {
        EffectDefinition::with_id(
            SHARPEN_ID,
            "Sharpen",
            EffectCategory::Stylize,
            include_str!("shaders/effect_sharpen.wgsl"),
            vec![
                EffectParameterDef::float_range("amount", "Amount", 1.0, 0.0, 3.0),
                EffectParameterDef::float_range("radius", "Radius", 1.0, 0.5, 5.0),
            ],
        ).with_description("Sharpen image details")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_effects() {
        let effects = EffectRegistry::get_all();
        assert!(effects.len() >= 8);
    }

    #[test]
    fn test_get_by_name() {
        let grayscale = EffectRegistry::get_by_name("grayscale");
        assert!(grayscale.is_some());
        assert_eq!(grayscale.unwrap().name, "Grayscale");

        let unknown = EffectRegistry::get_by_name("unknown_effect");
        assert!(unknown.is_none());
    }
}
