# Configuration

trellis reads an optional TOML file. A missing file means defaults; an invalid
file (bad TOML, unknown key, bad key string, unknown theme) prints an error
naming the file and exits with status 1.

## Location

| Platform        | Path                                                            |
| --------------- | --------------------------------------------------------------- |
| Linux and macOS | `$XDG_CONFIG_HOME/trellis/config.toml`, else `~/.config/trellis/config.toml` |
| Windows         | `%APPDATA%\trellis\config.toml`                                 |

Set `TRELLIS_CONFIG=/path/to/file.toml` to override.

## Example

```toml
refresh_interval_secs = 0   # seconds between background refreshes; 0 = manual only
default_remote = "origin"   # remote used by submit
nerd_fonts = true           # false switches to ASCII glyphs
theme = "pastel"            # only built-in theme today

[keybindings]
# action = "key" or ["key", "key"]
back = ["esc", "q"]         # "quit" is accepted as an alias
move_down = ["j", "down"]
move_up = ["k", "up"]
half_page_down = "ctrl+d"
help = "?"
```

All fields are optional; unknown fields are rejected.

## Keybindings

Overriding an action replaces all of its default keys. A key claimed by an
override is removed from any other action's defaults.

Press `?` in the app to see every binding currently in effect, including
your overrides. The footer shows the most useful keys for the focused panel,
also using your bindings.

Actions: `move_down`, `move_up`, `focus_next`, `focus_previous`, `drill_in`,
`back` (alias `quit`), `refresh`, `checkout`, `add_layer`, `unstack`, `unstack_remote`, `toggle_diff`,
`submit`, `toggle_submit_auto`, `toggle_submit_open`, `sync` (default `S`),
`toggle_sync_prune` (default `P`), `merge` (default `M`; merges the
checked-out stack after a typed confirmation), `cycle_merge_method` (default
`m`; merge, squash or rebase), `help`, `page_down`,
`page_up`, `half_page_down`, `half_page_up`, `end`, `open_external`,
`dismiss_message`.

Key strings: a single character (`q`, `?`, `G`), a named key (`enter`, `esc`,
`tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `home`,
`end`, `pageup`, `pagedown`), optionally prefixed with `ctrl+`, `alt+` or
`shift+` (`shift+tab` is back-tab). Names are case-insensitive except single
characters.
