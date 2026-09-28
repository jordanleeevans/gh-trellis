//! Stateless render helpers shared by components.

mod layout;
mod panel;
mod spinner;

pub(crate) use layout::centered_rect;
pub(crate) use panel::panel_block;
pub(crate) use spinner::spinner_frame;

use crate::theme::glyphs::GlyphSet;

/// The glyph set chosen by the `nerd_fonts` config option.
pub(crate) fn glyphs() -> &'static GlyphSet {
    crate::theme::glyphs::current()
}
