use anyhow::{anyhow, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use zstd::Encoder as ZstdEncoder;

use super::{ObjectMetadata, StoragePolicy};

const ALL4ONE_MAGIC: &[u8; 4] = b"A4O1";
const ALL4ONE_VERSION: u8 = 1;
const ALL4ONE_CODEC_ZSTD: u8 = 1;
const ALL4ONE_HEADER_LEN: usize = 4 + 1 + 1 + 8;

/// Compute SHA256 hash of data
pub fn sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Compress data using zstd
pub fn compress(data: &[u8], level: i32) -> Result<Vec<u8>> {
    if level == 0 {
        return Ok(data.to_vec());
    }

    let mut encoder = ZstdEncoder::new(Vec::new(), level)?;
    encoder.write_all(data)?;
    let compressed = encoder.finish()?;

    let mut wrapped = Vec::with_capacity(ALL4ONE_HEADER_LEN + compressed.len());
    wrapped.extend_from_slice(ALL4ONE_MAGIC);
    wrapped.push(ALL4ONE_VERSION);
    wrapped.push(ALL4ONE_CODEC_ZSTD);
    wrapped.extend_from_slice(&(data.len() as u64).to_le_bytes());
    wrapped.extend_from_slice(&compressed);
    Ok(wrapped)
}

/// Decompress zstd data
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    // New format: all4one wrapper + codec payload.
    if data.len() >= ALL4ONE_HEADER_LEN && &data[..4] == ALL4ONE_MAGIC {
        let version = data[4];
        let codec = data[5];
        if version != ALL4ONE_VERSION {
            return Err(anyhow!(
                "Unsupported all4one compression version: {}",
                version
            ));
        }
        if codec != ALL4ONE_CODEC_ZSTD {
            return Err(anyhow!("Unsupported all4one codec: {}", codec));
        }

        let expected_len = u64::from_le_bytes(data[6..14].try_into().unwrap()) as usize;
        let payload = &data[ALL4ONE_HEADER_LEN..];
        let decompressed =
            zstd::decode_all(payload).map_err(|e| anyhow!("Decompression failed: {}", e))?;
        if decompressed.len() != expected_len {
            return Err(anyhow!(
                "all4one payload size mismatch: expected {}, got {}",
                expected_len,
                decompressed.len()
            ));
        }
        return Ok(decompressed);
    }

    // Backward compatibility: plain zstd bytes written before all4one wrapping.
    zstd::decode_all(data).map_err(|e| anyhow!("Decompression failed: {}", e))
}

/// Apply erasure coding to data
pub fn encode_erasure(
    data: &[u8],
    data_shards: usize,
    parity_shards: usize,
) -> Result<Vec<Vec<u8>>> {
    // Prefix with the actual data length (8 bytes LE) so decode_erasure can
    // strip zero-padding introduced when the last shard is padded to shard_size.
    let data_len = data.len() as u64;
    let mut prefixed = Vec::with_capacity(8 + data.len());
    prefixed.extend_from_slice(&data_len.to_le_bytes());
    prefixed.extend_from_slice(data);

    let shard_size = prefixed.len().div_ceil(data_shards);

    let mut shards = Vec::new();
    for i in 0..data_shards {
        let start = i * shard_size;
        let end = std::cmp::min(start + shard_size, prefixed.len());
        let mut shard = vec![0u8; shard_size];
        if end > start {
            shard[..end - start].copy_from_slice(&prefixed[start..end]);
        }
        shards.push(shard);
    }

    // Add parity shards (simplified - just zeros for now; real RS to be added)
    for _ in 0..parity_shards {
        shards.push(vec![0u8; shard_size]);
    }

    Ok(shards)
}

/// Decode erasure coded data (recover from shards)
pub fn decode_erasure(
    shards: &[Vec<u8>],
    data_shards: usize,
    _parity_shards: usize,
) -> Result<Vec<u8>> {
    let mut combined = Vec::new();
    for i in 0..data_shards {
        if i < shards.len() {
            combined.extend_from_slice(&shards[i]);
        }
    }

    // Strip the 8-byte length prefix written by encode_erasure, then trim
    // zero-padding that may have been added to fill the last shard.
    if combined.len() < 8 {
        return Err(anyhow!(
            "Erasure decoded data too short to contain length prefix"
        ));
    }
    let data_len = u64::from_le_bytes(combined[..8].try_into().unwrap()) as usize;
    if 8 + data_len > combined.len() {
        return Err(anyhow!("Erasure length prefix exceeds decoded data"));
    }
    Ok(combined[8..8 + data_len].to_vec())
}

