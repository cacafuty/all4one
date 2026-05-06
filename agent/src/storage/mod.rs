// Storage module for distributed object storage
// Supports chunking, erasure coding, compression, and tiered replication

pub mod chunks;
pub mod index;
pub mod placement;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Policy for object storage tiering
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum StoragePolicy {
    /// Hot: 3x replication, no compression
    Hot,
    /// Warm: RS(4,2) erasure, zstd level 3
    Warm,
    /// Cold: RS(6,3) erasure, zstd level 19
    Cold,
    /// Archive: RS(8,4) erasure, zstd level 22
    Archive,
}

impl Default for StoragePolicy {
    fn default() -> Self {
        StoragePolicy::Warm
    }
}

impl StoragePolicy {
    pub fn replicas(&self) -> usize {
        match self {
            StoragePolicy::Hot => 3,
            _ => 1,
        }
    }

    pub fn erasure_coding(&self) -> Option<(usize, usize)> {
        match self {
            StoragePolicy::Hot => None,
            StoragePolicy::Warm => Some((4, 2)),   // 4 data, 2 parity
            StoragePolicy::Cold => Some((6, 3)),   // 6 data, 3 parity
            StoragePolicy::Archive => Some((8, 4)), // 8 data, 4 parity
        }
    }

    pub fn zstd_level(&self) -> i32 {
        match self {
            StoragePolicy::Hot => 0,    // No compression
            StoragePolicy::Warm => 3,
            StoragePolicy::Cold => 19,
            StoragePolicy::Archive => 22,
        }
    }
}

/// Metadata about an object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMetadata {
    pub bucket: String,
    pub key: String,
    pub size_bytes: u64,
    pub created_at: String,
    pub modified_at: String,
    pub etag: String,
    pub policy: String, // "hot", "warm", "cold", "archive"
    pub replicas: usize,
}

/// Initialize storage system with given data directory
pub async fn init_storage(data_dir: impl AsRef<Path>) -> Result<()> {
    index::init_index(data_dir.as_ref()).await?;
    Ok(())
}

/// Store object chunks (MVP: just stores whole object for now)
pub async fn put_object(
    data_dir: impl AsRef<Path>,
    bucket: &str,
    key: &str,
    data: &[u8],
    policy: StoragePolicy,
) -> Result<ObjectMetadata> {
    let data_dir = data_dir.as_ref();
    chunks::put_chunks(data_dir, bucket, key, data, policy).await
}

/// Retrieve object chunks
pub async fn get_object(
    data_dir: impl AsRef<Path>,
    bucket: &str,
    key: &str,
) -> Result<Vec<u8>> {
    let data_dir = data_dir.as_ref();
    chunks::get_chunks(data_dir, bucket, key).await
}

/// Delete object
pub async fn delete_object(
    data_dir: impl AsRef<Path>,
    bucket: &str,
    key: &str,
) -> Result<()> {
    let data_dir = data_dir.as_ref();
    index::delete_object(data_dir, bucket, key).await
}

/// List objects in bucket with pagination
pub async fn list_objects(
    data_dir: impl AsRef<Path>,
    bucket: &str,
    prefix: Option<&str>,
    max_keys: Option<usize>,
) -> Result<Vec<ObjectMetadata>> {
    let data_dir = data_dir.as_ref();
    index::list_objects(data_dir, bucket, prefix, max_keys).await
}

/// Read all shards for a stored object and return (shard_index, shard_hash, shard_data).
/// Used by the REST layer to fan-out shards to peer storage nodes after a local write.
pub async fn read_shards(
    data_dir: impl AsRef<Path>,
    bucket: &str,
    key: &str,
) -> Result<Vec<(u32, String, Vec<u8>)>> {
    let data_dir = data_dir.as_ref();
    let meta = index::get_object(data_dir, bucket, key).await?;

    let total_shards: u32 = match meta.policy.as_str() {
        "hot"     => 1,
        "warm"    => 6,  // 4 data + 2 parity
        "cold"    => 9,  // 6 data + 3 parity
        "archive" => 12, // 8 data + 4 parity
        _         => 1,
    };

    let safe_key = key.replace('/', "-");
    let object_id = format!("{}-{}", bucket, safe_key);
    let chunks_dir = data_dir.join("chunks").join(bucket);

    let mut result = Vec::new();
    for i in 0..total_shards {
        let shard_id = format!("{}-shard-{}", object_id, i);
        let shard_path = chunks_dir.join(&shard_id);
        if shard_path.exists() {
            let data = std::fs::read(&shard_path)?;
            let hash = chunks::sha256(&data);
            result.push((i, hash, data));
        }
    }
    Ok(result)
}
