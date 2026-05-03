//! Minimal `home_dir()` shim. We keep this in-tree to avoid pulling in
//! the `dirs` crate just for one lookup.
//!
//! Lookup order:
//! - Unix: `$HOME` (every login shell sets this).
//! - Windows: `%USERPROFILE%` first, falling back to `%HOME%` (some
//!   MSYS / Git Bash setups export `HOME` instead of `USERPROFILE`,
//!   so honoring both keeps `cargo truce` working in both shells
//!   without a `dirs` dependency).
//!
//! Returns `None` only when no usable env var is set; callers that
//! need a hard requirement (e.g. CLAP user-scope install) should
//! propagate the `None` as an error instead of `unwrap()`-ing.

use std::path::PathBuf;

pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// Hard-required form of [`home_dir`]. Returns a typed error so the
/// surrounding command can print one line ("can't determine home
/// directory: set HOME / USERPROFILE") instead of panicking on the
/// `Option::unwrap` the audit flagged across `cmd_status`,
/// `cmd_remove`, install paths, and `cmd_reset_au`.
//
// Gated to macOS: every caller (`cmd_status`, `cmd_reset_au` macOS impl)
// is itself macOS-gated, so on Linux/Windows this would be dead code.
#[cfg(target_os = "macos")]
pub(crate) fn require_home_dir() -> Result<PathBuf, crate::BoxErr> {
    home_dir().ok_or_else(|| -> crate::BoxErr {
        if cfg!(windows) {
            "can't determine home directory: neither USERPROFILE nor HOME is set".into()
        } else {
            "can't determine home directory: HOME is not set".into()
        }
    })
}
