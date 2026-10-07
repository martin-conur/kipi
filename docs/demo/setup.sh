# Demo setup for docs/demo/demo.tape: source it (`. docs/demo/setup.sh`) from the repo root.
#
# Builds a throwaway HOME holding copies of examples/acme-store and examples/personal-store
# plus empty git repos for their entries, so the recording shows a real scan and sync
# without touching your own home. Puts a fresh debug build of `kipi` first on PATH.

demo_repo=$(pwd)
cargo build -q -p kipi-cli || return 1

# A fixed path, so the paths on screen (`kipi where`) are the same on every recording.
HOME=/tmp/kipi-demo
if [ -e "$HOME" ] && [ ! -f "$HOME/.kipi-demo" ]; then
  echo "setup.sh: $HOME exists and is not a demo home; remove it first" >&2
  return 1
fi
rm -rf "$HOME" && mkdir -p "$HOME" && touch "$HOME/.kipi-demo"
export HOME
unset KIPI_STORE KIPI_CONFIG_DIR KIPI_CACHE_DIR XDG_CONFIG_HOME XDG_CACHE_HOME
export GIT_CONFIG_NOSYSTEM=1 PATH="$demo_repo/target/debug:$PATH"
git config --global user.name demo
git config --global user.email demo@example.invalid
git config --global init.defaultBranch main

demo_mkrepo() { # demo_mkrepo <dir> <remote> <readme line>
  mkdir -p "$1"
  git -C "$1" init -q
  git -C "$1" remote add origin "$2"
  printf '# %s\n\n%s\n' "$(basename "$1")" "$3" > "$1/README.md"
  git -C "$1" add README.md
  git -C "$1" commit -q -m init
}
demo_mkrepo "$HOME/work/acme/billing-api" git@github.com:acme-org/billing-api.git "Invoicing."
demo_mkrepo "$HOME/work/acme/api" git@github.com:acme-org/api.git "Public REST gateway."
demo_mkrepo "$HOME/work/acme/web-dashboard" git@github.com:acme-org/web-dashboard.git "Dashboard."
demo_mkrepo "$HOME/work/acme/ml/events-pipeline" git@github.com:acme-org/events-pipeline.git "Events."
demo_mkrepo "$HOME/work/acme/ml/ml-platform" git@github.com:acme-org/ml-platform.git "ML tooling."
demo_mkrepo "$HOME/code/side-tool" git@github.com:example-user/side-tool.git "A side tool."
demo_mkrepo "$HOME/code/api" git@github.com:example-user/api.git "A personal API."
demo_mkrepo "$HOME/code/blog" git@github.com:example-user/blog.git "A blog."
demo_mkrepo "$HOME/code/dotfiles" git@github.com:example-user/dotfiles.git "Dotfiles."

mkdir -p "$HOME/.agents"
cp -R "$demo_repo/examples/personal-store" "$HOME/.agents/store"
cp -R "$demo_repo/examples/acme-store" "$HOME/work/.kipi-acme"
kipi init --store "$HOME/.agents/store" >/dev/null

cd "$HOME" || return 1
export PS1='$ '
clear
