use crate::config::keymap::KeyBinding;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyIntent {
    MoveDown,
    MoveUp,
    FocusNext,
    FocusPrevious,
    DrillIn,
    Back,
    Refresh,
    Checkout,
    AddLayer,
    Unstack,
    UnstackRemote,
    ToggleDiff,
    Submit,
    ToggleSubmitAuto,
    ToggleSubmitOpen,
    Sync,
    ToggleSyncPrune,
    Merge,
    CycleMergeMethod,
    RebaseStack,
    RebaseUpstack,
    ConflictEdit,
    ConflictMarkResolved,
    RebaseContinue,
    RebaseAbort,
    Help,
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    End,
    OpenExternal,
    DismissMessage,
}

const NONE: KeyModifiers = KeyModifiers::NONE;

const fn bind(code: KeyCode) -> KeyBinding {
    KeyBinding::new(code, NONE)
}

const fn ch(c: char) -> KeyBinding {
    bind(KeyCode::Char(c))
}

/// Snake-case names used in the `[keybindings]` config table.
const INTENT_NAMES: &[(&str, KeyIntent)] = &[
    ("move_down", KeyIntent::MoveDown),
    ("move_up", KeyIntent::MoveUp),
    ("focus_next", KeyIntent::FocusNext),
    ("focus_previous", KeyIntent::FocusPrevious),
    ("drill_in", KeyIntent::DrillIn),
    ("back", KeyIntent::Back),
    ("quit", KeyIntent::Back),
    ("refresh", KeyIntent::Refresh),
    ("checkout", KeyIntent::Checkout),
    ("add_layer", KeyIntent::AddLayer),
    ("unstack", KeyIntent::Unstack),
    ("unstack_remote", KeyIntent::UnstackRemote),
    ("toggle_diff", KeyIntent::ToggleDiff),
    ("submit", KeyIntent::Submit),
    ("toggle_submit_auto", KeyIntent::ToggleSubmitAuto),
    ("toggle_submit_open", KeyIntent::ToggleSubmitOpen),
    ("sync", KeyIntent::Sync),
    ("toggle_sync_prune", KeyIntent::ToggleSyncPrune),
    ("merge", KeyIntent::Merge),
    ("cycle_merge_method", KeyIntent::CycleMergeMethod),
    ("rebase_stack", KeyIntent::RebaseStack),
    ("rebase_upstack", KeyIntent::RebaseUpstack),
    ("conflict_edit", KeyIntent::ConflictEdit),
    ("conflict_mark_resolved", KeyIntent::ConflictMarkResolved),
    ("rebase_continue", KeyIntent::RebaseContinue),
    ("rebase_abort", KeyIntent::RebaseAbort),
    ("help", KeyIntent::Help),
    ("page_down", KeyIntent::PageDown),
    ("page_up", KeyIntent::PageUp),
    ("half_page_down", KeyIntent::HalfPageDown),
    ("half_page_up", KeyIntent::HalfPageUp),
    ("end", KeyIntent::End),
    ("open_external", KeyIntent::OpenExternal),
    ("dismiss_message", KeyIntent::DismissMessage),
];

fn default_bindings() -> Vec<(KeyBinding, KeyIntent)> {
    use KeyIntent::*;
    let ctrl = |c| KeyBinding::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    vec![
        (ctrl('d'), HalfPageDown),
        (ctrl('u'), HalfPageUp),
        (ch('G'), End),
        (bind(KeyCode::Down), MoveDown),
        (ch('j'), MoveDown),
        (bind(KeyCode::Up), MoveUp),
        (ch('k'), MoveUp),
        (bind(KeyCode::Tab), FocusNext),
        (bind(KeyCode::BackTab), FocusPrevious),
        (bind(KeyCode::Enter), DrillIn),
        (ch(' '), DrillIn),
        (bind(KeyCode::Esc), Back),
        (ch('q'), Back),
        (ch('r'), Refresh),
        (ch('c'), Checkout),
        (ch('a'), AddLayer),
        (ch('D'), Unstack),
        (ch('U'), UnstackRemote),
        (ch('d'), ToggleDiff),
        (ch('s'), Submit),
        (ch('t'), ToggleSubmitAuto),
        (ch('p'), ToggleSubmitOpen),
        (ch('S'), Sync),
        (ch('P'), ToggleSyncPrune),
        (ch('M'), Merge),
        (ch('m'), CycleMergeMethod),
        (ch('R'), RebaseStack),
        (ch('u'), RebaseUpstack),
        (ch('e'), ConflictEdit),
        (ch('+'), ConflictMarkResolved),
        (ch('C'), RebaseContinue),
        (ch('A'), RebaseAbort),
        (ch('?'), Help),
        (bind(KeyCode::PageDown), PageDown),
        (bind(KeyCode::PageUp), PageUp),
        (bind(KeyCode::Backspace), PageUp),
        (ch('o'), OpenExternal),
        (ch('O'), OpenExternal),
        (ch('x'), DismissMessage),
    ]
}

