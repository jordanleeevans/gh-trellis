# Architecture

Trellis is a terminal UI over the `gh stack` CLI extension. It uses
**Component + Action**, the pattern most non-trivial ratatui apps settle on
(gitui, the ratatui async template):

- Panels are **components**. Each one turns key presses into `Action`s,
  reacts to applied actions, and draws itself.
- An `Action` enum is the only message type. User intents, async results
  and navigation are all actions.
- A synchronous **reducer** applies actions to a single `AppState`.
- An **effects** runner spawns every process-touching action as a tokio
  task. Each task sends its result back as another `Action`, so the render
  loop never blocks on `git` or `gh`.

## Module layout

```
src/
  main.rs         load config → dependency check → tui::run
  config/         config.toml loading and key-string parsing (see docs/config.md)
  doctor/         git / gh / gh-stack presence, version and auth checks
  shell/          Shell trait, ProcessShell, MockShell (process I/O only)
  git/            typed wrappers over plain `git` (diff, log, branch, rebase, status)
  stack/          gh-stack domain: models (Stack, Layer, PullRequestRef, StackSummary...)
                  and operations (list_stacks, hydrate_layer_detail, submit_stack...)
  theme/          palette, text styles, glyph sets (Nerd Font / ASCII)
  tui/
    mod.rs        exposes run()
    app/          App: owns state + components; event loop; reducer
      mod.rs         event loop, key routing, dispatch, and apply_action: an
                     exhaustive router that sends each Action to a feature module
      refresh.rs     stack list refresh      layers.rs    layer detail/diff loading
      stack_ops.rs   checkout, add layer, open PR, unstack
      submit.rs      submit + progress       sync.rs      gh stack sync
      feedback.rs    errors, status, confirm modal
      test_support.rs  reducer test harness (settle/apply/dispatch_now)
    action.rs     Action enum
    component.rs  Component trait
    effects.rs    Effects: spawns shell work, sends results back as Actions
    keymap.rs     KeyIntent + data-driven key table (overridable from config)
    messages.rs   user-facing wording for shell failures
    state/        AppState, Screen, cache keys; layer_resource (detail/diff cache);
                  submit_progress (per-layer submit status)
    components/   one module per panel or overlay
      stack_browser/   the main browser component
        mod.rs         view state and layout; Component impl delegates to:
        keys.rs        key press -> Action for the focused panel
        selection.rs   selection, focus and scroll updates on applied Actions
        navigator.rs   stack list with the selected stack expanded into layers
        layer_detail.rs PR summary for the selected layer
        diff.rs        diff parsing, changed-files tree, diff view
        chrome.rs      header, and a footer of hints for the focused panel,
                       fitted to the terminal width
      confirm.rs       shared ConfirmModal for every destructive action
      help.rs          `?` overlay listing every binding and toggle state
      add_layer_prompt.rs
      submit_progress.rs
    widgets/      stateless render helpers: panel_block, centered_rect, spinner, glyphs()
```

### Why this differs from the original #43 proposal

The first proposal merged `shell`, `stack` and `git` into a single `git/`
module. We kept them separate instead. They were already cleanly split by
concern, none of them depends on ratatui, and merging them would have been
churn with no benefit. The `tui/` layout does follow the proposal.

## Data flow

```
 key press ──► App::handle_key ──► confirm modal (if open)
                                  └► component.handle_key ──► Vec<Action>
                                                                │
          ┌─────────────────────────────────────────────────────┘
          ▼
 App::dispatch(actions)
   for each action:
     reducer  apply_action(&action, &effects) ─► follow-up Actions (queued)
                 │  └─ needs a process? effects.load_diff / checkout / submit_stack ...
                 │                          │ tokio::spawn(shell work)
                 │                          ▼
                 │                 tx.send(Action::LayerDiffLoaded / SetError / ...)
     component.update(&action, &mut state)     (view state: selection, focus, scroll)
          ▲
 event loop: drains the channel each tick, dispatches results, draws, polls keys
```

A typical round trip is selecting a layer:

1. `j` produces `SelectNext`.
2. `dispatch` queues `LoadLayerDiff`.
3. The reducer marks the diff cache entry `Loading` and calls
   `effects.load_diff`.
4. The panel renders "Loading diff...".
5. The spawned task sends back `LayerDiffLoaded`.
6. The reducer stores the result and the panel renders the diff.

## Boundaries

These rules are what keep the layout workable as the number of panels and
actions grows. Review against them.

1. **Only `tui/` and `main.rs` import ratatui or crossterm UI types.**
   `config/` may use crossterm key codes to parse bindings. Everything else
   can be unit-tested with no terminal.
2. **Components never touch a `Shell`.** They return `Action`s.
3. **The reducer never awaits.** Any action that runs a process is handed
   to `Effects`, and its outcome comes back as an `Action`. To add a write
   action (sync, rebase, merge...), add an `Effects` method for it and
   follow `checkout` or `submit_stack`.
4. **Multi-step orchestration lives in `stack/`, not `tui/`.**
   `stack::submit_stack` reports domain events (`SubmitEvent`), and `tui/`
   maps them to display state. Follow this for sync, rebase and merge.
5. **Every destructive action confirms through `components::confirm`.**
   Irreversible actions (merge, remote unstack) use `danger: true`, which
   requires a typed phrase. No action builds its own one-off modal.
6. **User-facing error text goes through `tui::messages`**, so the same
   failure reads the same way from every action.
7. **Key labels come from the keymap.** The footer and help overlay ask
   `keymap::current()` for labels instead of hardcoding keys, so rebinding
   a key in `config.toml` updates both. To add a key, add a `KeyIntent`,
   its default binding and config name in `tui/keymap.rs`, a row in
   `components/help.rs`, and, if it's among the most useful keys for a
   panel, a hint in `stack_browser/chrome.rs`.

## Testing

- **Domain modules** (`git/`, `stack/`, `doctor/`) are tested against
  `MockShell` with canned `(program, args)` responses. `stack/fixtures/`
  holds real `gh stack view --json` samples.
- **The reducer** is tested through `App::settle(action, &mock)` from
  `tui/app/test_support.rs`, with each feature's tests beside its module. It applies an action, runs the resulting effects against
  the mock to completion, and returns what came back. This is the same
  effect path production uses; there is no separate test-only code path.
  `App::apply` applies an action without running its effects.
- **Components** are tested by feeding keys and actions to the component
  and asserting on the `Action`s it emits or the lines it renders.
- **CI** (`.github/workflows/ci.yml`) runs `cargo fmt --check`,
  `cargo clippy --all-targets --all-features` and `cargo test`.
