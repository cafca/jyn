//! The Flutter-facing API surface. Everything the app can call lives here;
//! `flutter_rust_bridge_codegen generate` scans this module.
//!
//! Shape: commands are async functions that resolve (or throw) with their
//! outcome; state flows the other way as [`crate::runtime::JynEvent`]
//! snapshots through the stream installed by [`lifecycle::events`].

pub mod commands;
pub mod lifecycle;
pub mod media;
pub mod settings;
