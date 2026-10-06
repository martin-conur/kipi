//! Where kipi's files live on this machine, and `~` expansion.
//!
//! Everything machine-specific comes in through [`Env`], built once from the process
//! environment ([`Env::from_process`]) or from explicit values in tests
//! ([`Env::from_lookup`]), so no loader ever reads the real home directory by itself.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::Config;

/// Overrides `~/.config/kipi` (where `config.toml`, `machine.toml` and the lock live).
pub const ENV_CONFIG_DIR: &str = "KIPI_CONFIG_DIR";
/// Overrides `~/.cache/kipi`.
pub const ENV_CACHE_DIR: &str = "KIPI_CACHE_DIR";
/// Overrides the configured store list.
pub const ENV_STORE: &str = "KIPI_STORE";

/// The default store, relative to the home directory.
pub const DEFAULT_STORE: &str = ".agents/store";

/// The machine-specific inputs to loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    /// The home directory `~` expands to.
    pub home: PathBuf,
    /// `$KIPI_CONFIG_DIR`, else `~/.config/kipi`.
    pub config_dir: PathBuf,
    /// `$KIPI_CACHE_DIR`, else `~/.cache/kipi`.
    pub cache_dir: PathBuf,
    /// `$KIPI_STORE`, if set.
    pub store: Option<PathBuf>,
}

impl Env {
    /// Read the process environment and the platform home directory.
    pub fn from_process() -> Result<Self> {
        Self::from_lookup(|k| std::env::var_os(k), dirs::home_dir())
    }

    /// Build from an env lookup and a home directory. Empty variables count as unset.
    pub fn from_lookup(
        var: impl Fn(&str) -> Option<OsString>,
        home: Option<PathBuf>,
    ) -> Result<Self> {
        let home = home.filter(|h| h.is_absolute()).ok_or(Error::NoHome)?;
        let var = |k: &str| {
            var(k)
                .filter(|v| !v.is_empty())
                .map(|v| expand_tilde(&v.to_string_lossy(), &home))
        };
        Ok(Env {
            config_dir: var(ENV_CONFIG_DIR).unwrap_or_else(|| home.join(".config/kipi")),
            cache_dir: var(ENV_CACHE_DIR).unwrap_or_else(|| home.join(".cache/kipi")),
            store: var(ENV_STORE),
            home,
        })
    }

    /// `<config_dir>/config.toml`.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join(super::CONFIG_FILE)
    }

    /// `<config_dir>/machine.toml`.
    pub fn machine_file(&self) -> PathBuf {
        self.config_dir.join(super::MACHINE_FILE)
    }
}

/// `~` → home, `~/x` → home/x; anything else unchanged.
pub fn expand_tilde(raw: &str, home: &Path) -> PathBuf {
    match raw.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(raw),
    }
}

/// A user-written path that must be absolute or `~`-relative, expanded.
pub(crate) fn absolute_path(raw: &str, home: &Path) -> Result<PathBuf, String> {
    let path = expand_tilde(raw, home);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("path `{raw}` is relative"))
    }
}

/// Hint for [`absolute_path`] failures.
pub(crate) const ABSOLUTE_PATH_HINT: &str = "write an absolute path or one starting with `~/`";

/// The stores to mount, by precedence: `--store` flags > `$KIPI_STORE` > `config.toml`
/// `stores` > `~/.agents/store`.
pub fn store_paths(flags: &[PathBuf], env: &Env, config: &Config) -> Vec<PathBuf> {
    if !flags.is_empty() {
        let expand = |p: &PathBuf| expand_tilde(&p.to_string_lossy(), &env.home);
        return flags.iter().map(expand).collect();
    }
    if let Some(store) = &env.store {
        return vec![store.clone()];
    }
    if !config.stores.is_empty() {
        return config.stores.clone();
    }
    vec![env.home.join(DEFAULT_STORE)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> Env {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        Env::from_lookup(
            |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()),
            Some(PathBuf::from("/h")),
        )
        .unwrap()
    }

    #[test]
    fn tilde_expansion() {
        let home = Path::new("/h");
        assert_eq!(expand_tilde("~", home), PathBuf::from("/h"));
        assert_eq!(expand_tilde("~/a/b", home), PathBuf::from("/h/a/b"));
        assert_eq!(expand_tilde("/abs", home), PathBuf::from("/abs"));
        assert_eq!(expand_tilde("~other/x", home), PathBuf::from("~other/x"));
        assert!(absolute_path("rel/x", home).is_err());
        assert!(absolute_path("~other", home).is_err());
    }

    #[test]
    fn env_defaults_and_overrides() {
        let e = env(&[]);
        assert_eq!(e.config_dir, PathBuf::from("/h/.config/kipi"));
        assert_eq!(e.cache_dir, PathBuf::from("/h/.cache/kipi"));
        assert_eq!(e.store, None);
        assert_eq!(
            e.machine_file(),
            PathBuf::from("/h/.config/kipi/machine.toml")
        );
        let e = env(&[
            (ENV_CONFIG_DIR, "/t/cfg"),
            (ENV_CACHE_DIR, "~/c"),
            (ENV_STORE, ""),
        ]);
        assert_eq!(e.config_dir, PathBuf::from("/t/cfg"));
        assert_eq!(e.cache_dir, PathBuf::from("/h/c"));
        assert_eq!(e.store, None, "empty means unset");
    }

    #[test]
    fn missing_or_relative_home_is_an_error() {
        assert!(matches!(
            Env::from_lookup(|_| None, None),
            Err(Error::NoHome)
        ));
        let rel = Env::from_lookup(|_| None, Some(PathBuf::from("rel")));
        assert!(matches!(rel, Err(Error::NoHome)));
    }

    #[test]
    fn store_precedence() {
        let config = Config {
            stores: vec![PathBuf::from("/cfg/a"), PathBuf::from("/cfg/b")],
            ..Config::default()
        };
        let none = Config::default();
        let flag = [PathBuf::from("~/flag")];
        let with_env = env(&[(ENV_STORE, "/env/s")]);
        assert_eq!(
            store_paths(&flag, &with_env, &config),
            [PathBuf::from("/h/flag")]
        );
        assert_eq!(
            store_paths(&[], &with_env, &config),
            [PathBuf::from("/env/s")]
        );
        assert_eq!(store_paths(&[], &env(&[]), &config), config.stores);
        assert_eq!(
            store_paths(&[], &env(&[]), &none),
            [PathBuf::from("/h/.agents/store")]
        );
    }
}
