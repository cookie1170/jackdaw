//! Editor-driven build pipeline for extension and game projects.
//!
//! User-scaffolded projects are plain single-crate cargo projects
//! with `bevy = "0.18"` in `[dependencies]` and `crate-type =
//! ["cdylib"]` on the library. Jackdaw compiles them via `cargo
//! build` with `RUSTC_WRAPPER` pointing at `jackdaw-rustc-wrapper`,
//! which intercepts rustc and rewrites `--extern bevy=<user>.rlib`
//! to `--extern bevy=libjackdaw_sdk.so`. That keeps the user's
//! cdylib TypeIds in sync with the editor.
//!
//! Why not `bevy build`? The bevy CLI's build subcommand requires
//! a binary target and errors on library-only projects ("No
//! binaries available!"). Scaffolded jackdaw projects are cdylibs
//! so the editor can `dlopen` them, so `bevy build` can't drive
//! them. We still use `bevy new` for scaffolding — that part of
//! the toolchain fits cleanly. If bevy CLI later grows library
//! support we'll switch the build path too.
//!
//! [`build_extension_project`] is the entry point. Call it from the
//! Extensions dialog's "Build and Install…" button, from the
//! scaffold-new-project flow after `bevy new` completes, or from
//! the `--build-ext <path>` CLI flag when it's wired up.
//!
//! The function blocks until the subprocess exits; do not call it
//! from a Bevy system. Use a task pool if you need non-blocking
//! builds.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::sdk_paths::SdkPaths;

/// Everything that can go wrong while building an extension/game
/// project.
#[derive(Debug)]
pub enum BuildError {
    NotADirectory(PathBuf),
    MissingCargoToml(PathBuf),
    SdkNotFound {
        expected_path: PathBuf,
        hint: &'static str,
    },
    WrapperNotFound {
        expected_path: PathBuf,
        hint: &'static str,
    },
    BuildSpawn(std::io::Error),
    BuildFailed {
        status: std::process::ExitStatus,
        stdout: String,
        stderr: String,
    },
    OutputNotProduced {
        expected: PathBuf,
    },
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotADirectory(p) => write!(f, "{} is not a directory", p.display()),
            Self::MissingCargoToml(p) => {
                write!(f, "{} has no Cargo.toml", p.display())
            }
            Self::SdkNotFound {
                expected_path,
                hint,
            } => write!(
                f,
                "SDK dylib not found at {}. {}",
                expected_path.display(),
                hint
            ),
            Self::WrapperNotFound {
                expected_path,
                hint,
            } => write!(
                f,
                "rustc wrapper not found at {}. {}",
                expected_path.display(),
                hint
            ),
            Self::BuildSpawn(e) => write!(f, "failed to spawn cargo: {e}"),
            Self::BuildFailed { status, stderr, .. } => {
                write!(f, "cargo exited with {status}\n{stderr}")
            }
            Self::OutputNotProduced { expected } => write!(
                f,
                "cargo succeeded but no .so was produced at {}",
                expected.display()
            ),
        }
    }
}

impl std::error::Error for BuildError {}

/// Discover `libjackdaw_sdk` + `jackdaw-rustc-wrapper` on disk, or
/// surface a typed error the Build-and-Install dialog can translate
/// into a user-actionable message.
fn discover_sdk() -> Result<SdkPaths, BuildError> {
    let paths = SdkPaths::compute();
    if !paths.dylib_exists() {
        return Err(BuildError::SdkNotFound {
            expected_path: paths.dylib,
            hint: "Rebuild the editor with `--features dylib` so \
                   libjackdaw_sdk is emitted, or set JACKDAW_SDK_DIR \
                   to the directory that contains it.",
        });
    }
    if !paths.wrapper_exists() {
        return Err(BuildError::WrapperNotFound {
            expected_path: paths.wrapper,
            hint: "Run `cargo build -p jackdaw_rustc_wrapper` so the \
                   wrapper binary is emitted next to the editor.",
        });
    }
    Ok(paths)
}

/// Build the extension or game project rooted at `project_dir`.
///
/// The project is expected to be a single-crate cargo project with
/// its own `[workspace]` marker, `crate-type = ["cdylib"]` on the
/// library, and `bevy = "0.18"` declared under `[dependencies]`. No
/// `[patch.crates-io]` is required — the wrapper redirects the
/// `--extern bevy=` flag regardless of what cargo resolves bevy to.
///
/// Returns the absolute path to the produced `.so` / `.dylib` /
/// `.dll` on success.
pub fn build_extension_project(project_dir: &Path) -> Result<PathBuf, BuildError> {
    let project_dir = project_dir
        .canonicalize()
        .map_err(|_| BuildError::NotADirectory(project_dir.to_path_buf()))?;

    if !project_dir.is_dir() {
        return Err(BuildError::NotADirectory(project_dir));
    }
    let manifest = project_dir.join("Cargo.toml");
    if !manifest.is_file() {
        return Err(BuildError::MissingCargoToml(project_dir));
    }

    let sdk = discover_sdk()?;

    let mut cmd = Command::new("cargo");
    cmd.current_dir(&project_dir);
    cmd.args([
        "build",
        "--manifest-path",
        manifest
            .to_str()
            .expect("Cargo.toml path must be valid UTF-8"),
    ]);
    // The wrapper reads these to redirect --extern bevy / inject
    // --extern jackdaw_api. bevy CLI passes env through to cargo,
    // which passes it through to every rustc invocation.
    cmd.env("RUSTC_WRAPPER", &sdk.wrapper);
    cmd.env("JACKDAW_SDK_DYLIB", &sdk.dylib);
    cmd.env("JACKDAW_SDK_DEPS", &sdk.deps);

    let output = cmd.output().map_err(BuildError::BuildSpawn)?;
    if !output.status.success() {
        return Err(BuildError::BuildFailed {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let artifact_name = artifact_file_name(&project_dir);
    let artifact = project_dir.join("target/debug").join(&artifact_name);
    if !artifact.is_file() {
        return Err(BuildError::OutputNotProduced { expected: artifact });
    }
    Ok(artifact)
}

/// Derive the expected cdylib filename from the project's package
/// name. Falls back to `libunnamed.<ext>` if the manifest doesn't
/// declare a name (which cargo would have rejected anyway, but it
/// keeps this helper infallible).
fn artifact_file_name(project_dir: &Path) -> String {
    let package_name = std::fs::read_to_string(project_dir.join("Cargo.toml"))
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("name")
                    .and_then(|rest| rest.trim().strip_prefix('='))
                    .map(|rest| rest.trim().trim_matches('"').trim_matches('\'').to_owned())
            })
        })
        .unwrap_or_else(|| "unnamed".to_string());

    if cfg!(target_os = "windows") {
        format!("{package_name}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{package_name}.dylib")
    } else {
        format!("lib{package_name}.so")
    }
}
