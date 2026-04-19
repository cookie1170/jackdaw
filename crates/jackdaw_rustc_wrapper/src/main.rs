//! Thin rustc wrapper for jackdaw extension/game projects.
//!
//! # What it does
//!
//! Cargo invokes this binary as `RUSTC_WRAPPER`, meaning every rustc
//! call in the project passes through here first. For the user's own
//! crate — detected via `CARGO_PRIMARY_PACKAGE=1` — we rewrite the
//! rustc argv so that:
//!
//! * `--extern bevy=<anything>` is replaced with
//!   `--extern bevy=$JACKDAW_SDK_DYLIB`. The user's Cargo.toml
//!   declares `bevy = "0.18"` so that bevy's proc macros read it
//!   via `CARGO_MANIFEST_DIR` and emit correct `::bevy::…` paths.
//!   Cargo compiles real bevy into the user's target dir; the
//!   resulting rlib is ignored because the wrapper redirects the
//!   `--extern` flag to `libjackdaw_sdk.so`. The wasted compile is
//!   a one-time cost; it keeps the user's Cargo.toml completely
//!   normal (no `[patch.crates-io]` tricks, no stub crate).
//! * `--extern jackdaw_api=$JACKDAW_SDK_DYLIB` is appended
//!   unconditionally. The user never declares `jackdaw_api` in
//!   Cargo.toml — the wrapper injects it so `use jackdaw_api::…`
//!   just works.
//! * `-L dependency=$JACKDAW_SDK_DEPS` is appended so rustc can find
//!   transitive rlib metadata when resolving re-exported types.
//! * `-C prefer-dynamic` is appended so rustc links against the SDK
//!   dylib rather than statically embedding its (non-existent at the
//!   stub) rlib form.
//!
//! For every other rustc invocation (compiling the tiny `bevy` stub,
//! compiling build scripts, etc.) we pass argv through untouched.
//!
//! # Why
//!
//! Cargo's `-Cmetadata` hash is not deterministic across independent
//! workspaces, so the "build bevy twice and hope the hashes line up"
//! approach doesn't work. By forcing the user crate to link against
//! the one `libjackdaw_sdk.so` that ships alongside the editor, every
//! `TypeId::of::<T>()` call site in the user's code uses the same
//! crate hash as the editor, and reflection / dlopen works.
//!
//! # Env vars the wrapper reads
//!
//! | Var                   | Required | Purpose                            |
//! |-----------------------|----------|------------------------------------|
//! | `JACKDAW_SDK_DYLIB`   | yes      | Absolute path to `libjackdaw_sdk.so` |
//! | `JACKDAW_SDK_DEPS`    | yes      | Absolute path to the `deps/` dir   |
//! | `JACKDAW_WRAPPER_LOG` | no       | If `1`, log rewrites to stderr     |
//! | `CARGO_PRIMARY_PACKAGE` | (set by cargo) | `1` while compiling the user crate |

use std::env;
use std::ffi::OsString;
use std::process::{Command, ExitCode};

const ENV_SDK_DYLIB: &str = "JACKDAW_SDK_DYLIB";
const ENV_SDK_DEPS: &str = "JACKDAW_SDK_DEPS";
const ENV_PRIMARY_PACKAGE: &str = "CARGO_PRIMARY_PACKAGE";
const ENV_LOG: &str = "JACKDAW_WRAPPER_LOG";

/// Crate aliases we redirect to `libjackdaw_sdk.so` whenever cargo
/// emits an `--extern` flag for them. User code writes
/// `use bevy::prelude::*;` and cargo passes `--extern bevy=<stub>.rlib`
/// to rustc; we rewrite the value here.
const REDIRECTED_CRATES: &[&str] = &["bevy"];

/// Crate aliases we inject unconditionally so `use jackdaw_api::…`
/// resolves without the user having to declare `jackdaw_api` in
/// their Cargo.toml. The rustc command picks up these `--extern`
/// flags exactly as cargo-emitted ones would be.
const INJECTED_CRATES: &[&str] = &["jackdaw_api"];

