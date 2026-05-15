use crate::config::schema::Config;
use reqwest::{Client, Url};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::time::sleep;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileFingerprint {
    size_bytes: u64,
    modified_unix: u64,
}

#[derive(Clone, Debug)]
struct ScannedFile {
    key: String,
    path: PathBuf,
    fingerprint: FileFingerprint,
}

pub fn spawn_shared_volume_listener(config: Arc<Config>, local_rest_endpoint: String) {
    if !config.shared_volume.enabled {
        return;
    }

    if !config.roles.storage {
        eprintln!(
            "WARN Shared volume listener enabled but roles.storage=false; watcher disabled"
        );
        return;
    }

    let base_dir = config
        .shared_volume
        .local_dir
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&config.node.data_dir).join("shared"));
    let bucket = config.shared_volume.bucket.clone();
    let policy = config.shared_volume.policy.clone();
    let interval_seconds = config.shared_volume.scan_interval_seconds.max(1);

    if let Err(e) = std::fs::create_dir_all(&base_dir) {
        eprintln!(
            "WARN Failed to prepare shared volume directory {}: {}",
            base_dir.display(),
            e
        );
        return;
    }

    println!(
        "INFO Shared volume listener active dir={} bucket={} interval={}s",
        base_dir.display(),
        bucket,
        interval_seconds
    );

    tokio::spawn(async move {
        let client = Client::new();
        let mut previous: HashMap<String, FileFingerprint> = HashMap::new();

        loop {
            let scan = match scan_shared_dir(&base_dir) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("WARN Shared volume scan failed: {}", e);
                    sleep(Duration::from_secs(interval_seconds)).await;
                    continue;
                }
            };

            let current: HashMap<String, FileFingerprint> = scan
                .iter()
                .map(|f| (f.key.clone(), f.fingerprint.clone()))
                .collect();

            for file in &scan {
                let changed = previous
                    .get(&file.key)
                    .map(|prev| prev != &file.fingerprint)
                    .unwrap_or(true);

                if changed {
                    match tokio::fs::read(&file.path).await {
                        Ok(payload) => {
                            if let Err(e) = upload_object(
                                &client,
                                &local_rest_endpoint,
                                &bucket,
                                &file.key,
                                &policy,
                                payload,
                            )
                            .await
                            {
                                eprintln!(
                                    "WARN Shared volume upload failed key={} err={}",
                                    file.key, e
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "WARN Shared volume cannot read changed file path={} err={}",
                                file.path.display(),
                                e
                            );
                        }
                    }
                }
            }

            for removed_key in previous.keys().filter(|k| !current.contains_key(*k)) {
                if let Err(e) = delete_object(&client, &local_rest_endpoint, &bucket, removed_key).await {
                    eprintln!(
                        "WARN Shared volume delete sync failed key={} err={}",
                        removed_key, e
                    );
                }
            }

            previous = current;
            sleep(Duration::from_secs(interval_seconds)).await;
        }
    });
}

fn scan_shared_dir(base_dir: &Path) -> anyhow::Result<Vec<ScannedFile>> {
    let mut out = Vec::new();
    walk_dir(base_dir, base_dir, &mut out)?;
    Ok(out)
}

fn walk_dir(base_dir: &Path, current_dir: &Path, out: &mut Vec<ScannedFile>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(current_dir)? {
        let entry = entry?;
        let path = entry.path();
        let meta = entry.metadata()?;

        if meta.is_dir() {
            walk_dir(base_dir, &path, out)?;
            continue;
        }

        if !meta.is_file() {
            continue;
        }

        let rel = path
            .strip_prefix(base_dir)
            .map_err(|e| anyhow::anyhow!("relative-path failure: {}", e))?;
        let key = rel
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();
        if key.is_empty() {
            continue;
        }

        let modified_unix = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        out.push(ScannedFile {
            key,
            path,
            fingerprint: FileFingerprint {
                size_bytes: meta.len(),
                modified_unix,
            },
        });
    }

    Ok(())
}

fn storage_url(rest_endpoint: &str, bucket: &str, key: &str) -> anyhow::Result<Url> {
    let base = if rest_endpoint.starts_with("http://") || rest_endpoint.starts_with("https://") {
        rest_endpoint.to_string()
    } else {
        format!("http://{}", rest_endpoint)
    };
    let mut url = Url::parse(&base)?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("cannot set URL path segments"))?;
        segments.push("v1");
        segments.push("storage");
        segments.push(bucket);
        for segment in key.split('/') {
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
    }
    Ok(url)
}

async fn upload_object(
    client: &Client,
    rest_endpoint: &str,
    bucket: &str,
    key: &str,
    policy: &str,
    payload: Vec<u8>,
) -> anyhow::Result<()> {
    let url = storage_url(rest_endpoint, bucket, key)?;
    let response = client
        .put(url)
        .header("x-all4one-policy", policy)
        .body(payload)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "status={} while uploading key={}",
            response.status(),
            key
        ));
    }

    Ok(())
}

async fn delete_object(client: &Client, rest_endpoint: &str, bucket: &str, key: &str) -> anyhow::Result<()> {
    let url = storage_url(rest_endpoint, bucket, key)?;
    let response = client.delete(url).send().await?;
    if response.status().as_u16() == 404 {
        return Ok(());
    }
    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "status={} while deleting key={}",
            response.status(),
            key
        ));
    }
    Ok(())
}
