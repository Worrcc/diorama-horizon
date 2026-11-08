//! Diorama as a Horizon client mod (wasm32). M0 stub: no Horizon SDK yet.
//! TODO(M1): implement the Mod trait (init/frame/tick/command) on top of horizon-mod-sdk.

pub use diorama_core::DEFAULT_PITCH_DEGREES;

/// Placeholder so the cdylib has a symbol; replaced by the real mod entry points in M1.
#[no_mangle]
pub extern "C" fn diorama_version() -> u32 {
    0
}
