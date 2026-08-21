use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

pub(crate) const DEFAULT_TREE_PANE_PERCENT: u16 = 58;
const STATE_VERSION: u8 = 1;
const STATE_FILE_NAME: &str = "ui-state.json";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct UiState {
    version: u8,
    pub(crate) tree_pane_percent: u16,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            tree_pane_percent: DEFAULT_TREE_PANE_PERCENT,
        }
    }
}

pub(crate) fn state_path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "salishdev", "jex").map(|directories| {
        directories
            .state_dir()
            .unwrap_or_else(|| directories.data_local_dir())
            .join(STATE_FILE_NAME)
    })
}

pub(crate) fn load_or_default(path: &Path) -> UiState {
    load(path).unwrap_or_default()
}

fn load(path: &Path) -> Option<UiState> {
    let state: UiState = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (state.version == STATE_VERSION && (1..=99).contains(&state.tree_pane_percent)).then_some(state)
}

pub(crate) fn save(path: &Path, tree_pane_percent: u16) -> io::Result<()> {
    if !(1..=99).contains(&tree_pane_percent) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tree pane percentage must be between 1 and 99",
        ));
    }

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "UI state path has no parent")
    })?;
    fs::create_dir_all(parent)?;

    let state = UiState {
        version: STATE_VERSION,
        tree_pane_percent,
    };
    let mut temporary = NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, &state).map_err(io::Error::other)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_malformed_state_uses_the_default() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(STATE_FILE_NAME);

        assert_eq!(load_or_default(&path), UiState::default());

        fs::write(&path, "not json").unwrap();
        assert_eq!(load_or_default(&path), UiState::default());
    }

    #[test]
    fn unsupported_or_invalid_state_uses_the_default() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(STATE_FILE_NAME);

        fs::write(&path, r#"{"version":2,"tree_pane_percent":70}"#).unwrap();
        assert_eq!(load_or_default(&path), UiState::default());

        fs::write(&path, r#"{"version":1,"tree_pane_percent":100}"#).unwrap();
        assert_eq!(load_or_default(&path), UiState::default());
    }

    #[test]
    fn saved_state_round_trips_and_replaces_the_previous_value() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join(STATE_FILE_NAME);

        save(&path, 64).unwrap();
        assert_eq!(load_or_default(&path).tree_pane_percent, 64);

        save(&path, 71).unwrap();
        assert_eq!(load_or_default(&path).tree_pane_percent, 71);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn save_rejects_an_invalid_percentage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(STATE_FILE_NAME);

        let error = save(&path, 0).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!path.exists());
    }
}
