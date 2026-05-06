use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Application commands (Raft log entries)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RaftCommand {
    /// Idempotent job registration — first writer wins.
    RegisterJob {
        job_id: String,
        node_id: String,
        runtime: String,
        created_at: String,
    },
    /// Update job status after it transitions to a terminal state.
    UpdateJobStatus {
        job_id: String,
        status: String,
        node_id: String,
    },
    /// Add a cluster member (used during enrolment — Phase 2 full).
    AddMember {
        node_id: String,
        grpc_endpoint: String,
    },
    /// Remove a cluster member.
    RemoveMember {
        node_id: String,
    },
    /// Store a single-use enrolment token.
    StoreEnrollToken {
        token: String,
        expires_at: String,
        issued_by: String,
    },
    /// Mark a token as consumed; fails if already used.
    ConsumeEnrollToken {
        token: String,
    },
}

// ---------------------------------------------------------------------------
// Response returned by the state machine after applying a command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftCommandResponse {
    pub success: bool,
    pub error: Option<String>,
    pub data: Option<String>,
}

// ---------------------------------------------------------------------------
// In-memory state tracked by the state machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredJob {
    pub job_id: String,
    pub node_id: String,
    pub runtime: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollToken {
    pub token: String,
    pub issued_by: String,
    pub expires_at: String,
    pub used: bool,
}
