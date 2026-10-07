//! Telamon Installer, Rust side. `cpp/` only starts Qt and loads the QML;
//! everything QML talks to is the `Backend` QObject in `backend.rs`.

// Install takes everything the helper does, as QML hands it over.
#![allow(clippy::too_many_arguments)]

mod backend;
mod demo;
mod helper;
mod network;
mod system;
mod view;

use std::ffi::c_void;

/// Called once from `main.cpp`. Returns the `Backend` QObject (no parent;
/// the caller owns it).
#[unsafe(no_mangle)]
pub extern "C" fn telamon_backend_new() -> *mut c_void {
    backend::qobject::backend_make_unique().into_raw().cast()
}
