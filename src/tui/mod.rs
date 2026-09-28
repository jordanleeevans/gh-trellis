//! The interactive terminal UI: a unified stack browser that keeps the stack
//! list, layers, and layer details visible together, similar to LazyGit.

mod app;
mod confirm;
mod keymap;
mod layer_resource;
mod panel;
mod stack_layers;
mod submit_progress;

pub use app::run;
