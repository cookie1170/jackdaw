//! Boilerplate generators for dylib extensions and games.
//!
//! An extension crate uses [`export_extension!`](crate::export_extension);
//! a game project uses [`export_game!`](crate::export_game). Both
//! emit the `#[unsafe(no_mangle)]` entry function the loader looks
//! up plus the [`ExtensionEntry`](crate::ffi::ExtensionEntry) or
//! [`GameEntry`](crate::ffi::GameEntry) construction.
//!
//! # Extension example
//!
//! ```ignore
//! use jackdaw_api::prelude::*;
//! use jackdaw_api::export_extension;
//!
//! pub struct MyExtension;
//!
//! impl JackdawExtension for MyExtension {
//!     fn name(&self) -> &str { "my_extension" }
//!     fn register(&self, _ctx: &mut ExtensionContext) {}
//! }
//!
//! export_extension!("my_extension", || Box::new(MyExtension));
//! ```
//!
//! # Game example
//!
//! ```ignore
//! use bevy::prelude::*;
//! use jackdaw_api::export_game;
//!
//! pub struct GamePlugin;
//!
//! impl Plugin for GamePlugin {
//!     fn build(&self, app: &mut App) {
//!         app.add_systems(Update, spin_cube);
//!     }
//! }
//!
//! fn spin_cube() { /* run-condition gated on PlayState::Playing */ }
//!
//! export_game!("my_game", GamePlugin);
//! ```

/// Emit the `extern "C"` entry point a dylib extension needs.
///
/// The first argument is the extension name as a string literal; the
/// second is a `fn() -> Box<dyn JackdawExtension>` constructor. Both
/// are baked into the [`ExtensionEntry`](crate::ffi::ExtensionEntry)
/// returned by the generated `jackdaw_extension_entry_v1` symbol.
///
/// Invoke at most once per crate. Producing two extensions from one
/// dylib would fail at link time anyway (duplicate symbol).
#[macro_export]
macro_rules! export_extension {
    ($name:literal, $ctor:expr) => {
        const _: () = {
            const __JACKDAW_NAME: &::core::ffi::CStr = match ::core::ffi::CStr::from_bytes_with_nul(
                ::core::concat!($name, "\0").as_bytes(),
            ) {
                ::core::result::Result::Ok(s) => s,
                ::core::result::Result::Err(_) => {
                    ::core::panic!("extension name contains interior NUL byte")
                }
            };

            unsafe extern "C" fn __jackdaw_ctor() -> ::std::boxed::Box<dyn $crate::JackdawExtension>
            {
                let make: fn() -> ::std::boxed::Box<dyn $crate::JackdawExtension> = $ctor;
                make()
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn jackdaw_extension_entry_v1() -> $crate::ffi::ExtensionEntry {
                $crate::ffi::ExtensionEntry {
                    api_version: $crate::ffi::API_VERSION,
                    bevy_version: $crate::ffi::BEVY_VERSION.as_ptr(),
                    profile: $crate::ffi::PROFILE.as_ptr(),
                    name: __JACKDAW_NAME.as_ptr(),
                    ctor: __jackdaw_ctor,
                }
            }
        };
    };
}

/// Emit the `extern "C"` entry point a dylib game needs.
///
/// The first argument is the game name as a string literal; the
/// second is a type implementing `bevy::app::Plugin`. The plugin's
/// `build` method runs once at startup against the editor's App —
/// register game systems there and gate them on
/// `PlayState::Playing` so they only tick when the user has Play
/// engaged.
///
/// Invoke at most once per crate. Producing two entries from one
/// dylib would fail at link time anyway (duplicate symbol).
///
/// # Example
///
/// ```ignore
/// use bevy::prelude::*;
/// use jackdaw_api::export_game;
///
/// pub struct GamePlugin;
///
/// impl Plugin for GamePlugin {
///     fn build(&self, app: &mut App) {
///         app.add_systems(Update, my_game_system);
///     }
/// }
///
/// export_game!("my_game", GamePlugin);
/// ```
#[macro_export]
macro_rules! export_game {
    ($name:literal, $plugin:expr) => {
        const _: () = {
            const __JACKDAW_GAME_NAME: &::core::ffi::CStr =
                match ::core::ffi::CStr::from_bytes_with_nul(
                    ::core::concat!($name, "\0").as_bytes(),
                ) {
                    ::core::result::Result::Ok(s) => s,
                    ::core::result::Result::Err(_) => {
                        ::core::panic!("game name contains interior NUL byte")
                    }
                };

            unsafe extern "C" fn __jackdaw_game_build(app: *mut ::bevy::app::App) {
                // SAFETY: the loader calls this with a valid `&mut App`
                // pointer obtained via `Box::leak`-style coercion. The
                // pointer lives as long as the editor's App.
                let app: &mut ::bevy::app::App = unsafe { &mut *app };
                ::bevy::app::App::add_plugins(app, $plugin);
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn jackdaw_game_entry_v1() -> $crate::ffi::GameEntry {
                $crate::ffi::GameEntry {
                    api_version: $crate::ffi::API_VERSION,
                    bevy_version: $crate::ffi::BEVY_VERSION.as_ptr(),
                    profile: $crate::ffi::PROFILE.as_ptr(),
                    name: __JACKDAW_GAME_NAME.as_ptr(),
                    build: __jackdaw_game_build,
                }
            }
        };
    };
}
