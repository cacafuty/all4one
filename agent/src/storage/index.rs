use anyhow::{anyhow, Result};
use serde_json;
use sled::{Db, Tree};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use super::{ObjectMetadata, StoragePolicy};

static DB_CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Db>>>> = OnceLock::new();

fn db_cache() -> &'static Mutex<HashMap<PathBuf, Arc<Db>>> {
    DB_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Initialize sled-based storage index
pub async fn init_index(data_dir: &Path) -> Result<()> {
    let _db = get_db(data_dir)?;
    Ok(())
}

/// Get sled database handle
fn get_db(data_dir: &Path) -> Result<Arc<Db>> {
    let db_path = data_dir.join("objects.db");

    {
        let cache = db_cache()
            .lock()
            .map_err(|_| anyhow!("Failed to lock DB cache"))?;
        if let Some(db) = cache.get(&db_path) {
            return Ok(Arc::clone(db));
        }
    }

    let opened =
        Arc::new(sled::open(&db_path).map_err(|e| anyhow!("Failed to open index DB: {}", e))?);

    let mut cache = db_cache()
        .lock()
        .map_err(|_| anyhow!("Failed to lock DB cache"))?;
    let entry = cache.entry(db_path).or_insert_with(|| opened.clone());
    Ok(entry.clone())
}

/// Get bucket tree
fn get_bucket_tree(db: &Db, bucket: &str) -> Result<Tree> {
    db.open_tree(format!("bucket:{}", bucket))
        .map_err(|e| anyhow!("Failed to open bucket tree: {}", e))
}

/// Store object metadata
pub async fn put_object(
    data_dir: &Path,
    bucket: &str,
    key: &str,
    etag: &str,
    size_bytes: usize,
    policy: &StoragePolicy,
) -> Result<()> {
    let db = get_db(data_dir)?;
    let tree = get_bucket_tree(&db, bucket)?;

    let metadata = ObjectMetadata {
        bucket: bucket.to_string(),
        key: key.to_string(),
        size_bytes: size_bytes as u64,
        created_at: chrono::Utc::now().to_rfc3339(),
        modified_at: chrono::Utc::now().to_rfc3339(),
        etag: etag.to_string(),
        policy: format!("{:?}", policy).to_lowercase(),
        replicas: policy.replicas(),
    };

    let json = serde_json::to_string(&metadata)?;
    tree.insert(key.as_bytes(), json.as_bytes())?;

    Ok(())
}

/// Get object metadata
pub async fn get_object(data_dir: &Path, bucket: &str, key: &str) -> Result<ObjectMetadata> {
    let db = get_db(data_dir)?;
    let tree = get_bucket_tree(&db, bucket)?;

    let data = tree
        .get(key.as_bytes())?
        .ok_or_else(|| anyhow!("Object not found: {}/{}", bucket, key))?;

    let metadata: ObjectMetadata = serde_json::from_slice(&data)?;
    Ok(metadata)
}

/// Delete object metadata and chunks
pub async fn delete_object(data_dir: &Path, bucket: &str, key: &str) -> Result<()> {
    let db = get_db(data_dir)?;
    let tree = get_bucket_tree(&db, bucket)?;

    tree.remove(key.as_bytes())?;
    Ok(())
}

/// List objects in bucket
pub async fn list_objects(
    data_dir: &Path,
    bucket: &str,
    prefix: Option<&str>,
    max_keys: Option<usize>,
) -> Result<Vec<ObjectMetadata>> {
    let db = get_db(data_dir)?;
    let tree = get_bucket_tree(&db, bucket)?;

    let max = max_keys.unwrap_or(1000);
    let mut results = Vec::new();

    for item in tree.iter() {
        if results.len() >= max {
            break;
        }

        let (_key, value) = item?;
        let metadata: ObjectMetadata = serde_json::from_slice(&value)?;

        if let Some(p) = prefix {
            if !metadata.key.starts_with(p) {
                continue;
            }
        }

        results.push(metadata);
    }

    Ok(results)
}

/// Check if bucket exists
pub async fn bucket_exists(data_dir: &Path, bucket: &str) -> Result<bool> {
    let db = get_db(data_dir)?;
    Ok(db.open_tree(format!("bucket:{}", bucket)).is_ok())
}

/// Create bucket
pub async fn create_bucket(data_dir: &Path, bucket: &str) -> Result<()> {
    let db = get_db(data_dir)?;
    let _tree = db.open_tree(format!("bucket:{}", bucket))?;
    Ok(())
}

