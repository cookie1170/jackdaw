//! Runtime discovery and loading of Jackdaw extension dylibs.
//!
//! # Overview
//!
//! Add [`DylibLoaderPlugin`] to the editor `App`. During `build` it
//! walks every configured search path, opens each dynamic library
//! with `libloading`, looks up the
//! `jackdaw_extension_entry_v1` symbol (see
//! [`jackdaw_api::ffi::ENTRY_SYMBOL`]), verifies ABI compatibility,
//! and — on success — registers the extension through the normal
//! [`jackdaw_api::register_extension`] path used for static
//! extensions.
//!
//! The plugin lives in [`LoadedDylibs`] as long as the `App` lives;
//! unloading a library while systems still reference code inside it
//! is UB, so libraries are only dropped when the `App` is destroyed.
//!
//! # Search paths
//!
//! By default the loader searches the per-user config directory
//! (`~/.config/jackdaw/extensions/` and platform equivalents). The
//! `JACKDAW_EXTENSIONS_DIR` environment variable adds another path.
//! Callers can add their own via [`DylibLoaderPlugin::extra_paths`].
//!
//! # Safety
//!
//! Loading third-party native code is inherently unsafe. The host
//! and every loaded extension must agree on ABI; the `compat`
//! module enforces the subset we can check automatically (API
//! version, Bevy version, build profile). Beyond that, extensions
//! are trusted to be well-formed: a panic in the entry function is
//! contained via `catch_unwind`, but a segfault from the extension
//! will take the process down.
//!
//! # Shared-type ABI requirement
//!
//! For an extension dylib's `register()` body to manipulate host
//! resources safely, both sides have to share one compiled copy of
//! the jackdaw types that cross the boundary (so `TypeId::of::<T>()`
//! agrees on both sides). That's what `jackdaw_api`'s
//! `dynamic_linking` feature sets up — it links
//! `jackdaw_dylib`, which is a single `.so` that bundles
//! `jackdaw_api_internal`, `jackdaw_panels`, and
//! `jackdaw_commands`. The host binary must be built with the
//! matching `jackdaw`'s `dylib` feature; otherwise this loader
//! still successfully reads the entry point and compat stamp, but
//! the extension panics as soon as it touches
//! `ExtensionContext::register_window` (or similar) because the
//! host world's `WindowRegistry` is keyed by a different `TypeId`.

mod compat;

use std::ffi::CStr;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use jackdaw_api::ffi::{ENTRY_SYMBOL, ExtensionEntry, GAME_ENTRY_SYMBOL, GameEntry};

pub use compat::CompatError;

/// Names of all games whose dylibs have been successfully loaded
/// this session. Populated at startup by the loader; consumed by
/// the editor's PIE plugin to show the "game loaded" indicator
/// and to know which game the Play button should run.
#[derive(Resource, Default, Debug, Clone)]
pub struct GameCatalog {
    pub games: Vec<String>,
}

/// Sub-directory inside the platform config directory where the
/// loader looks for per-user extensions (editor tools, panels,
/// operators).
pub const DEFAULT_EXTENSIONS_SUBDIR: &str = "jackdaw/extensions";

/// Sub-directory for per-user game dylibs. Kept separate from
/// extensions so the two don't fight over filenames and the user
/// can manage each category independently.
pub const DEFAULT_GAMES_SUBDIR: &str = "jackdaw/games";

/// Environment variable whose value, if set to a directory path,
/// is added to the loader's search paths at startup for extensions.
pub const ENV_EXTENSIONS_PATH: &str = "JACKDAW_EXTENSIONS_DIR";

/// Environment variable whose value, if set to a directory path,
/// is added to the loader's search paths at startup for games.
pub const ENV_GAMES_PATH: &str = "JACKDAW_GAMES_DIR";

/// Back-compat alias for `ENV_EXTENSIONS_PATH` — older docs and
/// scripts reference this name. Prefer the split env vars above.
#[deprecated(note = "use ENV_EXTENSIONS_PATH or ENV_GAMES_PATH")]
pub const ENV_SEARCH_PATH: &str = ENV_EXTENSIONS_PATH;

/// Keeps `libloading::Library` handles alive for the lifetime of the
/// `App`. The resource is inserted by [`DylibLoaderPlugin::build`]
/// and never drained — dropping a `Library` while systems still
/// reference its code is UB.
#[derive(Resource, Default)]
pub struct LoadedDylibs {
    libs: Vec<libloading::Library>,
}

