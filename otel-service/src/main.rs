use std::net::SocketAddr;

use axum::{Router, response::IntoResponse, routing::post};
use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use http::header::{CONTENT_TYPE, HeaderValue};
use opentelemetry_proto::tonic::collector::logs::v1::{
    ExportLogsServiceRequest, ExportLogsServiceResponse,
    logs_service_server::{LogsService, LogsServiceServer},
};
use opentelemetry_proto::tonic::collector::metrics::v1::{
    ExportMetricsServiceRequest, ExportMetricsServiceResponse,
    metrics_service_server::{MetricsService, MetricsServiceServer},
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
    trace_service_server::{TraceService, TraceServiceServer},
};
use prost::Message;
use tokio::try_join;
use tonic::{Request, Response, Status, async_trait, transport::Server};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Default)]
struct TracesSvc;

#[derive(Default)]
struct MetricsSvc;

#[derive(Default)]
struct LogsSvc;

#[async_trait]
impl TraceService for TracesSvc {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let body = request.into_inner();
        info!(?body, "Received gRPC trace export");
        Ok(Response::new(ExportTraceServiceResponse::default()))
    }
}

#[async_trait]
impl MetricsService for MetricsSvc {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let body = request.into_inner();
        info!(?body, "Received gRPC metrics export");
        Ok(Response::new(ExportMetricsServiceResponse::default()))
    }
}

#[async_trait]
impl LogsService for LogsSvc {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let body = request.into_inner();
        info!(?body, "Received gRPC logs export");
        Ok(Response::new(ExportLogsServiceResponse::default()))
    }
}

async fn handle_http_traces(headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    decode_and_reply::<ExportTraceServiceRequest, ExportTraceServiceResponse>(
        "HTTP traces",
        headers,
        body,
    )
}

async fn handle_http_metrics(headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    decode_and_reply::<ExportMetricsServiceRequest, ExportMetricsServiceResponse>(
        "HTTP metrics",
        headers,
        body,
    )
}

async fn handle_http_logs(headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    decode_and_reply::<ExportLogsServiceRequest, ExportLogsServiceResponse>(
        "HTTP logs",
        headers,
        body,
    )
}

fn decode_and_reply<Req, Resp>(
    kind: &str,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response
where
    Req: Message + std::fmt::Debug + Default,
    Resp: Message + Default,
{
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.contains("application/json") {
        info!(
            payload = %String::from_utf8_lossy(&body),
            "Received {kind} export (json passthrough)"
        );
        let resp = Resp::default();
        let bytes = resp.encode_to_vec();
        return build_proto_response(bytes);
    }

    match Req::decode(body) {
        Ok(decoded) => {
            info!(?decoded, "Received {kind} export");
            let resp = Resp::default();
            let bytes = resp.encode_to_vec();
            build_proto_response(bytes)
        }
        Err(err) => {
            warn!(error = ?err, "Failed to decode {kind} payload");
            (
                StatusCode::BAD_REQUEST,
                format!("invalid {kind} payload: {err}"),
            )
            .into_response()
        }
    }
}

fn build_proto_response(body: Vec<u8>) -> axum::response::Response {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-protobuf"),
    );
    (StatusCode::OK, headers, body).into_response()
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let grpc_addr: SocketAddr = "0.0.0.0:4317".parse()?;
    let http_addr: SocketAddr = "0.0.0.0:4318".parse()?;

    let grpc_server = Server::builder()
        .add_service(TraceServiceServer::new(TracesSvc::default()))
        .add_service(MetricsServiceServer::new(MetricsSvc::default()))
        .add_service(LogsServiceServer::new(LogsSvc::default()))
        .serve(grpc_addr);

    let http_app = Router::new()
        .route("/v1/traces", post(handle_http_traces))
        .route("/v1/metrics", post(handle_http_metrics))
        .route("/v1/logs", post(handle_http_logs));
    let http_server = axum::Server::bind(&http_addr).serve(http_app.into_make_service());

    info!("Starting gRPC OTLP receiver on {grpc_addr}");
    info!("Starting HTTP OTLP receiver on {http_addr}");

    try_join!(
        async move {
            grpc_server.await?;
            Ok::<(), anyhow::Error>(())
        },
        async move {
            http_server.await?;
            Ok::<(), anyhow::Error>(())
        }
    )?;

    Ok(())
}
