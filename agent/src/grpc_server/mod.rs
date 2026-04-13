use crate::api_rest::{enqueue_job, AppState};
use crate::grpc_client::{from_proto, proto};
use tonic::{Request, Response, Status};

#[derive(Clone)]
struct ServiceImpl {
    state: AppState,
}

#[tonic::async_trait]
impl proto::agent_service_server::AgentService for ServiceImpl {
    async fn submit_job(
        &self,
        request: Request<proto::SubmitJobRequest>,
    ) -> Result<Response<proto::SubmitJobResponse>, Status> {
        let req = from_proto(request.into_inner());
        let submitted = enqueue_job(self.state.clone(), req)
            .await
            .map_err(Status::invalid_argument)?;

        Ok(Response::new(proto::SubmitJobResponse {
            job_id: submitted.job_id.to_string(),
            status: "queued".to_string(),
            assigned_to: submitted.assigned_to.to_string(),
            created_at: submitted.created_at.to_rfc3339(),
        }))
    }
}

pub async fn start_background(state: AppState) {
    let addr = format!("{}:{}", state.config.network.bind_address, state.config.network.grpc_port)
        .parse();

    let Ok(addr) = addr else {
        eprintln!("ERROR invalid gRPC bind address");
        return;
    };

    let svc = ServiceImpl { state };
    tokio::spawn(async move {
        let result = tonic::transport::Server::builder()
            .add_service(proto::agent_service_server::AgentServiceServer::new(svc))
            .serve(addr)
            .await;

        if let Err(err) = result {
            eprintln!("ERROR gRPC server stopped: {err}");
        }
    });
}
