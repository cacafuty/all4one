use crate::raft::commands::{EnrollToken, RaftCommand, RaftCommandResponse, RegisteredJob};
use crate::raft::{RaftNodeId, TypeConfig};
use chrono::Utc;
use openraft::storage::{RaftLogStorage, RaftStateMachine};
use openraft::{
    storage::LogFlushed, AnyError, BasicNode, Entry, EntryPayload, ErrorSubject, ErrorVerb, LogId,
    LogState, RaftLogReader, RaftSnapshotBuilder, Snapshot, SnapshotMeta, StorageError,
    StorageIOError, StoredMembership, Vote,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::path::Path;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn io_err<E: std::error::Error + Sync + Send + 'static>(
    subject: ErrorSubject<RaftNodeId>,
    verb: ErrorVerb,
    e: E,
) -> StorageError<RaftNodeId> {
    StorageError::IO {
        source: StorageIOError::new(subject, verb, AnyError::new(&e)),
    }
}

fn log_read_err<E: std::error::Error + Sync + Send + 'static>(e: E) -> StorageError<RaftNodeId> {
    io_err(ErrorSubject::Logs, ErrorVerb::Read, e)
}

fn log_write_err<E: std::error::Error + Sync + Send + 'static>(e: E) -> StorageError<RaftNodeId> {
    io_err(ErrorSubject::Logs, ErrorVerb::Write, e)
}

fn vote_read_err<E: std::error::Error + Sync + Send + 'static>(e: E) -> StorageError<RaftNodeId> {
    io_err(ErrorSubject::Vote, ErrorVerb::Read, e)
}

fn vote_write_err<E: std::error::Error + Sync + Send + 'static>(e: E) -> StorageError<RaftNodeId> {
    io_err(ErrorSubject::Vote, ErrorVerb::Write, e)
}

fn sm_write_err<E: std::error::Error + Sync + Send + 'static>(e: E) -> StorageError<RaftNodeId> {
    io_err(ErrorSubject::StateMachine, ErrorVerb::Write, e)
}

fn sm_read_err<E: std::error::Error + Sync + Send + 'static>(e: E) -> StorageError<RaftNodeId> {
    io_err(ErrorSubject::StateMachine, ErrorVerb::Read, e)
}

fn encode_index(index: u64) -> [u8; 8] {
    index.to_be_bytes()
}

fn decode_index(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(buf)
}

// ---------------------------------------------------------------------------
// Persisted state machine state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RaftState {
    pub last_applied: Option<LogId<RaftNodeId>>,
    pub last_membership: StoredMembership<RaftNodeId, BasicNode>,
    pub job_registry: HashMap<String, RegisteredJob>,
    pub enroll_tokens: HashMap<String, EnrollToken>,
}

// ---------------------------------------------------------------------------
// Log store (sled-backed)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SledLogStore {
    db: sled::Db,
}

impl SledLogStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let db = sled::open(data_dir.join("raft-log"))?;
        Ok(Self { db })
    }

    fn log_tree(&self) -> sled::Tree {
        self.db.open_tree("entries").expect("open log tree")
    }

    fn meta_tree(&self) -> sled::Tree {
        self.db.open_tree("meta").expect("open meta tree")
    }
}

impl RaftLogReader<TypeConfig> for SledLogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + std::fmt::Debug + Send>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<RaftNodeId>> {
        let tree = self.log_tree();
        let mut entries: Vec<Entry<TypeConfig>> = Vec::new();

        for item in tree.iter() {
            let (k, v) = item.map_err(log_read_err)?;
            let index = decode_index(&k);
            if range.contains(&index) {
                let entry: Entry<TypeConfig> = serde_json::from_slice(&v).map_err(log_read_err)?;
                entries.push(entry);
            }
        }
        entries.sort_by_key(|e| e.log_id.index);
        Ok(entries)
    }
}

impl RaftLogStorage<TypeConfig> for SledLogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<RaftNodeId>> {
        let meta = self.meta_tree();
        let tree = self.log_tree();

        let last_purged: Option<LogId<RaftNodeId>> =
            match meta.get("last_purged").map_err(log_read_err)? {
                Some(b) => Some(serde_json::from_slice(&b).map_err(log_read_err)?),
                None => None,
            };

