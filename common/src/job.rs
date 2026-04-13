use crate::types::{JobId, NodeId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Scheduled,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Docker,
    Python,
    Jar,
    Executable,
    Wasm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobResources {
    pub cpu_cores: u32,
    pub memory_mb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobSpec {
    pub id: JobId,
    pub runtime: Runtime,
    pub source: String,
    pub command: Vec<String>,
    pub resources: JobResources,
    pub labels: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobEvent {
    Started { job_id: JobId, node_id: NodeId, at: DateTime<Utc> },
    OutputLine { job_id: JobId, is_stderr: bool, line: String, at: DateTime<Utc> },
    Completed { job_id: JobId, exit_code: i32, at: DateTime<Utc> },
    Failed { job_id: JobId, error: String, at: DateTime<Utc> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_spec_roundtrip_json() {
        let spec = JobSpec {
            id: JobId::new(),
            runtime: Runtime::Docker,
            source: "alpine:3.19".to_string(),
            command: vec!["sh".to_string(), "-c".to_string(), "echo hello".to_string()],
            resources: JobResources {
                cpu_cores: 1,
                memory_mb: 128,
            },
            labels: BTreeMap::from([("env".to_string(), "dev".to_string())]),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&spec).expect("serialize job spec");
        let decoded: JobSpec = serde_json::from_str(&json).expect("deserialize job spec");
        assert_eq!(spec, decoded);
    }
}