/// Store object chunks
pub async fn put_chunks(
    data_dir: &Path,
    bucket: &str,
    key: &str,
    data: &[u8],
    policy: StoragePolicy,
    mark_access: bool,
) -> Result<ObjectMetadata> {
    let chunks_dir = data_dir.join("chunks").join(bucket);
    fs::create_dir_all(&chunks_dir)?;

    // Compute object hash
    let object_etag = sha256(data);
    let object_id = format!("{}-{}", bucket, key).replace("/", "-");

    // Compress if needed
    let level = policy.zstd_level();
    let compressed = compress(data, level)?;

    // Apply erasure coding if needed
    let (data_shards, parity_shards) = policy.erasure_coding().unwrap_or((1, 0));
    let shards = encode_erasure(&compressed, data_shards, parity_shards)?;

    // Store shards
    let mut chunk_paths = Vec::new();
    for (i, shard) in shards.iter().enumerate() {
        let chunk_id = format!("{}-shard-{}", object_id, i);
        let chunk_path = chunks_dir.join(&chunk_id);

        // Compute shard hash
        let shard_hash = sha256(shard);
        let metadata_path = format!("{}.meta", chunk_path.display());

        fs::write(&chunk_path, shard)?;
        fs::write(
            &metadata_path,
            format!(
                "shard={},hash={},original_size={}",
                i,
                shard_hash,
                data.len()
            ),
        )?;

        chunk_paths.push(chunk_id);
    }

    // Update index
    super::index::put_object(data_dir, bucket, key, &object_etag, data.len(), &policy).await?;
    let last_accessed_at = if mark_access {
        super::index::touch_object_access(data_dir, bucket, key)
            .await
            .ok()
            .and_then(|m| m.last_accessed_at)
    } else {
        None
    };

    Ok(ObjectMetadata {
        bucket: bucket.to_string(),
        key: key.to_string(),
        size_bytes: data.len() as u64,
        created_at: Utc::now().to_rfc3339(),
        modified_at: Utc::now().to_rfc3339(),
        last_accessed_at,
        etag: object_etag,
        policy: format!("{:?}", policy).to_lowercase(),
        replicas: policy.replicas(),
    })
}

