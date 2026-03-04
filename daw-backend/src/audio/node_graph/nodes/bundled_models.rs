use nam_ffi::NamModel;

struct BundledModel {
    name: &'static str,
    filename: &'static str,
    data: &'static [u8],
}

const BUNDLED_MODELS: &[BundledModel] = &[
    BundledModel {
        name: "BossSD1",
        filename: "BossSD1-WaveNet.nam",
        data: include_bytes!("../../../../../src/assets/nam_models/BossSD1-WaveNet.nam"),
    },
    BundledModel {
        name: "DeluxeReverb",
        filename: "DeluxeReverb.nam",
        data: include_bytes!("../../../../../src/assets/nam_models/DeluxeReverb.nam"),
    },
    BundledModel {
        name: "DingwallBass",
        filename: "DingwallBass.nam",
        data: include_bytes!("../../../../../src/assets/nam_models/DingwallBass.nam"),
    },
    BundledModel {
        name: "Rhythm",
        filename: "Rhythm.nam",
        data: include_bytes!("../../../../../src/assets/nam_models/Rhythm.nam"),
    },
];

/// Return display names of all bundled NAM models.
pub fn bundled_model_names() -> Vec<&'static str> {
    BUNDLED_MODELS.iter().map(|m| m.name).collect()
}

/// Load a bundled NAM model by display name.
/// Returns `None` if the name isn't found, `Some(Err(...))` on load failure.
pub fn load_bundled_model(name: &str) -> Option<Result<NamModel, String>> {
    eprintln!("[NAM] load_bundled_model: looking up {:?}", name);
    let model = BUNDLED_MODELS.iter().find(|m| m.name == name)?;
    eprintln!("[NAM] Found bundled model: name={}, filename={}, data_len={}", model.name, model.filename, model.data.len());
    Some(
        NamModel::from_bytes(model.filename, model.data)
            .map_err(|e| {
                eprintln!("[NAM] from_bytes failed for {}: {}", model.filename, e);
                e.to_string()
            }),
    )
}
