use std::ffi::CString;
use std::path::Path;

#[allow(dead_code)]
mod ffi {
    use std::os::raw::{c_char, c_float, c_int};

    #[repr(C)]
    pub struct NeuralModel {
        _opaque: [u8; 0],
    }

    unsafe extern "C" {
        pub fn CreateModelFromFile(model_path: *const c_char) -> *mut NeuralModel;
        pub fn DeleteModel(model: *mut NeuralModel);

        pub fn SetLSTMLoadMode(load_mode: c_int);
        pub fn SetWaveNetLoadMode(load_mode: c_int);
        pub fn SetAudioInputLevelDBu(audio_dbu: c_float);
        pub fn SetDefaultMaxAudioBufferSize(max_size: c_int);

        pub fn GetLoadMode(model: *mut NeuralModel) -> c_int;
        pub fn IsStatic(model: *mut NeuralModel) -> bool;
        pub fn SetMaxAudioBufferSize(model: *mut NeuralModel, max_size: c_int);
        pub fn GetRecommendedInputDBAdjustment(model: *mut NeuralModel) -> c_float;
        pub fn GetRecommendedOutputDBAdjustment(model: *mut NeuralModel) -> c_float;
        pub fn GetSampleRate(model: *mut NeuralModel) -> c_float;

        pub fn Process(
            model: *mut NeuralModel,
            input: *mut c_float,
            output: *mut c_float,
            num_samples: usize,
        );
    }
}

#[derive(Debug)]
pub enum NamError {
    NullPath,
    ModelLoadFailed(String),
}

impl std::fmt::Display for NamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NamError::NullPath => write!(f, "Path contains null byte"),
            NamError::ModelLoadFailed(path) => write!(f, "Failed to load NAM model: {}", path),
        }
    }
}

pub struct NamModel {
    ptr: *mut ffi::NeuralModel,
}

impl NamModel {
    pub fn from_file(path: &Path) -> Result<Self, NamError> {
        let path_str = path.to_string_lossy();
        let c_path = CString::new(path_str.as_bytes()).map_err(|_| NamError::NullPath)?;

        let ptr = unsafe { ffi::CreateModelFromFile(c_path.as_ptr()) };
        if ptr.is_null() {
            return Err(NamError::ModelLoadFailed(path_str.into_owned()));
        }

        Ok(NamModel { ptr })
    }

    pub fn sample_rate(&self) -> f32 {
        unsafe { ffi::GetSampleRate(self.ptr) }
    }

    pub fn recommended_input_db(&self) -> f32 {
        unsafe { ffi::GetRecommendedInputDBAdjustment(self.ptr) }
    }

    pub fn recommended_output_db(&self) -> f32 {
        unsafe { ffi::GetRecommendedOutputDBAdjustment(self.ptr) }
    }

    pub fn set_max_buffer_size(&mut self, size: i32) {
        unsafe { ffi::SetMaxAudioBufferSize(self.ptr, size) }
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let len = input.len().min(output.len());
        if len == 0 {
            return;
        }
        // The C API takes mutable input pointer (even though it doesn't modify it).
        // Copy to a mutable scratch to avoid UB from casting away const.
        let mut input_copy: Vec<f32> = input[..len].to_vec();
        unsafe {
            ffi::Process(
                self.ptr,
                input_copy.as_mut_ptr(),
                output.as_mut_ptr(),
                len,
            );
        }
    }
}

impl Drop for NamModel {
    fn drop(&mut self) {
        unsafe { ffi::DeleteModel(self.ptr) }
    }
}

// SAFETY: NeuralModel is a self-contained C++ object with no thread-local state.
// It is safe to move between threads, but not to share across threads (no Sync).
unsafe impl Send for NamModel {}
