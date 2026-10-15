//! Health, readiness, and metrics HTTP API endpoints (RS-06).

use chrono::Utc;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::engine::health_gate::HealthGate;

pub struct HttpServer {
    port: u16,
    health_gate: Arc<HealthGate>,
}

impl HttpServer {
    pub fn new(port: u16, health_gate: Arc<HealthGate>) -> Self {
        Self { port, health_gate }
    }

    /// Starts the HTTP server listening on 0.0.0.0:port.
    pub async fn run(&self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port)).await?;
        info!(port = self.port, "HTTP health and metrics server listening");

        loop {
            match listener.accept().await {
                Ok((mut stream, _)) => {
                    let gate = Arc::clone(&self.health_gate);
                    tokio::spawn(async move {
                        let mut buffer = [0u8; 1024];
                        if let Ok(n) = stream.read(&mut buffer).await {
                            if n == 0 {
                                return;
                            }
                            let request = String::from_utf8_lossy(&buffer[..n]);
                            let first_line = request.lines().next().unwrap_or("");
                            let mut parts = first_line.split_whitespace();
                            let _method = parts.next().unwrap_or("");
                            let path = parts.next().unwrap_or("/");

                            let now_secs = Utc::now().timestamp().max(0) as u64;
                            let summary = gate.summary(now_secs);

                            let response = match path {
                                "/health" => {
                                    let json = serde_json::to_string(&summary).unwrap_or_default();
                                    format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                        json.len(),
                                        json
                                    )
                                }
                                "/ready" => {
                                    if summary.can_emit_signals {
                                        let body = "READY";
                                        format!(
                                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                            body.len(),
                                            body
                                        )
                                    } else {
                                        let json =
                                            serde_json::to_string(&summary).unwrap_or_default();
                                        format!(
                                            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                            json.len(),
                                            json
                                        )
                                    }
                                }
                                "/metrics" => {
                                    let metrics = format!(
                                        "# HELP risk_engine_is_frozen 1 if protocol/oracle is frozen, 0 otherwise\n\
                                         # TYPE risk_engine_is_frozen gauge\n\
                                         risk_engine_is_frozen {}\n\
                                         # HELP risk_engine_unresolved_retracts Current count of unresolved chain reorg retractions\n\
                                         # TYPE risk_engine_unresolved_retracts gauge\n\
                                         risk_engine_unresolved_retracts {}\n\
                                         # HELP risk_engine_lag_seconds Ingestion lag in seconds\n\
                                         # TYPE risk_engine_lag_seconds gauge\n\
                                         risk_engine_lag_seconds {}\n",
                                        if summary.is_frozen { 1 } else { 0 },
                                        summary.unresolved_retracts,
                                        summary.lag_seconds
                                    );
                                    format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                        metrics.len(),
                                        metrics
                                    )
                                }
                                _ => {
                                    let body = "Not Found";
                                    format!(
                                        "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                        body.len(),
                                        body
                                    )
                                }
                            };

                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.flush().await;
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "HTTP accept error");
                }
            }
        }
    }
}