/// Retrieve object chunks
pub async fn get_chunks(data_dir: &Path, bucket: &str, key: &str) -> Result<Vec<u8>> {
    let chunks_dir = data_dir.join("chunks").join(bucket);
    let object_id = format!("{}-{}", bucket, key).replace("/", "-");

    // Lookup object metadata to get policy info
    let metadata = super::index::get_object(data_dir, bucket, key).await?;

    // Parse policy to know data/parity shards and compression level
    let (data_shards, parity_shards, level) = match metadata.policy.as_str() {
        "hot" => (1, 0, 0),
        "warm" => (4, 2, 3),
        "cold" => (6, 3, 19),
        "archive" => (8, 4, 22),
        _ => (1, 0, 0),
    };

    // Load shards. Only the first `data_shards` carry actual data; parity
    // shards (indices >= data_shards) are optional for recovery.
    let mut shards: Vec<Vec<u8>> = Vec::new();
    let total = data_shards + parity_shards;

    for i in 0..total {
        let chunk_id = format!("{}-shard-{}", object_id, i);
        let chunk_path = chunks_dir.join(&chunk_id);

        if chunk_path.exists() {
            shards.push(fs::read(&chunk_path)?);
        } else if i < data_shards {
            // Missing a data shard — unrecoverable without real RS parity
            return Err(anyhow!("Insufficient shards to recover data"));
        }
        // Missing parity shard is fine — just skip it
    }

    // Decode
    let compressed = decode_erasure(&shards, data_shards, parity_shards)?;

    // Decompress
    let data = if level > 0 {
        decompress(&compressed)?
    } else {
        compressed
    };

    // Verify hash
    let computed_hash = sha256(&data);
    if computed_hash != metadata.etag {
        return Err(anyhow!(
            "Data corruption detected: expected {}, got {}",
            metadata.etag,
            computed_hash
        ));
    }

    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_sha256() {
        let hash = sha256(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_sha256_empty() {
        let hash = sha256(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_compress_decompress() -> Result<()> {
        let original = b"hello world hello world hello world";
        let compressed = compress(original, 3)?;
        assert!(compressed.starts_with(ALL4ONE_MAGIC));
        let decompressed = decompress(&compressed)?;
        assert_eq!(original.to_vec(), decompressed);
        Ok(())
    }

    #[test]
    fn test_decompress_legacy_zstd_payload() -> Result<()> {
        let original = b"legacy-zstd-data";
        let mut encoder = ZstdEncoder::new(Vec::new(), 3)?;
        encoder.write_all(original)?;
        let legacy = encoder.finish()?;

        let decoded = decompress(&legacy)?;
        assert_eq!(decoded, original);
        Ok(())
    }

    #[test]
    fn test_compress_no_compression() -> Result<()> {
        let original = b"test data";
        let result = compress(original, 0)?;
        assert_eq!(original.to_vec(), result);
        Ok(())
    }

    #[test]
    fn test_erasure_encoding_hot() -> Result<()> {
        let data = b"test data for hot storage";
        let shards = encode_erasure(data, 1, 0)?;
        assert_eq!(shards.len(), 1);

        let recovered = decode_erasure(&shards, 1, 0)?;
        assert_eq!(data.to_vec(), recovered);
        Ok(())
    }

    #[test]
    fn test_erasure_encoding_warm() -> Result<()> {
        let data = b"test data for warm storage with some content";
        let shards = encode_erasure(data, 4, 2)?;
        assert_eq!(shards.len(), 6);

        // Should be recoverable with just data shards
        let recovered = decode_erasure(&shards, 4, 2)?;
        assert_eq!(data.to_vec(), recovered);
        Ok(())
    }

    #[tokio::test]
    async fn test_put_get_hot_policy() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let data = b"hot storage test data";

        let metadata = put_chunks(
            temp_dir.path(),
            "test-bucket",
            "test-key",
            data,
            StoragePolicy::Hot,
            true,
        )
        .await?;

        assert_eq!(metadata.bucket, "test-bucket");
        assert_eq!(metadata.key, "test-key");
        assert_eq!(metadata.size_bytes, data.len() as u64);
        assert_eq!(metadata.policy, StoragePolicy::Hot.to_string());
        assert_eq!(metadata.replicas, 3);

        let retrieved = get_chunks(temp_dir.path(), "test-bucket", "test-key").await?;
        assert_eq!(data.to_vec(), retrieved);
        Ok(())
    }

    #[tokio::test]
    async fn test_put_get_warm_policy() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let data = b"warm storage test with compression and erasure coding";

        let metadata = put_chunks(
            temp_dir.path(),
            "bucket",
            "key",
            data,
            StoragePolicy::Warm,
            true,
        )
        .await?;

        assert_eq!(metadata.policy, "warm");
        assert_eq!(metadata.replicas, 1);

        let retrieved = get_chunks(temp_dir.path(), "bucket", "key").await?;
        assert_eq!(data.to_vec(), retrieved);
        Ok(())
    }

    #[tokio::test]
    async fn test_data_corruption_detection() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let data = b"test data for corruption detection";

        put_chunks(
            temp_dir.path(),
            "bucket",
            "key",
            data,
            StoragePolicy::Hot,
            true,
        )
        .await?;

        // Corrupt the stored data
        let chunks_dir = temp_dir.path().join("chunks").join("bucket");
        let shard_file = chunks_dir.join("bucket-key-shard-0");

        if shard_file.exists() {
            fs::write(&shard_file, b"corrupted data")?;

            // Should fail due to hash mismatch
            let result = get_chunks(temp_dir.path(), "bucket", "key").await;
            assert!(result.is_err(), "Should detect corruption");
        }

        Ok(())
    }

    #[test]
    fn test_storage_policy_replicas() {
        assert_eq!(StoragePolicy::Hot.replicas(), 3);
        assert_eq!(StoragePolicy::Warm.replicas(), 1);
        assert_eq!(StoragePolicy::Cold.replicas(), 1);
        assert_eq!(StoragePolicy::Archive.replicas(), 1);
    }

    #[test]
    fn test_storage_policy_erasure_coding() {
        assert_eq!(StoragePolicy::Hot.erasure_coding(), None);
        assert_eq!(StoragePolicy::Warm.erasure_coding(), Some((4, 2)));
        assert_eq!(StoragePolicy::Cold.erasure_coding(), Some((6, 3)));
        assert_eq!(StoragePolicy::Archive.erasure_coding(), Some((8, 4)));
    }

    #[test]
    fn test_storage_policy_zstd_levels() {
        assert_eq!(StoragePolicy::Hot.zstd_level(), 0);
        assert_eq!(StoragePolicy::Warm.zstd_level(), 3);
        assert_eq!(StoragePolicy::Cold.zstd_level(), 19);
        assert_eq!(StoragePolicy::Archive.zstd_level(), 22);
    }
}