impl LoadedDylibs {
    pub fn len(&self) -> usize {
        self.libs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.libs.is_empty()
    }
}

/// Installs the dylib extension loader.
///
/// Configuration lives on the plugin itself because loading happens
/// during `build()` (so the loader can use `&mut App` to reuse
/// `jackdaw_api::register_extension`).
pub struct DylibLoaderPlugin {
    /// Extra search paths added on top of the defaults.
    pub extra_paths: Vec<PathBuf>,
    /// If `true` (default), also search the per-user config dir.
    pub include_user_dir: bool,
    /// If `true` (default), also search
    /// `$JACKDAW_EXTENSIONS_DIR` when that env var is set.
    pub include_env_dir: bool,
}

impl Default for DylibLoaderPlugin {
    fn default() -> Self {
        Self {
            extra_paths: Vec::new(),
            include_user_dir: true,
            include_env_dir: true,
        }
    }
}

impl Plugin for DylibLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadedDylibs>();
        app.init_resource::<GameCatalog>();

        let paths = self.collect_search_paths();
        if paths.is_empty() {
            info!("Dylib loader: no search paths configured");
            return;
        }

        let mut loaded = 0u32;
        let mut failed = 0u32;
        for file in walk_dylibs(&paths) {
            match try_load(app, &file) {
                Ok(LoadedKind::Extension(name)) => {
                    info!("Loaded extension `{name}` from {}", file.display());
                    loaded += 1;
                }
                Ok(LoadedKind::Game(name)) => {
                    info!("Loaded game `{name}` from {}", file.display());
                    loaded += 1;
                }
                Err(err) => {
                    warn!("Failed to load {}: {err}", file.display());
                    failed += 1;
                }
            }
        }

        match (loaded, failed) {
            (0, 0) => info!("Dylib loader: no dylibs found"),
            _ => info!("Dylib loader: {loaded} loaded, {failed} failed"),
        }
    }
}

/// Report from a successful `try_load` / `load_from_path`.
#[derive(Debug, Clone)]
pub enum LoadedKind {
    Extension(String),
    Game(String),
}

impl LoadedKind {
    pub fn name(&self) -> &str {
        match self {
            Self::Extension(n) | Self::Game(n) => n,
        }
    }
}

/// Peek at a dylib's entry symbol to classify it as an extension or
/// a game without wiring it into the editor. The caller uses this to
/// decide where to copy the file before installing. The library
/// handle returned by the internal `open_and_verify` is dropped at
/// the end of this call, so the peeked dylib is unloaded before it
/// gets reopened from its final destination.
pub fn peek_kind(path: &Path) -> Result<LoadedKind, LoadError> {
    match open_and_verify(path)? {
        OpenedDylib::Extension { name, .. } => Ok(LoadedKind::Extension(name)),
        OpenedDylib::Game { name, .. } => Ok(LoadedKind::Game(name)),
    }
}

impl DylibLoaderPlugin {
    fn collect_search_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if self.include_user_dir {
            if let Some(config) = dirs::config_dir() {
                paths.push(config.join(DEFAULT_EXTENSIONS_SUBDIR));
                paths.push(config.join(DEFAULT_GAMES_SUBDIR));
            }
        }
        if self.include_env_dir {
            if let Ok(env_path) = std::env::var(ENV_EXTENSIONS_PATH) {
                paths.push(PathBuf::from(env_path));
            }
            if let Ok(env_path) = std::env::var(ENV_GAMES_PATH) {
                paths.push(PathBuf::from(env_path));
            }
        }
        paths.extend(self.extra_paths.iter().cloned());
        paths
    }
}

/// Everything that can go wrong loading one extension dylib. Each
/// failure is reported per-file and does not stop the loader from
/// trying the rest.
#[derive(Debug)]
pub enum LoadError {
    Libloading(libloading::Error),
    EntryPanicked,
    Compat(CompatError),
    InvalidName,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Libloading(e) => write!(f, "libloading: {e}"),
            Self::EntryPanicked => write!(f, "extension entry function panicked"),
            Self::Compat(e) => write!(f, "{e}"),
            Self::InvalidName => {
                write!(f, "extension name is not valid UTF-8 or contains NUL")
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl From<libloading::Error> for LoadError {
    fn from(value: libloading::Error) -> Self {
        Self::Libloading(value)
    }
}

impl From<CompatError> for LoadError {
    fn from(value: CompatError) -> Self {
        Self::Compat(value)
    }
}

fn walk_dylibs(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in paths {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_dylib(&path) {
                out.push(path);
            }
        }
    }
    out
}

fn is_dylib(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| matches!(ext, "so" | "dylib" | "dll"))
}

