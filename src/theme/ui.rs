use ratatui::style::{Color, Modifier, Style};

use crate::theme::pallete::PastelPallette;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub colors: Colors,
    pub text: TextStyles,
}

#[derive(Debug, Clone, Copy)]
#[expect(dead_code, reason = "part of the palette for theming (#33)")]
pub struct Colors {
    pub primary: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub surface: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_inverse: Color,
    pub success: Color,
    pub danger: Color,
    pub warning: Color,
    pub link: Color,
}

#[derive(Debug, Clone, Copy)]
pub struct TextStyles {
    pub body: Style,
    pub muted: Style,
    pub heading: Style,
    pub label: Style,
    pub key: Style,
    pub active_title: Style,
    pub inactive_title: Style,
    pub selected: Style,
}

pub const THEME: Theme = Theme {
    colors: Colors {
        primary: PastelPallette::CYAN,
        secondary: PastelPallette::MAGENTA,
        tertiary: PastelPallette::BLUE,
        surface: PastelPallette::SURFACE,
        text: PastelPallette::TEXT,
        text_muted: PastelPallette::TEXT_MUTED,
        text_inverse: PastelPallette::TEXT_INVERSE,
        success: PastelPallette::GREEN,
        danger: PastelPallette::RED,
        warning: PastelPallette::YELLOW,
        link: PastelPallette::LINK,
    },
    text: TextStyles {
        body: Style::new().fg(PastelPallette::TEXT),
        muted: Style::new().fg(PastelPallette::TEXT_MUTED),

        heading: Style::new()
            .fg(PastelPallette::CYAN)
            .add_modifier(Modifier::BOLD),

        label: Style::new()
            .fg(PastelPallette::TEXT_MUTED)
            .add_modifier(Modifier::BOLD),

        key: Style::new()
            .fg(PastelPallette::CYAN)
            .add_modifier(Modifier::BOLD),

        active_title: Style::new()
            .fg(PastelPallette::TEXT_INVERSE)
            .bg(PastelPallette::MAGENTA)
            .add_modifier(Modifier::BOLD),

        inactive_title: Style::new()
            .fg(PastelPallette::TEXT)
            .add_modifier(Modifier::BOLD),

        selected: Style::new()
            .fg(PastelPallette::TEXT_INVERSE)
            .bg(PastelPallette::MAGENTA)
            .add_modifier(Modifier::BOLD),
    },
};

impl Theme {
    pub fn border(self, active: bool) -> Style {
        Style::new().fg(if active {
            self.colors.secondary
        } else {
            self.colors.text_muted
        })
    }

    pub fn primary_border(self) -> Style {
        Style::new().fg(self.colors.primary)
    }

    #[expect(dead_code, reason = "part of the palette for theming (#33)")]
    pub fn tertiary_border(self) -> Style {
        Style::new().fg(self.colors.tertiary)
    }
}
