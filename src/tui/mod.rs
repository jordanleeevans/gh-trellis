//! The interactive terminal UI: a unified stack browser that keeps the stack
//! list, layers, and layer details visible together, similar to LazyGit.
//!
//! See `ARCHITECTURE.md` for how actions, state, effects and components fit
//! together.

mod action;
mod app;
mod component;
mod components;
mod effects;
pub(crate) mod keymap;
mod messages;
pub(crate) mod state;
mod widgets;

pub use app::run;
