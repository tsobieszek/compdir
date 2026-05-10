# Repository Guidelines

## Project Structure & Module Organization
- `Cargo.toml` defines the `compdir` binary crate.
- `src/main.rs` contains the CLI, argument parsing, SSH/path resolution, directory scanning, and unit tests.
- `README.md` summarizes the current behavior and key code locations.
- `DONE` details the behavior.
- `target/` is build output and should not be edited manually.

## Build, Test, and Development Commands
- `cargo build` compiles the binary locally.
- `cargo test` runs the unit tests in `src/main.rs`.
- `cargo fmt` formats the Rust source before submission.
- `cargo run -- --help` prints the CLI synopsis.
- `cargo run -- <right>` or `cargo run -- <left> <right>` runs the tool without installing it.

## Coding Style & Naming Conventions
- Use Rust 2021 edition conventions and keep formatting `rustfmt`-clean.
- Prefer clear, descriptive names for path-handling types and helper functions.
- Keep the existing style of small, pure helper functions for parsing, resolution, scanning, and rendering.
- Use `BTreeSet`/`BTreeMap` when deterministic ordering matters.

## Testing Guidelines
- Add focused unit tests alongside the implementation in `src/main.rs` under `#[cfg(test)]`.
- Name tests by behavior, for example `renders_expected_grouping`.
- Cover parsing edge cases, path normalization, SSH command construction, and output ordering.
- Run `cargo test` after any change that touches comparison logic or CLI behavior.

## Commit & Pull Request Guidelines
- This repository currently has no commit history, so there is no established commit-message convention yet.
- Use short, imperative commit subjects such as `Add ssh path normalization`.
- PRs should include a summary of behavior changes, test coverage, and any SSH/path assumptions.
- If output format changes, include an example before/after block in the PR description.

## Security & Configuration Tips
- Treat host arguments as shell-sensitive input; keep quoting rules conservative.
- The tool shells out to `ssh`, so it depends on the local SSH client and reachable hosts.
- Avoid committing secrets, private hostnames, or environment-specific paths in tests.
