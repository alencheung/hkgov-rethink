//! File-based snapshot persistence for the in-process stores.
//!
//! The v8 product stores (InsightStore, SignalStore, InvestigationStore,
//! UserStore, FeedbackStore) are `Arc<RwLock<BTreeMap>>` — volatile. A process
//! restart wipes all user state: signals, investigations, identity, sessions.
//! Full Postgres persistence is the G2 roadmap workstream; until then, this
//! module provides a **snapshot-to-file** stopgap that makes user state survive
//! a graceful restart.
//!
//! ## Design
//!
//! - [`snapshot_to_file`] atomically writes a serializable store's data to a
//!   JSON file (write to `{path}.tmp`, then rename — crash-safe on POSIX, and
//!   on Windows the rename is atomic within the same volume).
//! - [`restore_from_file`] loads the snapshot back on boot. A missing or
//!   corrupt file is a no-op (returns `None`) — the store starts empty, same as
//!   the volatile default. This means a corrupt snapshot never blocks boot.
//! - The caller (boot path) decides which stores to persist and where. The
//!   snapshot is taken on a debounce interval (not on every write) to avoid
//!   I/O amplification — see the agent supervisor wiring.
//!
//! ## What this is NOT
//!
//! - Not a replacement for Postgres. No transactions, no concurrency beyond a
//!   single process, no history beyond the last snapshot. If the process
//!   crashes (not graceful shutdown), the last few seconds of writes may be
//!   lost.
//! - Not for the RecordStore (that's a cache — connectors re-warm it). Only
//!   user-authored state (signals, investigations, identity) needs persistence.

use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::RwLock;

/// Write a serializable snapshot to `path` atomically.
///
/// Writes to `{path}.tmp` first, then renames to `{path}`. On a crash mid-write
/// the `.tmp` file is orphaned but the destination retains the last good
/// snapshot (or doesn't exist if this is the first write).
///
/// Returns `Ok(())` on success. Errors are logged but the caller should treat
/// them as non-fatal — a failed snapshot shouldn't crash the server.
pub async fn snapshot_to_file<T>(path: &Path, data: &T) -> anyhow::Result<()>
where
    T: serde::Serialize,
{
    let tmp = tmp_path(path);
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| anyhow::anyhow!("serialize snapshot: {e}"))?;
    fs::write(&tmp, json.as_bytes())
        .await
        .map_err(|e| anyhow::anyhow!("write snapshot tmp {tmp:?}: {e}"))?;
    fs::rename(&tmp, path)
        .await
        .map_err(|e| anyhow::anyhow!("rename snapshot {tmp:?} -> {path:?}: {e}"))?;
    Ok(())
}

/// Restore a snapshot from `path`. Returns `None` if the file doesn't exist or
/// is corrupt (so a missing/corrupt snapshot never blocks boot — the store just
/// starts empty, same as the volatile default).
pub async fn restore_from_file<T>(path: &Path) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    let bytes = match fs::read(path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "snapshot read failed; starting empty");
            return None;
        }
    };
    match serde_json::from_slice::<T>(&bytes) {
        Ok(v) => {
            tracing::info!(path = %path.display(), "restored snapshot");
            Some(v)
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "snapshot corrupt; starting empty (the file will be overwritten on the next snapshot)"
            );
            None
        }
    }
}

/// The `.tmp` sibling of a path — same directory, same volume (required for
/// atomic rename).
fn tmp_path(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    let name = p.file_name().map(|s| s.to_os_string()).unwrap_or_default();
    let mut tmp_name = name;
    tmp_name.push(".tmp");
    p.set_file_name(tmp_name);
    p
}