/// Data-driven key table: default bindings with optional per-intent overrides.
#[derive(Debug, Clone)]
pub(crate) struct Keymap {
    entries: Vec<(KeyBinding, KeyIntent)>,
}

impl Keymap {
    pub(crate) fn default_keymap() -> Self {
        Self {
            entries: default_bindings(),
        }
    }

    /// Apply overrides keyed by intent name. An overridden intent loses all of
    /// its default keys, and any key it claims is taken from other intents.
    pub(crate) fn with_overrides(
        mut self,
        overrides: &BTreeMap<String, Vec<KeyBinding>>,
    ) -> Result<Self, String> {
        let mut resolved = Vec::new();
        for (name, bindings) in overrides {
            let intent = INTENT_NAMES
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, i)| *i)
                .ok_or_else(|| {
                    let names: Vec<&str> = INTENT_NAMES.iter().map(|(n, _)| *n).collect();
                    format!(
                        "[keybindings] unknown action {name:?} (available: {})",
                        names.join(", ")
                    )
                })?;
            resolved.push((intent, bindings));
        }
        for (intent, bindings) in resolved {
            self.entries
                .retain(|(b, i)| *i != intent && !bindings.contains(b));
            self.entries.extend(bindings.iter().map(|b| (*b, intent)));
        }
        Ok(self)
    }

    /// Every key bound to `intent`, in table order.
    pub(crate) fn bindings_for(&self, intent: KeyIntent) -> Vec<KeyBinding> {
        self.entries
            .iter()
            .filter(|(_, i)| *i == intent)
            .map(|(b, _)| *b)
            .collect()
    }

    /// The shortest key label for `intent` (e.g. `j` rather than `down`),
    /// for compact hints; `None` if the intent is unbound.
    pub(crate) fn short_label(&self, intent: KeyIntent) -> Option<String> {
        self.bindings_for(intent)
            .iter()
            .map(ToString::to_string)
            .min_by_key(|label| label.chars().count())
    }

    /// Every key label for `intent`, joined with `/`, or `unbound`.
    pub(crate) fn all_labels(&self, intent: KeyIntent) -> String {
        let labels: Vec<String> = self
            .bindings_for(intent)
            .iter()
            .map(ToString::to_string)
            .collect();
        if labels.is_empty() {
            "unbound".to_string()
        } else {
            labels.join("/")
        }
    }

    /// Most specific (most modifiers) matching binding wins; ties go to table order.
    pub(crate) fn lookup(&self, key: KeyEvent) -> Option<KeyIntent> {
        self.entries
            .iter()
            .rev()
            .filter(|(b, _)| b.matches(key.code, key.modifiers))
            .max_by_key(|(b, _)| b.modifiers.bits().count_ones())
            .map(|(_, i)| *i)
    }
}

static KEYMAP: OnceLock<Keymap> = OnceLock::new();

/// Install the process-wide keymap from config (first call wins).
pub(crate) fn install(keymap: Keymap) {
    let _ = KEYMAP.set(keymap);
}

/// The process-wide keymap: config overrides if installed, else defaults.
pub(crate) fn current() -> &'static Keymap {
    KEYMAP.get_or_init(Keymap::default_keymap)
}

