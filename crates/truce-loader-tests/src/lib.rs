//! Integration-test host for `truce-loader`.
//!
//! This crate has no library surface. It exists only so the tests
//! under `tests/` can depend on the umbrella `truce` crate (needed
//! by `#[derive(Params)]` / `#[derive(State)]` expansion, which
//! emits `::truce::params::*` and `::truce::core::*` paths) without
//! the back-edge into `truce-loader` becoming a `truce <->
//! truce-loader` cycle in cargo metadata.
//!
//! See `crates/truce-loader/Cargo.toml` for the rationale.
