use crate::api_rest::AppState;
use crate::raft::TypeConfig;
use openraft::raft::{AppendEntriesRequest, InstallSnapshotRequest, VoteRequest};
use tonic::{Request, Response, Status};

pub mod proto_raft {
    tonic::include_proto!("all4one.raft.v1");
}

use proto_raft::raft_service_server::RaftService;

#[derive(Clone)]
pub struct RaftServiceImpl {
    pub state: AppState,
}

#[tonic::async_trait]
impl RaftService for RaftServiceImpl {
    async fn append_entries(
        &self,
        request: Request<proto_raft::RaftMessage>,
    ) -> Result<Response<proto_raft::RaftMessage>, Status> {
        let raft = self.raft()?;
        let req: AppendEntriesRequest<TypeConfig> =
            serde_json::from_slice(&request.into_inner().payload)
                .map_err(|e| Status::invalid_argument(format!("deserialize: {e}")))?;

        let resp = raft
            .raft
            .append_entries(req)
            .await
            .map_err(|e| Status::internal(format!("raft: {e}")))?;

        let payload =
            serde_json::to_vec(&resp).map_err(|e| Status::internal(format!("serialize: {e}")))?;

        Ok(Response::new(proto_raft::RaftMessage { payload }))
    }

    async fn vote(
        &self,
        request: Request<proto_raft::RaftMessage>,
    ) -> Result<Response<proto_raft::RaftMessage>, Status> {
        let raft = self.raft()?;
        let req: VoteRequest<crate::raft::RaftNodeId> =
            serde_json::from_slice(&request.into_inner().payload)
                .map_err(|e| Status::invalid_argument(format!("deserialize: {e}")))?;

        let resp = raft
            .raft
            .vote(req)
            .await
            .map_err(|e| Status::internal(format!("raft: {e}")))?;

        let payload =
            serde_json::to_vec(&resp).map_err(|e| Status::internal(format!("serialize: {e}")))?;

        Ok(Response::new(proto_raft::RaftMessage { payload }))
    }

    async fn install_snapshot(
        &self,
        request: Request<proto_raft::RaftMessage>,
    ) -> Result<Response<proto_raft::RaftMessage>, Status> {
        let raft = self.raft()?;
        let req: InstallSnapshotRequest<TypeConfig> =
            serde_json::from_slice(&request.into_inner().payload)
                .map_err(|e| Status::invalid_argument(format!("deserialize: {e}")))?;

        let resp = raft
            .raft
            .install_snapshot(req)
            .await
            .map_err(|e| Status::internal(format!("raft: {e}")))?;

        let payload =
            serde_json::to_vec(&resp).map_err(|e| Status::internal(format!("serialize: {e}")))?;

        Ok(Response::new(proto_raft::RaftMessage { payload }))
    }
}

impl RaftServiceImpl {
    fn raft(&self) -> Result<&crate::raft::RaftNode, Status> {
        self.state
            .raft
            .as_ref()
            .ok_or_else(|| Status::unavailable("not a quorum participant"))
    }
}
