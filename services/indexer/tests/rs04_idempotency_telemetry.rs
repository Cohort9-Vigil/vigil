//! RS-04 Verification Test Suite: Message Schema, Idempotency Keys, Trace IDs, and Telemetry
//!
//! Requirement RS-04:
//! "Every published event carries a deterministic idempotency key (block_hash, log_index)
//! and a propagated trace_id. Required verification test: Unit tests on message schema
//! and NATS header injection."

extern crate vigil_indexer as indexer;

use common::events::{ChainEvent, IdempotencyKey, IndexedEvent, RetractEvent};
use common::types::Address;
use indexer::publisher::{EventPublisher, MockEventPublisher};
use indexer::telemetry::{start_health_and_metrics_server, Metrics};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_rs04_deterministic_idempotency_key_generation() {
    let block_hash_a = "0x4a72d3e9185a12ef680693a61e389b25";
    let block_hash_b = "0x8f19c43b22079dae14589d3809afc4e1";

    let key_a_0 = IdempotencyKey::new(block_hash_a, 0);
    let key_a_0_dup = IdempotencyKey::new(block_hash_a, 0);
    let key_a_1 = IdempotencyKey::new(block_hash_a, 1);
    let key_b_0 = IdempotencyKey::new(block_hash_b, 0);

    // Deterministic equality
    assert_eq!(key_a_0, key_a_0_dup);
    assert_eq!(key_a_0.to_key_string(), key_a_0_dup.to_key_string());
    assert_eq!(
        key_a_0.to_key_string(),
        format!("{block_hash_a}:0")
    );

    // Uniqueness across distinct log indices
    assert_ne!(key_a_0, key_a_1);
    assert_ne!(key_a_0.to_key_string(), key_a_1.to_key_string());

    // Uniqueness across distinct block hashes
    assert_ne!(key_a_0, key_b_0);
    assert_ne!(key_a_0.to_key_string(), key_b_0.to_key_string());
}

