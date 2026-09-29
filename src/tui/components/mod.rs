//! One module per panel or overlay. Each owns its own view state and either
//! implements [`super::component::Component`] or is rendered by one that does.

pub(crate) mod add_layer_prompt;
pub(crate) mod confirm;
pub(crate) mod help;
pub(crate) mod merge_confirm;
pub(crate) mod stack_browser;
pub(crate) mod submit_progress;
pub(crate) mod sync_confirm;
