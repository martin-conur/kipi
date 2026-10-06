# Install

kipi runs on Linux and macOS (x86_64 and aarch64). It needs `git` on `PATH` and nothing else:
release binaries are static on Linux (musl) and link only the system libraries on macOS.

## Homebrew

```bash
brew install martin-conur/tap/kipi
```

## Shell installer

Downloads the binary for your platform from the GitHub release, checks its SHA-256 and installs it
into `~/.cargo/bin`:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/martin-conur/kipi/releases/latest/download/kipi-cli-installer.sh | sh
```

## Prebuilt archives

Every release carries `kipi-cli-<target>.tar.xz` for `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`, `x86_64-apple-darwin` and `aarch64-apple-darwin`, each with a
`.sha256` file, plus a combined `sha256.sum`. Unpack one and put `kipi` on your `PATH`.

## From source

With the Rust toolchain pinned in `rust-toolchain.toml` (1.92):

```bash
cargo install --locked --git https://github.com/martin-conur/kipi kipi-cli
```

## Shell completions

`kipi completions <shell>` prints a short script for `bash`, `zsh`, `fish`, `elvish` or
`powershell`. It completes subcommands and flags, and asks `kipi` itself for **repo entry names,
aliases, skills, scopes and store names** from the stores your `config.toml` (or `$KIPI_STORE`)
mounts, so new entries complete as soon as they exist. `kipi edit` also completes
`notes:<repo>` and `conventions:<scope>`.

Load it from your shell's startup file, so the script always matches the installed `kipi`:

```bash
# ~/.bashrc  (bash 3.2 on macOS cannot `source <(...)`, so use eval)
eval "$(kipi completions bash)"

# ~/.zshrc  (after compinit)
source <(kipi completions zsh)

# ~/.config/fish/config.fish
kipi completions fish | source

# ~/.config/elvish/rc.elv
eval (kipi completions elvish | slurp)
```

While completing, the shell runs `kipi` with `KIPI_COMPLETE=<shell>` set; that run only prints
candidates and never writes anything. If the stores cannot be loaded (none configured, a broken
`bindings.toml`), names are simply not offered — completion never prints an error into your
prompt. Names come from the configured stores; a `--store` flag typed on the same command line is
not taken into account.

## Next

[Quickstart: ten repos](quickstart.md).
