//! Helpers for root-only state files: directories are 0700, files are 0600 and
//! written atomically (temp file + fsync + rename).

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

use serde::{Serialize, de::DeserializeOwned};

/// Creates `dir` (and parents) if needed and makes sure it is 0700.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let meta = fs::metadata(dir)?;
    if meta.permissions().mode() & 0o077 != 0 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Reads and parses a JSON file, returning `None` if it does not exist.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Atomically replaces `path` with the JSON encoding of `value`, mode 0600.
pub fn write_json_private<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        // `mode` only applies on creation; enforce it for a stale temp file.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&json)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        if let Some(parent) = path.parent() {
            // Persist the rename itself. Not all filesystems support fsync on
            // directories, so this is best effort.
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn round_trip_with_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        ensure_private_dir(&state).unwrap();
        assert_eq!(
            fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o700
        );

        let path = state.join("data.json");
        assert!(read_json::<BTreeMap<String, u32>>(&path).unwrap().is_none());

        let value = BTreeMap::from([("a".to_owned(), 1u32)]);
        write_json_private(&path, &value).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            read_json::<BTreeMap<String, u32>>(&path).unwrap(),
            Some(value)
        );

        // No temp files left behind.
        let entries: Vec<_> = fs::read_dir(&state).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn tightens_loose_directory_permissions() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(dir.path()).unwrap();
        assert_eq!(
            fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn corrupt_json_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, b"{not json").unwrap();
        let err = read_json::<BTreeMap<String, u32>>(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
