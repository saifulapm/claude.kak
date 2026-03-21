use crate::error;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct LockFile {
    pub path: PathBuf,
    pub auth_token: String,
}

impl LockFile {
    /// Create lock file using default config dir ($CLAUDE_CONFIG_DIR or ~/.claude)
    pub fn create(pid: u32, port: u16, workspace_folders: &[&str]) -> error::Result<Self> {
        Self::create_in(&config_dir(), pid, port, workspace_folders)
    }

    /// Create lock file in a specific base directory (testable, no env vars)
    pub fn create_in(base_dir: &Path, pid: u32, port: u16, workspace_folders: &[&str]) -> error::Result<Self> {
        let auth_token = Uuid::new_v4().to_string();
        let ide_dir = base_dir.join("ide");
        fs::create_dir_all(&ide_dir)?;

        let path = ide_dir.join(format!("{}.lock", port));
        let content = serde_json::json!({
            "pid": pid,
            "workspaceFolders": workspace_folders,
            "ideName": "Kakoune",
            "transport": "ws",
            "authToken": auth_token
        });
        fs::write(&path, serde_json::to_string_pretty(&content)?)?;

        Ok(Self { path, auth_token })
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        PathBuf::from(dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".claude")
    } else {
        PathBuf::from("/tmp/.claude")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lockfile_create_and_read() {
        let dir = TempDir::new().unwrap();
        let lf = LockFile::create_in(dir.path(), 12345, 9876, &["/tmp/project"]).unwrap();
        assert!(lf.path.exists());
        let contents: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&lf.path).unwrap()).unwrap();
        assert_eq!(contents["pid"], 12345);
        assert_eq!(contents["ideName"], "Kakoune");
        assert_eq!(contents["transport"], "ws");
        assert!(contents["authToken"].as_str().unwrap().len() > 10);
    }

    #[test]
    fn test_lockfile_cleanup_on_drop() {
        let dir = TempDir::new().unwrap();
        let path;
        {
            let lf = LockFile::create_in(dir.path(), 111, 8080, &["/tmp"]).unwrap();
            path = lf.path.clone();
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    #[test]
    fn test_auth_token_is_uuid() {
        let dir = TempDir::new().unwrap();
        let lf = LockFile::create_in(dir.path(), 1, 80, &["/tmp"]).unwrap();
        assert!(lf.auth_token.len() == 36); // UUID v4 format
    }
}