#[tokio::test]
async fn test_rs04_indexed_event_schema_and_serialization() {
    let trace_id = "trace-rs04-root-987654";
    let block_hash = "0xabcdef1234567890";
    let tx_hash = "0x555544443333222211110000";
    let log_index = 4;
    let block_number = 42;
    let timestamp = 1718000000;

    let deposit_event = ChainEvent::Deposit {
        account: Address::new("0x1111111111111111111111111111111111111111"),
        asset: Address::new("0x2222222222222222222222222222222222222222"),
        amount: 25_000_000_000,
        block_number,
        tx_hash: tx_hash.to_string(),
    };

    let event = IndexedEvent::new(
        trace_id,
        block_number,
        block_hash,
        tx_hash,
        log_index,
        deposit_event.clone(),
        timestamp,
    );

    // Verify fields
    assert_eq!(event.trace_id, trace_id);
    assert_eq!(event.block_number, block_number);
    assert_eq!(event.block_hash, block_hash);
    assert_eq!(event.tx_hash, tx_hash);
    assert_eq!(event.log_index, log_index);
    assert_eq!(event.timestamp, timestamp);
    assert_eq!(
        event.idempotency_key,
        IdempotencyKey::new(block_hash, log_index)
    );

    // Verify JSON schema round-trip serialization
    let serialized = serde_json::to_string(&event).expect("serialize event to JSON");
    assert!(serialized.contains(r#""trace_id":"trace-rs04-root-987654""#));
    assert!(serialized.contains(r#""idempotency_key":{"block_hash":"0xabcdef1234567890","log_index":4}"#));
    assert!(serialized.contains(r#""Deposit""#));

    let deserialized: IndexedEvent =
        serde_json::from_str(&serialized).expect("deserialize event from JSON");
    assert_eq!(event, deserialized);
}

#[tokio::test]
async fn test_rs04_retract_event_schema_and_serialization() {
    let trace_id = "trace-rs04-retract-112233";
    let block_hash = "0xorphaned112233";
    let tx_hash = "0xtxreorged";
    let log_index = 2;
    let block_number = 88;
    let timestamp = 1718000500;

    let original = ChainEvent::Borrow {
        account: Address::new("0x3333333333333333333333333333333333333333"),
        amount: 500_000_000,
        block_number,
        tx_hash: tx_hash.to_string(),
    };

    let retract = RetractEvent::new(
        trace_id,
        block_number,
        block_hash,
        tx_hash,
        log_index,
        Some(original.clone()),
        timestamp,
    );

    assert_eq!(retract.trace_id, trace_id);
    assert_eq!(retract.block_number, block_number);
    assert_eq!(retract.idempotency_key.to_key_string(), format!("{block_hash}:{log_index}"));
    assert_eq!(retract.original_event, Some(original));

    let serialized = serde_json::to_string(&retract).expect("serialize retract event");
    let deserialized: RetractEvent =
        serde_json::from_str(&serialized).expect("deserialize retract event");
    assert_eq!(retract, deserialized);
}

#[tokio::test]
async fn test_rs04_nats_header_injection_and_trace_propagation() {
    let publisher = MockEventPublisher::new();

    let event = IndexedEvent::new(
        "trace-span-alpha-123",
        50,
        "0xhash50",
        "0xtx50",
        3,
        ChainEvent::Deposit {
            account: Address::new("0xaaaa"),
            asset: Address::new("0xbbbb"),
            amount: 100,
            block_number: 50,
            tx_hash: "0xtx50".into(),
        },
        1700000050,
    );

    publisher.publish_indexed_event(&event).await.expect("publish indexed");

    let retract = RetractEvent::new(
        "trace-span-retract-456",
        50,
        "0xhash50",
        "0xtx50",
        3,
        None,
        1700000051,
    );

    publisher.publish_retract_event(&retract).await.expect("publish retract");

    // Verify injected headers for forward event
    let indexed_records = publisher.get_indexed_records();
    assert_eq!(indexed_records.len(), 1);
    assert_eq!(indexed_records[0].nats_msg_id, "0xhash50:3");
    assert_eq!(indexed_records[0].trace_id, "trace-span-alpha-123");

    // Verify injected headers for retraction event
    let retract_records = publisher.get_retract_records();
    assert_eq!(retract_records.len(), 1);
    assert_eq!(retract_records[0].nats_msg_id, "retract:0xhash50:3");
    assert_eq!(retract_records[0].trace_id, "trace-span-retract-456");
}

async fn fetch_http(addr: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect to http");
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.expect("write request");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).await.expect("read response");
    resp
}

#[tokio::test]
async fn test_rs04_telemetry_http_endpoints_and_prometheus_metrics() {
    let metrics = Metrics::new();
    metrics.inc_events(12);
    metrics.inc_reorgs();
    metrics.inc_retractions(3);
    metrics.set_last_block(120);

    // Find an ephemeral port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let handle = start_health_and_metrics_server(port, metrics.clone())
        .await
        .expect("start telemetry server");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let addr = format!("127.0.0.1:{port}");

    // 1. Test /health HTTP endpoint
    let health_resp = fetch_http(&addr, "/health").await;
    assert!(health_resp.contains("HTTP/1.1 200 OK"));
    assert!(health_resp.contains("application/json"));
    assert!(health_resp.contains(r#"{"status":"healthy","last_indexed_block":120}"#));

    // 2. Test /metrics HTTP endpoint
    let metrics_resp = fetch_http(&addr, "/metrics").await;
    assert!(metrics_resp.contains("HTTP/1.1 200 OK"));
    assert!(metrics_resp.contains("events_processed_total 12"));
    assert!(metrics_resp.contains("reorgs_handled_total 1"));
    assert!(metrics_resp.contains("retractions_emitted_total 3"));
    assert!(metrics_resp.contains("last_indexed_block 120"));

    handle.abort();
}
