# Contributing to Jackdaw

Thank you for your interest in contributing to Jackdaw! This document covers the basics of getting set up and submitting changes.

## Development Setup

### Prerequisites

- **Rust nightly toolchain** - Jackdaw uses edition 2024 features
  ```sh
  rustup toolchain install nightly
  rustup default nightly
  ```
- **System dependencies** - GPU drivers with Vulkan support (or Metal on macOS)
- **Linux extras** - `libudev-dev`, `libasound2-dev`, `libwayland-dev` (or equivalent for your distro)

### Clone and Build

```sh
git clone https://github.com/jbuehler23/jackdaw.git
cd jackdaw
cargo build
```

### Running

```sh
# Run the basic example
cargo run --example basic

# Working on extension loading? Build with the dylib feature so the
# dylib loader is exercised end-to-end (editor binary links against
# the shared libbevy_dylib + libjackdaw_dylib). First build is slow
# because Bevy and the workspace's shared types recompile as
# dylibs; subsequent incremental builds are fast.
cargo run --features dylib
```

## Checks

Before submitting a PR, make sure the following pass:

```sh
# Format
cargo fmt --all --check

# Lint
cargo clippy --workspace -- -D warnings

# Tests
cargo test --workspace

# Doc build
cargo doc --workspace --no-deps
```

## Pull Requests

1. Fork the repository and create a feature branch from `main`
2. Keep changes focused if possible, but chat to me on discord if you want more overarching changes!
3. Make sure all checks above pass
4. Open a PR against `main` with a clear description of what changed and why

## License

By contributing, you agree that your contributions will be licensed under the project's dual MIT/Apache-2.0 license.