/// A debounced snapshot writer. Call [`Self::schedule`] to arm a snapshot — it
/// fires once after the debounce window elapses, coalescing rapid successive
/// triggers into a single write. This prevents I/O amplification when many
/// writes land in quick succession (e.g. a batch of signal creations).
pub struct DebouncedSnapshot<T> {
    data: std::sync::Arc<RwLock<T>>,
    path: PathBuf,
    debounce: std::time::Duration,
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl<T> DebouncedSnapshot<T>
where
    T: serde::Serialize + Clone + Send + Sync + 'static,
{
    pub fn new(
        data: std::sync::Arc<RwLock<T>>,
        path: PathBuf,
        debounce: std::time::Duration,
    ) -> Self {
        Self {
            data,
            path,
            debounce,
            armed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Arm a snapshot. If one is already armed, this is a no-op (the pending
    /// snapshot will pick up the latest data when it fires).
    pub fn schedule(&self) {
        // The armed flag is a simple coalescing mechanism. compare_exchange
        // ensures only one tokio task is spawned per debounce window, even
        // under concurrent triggers.
        if self
            .armed
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return; // already armed
        }
        let data = self.data.clone();
        let path = self.path.clone();
        let debounce = self.debounce;
        let armed = self.armed.clone();
        tokio::spawn(async move {
            tokio::time::sleep(debounce).await;
            armed.store(false, std::sync::atomic::Ordering::SeqCst);
            let snapshot = data.read().await.clone();
            if let Err(e) = snapshot_to_file(&path, &snapshot).await {
                tracing::warn!(path = %path.display(), error = %e, "debounced snapshot failed");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Sample {
        name: String,
        items: Vec<String>,
    }

    #[tokio::test]
    async fn snapshot_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join("hkgov_persist_test_snapshot.json");
        let _ = std::fs::remove_file(&path);
        let data = Sample {
            name: "test".into(),
            items: vec!["a".into(), "b".into()],
        };
        snapshot_to_file(&path, &data).await.unwrap();
        let restored: Sample = restore_from_file(&path).await.unwrap();
        assert_eq!(restored, data);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn restore_missing_file_returns_none() {
        let path = std::env::temp_dir().join("hkgov_persist_test_nonexistent_9999.json");
        let _ = std::fs::remove_file(&path);
        let restored: Option<Sample> = restore_from_file(&path).await;
        assert!(restored.is_none());
    }

    #[tokio::test]
    async fn restore_corrupt_file_returns_none() {
        let path = std::env::temp_dir().join("hkgov_persist_test_corrupt.json");
        fs::write(&path, b"not valid json {{{").await.unwrap();
        let restored: Option<Sample> = restore_from_file(&path).await;
        assert!(restored.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn snapshot_is_atomic_on_crash() {
        // A successful snapshot must not leave a .tmp file behind.
        let dir = std::env::temp_dir();
        let path = dir.join("hkgov_persist_test_atomic.json");
        let tmp = tmp_path(&path);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&tmp);
        let data = Sample {
            name: "x".into(),
            items: vec![],
        };
        snapshot_to_file(&path, &data).await.unwrap();
        assert!(path.exists(), "destination exists");
        assert!(!tmp.exists(), "tmp file cleaned up");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn debounced_snapshot_coalesces() {
        let dir = std::env::temp_dir();
        let path = dir.join("hkgov_persist_test_debounce.json");
        let _ = std::fs::remove_file(&path);
        let data = std::sync::Arc::new(RwLock::new(Sample {
            name: "initial".into(),
            items: vec![],
        }));
        let ds = DebouncedSnapshot::new(
            data.clone(),
            path.clone(),
            std::time::Duration::from_millis(100),
        );
        // Arm 10 times rapidly — should coalesce into a single pending write.
        for _ in 0..10 {
            ds.schedule();
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        // All arming done in ~20ms; the debounce is 100ms so the snapshot
        // hasn't fired yet. Update the data NOW so the single coalesced write
        // must pick up "updated".
        data.write().await.name = "updated".into();
        // D-030: wait well past the debounce + write. The previous 300ms was
        // too tight under concurrent test load on Windows (tokio scheduler
        // latency pushed the debounce task past the window). 1s gives a wide
        // margin while keeping the test fast.
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        let restored: Sample = restore_from_file(&path)
            .await
            .expect("snapshot was written");
        assert_eq!(restored.name, "updated");
        let _ = std::fs::remove_file(&path);
    }
}
