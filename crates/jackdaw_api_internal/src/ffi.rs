//! Stable ABI used by the dylib extension and game loaders.
//!
//! Two kinds of loadable dylib are supported, distinguished by
//! which entry symbol they expose:
//!
//! * **Extensions** expose `jackdaw_extension_entry_v1` and return
//!   an [`ExtensionEntry`]. Loaded by the dylib loader at startup
//!   and via the runtime Install UI.
//! * **Games** expose `jackdaw_game_entry_v1` and return a
//!   [`GameEntry`]. Loaded at startup alongside extensions; their
//!   `build` callback is invoked against the editor's `App` so
//!   game plugins integrate natively.
//!
//! Both envelopes share the same version fields
//! ([`API_VERSION`] / [`BEVY_VERSION`] / [`PROFILE`]); the loader
//! verifies them identically.
//!
//! Authors don't write these structs by hand — the
//! [`export_extension!`](crate::export_extension) and
//! [`export_game!`](crate::export_game) macros emit the entry
//! functions. The loader lives in `crates/jackdaw_loader`.
//!
//! # ABI stability
//!
//! All three embedded version fields must match host values
//! exactly:
//!
//! * [`API_VERSION`] — bumped on any breaking change to
//!   `JackdawExtension`, the FFI struct layout, or entry semantics.
//! * [`BEVY_VERSION`] — Bevy minor-version string. Bevy's types
//!   (`App`, `World`, `Commands`) appear in the extension's vtable,
//!   so any Bevy version change risks vtable drift.
//! * [`PROFILE`] — debug vs release. The two are ABI-incompatible in
//!   practice (different feature combinations, different layout
//!   optimisations).

use core::ffi::{CStr, c_char};

/// Current ABI version. Bump on any breaking change to
/// [`ExtensionEntry`], [`crate::JackdawExtension`], or the loader's
/// expectations about the entry function.
pub const API_VERSION: u32 = 1;

/// Bevy minor-version string the host was built against. The loader
/// compares this against the dylib's embedded value and refuses to
/// load on mismatch.
pub const BEVY_VERSION: &CStr = c"0.18";

/// Compile-time build profile. Debug and release builds are
/// ABI-incompatible in practice, so the loader refuses to mix them.
pub const PROFILE: &CStr = if cfg!(debug_assertions) {
    c"debug"
} else {
    c"release"
};

/// Symbol name the loader looks up in extension dylibs. Includes
/// the trailing NUL so it can be passed directly to
/// `libloading::Library::get`.
pub const ENTRY_SYMBOL: &[u8] = b"jackdaw_extension_entry_v1\0";

/// Symbol name the loader looks up in game dylibs. Includes the
/// trailing NUL for the same reason as [`ENTRY_SYMBOL`].
///
/// When a dylib is opened, the loader tries this symbol first. If
/// it's absent the dylib is treated as an extension and
/// [`ENTRY_SYMBOL`] is looked up next.
pub const GAME_ENTRY_SYMBOL: &[u8] = b"jackdaw_game_entry_v1\0";

/// Shape returned by every dylib extension's entry function.
///
/// Declared `#[repr(C)]` so the layout is stable across compilation
/// units. Trait-object fields (`ctor`'s return type) require the
/// editor and extension to have been built against the same Bevy
/// version, hence the embedded version fields.
///
/// # Safety
///
/// Every pointer field must reference NUL-terminated static data
/// that outlives the host process. The returned `Box` from `ctor`
/// must be allocated with the same allocator the host uses; this is
/// guaranteed when both sides link against Bevy with
/// `dynamic_linking` enabled, which shares the Rust allocator.
///
/// `improper_ctypes_definitions` is silenced for the `ctor` field:
/// `Box<dyn Trait>` is a fat pointer with a rustc-defined layout,
/// which is sound here because editor and extension are required
/// to link against the same Bevy dylib and so agree on layout.
#[repr(C)]
#[allow(improper_ctypes_definitions)]
pub struct ExtensionEntry {
    pub api_version: u32,
    pub bevy_version: *const c_char,
    pub profile: *const c_char,
    pub name: *const c_char,
    pub ctor: unsafe extern "C" fn() -> Box<dyn crate::JackdawExtension>,
}

/// Shape returned by every game dylib's entry function.
///
/// Parallel to [`ExtensionEntry`] but carries a `build` callback
/// that receives the editor's `App` directly — the game author
/// provides a [`bevy::app::Plugin`] whose `build` installs the
/// game's systems into the editor's World. Game systems gate their
/// execution on jackdaw's `PlayState::Playing` run condition so
/// they're dormant until the user clicks the Play button.
///
/// # Safety
///
/// Same contract as [`ExtensionEntry`]: NUL-terminated static
/// strings, same allocator on both sides (guaranteed by
/// `bevy/dynamic_linking` + jackdaw_sdk's proxy dylib). The
/// `build` function pointer must be callable with any valid
/// `*mut App`; the loader wraps the call in `catch_unwind` so
/// panics in game setup don't abort the editor.
#[repr(C)]
#[allow(improper_ctypes_definitions)]
pub struct GameEntry {
    pub api_version: u32,
    pub bevy_version: *const c_char,
    pub profile: *const c_char,
    pub name: *const c_char,
    pub build: unsafe extern "C" fn(*mut bevy::app::App),
}