/// Result of a successfully-verified dylib open. The loader keeps
/// each variant's `Library` handle alive for the duration of the
/// `App`; dropping it while the entry code is still reachable is UB.
#[allow(improper_ctypes_definitions)]
enum OpenedDylib {
    Extension {
        lib: libloading::Library,
        name: String,
        ctor: unsafe extern "C" fn() -> Box<dyn jackdaw_api::JackdawExtension>,
    },
    Game {
        lib: libloading::Library,
        name: String,
        build: unsafe extern "C" fn(*mut bevy::app::App),
    },
}

/// Try to open `path`, dispatching on which entry symbol it
/// exposes. Game symbol wins if both somehow exist (they shouldn't
/// — a cdylib should only `export_game!` or `export_extension!`,
/// not both).
#[allow(improper_ctypes_definitions)]
fn open_and_verify(path: &Path) -> Result<OpenedDylib, LoadError> {
    // SAFETY: libloading's standard contract — the caller trusts
    // that `path` is a well-formed dynamic library. If not, the
    // call returns `Err`. Extensions / games are trusted native
    // code; the loader does not sandbox them.
    let lib = unsafe { libloading::Library::new(path)? };

    // Try the game symbol first. If it's present, the dylib is a
    // game and we take that path. If absent (most dylibs), fall
    // through to the extension symbol.
    //
    // `lib.get` returns `Err` for missing symbols rather than
    // panicking, so this is safe to try speculatively.
    type GameEntryFn = unsafe extern "C" fn() -> GameEntry;
    if let Ok(game_sym) = unsafe { lib.get::<GameEntryFn>(GAME_ENTRY_SYMBOL) } {
        let entry = std::panic::catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: game_sym is guaranteed non-null by libloading's
            // successful lookup; calling convention matches the
            // declared prototype.
            unsafe { game_sym() }
        }))
        .map_err(|_| LoadError::EntryPanicked)?;

        compat::verify_game_compat(&entry)?;

        // SAFETY: `verify_game_compat` rejected null; the library
        // stays alive at least until `lib` is dropped by the caller.
        let name = unsafe { CStr::from_ptr(entry.name) }
            .to_str()
            .map_err(|_| LoadError::InvalidName)?
            .to_owned();

        return Ok(OpenedDylib::Game {
            lib,
            name,
            build: entry.build,
        });
    }

    // SAFETY: the entry symbol has the signature declared by
    // `jackdaw_api::ffi::ExtensionEntry`. Calling it is isolated
    // inside `catch_unwind` below.
    type EntryFn = unsafe extern "C" fn() -> ExtensionEntry;
    let entry_sym: libloading::Symbol<EntryFn> = unsafe { lib.get(ENTRY_SYMBOL)? };

    let entry = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: entry_sym is guaranteed non-null by libloading's
        // successful lookup; calling convention matches the
        // declared prototype.
        unsafe { entry_sym() }
    }))
    .map_err(|_| LoadError::EntryPanicked)?;

    compat::verify_compat(&entry)?;

    // SAFETY: `verify_compat` rejected null; the library stays
    // alive at least until `lib` is dropped by the caller.
    let name_cstr = unsafe { CStr::from_ptr(entry.name) };
    let name = name_cstr
        .to_str()
        .map_err(|_| LoadError::InvalidName)?
        .to_owned();

    Ok(OpenedDylib::Extension {
        lib,
        name,
        ctor: entry.ctor,
    })
}