/// List all buckets
pub async fn list_buckets(data_dir: &Path) -> Result<Vec<String>> {
    let db = get_db(data_dir)?;
    let mut buckets = Vec::new();

    for tree_name in db.tree_names() {
        let name = String::from_utf8(tree_name.to_vec())?;
        if name.starts_with("bucket:") {
            buckets.push(name.strip_prefix("bucket:").unwrap().to_string());
        }
    }

    Ok(buckets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_init_index() -> Result<()> {
        let temp_dir = TempDir::new()?;
        init_index(temp_dir.path()).await?;

        let db_path = temp_dir.path().join("objects.db");
        assert!(db_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_put_get_object_metadata() -> Result<()> {
        let temp_dir = TempDir::new()?;
        init_index(temp_dir.path()).await?;

        put_object(
            temp_dir.path(),
            "test-bucket",
            "test-key",
            "abc123",
            1000,
            &StoragePolicy::Hot,
        )
        .await?;

        let metadata = get_object(temp_dir.path(), "test-bucket", "test-key").await?;
        assert_eq!(metadata.bucket, "test-bucket");
        assert_eq!(metadata.key, "test-key");
        assert_eq!(metadata.etag, "abc123");
        assert_eq!(metadata.size_bytes, 1000);
        assert_eq!(metadata.policy, "hot");
        assert_eq!(metadata.replicas, 3);
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_object() -> Result<()> {
        let temp_dir = TempDir::new()?;
        init_index(temp_dir.path()).await?;

        put_object(
            temp_dir.path(),
            "bucket",
            "key",
            "hash",
            100,
            &StoragePolicy::Warm,
        )
        .await?;

        delete_object(temp_dir.path(), "bucket", "key").await?;

        let result = get_object(temp_dir.path(), "bucket", "key").await;
        assert!(result.is_err(), "Object should not exist after deletion");
        Ok(())
    }

    #[tokio::test]
    async fn test_list_objects_pagination() -> Result<()> {
        let temp_dir = TempDir::new()?;
        init_index(temp_dir.path()).await?;

        // Add 5 objects
        for i in 0..5 {
            put_object(
                temp_dir.path(),
                "bucket",
                &format!("key{}", i),
                &format!("hash{}", i),
                100 * (i as usize),
                &StoragePolicy::Warm,
            )
            .await?;
        }

        let all_objects = list_objects(temp_dir.path(), "bucket", None, None).await?;
        assert_eq!(all_objects.len(), 5);

        let paginated = list_objects(temp_dir.path(), "bucket", None, Some(2)).await?;
        assert_eq!(paginated.len(), 2);

        Ok(())
    }

    #[tokio::test]
    async fn test_list_objects_with_prefix() -> Result<()> {
        let temp_dir = TempDir::new()?;
        init_index(temp_dir.path()).await?;

        put_object(
            temp_dir.path(),
            "bucket",
            "logs/app.log",
            "hash1",
            100,
            &StoragePolicy::Cold,
        )
        .await?;

        put_object(
            temp_dir.path(),
            "bucket",
            "logs/system.log",
            "hash2",
            200,
            &StoragePolicy::Cold,
        )
        .await?;

        put_object(
            temp_dir.path(),
            "bucket",
            "data/file.dat",
            "hash3",
            300,
            &StoragePolicy::Hot,
        )
        .await?;

        let logs = list_objects(temp_dir.path(), "bucket", Some("logs/"), None).await?;
        assert_eq!(logs.len(), 2);

        let data = list_objects(temp_dir.path(), "bucket", Some("data/"), None).await?;
        assert_eq!(data.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_create_bucket() -> Result<()> {
        let temp_dir = TempDir::new()?;
        init_index(temp_dir.path()).await?;

        create_bucket(temp_dir.path(), "my-bucket").await?;
        assert!(bucket_exists(temp_dir.path(), "my-bucket").await?);
        Ok(())
    }

    #[tokio::test]
    async fn test_list_buckets() -> Result<()> {
        let temp_dir = TempDir::new()?;
        init_index(temp_dir.path()).await?;

        create_bucket(temp_dir.path(), "bucket1").await?;
        create_bucket(temp_dir.path(), "bucket2").await?;
        create_bucket(temp_dir.path(), "bucket3").await?;

        let buckets = list_buckets(temp_dir.path()).await?;
        assert!(buckets.contains(&"bucket1".to_string()));
        assert!(buckets.contains(&"bucket2".to_string()));
        assert!(buckets.contains(&"bucket3".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_policies() -> Result<()> {
        let temp_dir = TempDir::new()?;
        init_index(temp_dir.path()).await?;

        put_object(
            temp_dir.path(),
            "bucket",
            "hot-object",
            "hash1",
            1000,
            &StoragePolicy::Hot,
        )
        .await?;

        put_object(
            temp_dir.path(),
            "bucket",
            "cold-object",
            "hash2",
            2000,
            &StoragePolicy::Cold,
        )
        .await?;

        let hot = get_object(temp_dir.path(), "bucket", "hot-object").await?;
        assert_eq!(hot.policy, "hot");
        assert_eq!(hot.replicas, 3);

        let cold = get_object(temp_dir.path(), "bucket", "cold-object").await?;
        assert_eq!(cold.policy, "cold");
        assert_eq!(cold.replicas, 1);

        Ok(())
    }
}
