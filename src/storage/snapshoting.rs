use actix_web::web::Data;
use serde_json;
use std::{env, io, path::PathBuf, time::SystemTime};
use tokio::{fs, time::{self, Duration}};
use crate::storage::engine::{current_timestamp, AppState};

/// Clean up old snapshot files in `dir`, keeping only the newest `max_snapshots`.
async fn clean_old_snapshots(dir: &PathBuf, max_snapshots: usize) -> io::Result<()> {
    let mut rd = fs::read_dir(dir).await?;
    let mut snaps: Vec<(SystemTime, PathBuf)> = Vec::new();

    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("snapshot_") && name.ends_with(".json") {
                if let Ok(meta) = entry.metadata().await {
                    if let Ok(m) = meta.modified() {
                        snaps.push((m, path));
                    }
                }
            }
        }
    }

    snaps.sort_by_key(|(m, _)| *m);
    if snaps.len() > max_snapshots {
        for (_, old) in snaps.clone().into_iter().take(snaps.len() - max_snapshots) {
            let _ = fs::remove_file(old).await;
        }
    }
    Ok(())
}

/// Spawn a background task that every 60minutes:
/// 1. JSON‑serialize the in‑memory store to `./snapshots/snapshot_<ts>.json`
/// 2. Clean up old snapshots, keeping only `SNAP_LIMIT` (env var, default 100)
pub fn start_snapshot_task(state: Data<AppState>) {
    // Read SNAP_LIMIT from env or default to 100
    let snap_limit = env::var("SNAP_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let snapshot_dir = PathBuf::from("./snapshots");

    tokio::spawn(async move {
        if let Err(e) = fs::create_dir_all(&snapshot_dir).await {
            eprintln!("snapshot: could not create dir {:?}: {}", snapshot_dir, e);
            return;
        }

        let mut ticker = time::interval(Duration::from_secs(60 * 60));
        loop {
            ticker.tick().await;
            let ts = current_timestamp();
            let file = snapshot_dir.join(format!("snapshot_{}.json", ts));

            // 1) grab read-lock, serialize to JSON, write to disk
            let store = state.store.read().await;
            match serde_json::to_vec_pretty(&*store) {
                Ok(buf) => {
                    if let Err(e) = fs::write(&file, &buf).await {
                        eprintln!("snapshot: write failed {}: {}", file.display(), e);
                    }
                }
                Err(e) => {
                    eprintln!("snapshot: serialization error: {}", e);
                }
            }
            // 2) always cleanup old snapshots
            if let Err(e) = clean_old_snapshots(&snapshot_dir, snap_limit).await {
                eprintln!("snapshot: cleanup failed: {}", e);
            }
        }
    });
}