fn try_load(app: &mut App, path: &Path) -> Result<LoadedKind, LoadError> {
    match open_and_verify(path)? {
        OpenedDylib::Extension { lib, name, ctor } => {
            jackdaw_api::register_extension(app, &name, move || {
                // SAFETY: the dylib stays loaded because its
                // Library handle is kept in LoadedDylibs;
                // `verify_compat` asserted the ABI contract at
                // load time, and the ctor pointer itself is just a
                // plain function pointer.
                unsafe { ctor() }
            });

            app.world_mut()
                .resource_mut::<LoadedDylibs>()
                .libs
                .push(lib);

            Ok(LoadedKind::Extension(name))
        }
        OpenedDylib::Game { lib, name, build } => {
            // Run the game's build fn against the editor's App,
            // isolated from editor panics via `catch_unwind`. The
            // build fn typically calls `app.add_plugins(GamePlugin)`
            // and registers systems gated on `PlayState::Playing`.
            let app_ptr: *mut bevy::app::App = app as *mut _;
            let build_result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                // SAFETY: `build` is a function pointer from a
                // compat-verified dylib; `app_ptr` is a valid
                // mutable reference for the duration of this call;
                // the dylib stays alive via LoadedDylibs.
                unsafe { build(app_ptr) }
            }));
            if build_result.is_err() {
                // Build panicked. The library is still loaded — we
                // keep it anyway rather than risk UB unloading
                // partially-executed init code.
                app.world_mut()
                    .resource_mut::<LoadedDylibs>()
                    .libs
                    .push(lib);
                return Err(LoadError::EntryPanicked);
            }

            app.world_mut()
                .resource_mut::<GameCatalog>()
                .games
                .push(name.clone());
            app.world_mut()
                .resource_mut::<LoadedDylibs>()
                .libs
                .push(lib);

            Ok(LoadedKind::Game(name))
        }
    }
}

/// Load a dylib at runtime from a `&mut World` context.
///
/// Requires the host binary to have been built with `jackdaw`'s
/// `dylib` feature (which pulls in `jackdaw_api/dynamic_linking`)
/// so both sides share one compiled copy of the jackdaw types.
/// Without that, `ExtensionContext::register_window` and similar
/// calls panic because the host keyed resources under different
/// `TypeId`s than the dylib sees.
///
/// Mirrors the startup loader path but skips the BEI input-context
/// registration that requires `&mut App`. In practice that means:
///
/// * Windows, operators, menu entries, and panel-extension sections
///   activate immediately.
/// * BEI keybinds declared via `add_input_context::<C>()` do **not**
///   activate until the editor restarts and picks the dylib up
///   through the normal [`DylibLoaderPlugin`] startup path.
///
/// The constructor is inserted into [`jackdaw_api::ExtensionCatalog`]
/// so the Extensions dialog's enable/disable toggle can reuse it, and
/// the `Library` handle is moved into [`LoadedDylibs`] so the entry
/// point stays valid for the rest of the app's life.
///
/// Returns the loaded kind (Extension or Game) on success.
pub fn load_from_path(world: &mut World, path: &Path) -> Result<LoadedKind, LoadError> {
    match open_and_verify(path)? {
        OpenedDylib::Extension { lib, name, ctor } => {
            // Read the extension's declared kind from a throwaway
            // instance so the catalog can classify it in the
            // Extensions dialog.
            let sample = unsafe { ctor() };
            let kind = sample.kind();
            drop(sample);

            world
                .resource_mut::<jackdaw_api::ExtensionCatalog>()
                .register(&name, kind, move || unsafe { ctor() });

            // Spawn the extension entity and run its `register()`
            // body so windows/operators/menu entries activate
            // immediately. BEI context registration is
            // intentionally skipped here — it needs `&mut App`
            // which we don't have from a world-only call site.
            let extension = unsafe { ctor() };
            jackdaw_api::load_static_extension(world, extension);

            world.resource_mut::<LoadedDylibs>().libs.push(lib);

            Ok(LoadedKind::Extension(name))
        }
        OpenedDylib::Game {
            lib,
            name,
            build: _,
        } => {
            // Games need `&mut App` at load time to install their
            // systems, which isn't available from a world-only
            // context. Keep the library alive and record the name
            // so a follow-up v2 can wake the game up, but for now
            // tell the user to restart.
            world.resource_mut::<LoadedDylibs>().libs.push(lib);
            world.resource_mut::<GameCatalog>().games.push(name.clone());
            warn!(
                "Game dylib `{name}` cannot be activated at runtime (requires startup \
                 context). Restart jackdaw to activate it."
            );
            Ok(LoadedKind::Game(name))
        }
    }
}
