# Records docs/images/overview.gif: browse a stack, then its diff.
# Read-only keys only. Run it from a repo that already has a gh-stack stack,
# with `gh-trellis` on your PATH. See docs/tapes/README.md.
Output docs/images/overview.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 700
Set Padding 10

Type "gh-trellis"
Enter
Sleep 3s

# Walk the layers of the first stack.
Tab
Sleep 1s
Down
Sleep 1s
Down
Sleep 1s

# Open the diff for the selected layer, scroll a little, then close it.
Type "d"
Sleep 2s
Down
Sleep 500ms
Down
Sleep 500ms
Type "d"
Sleep 1s

# Quit (q asks for confirmation; Enter accepts).
Type "q"
Sleep 1s
Enter
Sleep 500ms
