//! Map model, Commands and persistence.
//!
//! This crate must stay free of UI and GPU code so a Map can be driven
//! headlessly (ADR 0001). CI enforces that it never depends on egui or wgpu.
