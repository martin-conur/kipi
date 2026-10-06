#!/bin/sh
# Fresh-machine end to end (issue #18): install a built `kipi` into an empty home,
# create fake repos, and go from zero to synced with init, scan, sync --all and doctor.
#
#   scripts/fresh-machine-e2e.sh <path-to-kipi-binary-or-archive>
#
# Needs only a POSIX shell, git, tar and xz (for an archive). Runs under a throwaway
# HOME, so it never touches the real one. Exits non-zero on the first failed check.
set -eu

src=${1:?usage: fresh-machine-e2e.sh <kipi binary or .tar.xz archive>}
case $src in /*) ;; *) src=$PWD/$src ;; esac

HOME=$(mktemp -d)
export HOME
unset KIPI_STORE KIPI_CONFIG_DIR KIPI_CACHE_DIR XDG_CONFIG_HOME XDG_CACHE_HOME
export NO_COLOR=1 GIT_CONFIG_NOSYSTEM=1
git config --global user.name e2e
git config --global user.email e2e@example.invalid
git config --global init.defaultBranch main

step() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
has() { grep -q -- "$2" "$1" || fail "$1 lacks: $2"; }

step "install"
mkdir -p "$HOME/.local/bin"
case $src in
  *.tar.xz)
    tar -xJf "$src" -C "$HOME"
    find "$HOME" -type f -name kipi -exec cp {} "$HOME/.local/bin/kipi" \; ;;
  *) cp "$src" "$HOME/.local/bin/kipi" ;;
esac
chmod +x "$HOME/.local/bin/kipi"
PATH=$HOME/.local/bin:$PATH
export PATH
kipi --version

step "fake repos under ~/work/acme"
mkrepo() { # mkrepo <dir> <remote> <readme line>
  mkdir -p "$1"
  git -C "$1" init -q
  git -C "$1" remote add origin "$2"
  printf '# %s\n\n%s\n' "$(basename "$1")" "$3" > "$1/README.md"
  git -C "$1" add README.md
  git -C "$1" commit -q -m init
}
mkrepo "$HOME/work/acme/billing-api" git@github.com:acme-org/billing-api.git \
  "Invoicing and payment orchestration."
mkrepo "$HOME/work/acme/events-pipeline" https://github.com/acme-org/events-pipeline.git \
  "Kafka consumers that turn raw events into billing features."

step "kipi init"
kipi init --store "$HOME/work/.kipi-acme" --name acme
store=$HOME/work/.kipi-acme
test -f "$store/bindings.toml" || fail "init wrote no bindings.toml"
# A scope is configuration, written in the TOML like any user would (roadmap §5.1).
cat >> "$store/bindings.toml" <<'TOML'

[scopes.acme]
roots = ["~/work/acme"]
registry = true
TOML

step "kipi scan"
kipi scan
has "$store/bindings.toml" '\[repos.billing-api\]'
has "$store/bindings.toml" '\[repos.events-pipeline\]'
has "$HOME/.config/kipi/machine.toml" 'billing-api'

step "kipi bind + add skill"
kipi add skill code-review --scope acme
kipi bind events-pipeline --related billing-api

step "kipi sync --all"
kipi sync --all
for repo in billing-api events-pipeline; do
  dir=$HOME/work/acme/$repo
  has "$dir/CLAUDE.md" 'kipi:begin'
  has "$dir/AGENTS.md" 'kipi:begin'
  has "$dir/CLAUDE.md" 'billing-api'
  test -L "$dir/.claude/skills/code-review" || fail "$repo: no claude skill link"
  test -L "$dir/.agents/skills/code-review" || fail "$repo: no generic skill link"
  test -f "$dir/.kipi/manifest.toml" || fail "$repo: no manifest"
  has "$dir/.git/info/exclude" '.kipi/'
done
has "$HOME/work/acme/events-pipeline/.claude/settings.local.json" "$HOME/work/acme/billing-api"

step "sync again is a no-op"
before=$(cat "$HOME/work/acme/events-pipeline/CLAUDE.md")
kipi sync --all
test "$before" = "$(cat "$HOME/work/acme/events-pipeline/CLAUDE.md")" || fail "second sync changed CLAUDE.md"
kipi status | tee "$HOME/status.txt"
grep -q 'stale\|missing' "$HOME/status.txt" && fail "status not synced after sync"

step "kipi doctor"
kipi doctor || fail "doctor exited $?"

step "lookup"
kipi where billing | grep -q "$HOME/work/acme/billing-api" || fail "where billing"
kipi completions bash | grep -q KIPI_COMPLETE || fail "completions bash"

printf '\nfresh-machine e2e: OK\n'