        let last_log: Option<LogId<RaftNodeId>> = match tree.last().map_err(log_read_err)? {
            Some((_, v)) => {
                let e: Entry<TypeConfig> = serde_json::from_slice(&v).map_err(log_read_err)?;
                Some(e.log_id)
            }
            None => None,
        };

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id: last_log,
        })
    }

    async fn save_vote(&mut self, vote: &Vote<RaftNodeId>) -> Result<(), StorageError<RaftNodeId>> {
        let b = serde_json::to_vec(vote).map_err(vote_write_err)?;
        self.meta_tree().insert("vote", b).map_err(vote_write_err)?;
        self.meta_tree().flush().map_err(vote_write_err)?;
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<RaftNodeId>>, StorageError<RaftNodeId>> {
        match self.meta_tree().get("vote").map_err(vote_read_err)? {
            Some(b) => Ok(Some(serde_json::from_slice(&b).map_err(vote_read_err)?)),
            None => Ok(None),
        }
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<RaftNodeId>>,
    ) -> Result<(), StorageError<RaftNodeId>> {
        let meta = self.meta_tree();
        match committed {
            Some(id) => {
                let b = serde_json::to_vec(&id).map_err(log_write_err)?;
                meta.insert("committed", b).map_err(log_write_err)?;
            }
            None => {
                meta.remove("committed").map_err(log_write_err)?;
            }
        }
        meta.flush().map_err(log_write_err)?;
        Ok(())
    }

    async fn read_committed(
        &mut self,
    ) -> Result<Option<LogId<RaftNodeId>>, StorageError<RaftNodeId>> {
        match self.meta_tree().get("committed").map_err(log_read_err)? {
            Some(b) => Ok(Some(serde_json::from_slice(&b).map_err(log_read_err)?)),
            None => Ok(None),
        }
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError<RaftNodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
        I::IntoIter: Send,
    {
        let tree = self.log_tree();
        let mut batch = sled::Batch::default();
        for entry in entries {
            let key = encode_index(entry.log_id.index);
            let val = serde_json::to_vec(&entry).map_err(log_write_err)?;
            batch.insert(key.as_ref(), val);
        }
        tree.apply_batch(batch).map_err(log_write_err)?;
        tree.flush().map_err(log_write_err)?;
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(
        &mut self,
        log_id: LogId<RaftNodeId>,
    ) -> Result<(), StorageError<RaftNodeId>> {
        let tree = self.log_tree();
        let keys: Vec<_> = tree
            .iter()
            .filter_map(|r| r.ok())
            .filter(|(k, _)| decode_index(k) >= log_id.index)
            .map(|(k, _)| k)
            .collect();
        for k in keys {
            tree.remove(k).map_err(log_write_err)?;
        }
        tree.flush().map_err(log_write_err)?;
        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<RaftNodeId>) -> Result<(), StorageError<RaftNodeId>> {
        let tree = self.log_tree();
        let meta = self.meta_tree();

        let keys: Vec<_> = tree
            .iter()
            .filter_map(|r| r.ok())
            .filter(|(k, _)| decode_index(k) <= log_id.index)
            .map(|(k, _)| k)
            .collect();
        for k in keys {
            tree.remove(k).map_err(log_write_err)?;
        }
        let b = serde_json::to_vec(&log_id).map_err(log_write_err)?;
        meta.insert("last_purged", b).map_err(log_write_err)?;
        meta.flush().map_err(log_write_err)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// State machine (sled-backed)
// ---------------------------------------------------------------------------

pub struct SledStateMachine {
    db: sled::Db,
    pub state: RaftState,
}

impl SledStateMachine {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let db = sled::open(data_dir.join("raft-sm"))?;
        let state: RaftState = match db.get("state").ok().flatten() {
            Some(b) => serde_json::from_slice(&b).unwrap_or_default(),
            None => RaftState::default(),
        };
        Ok(Self { db, state })
    }

    fn apply_command(&mut self, cmd: RaftCommand) -> RaftCommandResponse {
        match cmd {
            RaftCommand::RegisterJob {
                job_id,
                node_id,
                runtime,
                created_at,
            } => {
                if self.state.job_registry.contains_key(&job_id) {
                    RaftCommandResponse {
                        success: false,
                        error: Some("already registered".to_string()),
                        data: None,
                    }
                } else {
                    self.state.job_registry.insert(
                        job_id.clone(),
                        RegisteredJob {
                            job_id,
                            node_id,
                            runtime,
                            status: "queued".to_string(),
                            created_at: created_at.clone(),
                            updated_at: created_at,
                        },
                    );
                    RaftCommandResponse {
                        success: true,
                        error: None,
                        data: None,
                    }
                }
            }
            RaftCommand::UpdateJobStatus {
                job_id,
                status,
                node_id: _,
            } => {
                if let Some(job) = self.state.job_registry.get_mut(&job_id) {
                    job.status = status;
                    job.updated_at = Utc::now().to_rfc3339();
                    RaftCommandResponse {
                        success: true,
                        error: None,
                        data: None,
                    }
                } else {
                    RaftCommandResponse {
                        success: false,
                        error: Some(format!("job {} not found", job_id)),
                        data: None,
                    }
                }
            }
            RaftCommand::AddMember { .. } | RaftCommand::RemoveMember { .. } => {
                // Membership changes are handled by Raft itself; these are stored for reference.
                RaftCommandResponse {
                    success: true,
                    error: None,
                    data: None,
                }
            }
            RaftCommand::StoreEnrollToken {
                token,
                expires_at,
                issued_by,
            } => {
                self.state.enroll_tokens.insert(
                    token.clone(),
                    EnrollToken {
                        token,
                        issued_by,
                        expires_at,
                        used: false,
                    },
                );
                RaftCommandResponse {
                    success: true,
                    error: None,
                    data: None,
                }
            }
            RaftCommand::ConsumeEnrollToken { token } => {
                if let Some(t) = self.state.enroll_tokens.get_mut(&token) {
                    if t.used {
                        RaftCommandResponse {
                            success: false,
                            error: Some("token already used".to_string()),
                            data: None,
                        }
                    } else {
                        t.used = true;
                        RaftCommandResponse {
                            success: true,
                            error: None,
                            data: None,
                        }
                    }
                } else {
                    RaftCommandResponse {
                        success: false,
                        error: Some("token not found".to_string()),
                        data: None,
                    }
                }
            }
        }
    }
}

// --- Snapshot builder -------------------------------------------------------

pub struct SledSnapshotBuilder {
    state: RaftState,
}

impl RaftSnapshotBuilder<TypeConfig> for SledSnapshotBuilder {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<RaftNodeId>> {
        let data = serde_json::to_vec(&self.state).map_err(sm_write_err)?;
        let snapshot_id = uuid::Uuid::new_v4().to_string();
        let last_applied = self.state.last_applied;
        let membership = self.state.last_membership.clone();
        Ok(Snapshot {
            meta: SnapshotMeta {
                last_log_id: last_applied,
                last_membership: membership,
                snapshot_id,
            },
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

// --- State machine impl -----------------------------------------------------

impl RaftStateMachine<TypeConfig> for SledStateMachine {
    type SnapshotBuilder = SledSnapshotBuilder;

    async fn applied_state(
        &mut self,
    ) -> Result<
        (
            Option<LogId<RaftNodeId>>,
            StoredMembership<RaftNodeId, BasicNode>,
        ),
        StorageError<RaftNodeId>,
    > {
        Ok((self.state.last_applied, self.state.last_membership.clone()))
    }

    async fn apply<I>(
        &mut self,
        entries: I,
    ) -> Result<Vec<RaftCommandResponse>, StorageError<RaftNodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
        I::IntoIter: Send,
    {
        let mut responses = Vec::new();
        for entry in entries {
            self.state.last_applied = Some(entry.log_id);
            let resp = match entry.payload {
                EntryPayload::Blank => RaftCommandResponse {
                    success: true,
                    error: None,
                    data: None,
                },
                EntryPayload::Normal(cmd) => self.apply_command(cmd),
                EntryPayload::Membership(m) => {
                    self.state.last_membership = StoredMembership::new(Some(entry.log_id), m);
                    RaftCommandResponse {
                        success: true,
                        error: None,
                        data: None,
                    }
                }
            };
            responses.push(resp);
        }
        let b = serde_json::to_vec(&self.state).map_err(sm_write_err)?;
        self.db.insert("state", b).map_err(sm_write_err)?;
        self.db.flush().map_err(sm_write_err)?;
        Ok(responses)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        SledSnapshotBuilder {
            state: self.state.clone(),
        }
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<Cursor<Vec<u8>>>, StorageError<RaftNodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<RaftNodeId, BasicNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<RaftNodeId>> {
        let data = snapshot.into_inner();
        self.state = serde_json::from_slice(&data).map_err(sm_write_err)?;
        self.state.last_applied = meta.last_log_id;
        self.state.last_membership = meta.last_membership.clone();
        let b = serde_json::to_vec(&self.state).map_err(sm_write_err)?;
        self.db.insert("state", b).map_err(sm_write_err)?;
        self.db.flush().map_err(sm_write_err)?;
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<TypeConfig>>, StorageError<RaftNodeId>> {
        let data = serde_json::to_vec(&self.state).map_err(sm_read_err)?;
        if data == b"null" || self.state.last_applied.is_none() {
            return Ok(None);
        }
        let snapshot_id = "latest".to_string();
        Ok(Some(Snapshot {
            meta: SnapshotMeta {
                last_log_id: self.state.last_applied,
                last_membership: self.state.last_membership.clone(),
                snapshot_id,
            },
            snapshot: Box::new(Cursor::new(data)),
        }))
    }
}