fn main() -> ExitCode {
    let mut argv: Vec<OsString> = env::args_os().collect();
    // argv[0] is our binary; argv[1] is the real rustc path; argv[2..]
    // are rustc's args.
    if argv.len() < 2 {
        eprintln!("jackdaw-rustc-wrapper: no rustc path provided");
        return ExitCode::from(1);
    }
    let rustc = argv.remove(1);
    let mut rustc_args: Vec<OsString> = argv.split_off(1);

    let is_primary = env::var_os(ENV_PRIMARY_PACKAGE).is_some_and(|v| v == "1");
    let log = env::var_os(ENV_LOG).is_some_and(|v| v == "1");

    if is_primary {
        if let Err(e) = rewrite_primary_args(&mut rustc_args, log) {
            eprintln!("jackdaw-rustc-wrapper: {e}");
            return ExitCode::from(1);
        }
    }

    let status = Command::new(&rustc).args(&rustc_args).status();

    match status {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("jackdaw-rustc-wrapper: failed to spawn {rustc:?}: {e}");
            ExitCode::from(1)
        }
    }
}

/// Rewrite the rustc argv for the user's primary-package compile.
/// Redirects `--extern bevy=...` and `--extern jackdaw_api=...` to
/// the SDK dylib, appends a `-L dependency=$JACKDAW_SDK_DEPS` so
/// rustc can find transitive rlib metadata, and adds
/// `-C prefer-dynamic` so the linker prefers the dylib form.
fn rewrite_primary_args(argv: &mut Vec<OsString>, log: bool) -> Result<(), String> {
    let dylib = env::var_os(ENV_SDK_DYLIB)
        .ok_or_else(|| format!("{ENV_SDK_DYLIB} not set; cannot redirect --extern"))?;
    let deps = env::var_os(ENV_SDK_DEPS)
        .ok_or_else(|| format!("{ENV_SDK_DEPS} not set; cannot point -L at deps/"))?;

    let mut i = 0;
    while i < argv.len() {
        if argv[i] == "--extern" && i + 1 < argv.len() {
            if let Some(new_value) = rewrite_extern(&argv[i + 1], &dylib) {
                if log {
                    eprintln!(
                        "jackdaw-rustc-wrapper: rewrite --extern {:?} -> {:?}",
                        argv[i + 1], new_value
                    );
                }
                argv[i + 1] = new_value;
            }
            i += 2;
            continue;
        }
        i += 1;
    }

    for alias in INJECTED_CRATES {
        let mut flag = OsString::from(alias);
        flag.push("=");
        flag.push(&dylib);
        argv.push(OsString::from("--extern"));
        argv.push(flag);
        if log {
            eprintln!(
                "jackdaw-rustc-wrapper: injected --extern {}={}",
                alias,
                dylib.to_string_lossy()
            );
        }
    }

    let mut deps_flag = OsString::from("dependency=");
    deps_flag.push(&deps);
    argv.push(OsString::from("-L"));
    argv.push(deps_flag);
    argv.push(OsString::from("-C"));
    argv.push(OsString::from("prefer-dynamic"));

    if log {
        eprintln!(
            "jackdaw-rustc-wrapper: appended -L dependency={} -C prefer-dynamic",
            deps.to_string_lossy()
        );
    }

    Ok(())
}

/// If `value` is `<alias>=<path>` with `<alias>` in
/// [`REDIRECTED_CRATES`], return the redirected form pointing at the
/// SDK dylib. Otherwise return `None` so the caller leaves it alone.
fn rewrite_extern(value: &OsString, sdk_dylib: &OsString) -> Option<OsString> {
    let s = value.to_str()?;
    let (alias, _rest) = s.split_once('=')?;
    if !REDIRECTED_CRATES.contains(&alias) {
        return None;
    }
    let mut out = OsString::from(alias);
    out.push("=");
    out.push(sdk_dylib);
    Some(out)
}
