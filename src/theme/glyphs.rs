pub const NERD_FONT: GlyphSet = GlyphSet {
    branch: "",
    current: "●",
    commit: "",

    pull_request: "",
    pull_request_open: "",
    pull_request_merged: "",
    pull_request_closed: "",

    check: "",
    cross: "",
    warning: "",
    pending: "",
    running: "",

    github: "",
    open_external: "",
    folder: "",
    folder_open: "",
    file: "󰈔",

    up: "",
    down: "",
};

pub const ASCII: GlyphSet = GlyphSet {
    branch: "*",
    current: ">",
    commit: "o",

    pull_request: "PR",
    pull_request_open: "PR",
    pull_request_merged: "M",
    pull_request_closed: "X",

    check: "[+]",
    cross: "[x]",
    warning: "[!]",
    pending: "[~]",
    running: "[~]",

    github: "GH",
    open_external: "->",
    folder: "[+]",
    folder_open: "[-]",
    file: "[F]",

    up: "^",
    down: "v",
};

#[derive(Debug, Clone, Copy)]
#[expect(
    dead_code,
    reason = "a complete icon set, kept in step across the Nerd Font and ASCII \
              variants; some icons are for panels not built yet (#24-#29)"
)]
pub struct GlyphSet {
    pub branch: &'static str,
    pub current: &'static str,
    pub commit: &'static str,

    pub pull_request: &'static str,
    pub pull_request_open: &'static str,
    pub pull_request_merged: &'static str,
    pub pull_request_closed: &'static str,

    pub check: &'static str,
    pub cross: &'static str,
    pub warning: &'static str,
    pub pending: &'static str,
    pub running: &'static str,

    pub github: &'static str,
    pub open_external: &'static str,
    pub folder: &'static str,
    pub folder_open: &'static str,
    pub file: &'static str,

    pub up: &'static str,
    pub down: &'static str,
}

static NERD_FONTS_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Choose the glyph set returned by [`current`] (driven by config `nerd_fonts`).
pub fn set_nerd_fonts(enabled: bool) {
    NERD_FONTS_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// The active glyph set.
pub fn current() -> &'static GlyphSet {
    if NERD_FONTS_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        &NERD_FONT
    } else {
        &ASCII
    }
}
