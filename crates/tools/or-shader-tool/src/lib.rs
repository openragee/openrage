// wgpu's auto-trait chain (`Device: Sync`) is deeper than the default 128 on this nightly.
#![recursion_limit = "256"]
pub mod backend_check;
#[cfg(feature = "native-front")]
pub mod bindings;
pub mod defs;
#[cfg(feature = "device-check")]
pub mod device_check;
#[cfg(all(test, feature = "native-front"))]
mod dxbc_testing;
pub mod inventory;
pub mod legalize;
pub mod pack_digest;
pub mod pack_protocol;
#[cfg(feature = "native-front")]
pub mod pass_check;
pub mod passes;
#[cfg(feature = "device-check")]
pub mod rdna_devices;
pub mod rdna_report;
#[cfg(feature = "render-drawable")]
pub mod render_drawable;
pub mod signature;
pub mod spirv_check;
#[cfg(feature = "native-front")]
pub mod spirv_pack;
#[cfg(test)]
mod spirv_testing;
#[cfg(feature = "native-front")]
pub mod translate_check;

pub use defs::{DefsBuild, DefsError};
