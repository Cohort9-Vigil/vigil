//! RS-07 Integration Test: HTTP Health, Readiness, and Metrics Endpoints
//!
//! Verifies:
//! 1. /health returns 200 OK with accurate JSON HealthSummary.
//! 2. /ready returns 200 OK when healthy, and 503 Service Unavailable when fail-closed.
//! 3. /metrics returns valid Prometheus metrics.

use risk_engine::api::HttpServer;
use risk_engine::engine::health_gate::HealthGate;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn get_ephemeral_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

async fn http_get(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .expect("connect to http server");

    let req = format!(
        "GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        path
    );
    stream.write_all(req.as_bytes()).await.expect("write req");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read resp");

    let status_line = response.lines().next().unwrap_or("");
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status_code, body)
}

#[tokio::test]
async fn test_rs07_health_and_metrics_endpoints() {
    let port = get_ephemeral_port().await;
    let gate = HealthGate::new(30);
    gate.record_heartbeat(chrono::Utc::now().timestamp() as u64);

    let server = HttpServer::new(port, Arc::clone(&gate));
    tokio::spawn(async move {
        let _ = server.run().await;
    });

    // Wait for server to bind
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 1. Check /health
    let (code, body) = http_get(port, "/health").await;
    assert_eq!(code, 200);
    assert!(body.contains("\"status\":\"UP\""));
    assert!(body.contains("\"can_emit_signals\":true"));

    // 2. Check /ready when healthy
    let (code, body) = http_get(port, "/ready").await;
    assert_eq!(code, 200);
    assert_eq!(body, "READY");

    // 3. Check /metrics
    let (code, body) = http_get(port, "/metrics").await;
    assert_eq!(code, 200);
    assert!(body.contains("risk_engine_is_frozen 0"));
    assert!(body.contains("risk_engine_unresolved_retracts 0"));

    // 4. Freeze protocol -> check /ready returns 503
    gate.set_frozen(true);
    let (code, body) = http_get(port, "/ready").await;
    assert_eq!(code, 503);
    assert!(body.contains("\"status\":\"DEGRADED\""));
    assert!(body.contains("Freeze Mode Active"));
}
