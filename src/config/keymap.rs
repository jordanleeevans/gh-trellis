//! Key-string parsing for the `[keybindings]` table.

use crossterm::event::{KeyCode, KeyModifiers};
use serde::Deserialize;

/// A single key press description: a key code plus required modifiers.
///
/// Modifiers are matched as "contains", so a binding without modifiers still
/// matches a press that carries extra ones (terminals report `G` with SHIFT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBinding {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    pub fn matches(&self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        self.code == code && modifiers.contains(self.modifiers)
    }
}

/// Formats a binding the way it's written in `config.toml`, so
/// `parse_key(&binding.to_string())` gives the binding back.
impl std::fmt::Display for KeyBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (modifier, name) in [
            (KeyModifiers::CONTROL, "ctrl+"),
            (KeyModifiers::ALT, "alt+"),
            (KeyModifiers::SHIFT, "shift+"),
        ] {
            if self.modifiers.contains(modifier) {
                f.write_str(name)?;
            }
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("space"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::Enter => f.write_str("enter"),
            KeyCode::Esc => f.write_str("esc"),
            KeyCode::Tab => f.write_str("tab"),
            KeyCode::BackTab => f.write_str("shift+tab"),
            KeyCode::Backspace => f.write_str("backspace"),
            KeyCode::Delete => f.write_str("delete"),
            KeyCode::Up => f.write_str("up"),
            KeyCode::Down => f.write_str("down"),
            KeyCode::Left => f.write_str("left"),
            KeyCode::Right => f.write_str("right"),
            KeyCode::Home => f.write_str("home"),
            KeyCode::End => f.write_str("end"),
            KeyCode::PageUp => f.write_str("pageup"),
            KeyCode::PageDown => f.write_str("pagedown"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// One key string or a list of them, as written in the config file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum KeySpec {
    One(String),
    Many(Vec<String>),
}

impl KeySpec {
    pub fn parse(&self) -> Result<Vec<KeyBinding>, String> {
        match self {
            KeySpec::One(s) => Ok(vec![parse_key(s)?]),
            KeySpec::Many(list) => list.iter().map(|s| parse_key(s)).collect(),
        }
    }
}

/// Parse strings such as `q`, `ctrl+d`, `enter`, `shift+tab`, `?`, `space`.
pub fn parse_key(input: &str) -> Result<KeyBinding, String> {
    let bad = |why: &str| format!("invalid key {input:?}: {why}");
    if input.is_empty() {
        return Err(bad("empty string"));
    }
    // The plus key itself may appear as "+" or as the last part of "ctrl++".
    let (mods_part, key_part) = if input == "+" {
        ("", "+")
    } else if let Some(prefix) = input.strip_suffix("++") {
        (prefix, "+")
    } else {
        match input.rfind('+') {
            Some(i) => (&input[..i], &input[i + 1..]),
            None => ("", input),
        }
    };

    let mut modifiers = KeyModifiers::NONE;
    if !mods_part.is_empty() {
        for m in mods_part.split('+') {
            modifiers |= match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => KeyModifiers::CONTROL,
                "alt" | "meta" => KeyModifiers::ALT,
                "shift" => KeyModifiers::SHIFT,
                other => return Err(bad(&format!("unknown modifier {other:?}"))),
            };
        }
    }

    let code = match key_part.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        _ => {
            let mut chars = key_part.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return Err(bad(&format!("unknown key {key_part:?}"))),
            }
        }
    };

    // shift+tab is reported by terminals as BackTab.
    if code == KeyCode::Tab && modifiers == KeyModifiers::SHIFT {
        return Ok(KeyBinding::new(KeyCode::BackTab, KeyModifiers::NONE));
    }
    // Shifted letters are represented by the uppercase character itself.
    if let KeyCode::Char(c) = code
        && modifiers.contains(KeyModifiers::SHIFT)
        && c.is_ascii_alphabetic()
    {
        modifiers.remove(KeyModifiers::SHIFT);
        return Ok(KeyBinding::new(
            KeyCode::Char(c.to_ascii_uppercase()),
            modifiers,
        ));
    }
    Ok(KeyBinding::new(code, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_round_trips_through_parse_key() {
        for input in [
            "q",
            "G",
            "?",
            "+",
            "space",
            "enter",
            "esc",
            "tab",
            "shift+tab",
            "ctrl+d",
            "alt+x",
            "up",
            "down",
            "pageup",
            "pagedown",
            "backspace",
        ] {
            let binding = parse_key(input).unwrap();
            assert_eq!(binding.to_string(), input);
            assert_eq!(parse_key(&binding.to_string()).unwrap(), binding);
        }
    }

    #[test]
    fn parses_common_keys() {
        assert_eq!(
            parse_key("q").unwrap(),
            KeyBinding::new(KeyCode::Char('q'), KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key("ctrl+d").unwrap(),
            KeyBinding::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
        );
        assert_eq!(parse_key("Enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key("esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(parse_key("shift+tab").unwrap().code, KeyCode::BackTab);
        assert_eq!(parse_key("?").unwrap().code, KeyCode::Char('?'));
        assert_eq!(parse_key("+").unwrap().code, KeyCode::Char('+'));
        assert_eq!(
            parse_key("ctrl++").unwrap().modifiers,
            KeyModifiers::CONTROL
        );
        assert_eq!(parse_key("G").unwrap().code, KeyCode::Char('G'));
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(parse_key("").is_err());
        assert!(parse_key("hyper+x").is_err());
        assert!(parse_key("notakey").is_err());
    }
}
