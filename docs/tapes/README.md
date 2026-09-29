# Recording the README GIFs

The GIFs in the main README are recorded with [vhs](https://github.com/charmbracelet/vhs).
They are not committed until someone records them.

The tapes launch `trellis` in the directory you run `vhs` from, so record from
a repository that has a gh-stack stack (a scratch repo made with the commands
in the main README works), not from the trellis checkout.

```sh
brew install vhs
cargo install --path /path/to/trellis      # puts `trellis` on your PATH
cd /path/to/scratch-repo-with-a-stack
vhs /path/to/trellis/docs/tapes/overview.tape
vhs /path/to/trellis/docs/tapes/help.tape
```

Each tape writes to a path relative to where `vhs` runs. Move the results into
the trellis checkout afterwards:

| Tape | Output (relative to the scratch repo) | Move to |
| --- | --- | --- |
| `overview.tape` | `docs/images/overview.gif` | `docs/images/overview.gif` |
| `help.tape` | `docs/images/help.gif` | `docs/images/help.gif` |

Create `docs/images/` in the scratch repo first (`mkdir -p docs/images`).

The tapes only press read-only keys (navigation, `Tab`, `d`, `?`, then `q` and
`Enter` to quit). Keep it that way: never record `s`, `S`, `R`, `u`, `M`, `D`,
`U`, `c`, `a` or `W` against a real repository.
