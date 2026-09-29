# Records docs/images/help.gif: the `?` key-binding overlay.
# Read-only keys only. Run it from a repo that already has a gh-stack stack,
# with `trellis` on your PATH. See docs/tapes/README.md.
Output docs/images/help.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 700
Set Padding 10

Type "trellis"
Enter
Sleep 3s

Type "?"
Sleep 2s
# Scroll the overlay, then close it.
Down
Sleep 500ms
Down
Sleep 500ms
Down
Sleep 500ms
Type "?"
Sleep 1s

# Quit (q asks for confirmation; Enter accepts).
Type "q"
Sleep 1s
Enter
Sleep 500ms
