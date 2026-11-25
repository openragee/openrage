//! DXBC to SPIR-V through the vendored dxbc-spirv translator, compiled by the `native` feature.

use thiserror::Error;

pub mod dxil;
mod layout;
pub mod rdna;
pub mod rdna_bind;
pub mod rdna_cache;
pub mod rdna_corpus;
pub use layout::{program_type, Binding, BindingLayout, ResourceClass, Window};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrontError {
    #[error("the DXBC translator is not compiled in: build with the `native` feature")]
    NotCompiledIn,
    #[error("the translator refused the shader: {0}")]
    Refused(String),
}

/// The dxbc-spirv revision vendored under `vendor/dxbc-spirv`.
pub const VENDORED_REVISION: &str = "c7f06970";

/// Names this translator build: the vendored revision and a hash of the shim and the layout, which change what it writes.
pub fn identity() -> String {
    format!(
        "dxbc-spirv {VENDORED_REVISION} {}",
        env!("OR_SHADER_FRONT_FINGERPRINT")
    )
}

/// Whether this build carries the translator.
pub const NATIVE: bool = cfg!(feature = "native");

#[cfg(feature = "native")]
mod native {
    use std::os::raw::c_char;

    #[repr(C)]
    pub struct OrFrontResult {
        pub spirv: *mut u8,
        pub spirv_len: usize,
        pub message: *mut c_char,
    }

    extern "C" {
        pub fn or_front_dxbc_to_spirv(
            data: *const u8,
            size: usize,
            layout: *const super::BindingLayout,
            out: *mut OrFrontResult,
        ) -> i32;
        pub fn or_front_free(result: *mut OrFrontResult);
    }
}

/// Translate one DXBC container to a SPIR-V module, as little-endian bytes, with [`BindingLayout::OPENRAGE`] for the container's own stage.
pub fn dxbc_to_spirv(dxbc: &[u8]) -> Result<Vec<u8>, FrontError> {
    let layout = BindingLayout::OPENRAGE.for_program_type(program_type(dxbc).unwrap_or(1));
    dxbc_to_spirv_with(dxbc, &layout)
}

/// [`dxbc_to_spirv`] placing each register where `layout` says, and refusing one it has no room for.
#[cfg(feature = "native")]
pub fn dxbc_to_spirv_with(dxbc: &[u8], layout: &BindingLayout) -> Result<Vec<u8>, FrontError> {
    use std::ffi::CStr;

    let mut result = native::OrFrontResult {
        spirv: std::ptr::null_mut(),
        spirv_len: 0,
        message: std::ptr::null_mut(),
    };
    // SAFETY: the shim only reads `dxbc` for its stated length, and fills `result` with memory it
    // allocated, which `or_front_free` releases once, after the copies below.
    let (status, spirv, message) = unsafe {
        let status = native::or_front_dxbc_to_spirv(dxbc.as_ptr(), dxbc.len(), layout, &mut result);
        let spirv = if result.spirv.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(result.spirv, result.spirv_len).to_vec()
        };
        let message = if result.message.is_null() {
            String::new()
        } else {
            CStr::from_ptr(result.message)
                .to_string_lossy()
                .into_owned()
        };
        native::or_front_free(&mut result);
        (status, spirv, message)
    };
    if status == 0 {
        Ok(spirv)
    } else {
        Err(FrontError::Refused(message.trim().to_string()))
    }
}

/// [`dxbc_to_spirv`] placing each register where `layout` says, and refusing one it has no room for.
#[cfg(not(feature = "native"))]
pub fn dxbc_to_spirv_with(_dxbc: &[u8], _layout: &BindingLayout) -> Result<Vec<u8>, FrontError> {
    Err(FrontError::NotCompiledIn)
}

#[cfg(test)]
mod tests;