pub(crate) fn key_intent(key: KeyEvent) -> Option<KeyIntent> {
    current().lookup(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_global_navigation_keys() {
        assert_eq!(
            key_intent(key(KeyCode::Char('j'))),
            Some(KeyIntent::MoveDown)
        );
        assert_eq!(key_intent(key(KeyCode::Down)), Some(KeyIntent::MoveDown));
        assert_eq!(key_intent(key(KeyCode::Char('k'))), Some(KeyIntent::MoveUp));
        assert_eq!(key_intent(key(KeyCode::Up)), Some(KeyIntent::MoveUp));
        assert_eq!(key_intent(key(KeyCode::Tab)), Some(KeyIntent::FocusNext));
        assert_eq!(
            key_intent(key(KeyCode::BackTab)),
            Some(KeyIntent::FocusPrevious)
        );
        assert_eq!(key_intent(key(KeyCode::Enter)), Some(KeyIntent::DrillIn));
        assert_eq!(
            key_intent(key(KeyCode::Char(' '))),
            Some(KeyIntent::DrillIn)
        );
        assert_eq!(key_intent(key(KeyCode::Esc)), Some(KeyIntent::Back));
        assert_eq!(key_intent(key(KeyCode::Char('q'))), Some(KeyIntent::Back));
        assert_eq!(
            key_intent(key(KeyCode::Char('c'))),
            Some(KeyIntent::Checkout)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('a'))),
            Some(KeyIntent::AddLayer)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('d'))),
            Some(KeyIntent::ToggleDiff)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('o'))),
            Some(KeyIntent::OpenExternal)
        );
        assert_eq!(
            key_intent(ctrl_key(KeyCode::Char('d'))),
            Some(KeyIntent::HalfPageDown)
        );
        assert_eq!(
            key_intent(ctrl_key(KeyCode::Char('u'))),
            Some(KeyIntent::HalfPageUp)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('D'))),
            Some(KeyIntent::Unstack)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('U'))),
            Some(KeyIntent::UnstackRemote)
        );
        assert_eq!(key_intent(key(KeyCode::Char('G'))), Some(KeyIntent::End));
        assert_eq!(key_intent(key(KeyCode::Char('?'))), Some(KeyIntent::Help));
        assert_eq!(
            key_intent(key(KeyCode::Char('x'))),
            Some(KeyIntent::DismissMessage)
        );
        assert_eq!(key_intent(key(KeyCode::Char('s'))), Some(KeyIntent::Submit));
        assert_eq!(
            key_intent(key(KeyCode::Char('t'))),
            Some(KeyIntent::ToggleSubmitAuto)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('p'))),
            Some(KeyIntent::ToggleSubmitOpen)
        );
        assert_eq!(key_intent(key(KeyCode::Char('S'))), Some(KeyIntent::Sync));
        assert_eq!(
            key_intent(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            Some(KeyIntent::Sync)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('P'))),
            Some(KeyIntent::ToggleSyncPrune)
        );
        assert_eq!(key_intent(key(KeyCode::Char('M'))), Some(KeyIntent::Merge));
        assert_eq!(
            key_intent(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT)),
            Some(KeyIntent::Merge)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('m'))),
            Some(KeyIntent::CycleMergeMethod)
        );
        for (c, intent) in [
            ('R', KeyIntent::RebaseStack),
            ('u', KeyIntent::RebaseUpstack),
            ('e', KeyIntent::ConflictEdit),
            ('+', KeyIntent::ConflictMarkResolved),
            ('C', KeyIntent::RebaseContinue),
            ('A', KeyIntent::RebaseAbort),
        ] {
            assert_eq!(key_intent(key(KeyCode::Char(c))), Some(intent), "{c}");
        }
    }

    /// Two intents sharing a default key means one of them silently never
    /// fires (the table's first entry wins), as happened when merge and the
    /// conflict view both claimed `m`.
    #[test]
    fn no_two_intents_share_a_default_key() {
        let defaults = default_bindings();
        for (i, (binding, intent)) in defaults.iter().enumerate() {
            for (other_binding, other_intent) in &defaults[i + 1..] {
                assert!(
                    binding != other_binding || intent == other_intent,
                    "{binding} is bound to both {intent:?} and {other_intent:?}"
                );
            }
        }
    }

    #[test]
    fn every_intent_has_a_config_name_and_a_default_key() {
        let defaults = Keymap::default_keymap();
        for (_, intent) in INTENT_NAMES {
            assert!(
                !defaults.bindings_for(*intent).is_empty(),
                "{intent:?} has no default key"
            );
        }
    }

    #[test]
    fn overrides_replace_default_keys() {
        let mut o = BTreeMap::new();
        o.insert(
            "quit".to_string(),
            vec![KeyBinding::new(KeyCode::Char('x'), KeyModifiers::NONE)],
        );
        let map = Keymap::default_keymap().with_overrides(&o).unwrap();
        assert_eq!(map.lookup(key(KeyCode::Char('x'))), Some(KeyIntent::Back));
        assert_eq!(map.lookup(key(KeyCode::Char('q'))), None);
        assert_eq!(map.lookup(key(KeyCode::Esc)), None);
        assert_eq!(
            map.lookup(key(KeyCode::Char('j'))),
            Some(KeyIntent::MoveDown)
        );
    }

    #[test]
    fn labels_follow_overrides() {
        let defaults = Keymap::default_keymap();
        assert_eq!(
            defaults.short_label(KeyIntent::MoveDown).as_deref(),
            Some("j")
        );
        assert_eq!(defaults.all_labels(KeyIntent::MoveDown), "down/j");

        let mut o = BTreeMap::new();
        o.insert(
            "move_down".to_string(),
            vec![KeyBinding::new(KeyCode::Char('n'), KeyModifiers::NONE)],
        );
        let map = Keymap::default_keymap().with_overrides(&o).unwrap();
        assert_eq!(map.short_label(KeyIntent::MoveDown).as_deref(), Some("n"));
        assert_eq!(map.all_labels(KeyIntent::MoveDown), "n");
    }

    #[test]
    fn unknown_action_is_rejected() {
        let mut o = BTreeMap::new();
        o.insert("nope".to_string(), vec![]);
        let e = Keymap::default_keymap().with_overrides(&o).unwrap_err();
        assert!(e.contains("nope"));
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }
}
