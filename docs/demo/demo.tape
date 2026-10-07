# The README demo, recorded with vhs (https://github.com/charmbracelet/vhs) against
# examples/acme-store and examples/personal-store in a throwaway HOME.
#
#   vhs docs/demo/demo.tape      # from the repo root; writes docs/demo/demo.gif and tree.png

Output docs/demo/demo.gif

Set Shell "bash"
Set FontSize 15
Set Width 1200
Set Height 600
Set Padding 20
Set TypingSpeed 40ms
Set Theme "Builtin Dark"

Hide
Type "export PS1='$ ' && . docs/demo/setup.sh" Enter
Sleep 8s
Show

Type "kipi init --store ~/work/.kipi-acme    # mount the work store" Enter
Sleep 1.5s
Type "kipi scan                              # record checkouts in machine.toml" Enter
Sleep 2.5s
Type "clear && kipi sync --all" Enter
Sleep 3s
Type "clear && kipi tree" Enter
Sleep 3s
Screenshot docs/demo/tree.png
Sleep 1s
Type "clear && kipi explain events-pipeline | head -24" Enter
Sleep 4s
Type "clear && sed -n '/kipi:begin/,/kipi:end/p' work/acme/ml/events-pipeline/CLAUDE.md" Enter
Sleep 5s
Type "clear && kipi where billing" Enter
Sleep 3s
