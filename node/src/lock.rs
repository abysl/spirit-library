use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const LOCK_FILE: &str = "lock";
pub const STALE_AFTER: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub struct StoreLock {
    path: PathBuf,
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(LOCK_FILE)
}

pub fn recorded(dir: &Path) -> Option<u32> {
    let path = path(dir);
    let metadata = std::fs::metadata(&path).ok()?;
    let age = SystemTime::now()
        .duration_since(metadata.modified().ok()?)
        .unwrap_or_default();
    if age > STALE_AFTER {
        return None;
    }
    std::fs::read_to_string(&path).ok()?.trim().parse().ok()
}

pub fn holder(dir: &Path) -> Option<u32> {
    recorded(dir).filter(|pid| alive(*pid))
}

pub fn alive(pid: u32) -> bool {
    pid == std::process::id() || liveness(pid).unwrap_or(true)
}

#[cfg(target_os = "linux")]
fn liveness(pid: u32) -> Option<bool> {
    let proc = Path::new("/proc");
    if !proc.join("self").exists() {
        return None;
    }
    Some(proc.join(pid.to_string()).exists())
}

#[cfg(not(target_os = "linux"))]
fn liveness(_pid: u32) -> Option<bool> {
    None
}

impl StoreLock {
    pub fn take(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        if let Some(pid) = recorded(dir) {
            if pid != std::process::id() {
                if alive(pid) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        format!(
                            "another process (pid {pid}) holds {}; one process per store",
                            dir.display()
                        ),
                    ));
                }
                eprintln!(
                    "info: taking over the lock on {} left by dead pid {pid}",
                    dir.display()
                );
            }
        }
        let path = path(dir);
        std::fs::write(&path, format!("{}\n", std::process::id()))?;
        Ok(Self { path })
    }

    pub fn touch(&self) {
        let _ = std::fs::write(&self.path, format!("{}\n", std::process::id()));
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        if recorded(self.path.parent().unwrap_or(&self.path)) == Some(std::process::id()) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spirit-lock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn our_own_pid_is_alive_and_relockable() {
        let dir = scratch("own");
        let lock = StoreLock::take(&dir).unwrap();
        assert_eq!(holder(&dir), Some(std::process::id()));
        assert!(alive(std::process::id()));
        let again = StoreLock::take(&dir).unwrap();
        assert_eq!(holder(&dir), Some(std::process::id()));
        drop(again);
        assert!(!path(&dir).exists());
        drop(lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_mtime_yields_even_when_the_pid_cannot_be_checked() {
        let dir = scratch("stale");
        let lock = StoreLock::take(&dir).unwrap();
        std::fs::write(path(&dir), format!("{}\n", std::process::id())).unwrap();
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(1);
        let file = std::fs::File::open(path(&dir)).unwrap();
        file.set_modified(old).unwrap();
        assert_eq!(recorded(&dir), None);
        assert_eq!(holder(&dir), None);
        assert!(StoreLock::take(&dir).is_ok());
        drop(lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_lock_left_by_a_dead_pid_is_taken_over() {
        let dir = scratch("dead");
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let reaped = child.id();
        child.wait().unwrap();
        std::fs::write(path(&dir), format!("{reaped}\n")).unwrap();
        assert_eq!(recorded(&dir), Some(reaped));
        assert_eq!(holder(&dir), None);
        let lock = StoreLock::take(&dir).unwrap();
        assert_eq!(holder(&dir), Some(std::process::id()));
        drop(lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_impossible_pid_is_dead() {
        let dir = scratch("impossible");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path(&dir), format!("{}\n", i32::MAX)).unwrap();
        assert!(!alive(i32::MAX as u32));
        assert_eq!(holder(&dir), None);
        let lock = StoreLock::take(&dir).unwrap();
        assert_eq!(holder(&dir), Some(std::process::id()));
        drop(lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_live_foreign_pid_is_refused() {
        let dir = scratch("live");
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        std::fs::write(path(&dir), format!("{}\n", child.id())).unwrap();
        assert!(alive(child.id()));
        assert_eq!(holder(&dir), Some(child.id()));
        let refused = StoreLock::take(&dir);
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(refused.is_err());
        assert_eq!(refused.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
