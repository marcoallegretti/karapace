use crate::CoreError;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct StoreLock {
    lock_file: File,
}

impl StoreLock {
    pub fn acquire(lock_path: &Path) -> Result<Self, CoreError> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(lock_path)?;

        file.lock_exclusive()
            .map_err(|e| CoreError::Io(std::io::Error::new(std::io::ErrorKind::WouldBlock, e)))?;

        Ok(Self { lock_file: file })
    }

    pub fn try_acquire(lock_path: &Path) -> Result<Option<Self>, CoreError> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(lock_path)?;

        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { lock_file: file })),
            Err(_) => Ok(None),
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
    }
}

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn install_signal_handler() {
    let _ = ctrlc::set_handler(move || {
        if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            std::process::exit(1);
        }
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
        eprintln!("\nshutdown requested, finishing current operation...");
    });
}

pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_acquire_and_release() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("test.lock");

        {
            let _lock = StoreLock::acquire(&lock_path).unwrap();
            assert!(lock_path.exists());
        }
    }

    #[test]
    fn try_acquire_returns_none_when_held() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("test.lock");

        let _lock = StoreLock::acquire(&lock_path).unwrap();
        let result = StoreLock::try_acquire(&lock_path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn lock_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("test.lock");

        {
            let _lock = StoreLock::acquire(&lock_path).unwrap();
        }

        let lock2 = StoreLock::try_acquire(&lock_path).unwrap();
        assert!(lock2.is_some());
    }
}
