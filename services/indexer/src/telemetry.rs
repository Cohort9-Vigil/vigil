//! Observability, Prometheus metrics, and HTTP health check endpoint (RS-04).
//!
//! Provides:
//! - Prometheus counters: `events_processed_total`, `reorgs_handled_total`, `retractions_emitted_total`
//! - Prometheus gauge: `last_indexed_block`
//! - Lightweight asynchronous HTTP server for `/health` and `/metrics`

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use anyhow::{anyhow, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, error, info};

/// Atomic counters and gauges tracking live Indexer telemetry.
#[derive(Debug, Default, Clone)]
pub struct Metrics {
    pub events_processed_total: Arc<AtomicU64>,
    pub reorgs_handled_total: Arc<AtomicU64>,
    pub retractions_emitted_total: Arc<AtomicU64>,
    pub last_indexed_block: Arc<AtomicU64>,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_events(&self, count: u64) {
        self.events_processed_total.fetch_add(count, Ordering::Relaxed);
    }

    pub fn inc_reorgs(&self) {
        self.reorgs_handled_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_retractions(&self, count: u64) {
        self.retractions_emitted_total.fetch_add(count, Ordering::Relaxed);
    }

    pub fn set_last_block(&self, block: u64) {
        self.last_indexed_block.store(block, Ordering::Relaxed);
    }

    pub fn get_events(&self) -> u64 {
        self.events_processed_total.load(Ordering::Relaxed)
    }

    pub fn get_reorgs(&self) -> u64 {
        self.reorgs_handled_total.load(Ordering::Relaxed)
    }

    pub fn get_retractions(&self) -> u64 {
        self.retractions_emitted_total.load(Ordering::Relaxed)
    }

    pub fn get_last_block(&self) -> u64 {
        self.last_indexed_block.load(Ordering::Relaxed)
    }

    /// Renders Prometheus text exposition format (version 0.0.4).
    pub fn render_prometheus(&self) -> String {
        format!(
            "# HELP events_processed_total Total number of chain events processed\n\
             # TYPE events_processed_total counter\n\
             events_processed_total {}\n\n\
             # HELP reorgs_handled_total Total number of chain reorganizations handled\n\
             # TYPE reorgs_handled_total counter\n\
             reorgs_handled_total {}\n\n\
             # HELP retractions_emitted_total Total number of retractions emitted during reorgs\n\
             # TYPE retractions_emitted_total counter\n\
             retractions_emitted_total {}\n\n\
             # HELP last_indexed_block Latest EVM block number successfully processed\n\
             # TYPE last_indexed_block gauge\n\
             last_indexed_block {}\n",
            self.get_events(),
            self.get_reorgs(),
            self.get_retractions(),
            self.get_last_block(),
        )
    }
}

/// Spawns an asynchronous HTTP server serving `/health` and `/metrics`.
pub async fn start_health_and_metrics_server(
    port: u16,
    metrics: Metrics,
) -> Result<tokio::task::JoinHandle<()>> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await
        .map_err(|e| anyhow!("Failed to bind HTTP health/metrics server on port {port}: {e}"))?;

    info!(port, "Telemetry & Health HTTP server listening on http://0.0.0.0:{port}/health and /metrics");

    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((mut socket, _addr)) => {
                    let metrics_clone = metrics.clone();
                    tokio::spawn(async move {
                        let mut buf = [0u8; 2048];
                        let n = match socket.read(&mut buf).await {
                            Ok(n) if n > 0 => n,
                            _ => return,
                        };

                        let request = String::from_utf8_lossy(&buf[..n]);
                        let first_line = request.lines().next().unwrap_or("");

                        let (status_line, content_type, body) = if first_line.starts_with("GET /health") {
                            let block = metrics_clone.get_last_block();
                            let json = format!(r#"{{"status":"healthy","last_indexed_block":{block}}}"#);
                            ("HTTP/1.1 200 OK", "application/json", json)
                        } else if first_line.starts_with("GET /metrics") {
                            let text = metrics_clone.render_prometheus();
                            ("HTTP/1.1 200 OK", "text/plain; version=0.0.4", text)
                        } else {
                            ("HTTP/1.1 404 Not Found", "text/plain", "Not Found".to_string())
                        };

                        let response = format!(
                            "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );

                        if let Err(e) = socket.write_all(response.as_bytes()).await {
                            debug!(error = %e, "Failed to write HTTP response");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "Error accepting connection in telemetry HTTP server");
                }
            }
        }
    });

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_metrics_counting_and_rendering() {
        let metrics = Metrics::new();
        assert_eq!(metrics.get_events(), 0);
        assert_eq!(metrics.get_reorgs(), 0);
        assert_eq!(metrics.get_retractions(), 0);
        assert_eq!(metrics.get_last_block(), 0);

        metrics.inc_events(5);
        metrics.inc_reorgs();
        metrics.inc_retractions(2);
        metrics.set_last_block(105);

        assert_eq!(metrics.get_events(), 5);
        assert_eq!(metrics.get_reorgs(), 1);
        assert_eq!(metrics.get_retractions(), 2);
        assert_eq!(metrics.get_last_block(), 105);

        let rendered = metrics.render_prometheus();
        assert!(rendered.contains("events_processed_total 5"));
        assert!(rendered.contains("reorgs_handled_total 1"));
        assert!(rendered.contains("retractions_emitted_total 2"));
        assert!(rendered.contains("last_indexed_block 105"));
    }
}
