use crate::raft::{RaftNodeId, TypeConfig};
use openraft::error::NetworkError;
use openraft::{
    error::{InstallSnapshotError, RPCError, RaftError},
    network::{RPCOption, RaftNetwork, RaftNetworkFactory},
    raft::{
        AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest,
        InstallSnapshotResponse, VoteRequest, VoteResponse,
    },
    BasicNode,
};
use tonic::transport::Channel;

pub mod proto_raft {
    tonic::include_proto!("all4one.raft.v1");
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct GrpcNetworkFactory;

impl RaftNetworkFactory<TypeConfig> for GrpcNetworkFactory {
    type Network = GrpcNetwork;

    async fn new_client(&mut self, target: RaftNodeId, node: &BasicNode) -> GrpcNetwork {
        GrpcNetwork {
            target_id: target,
            target_addr: node.addr.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-peer network client
// ---------------------------------------------------------------------------

pub struct GrpcNetwork {
    pub target_id: RaftNodeId,
    pub target_addr: String,
}

impl GrpcNetwork {
    async fn client(
        &self,
    ) -> Result<proto_raft::raft_service_client::RaftServiceClient<Channel>, tonic::Status> {
        let endpoint = format!("http://{}", self.target_addr);
        let channel = Channel::from_shared(endpoint)
            .map_err(|e| tonic::Status::internal(e.to_string()))?
            .connect()
            .await
            .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
        Ok(proto_raft::raft_service_client::RaftServiceClient::new(
            channel,
        ))
    }

    fn to_rpc_err<E: std::error::Error + 'static + Sync + Send>(
        e: E,
    ) -> RPCError<RaftNodeId, BasicNode, RaftError<RaftNodeId>> {
        RPCError::Network(NetworkError::new(&e))
    }
}

impl RaftNetwork<TypeConfig> for GrpcNetwork {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        AppendEntriesResponse<RaftNodeId>,
        RPCError<RaftNodeId, BasicNode, RaftError<RaftNodeId>>,
    > {
        let payload = serde_json::to_vec(&rpc).map_err(Self::to_rpc_err)?;
        let mut client = self.client().await.map_err(Self::to_rpc_err)?;
        let resp = client
            .append_entries(proto_raft::RaftMessage { payload })
            .await
            .map_err(Self::to_rpc_err)?
            .into_inner();

        serde_json::from_slice(&resp.payload).map_err(Self::to_rpc_err)
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<RaftNodeId>,
        RPCError<RaftNodeId, BasicNode, RaftError<RaftNodeId, InstallSnapshotError>>,
    > {
        let payload =
            serde_json::to_vec(&rpc).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let mut client = self
            .client()
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let resp = client
            .install_snapshot(proto_raft::RaftMessage { payload })
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?
            .into_inner();

        serde_json::from_slice(&resp.payload).map_err(|e| RPCError::Network(NetworkError::new(&e)))
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<RaftNodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<RaftNodeId>, RPCError<RaftNodeId, BasicNode, RaftError<RaftNodeId>>>
    {
        let payload = serde_json::to_vec(&rpc).map_err(Self::to_rpc_err)?;
        let mut client = self.client().await.map_err(Self::to_rpc_err)?;
        let resp = client
            .vote(proto_raft::RaftMessage { payload })
            .await
            .map_err(Self::to_rpc_err)?
            .into_inner();

        serde_json::from_slice(&resp.payload).map_err(Self::to_rpc_err)
    }
}
