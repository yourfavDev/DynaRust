use actix_web::web::Data;
use serde_json;
use std::{env, io, path::PathBuf, time::SystemTime};
use tokio::{fs, time::{self, Duration}};
use crate::storage::engine::{current_timestamp, AppState};
use crate::storage::persistance::encrypt;

/// Clean up old encrypted snapshot files in `dir`, keeping only the
/// newest `max_snapshots`.
async fn clean_old_snapshots(dir: &PathBuf, max_snapshots: usize)
                             -> io::Result<()>
{
    let mut rd = fs::read_dir(dir).await?;
    let mut snaps: Vec<(SystemTime, PathBuf)> = Vec::new();

    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("snapshot_")
                && name.ends_with(".json.enc")
            {
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
        let snaps_cloned = snaps.clone();
        for (_, old) in snaps
            .into_iter()
            .take(snaps_cloned.len() - max_snapshots)
        {
            let _ = fs::remove_file(old).await;
        }
    }
    Ok(())
}

/// Spawn a background task that every 60 minutes:
/// 1. JSON-serialize the in-memory store
/// 2. Encrypt it with `encrypt()`
/// 3. Write to `./snapshots/snapshot_<ts>.json.enc`
/// 4. Clean up old snapshots
pub fn start_snapshot_task(state: Data<AppState>) {
    let snap_limit = env::var("SNAP_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let snapshot_dir = PathBuf::from("./snapshots");

    tokio::spawn(async move {
        if let Err(e) = fs::create_dir_all(&snapshot_dir).await {
            eprintln!(
                "snapshot: could not create dir {:?}: {}",
                snapshot_dir, e
            );
            return;
        }

        let mut ticker = time::interval(Duration::from_secs(60 * 60));
        loop {
            ticker.tick().await;
            let ts = current_timestamp();
            let file = snapshot_dir.join(
                format!("snapshot_{}.json.enc", ts)
            );

            let store = state.store.read().await;
            match serde_json::to_string_pretty(&*store) {
                Ok(plain) => match encrypt(&plain) {
                    Ok(enc_b64) => {
                        if let Err(e) = fs::write(&file, enc_b64.as_bytes())
                            .await
                        {
                            eprintln!(
                                "snapshot: write failed {}: {}",
                                file.display(),
                                e
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("snapshot: encryption error: {}", e);
                    }
                },
                Err(e) => {
                    eprintln!("snapshot: serialization error: {}", e);
                }
            }

            if let Err(e) = clean_old_snapshots(&snapshot_dir, snap_limit)
                .await
            {
                eprintln!("snapshot: cleanup failed: {}", e);
            }
        }
    });
}
