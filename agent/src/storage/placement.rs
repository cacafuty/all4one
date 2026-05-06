use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::StoragePolicy;

/// Represents a node's storage capacity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapacity {
    pub node_id: Uuid,
    pub available_bytes: u64,
    pub used_bytes: u64,
}

/// Represents a placement decision for a chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkPlacement {
    pub chunk_id: String,
    pub node_ids: Vec<Uuid>, // Ordered list of nodes holding this chunk
    pub policy: String,
}

/// Determine which nodes should store a chunk based on policy
pub fn determine_placement(
    _chunk_id: &str,
    _chunk_size: u64,
    policy: StoragePolicy,
    available_nodes: &[NodeCapacity],
) -> Vec<Uuid> {
    if available_nodes.is_empty() {
        return Vec::new();
    }

    let placement_count = policy.replicas();
    let mut placement = Vec::new();

    // Simple round-robin placement for MVP
    // In production: consider node load, availability zones, etc.
    for i in 0..placement_count.min(available_nodes.len()) {
        placement.push(available_nodes[i % available_nodes.len()].node_id);
    }

    placement
}

/// Check if chunk is recoverable with current placement (for erasure coding tiers)
pub fn is_recoverable(
    placement: &[Uuid],
    policy: StoragePolicy,
    failed_nodes: &[Uuid],
) -> bool {
    match policy {
        StoragePolicy::Hot => {
            // Need at least 1 replica
            placement.iter().any(|n| !failed_nodes.contains(n))
        }
        StoragePolicy::Warm => {
            // RS(4,2): need at least 4 out of 6 shards
            let available = placement
                .iter()
                .filter(|n| !failed_nodes.contains(n))
                .count();
            available >= 4
        }
        StoragePolicy::Cold => {
            // RS(6,3): need at least 6 out of 9 shards
            let available = placement
                .iter()
                .filter(|n| !failed_nodes.contains(n))
                .count();
            available >= 6
        }
        StoragePolicy::Archive => {
            // RS(8,4): need at least 8 out of 12 shards
            let available = placement
                .iter()
                .filter(|n| !failed_nodes.contains(n))
                .count();
            available >= 8
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placement_hot() {
        let nodes = vec![
            NodeCapacity {
                node_id: Uuid::nil(),
                available_bytes: 1000,
                used_bytes: 0,
            },
            NodeCapacity {
                node_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                available_bytes: 1000,
                used_bytes: 0,
            },
            NodeCapacity {
                node_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap(),
                available_bytes: 1000,
                used_bytes: 0,
            },
        ];

        let placement = determine_placement("test-chunk", 100, StoragePolicy::Hot, &nodes);
        assert_eq!(placement.len(), 3); // 3x replication for Hot
    }

    #[test]
    fn test_placement_limited_nodes() {
        let nodes = vec![
            NodeCapacity {
                node_id: Uuid::nil(),
                available_bytes: 1000,
                used_bytes: 0,
            },
            NodeCapacity {
                node_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                available_bytes: 1000,
                used_bytes: 0,
            },
        ];

        let placement = determine_placement("test-chunk", 100, StoragePolicy::Hot, &nodes);
        assert_eq!(placement.len(), 2); // Only 2 nodes available, so place on both
    }

    #[test]
    fn test_placement_no_nodes() {
        let nodes = vec![];
        let placement = determine_placement("test-chunk", 100, StoragePolicy::Hot, &nodes);
        assert_eq!(placement.len(), 0);
    }

    #[test]
    fn test_recoverable_hot() {
        let node1 = Uuid::nil();
        let node2 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let node3 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap();

        let placement = vec![node1, node2, node3];
        let failed = vec![node1, node2]; // 2 out of 3 failed

        // Should still be recoverable with 1 replica
        assert!(is_recoverable(&placement, StoragePolicy::Hot, &failed));

        let failed_all = vec![node1, node2, node3]; // All failed
        assert!(!is_recoverable(&placement, StoragePolicy::Hot, &failed_all));
    }

    #[test]
    fn test_recoverable_warm_rs42() {
        let nodes: Vec<Uuid> = (0..6).map(|i| Uuid::from_u128(i as u128)).collect();
        let placement = nodes.clone();

        // With RS(4,2), need at least 4 shards
        let failed = vec![nodes[5]]; // 1 failed, 5 available
        assert!(is_recoverable(&placement, StoragePolicy::Warm, &failed));

        let failed = vec![nodes[4], nodes[5]]; // 2 failed, 4 available
        assert!(is_recoverable(&placement, StoragePolicy::Warm, &failed));

        let failed = vec![nodes[3], nodes[4], nodes[5]]; // 3 failed, 3 available
        assert!(!is_recoverable(&placement, StoragePolicy::Warm, &failed));
    }

    #[test]
    fn test_recoverable_cold_rs63() {
        let nodes: Vec<Uuid> = (0..9).map(|i| Uuid::from_u128(i as u128)).collect();
        let placement = nodes.clone();

        // With RS(6,3), need at least 6 shards
        let failed = vec![nodes[8]]; // 1 failed, 8 available
        assert!(is_recoverable(&placement, StoragePolicy::Cold, &failed));

        // RS(6,3): exactly at the boundary — 3 failures leaves 6 shards → recoverable
        let failed = (0..3).map(|i| nodes[6 + i]).collect::<Vec<_>>();
        assert!(is_recoverable(&placement, StoragePolicy::Cold, &failed));

        // 4 failures: 5 shards remain < 6 required → not recoverable
        let failed = (0..4).map(|i| nodes[5 + i]).collect::<Vec<_>>();
        assert!(!is_recoverable(&placement, StoragePolicy::Cold, &failed));

        let failed = (0..5).map(|i| nodes[4 + i]).collect::<Vec<_>>(); // 5 failed, 4 available
        assert!(!is_recoverable(&placement, StoragePolicy::Cold, &failed));
    }

    #[test]
    fn test_recoverable_archive_rs84() {
        let nodes: Vec<Uuid> = (0..12).map(|i| Uuid::from_u128(i as u128)).collect();
        let placement = nodes.clone();

        // With RS(8,4), need at least 8 shards
        let failed = vec![nodes[11]]; // 1 failed, 11 available
        assert!(is_recoverable(&placement, StoragePolicy::Archive, &failed));

        let failed = (0..4).map(|i| nodes[8 + i]).collect::<Vec<_>>(); // 4 failed
        assert!(is_recoverable(&placement, StoragePolicy::Archive, &failed));

        let failed = (0..5).map(|i| nodes[7 + i]).collect::<Vec<_>>(); // 5 failed, 7 available
        assert!(!is_recoverable(&placement, StoragePolicy::Archive, &failed));
    }

    #[test]
    fn test_node_capacity_tracking() {
        let mut capacity = NodeCapacity {
            node_id: Uuid::nil(),
            available_bytes: 1000,
            used_bytes: 0,
        };

        assert_eq!(capacity.available_bytes, 1000);
        capacity.used_bytes = 250;
        assert_eq!(capacity.available_bytes, 1000);
    }

    #[test]
    fn test_placement_distribution() {
        let nodes = (0..5)
            .map(|i| NodeCapacity {
                node_id: Uuid::from_u128(i as u128),
                available_bytes: 1000,
                used_bytes: 0,
            })
            .collect::<Vec<_>>();

        let placement = determine_placement("test", 100, StoragePolicy::Warm, &nodes);
        assert_eq!(placement.len(), 1); // Warm policy = 1 replica

        let hot_placement = determine_placement("test", 100, StoragePolicy::Hot, &nodes);
        assert_eq!(hot_placement.len(), 3); // Hot = 3x replication
    }
}
