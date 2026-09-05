//! Where settings and the receiver registry live on disk, and how they get
//! there without a torn file.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::Settings;

const SETTINGS_FILE: &str = "settings.toml";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("neither XDG_CONFIG_HOME nor HOME is set")]
    NoHome,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not valid TOML: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{path} could not be encoded as TOML: {source}")]
    Encode {
        path: PathBuf,
        source: toml::ser::Error,
    },
}

pub struct ConfigStore {
    dir: PathBuf,
}

impl ConfigStore {
    pub fn new(dir: PathBuf) -> Self {
        ConfigStore { dir }
    }

    /// Resolving the directory deliberately does not create it: reading the
    /// config on a machine that has never run glint stays a pure read.
    pub fn from_env() -> Result<Self, StoreError> {
        let xdg = std::env::var_os("XDG_CONFIG_HOME");
        let home = std::env::var_os("HOME");
        Ok(ConfigStore::new(resolve_dir(
            xdg.as_deref(),
            home.as_deref(),
        )?))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A missing file is the first run, not a failure: glint has to start on
    /// a machine with an empty `~/.config`.
    pub(crate) fn load_toml<T: DeserializeOwned + Default>(
        &self,
        file: &str,
    ) -> Result<T, StoreError> {
        let path = self.dir.join(file);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
            Err(source) => return Err(StoreError::Io { path, source }),
        };
        toml::from_str(&text).map_err(|source| StoreError::Parse { path, source })
    }

    pub(crate) fn save_toml<T: Serialize>(&self, file: &str, value: &T) -> Result<(), StoreError> {
        let path = self.dir.join(file);
        let text = toml::to_string(value).map_err(|source| StoreError::Encode {
            path: path.clone(),
            source,
        })?;
        fs::create_dir_all(&self.dir).map_err(|source| StoreError::Io {
            path: self.dir.clone(),
            source,
        })?;
        write_atomically(&path, &text).map_err(|source| StoreError::Io { path, source })
    }

    pub fn load_settings(&self) -> Result<Settings, StoreError> {
        self.load_toml(SETTINGS_FILE)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), StoreError> {
        self.save_toml(SETTINGS_FILE, settings)
    }
}

/// An empty value counts as unset, which is what the XDG base directory
/// specification asks for.
fn resolve_dir(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, StoreError> {
    if let Some(xdg) = xdg_config_home.filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(xdg).join("glint"));
    }
    let home = home.filter(|v| !v.is_empty()).ok_or(StoreError::NoHome)?;
    Ok(PathBuf::from(home).join(".config").join("glint"))
}

/// The temp file is a sibling of the target rather than a file in the system
/// temp directory: `rename` is only atomic within one filesystem. It is also
/// removed on the error path so a failed save leaves nothing behind.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    match fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, path)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Encoder;

    /// Removes its directory on drop so a failing assertion cannot leak a
    /// tree into the system temp directory.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("glint-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            TempDir(path)
        }

        pub(crate) fn path(&self) -> PathBuf {
            self.0.clone()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loading_settings_from_a_directory_that_does_not_exist_yields_the_defaults() {
        // arrange
        let dir = TempDir::new("settings-first-run");
        let store = ConfigStore::new(dir.path());
        // act
        let settings = store.load_settings().unwrap();
        // assert
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn settings_round_trip_through_the_store() {
        // arrange
        let dir = TempDir::new("settings-round-trip");
        let store = ConfigStore::new(dir.path());
        let original = Settings {
            preferred_encoder: Some(Encoder::VaH264),
            bitrate_cap_kbps: Some(8000),
            audio_follows_screen: false,
            retry_timeout_secs: 45,
        };
        // act
        store.save_settings(&original).unwrap();
        let loaded = store.load_settings().unwrap();
        // assert
        assert_eq!(loaded, original);
    }

    #[test]
    fn saving_settings_creates_the_directory_when_it_is_missing() {
        // arrange
        let dir = TempDir::new("settings-mkdir");
        let store = ConfigStore::new(dir.path());
        // act
        store.save_settings(&Settings::default()).unwrap();
        // assert
        assert!(dir.path().is_dir());
        assert!(dir.path().join("settings.toml").is_file());
    }

    #[test]
    fn a_corrupt_settings_file_is_a_parse_error_and_not_a_panic() {
        // arrange
        let dir = TempDir::new("settings-corrupt");
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(dir.path().join("settings.toml"), "retry_timeout_secs = [[[").unwrap();
        let store = ConfigStore::new(dir.path());
        // act
        let err = store.load_settings().unwrap_err();
        // assert
        assert!(matches!(err, StoreError::Parse { .. }), "got: {err:?}");
    }

    #[test]
    fn a_settings_file_that_sets_one_key_leaves_every_other_field_at_default() {
        // arrange
        let dir = TempDir::new("settings-partial");
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(dir.path().join("settings.toml"), "retry_timeout_secs = 90").unwrap();
        let store = ConfigStore::new(dir.path());
        // act
        let settings = store.load_settings().unwrap();
        // assert
        assert_eq!(settings.retry_timeout_secs, 90);
        assert!(settings.audio_follows_screen);
        assert_eq!(settings.preferred_encoder, None);
        assert_eq!(settings.bitrate_cap_kbps, None);
    }

    #[test]
    fn saving_leaves_no_temp_file_behind() {
        // arrange
        let dir = TempDir::new("settings-no-temp");
        let store = ConfigStore::new(dir.path());
        // act
        store.save_settings(&Settings::default()).unwrap();
        // assert
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "got: {leftovers:?}");
    }

    #[test]
    fn the_store_directory_is_not_created_just_by_naming_it() {
        // arrange
        let dir = TempDir::new("settings-lazy-dir");
        // act
        let store = ConfigStore::new(dir.path());
        // assert
        assert_eq!(store.dir(), dir.path());
        assert!(!dir.path().exists());
    }

    #[test]
    fn xdg_config_home_wins_when_it_is_set() {
        // act
        let dir = resolve_dir(Some(OsStr::new("/x/cfg")), Some(OsStr::new("/h/user"))).unwrap();
        // assert
        assert_eq!(dir, PathBuf::from("/x/cfg/glint"));
    }

    #[test]
    fn home_provides_the_fallback_when_xdg_config_home_is_unset() {
        // act
        let dir = resolve_dir(None, Some(OsStr::new("/h/user"))).unwrap();
        // assert
        assert_eq!(dir, PathBuf::from("/h/user/.config/glint"));
    }

    #[test]
    fn an_empty_xdg_config_home_falls_back_to_home() {
        // arrange
        // The XDG base directory spec says an empty value counts as unset.
        // act
        let dir = resolve_dir(Some(OsStr::new("")), Some(OsStr::new("/h/user"))).unwrap();
        // assert
        assert_eq!(dir, PathBuf::from("/h/user/.config/glint"));
    }

    #[test]
    fn resolving_with_neither_variable_set_is_an_error() {
        // act
        let err = resolve_dir(None, None).unwrap_err();
        // assert
        assert!(matches!(err, StoreError::NoHome), "got: {err:?}");
    }

    #[test]
    fn an_atomic_write_replaces_an_existing_file() {
        // arrange
        let dir = TempDir::new("atomic-replace");
        fs::create_dir_all(dir.path()).unwrap();
        let target = dir.path().join("settings.toml");
        fs::write(&target, "old").unwrap();
        // act
        write_atomically(&target, "new").unwrap();
        // assert
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    }
}
