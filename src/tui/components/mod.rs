//! One module per panel or overlay. Each owns its own view state and either
//! implements [`super::component::Component`] or is rendered by one that does.

pub(crate) mod confirm;
pub(crate) mod stack_browser;
