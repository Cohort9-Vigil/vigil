//! RS-03 Verification Test Suite: Crash-Safe Cursor Persistence and Kill-and-Restart Recovery
//!
//! Requirement RS-03:
//! "Persists cursor (block number + hash) in Postgres after NATS ack. Restart resumes with no gaps
//! and no permanent duplicates. Required verification test: Kill-and-restart test comparing published event stream."
//!
//! Instructor Checklist:
//! "Crash safety between any two lines: If the Indexer crashes after publishing to NATS but before
//! updating Postgres, it replays the block on restart; downstream consumers deduplicate via (block_hash, log_index)."

use std::collections::HashSet;
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use alloy::providers::Provider;
use common::events::ChainEvent;
use common::types::Address;
use indexer::cursor::{CursorStore, MockCursorStore};
use indexer::listener::BlockIngestor;
use indexer::publisher::{EventPublisher, MockEventPublisher};

const ANVIL_DEV_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

pub struct AnvilRunner {
    child: Child,
    pub rpc_url: String,
    pub port: u16,
}

impl AnvilRunner {
    pub fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("find port");
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let child = Command::new("anvil")
            .arg("--port")
            .arg(port.to_string())
            .arg("--silent")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn anvil");

        let rpc_url = format!("http://127.0.0.1:{port}");
        let start = Instant::now();
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            if start.elapsed() > Duration::from_secs(5) {
                panic!("Timed out waiting for Anvil on port {port}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(150));

        Self { child, rpc_url, port }
    }
}

impl Drop for AnvilRunner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn deploy_contract(rpc_url: &str, artifact_path: &str) -> String {
    let artifact_json = std::fs::read_to_string(artifact_path)
        .unwrap_or_else(|e| panic!("Failed to read {artifact_path}: {e}"));
    let parsed: serde_json::Value = serde_json::from_str(&artifact_json).expect("parse json");
    let bytecode = parsed["bytecode"]["object"]
        .as_str()
        .expect("bytecode string")
        .to_string();

    let output = Command::new("cast")
        .arg("send")
        .arg("--rpc-url")
        .arg(rpc_url)
        .arg("--private-key")
        .arg(ANVIL_DEV_PRIVATE_KEY)
        .arg("--create")
        .arg(bytecode)
        .output()
        .expect("deploy contract via cast");

    if !output.status.success() {
        panic!("Deployment failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("contractAddress") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return parts[1].to_string();
            }
        }
    }
    panic!("Could not find contractAddress in cast output:\n{stdout}");
}

fn call_cast_send(rpc_url: &str, to: &str, sig: &str, args: &[&str]) {
    let mut cmd = Command::new("cast");
    cmd.arg("send")
        .arg("--rpc-url")
        .arg(rpc_url)
        .arg("--private-key")
        .arg(ANVIL_DEV_PRIVATE_KEY)
        .arg(to)
        .arg(sig)
        .args(args);

    let output = cmd.output().expect("execute cast send");
    if !output.status.success() {
        panic!("cast send failed: {}", String::from_utf8_lossy(&output.stderr));
    }
}

#[tokio::test]
async fn test_rs03_kill_and_restart_resumes_without_gaps() {
    let anvil = AnvilRunner::spawn();
    // Block 1: Deployment of VigilPool
    let pool_address = deploy_contract(&anvil.rpc_url, "../../contracts/out/EventEmitterPool.sol/EventEmitterPool.json");

    let user = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let asset = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";

    // Block 2 and 3: Initial deposits
    for i in 1..=2 {
        call_cast_send(
            &anvil.rpc_url,
            &pool_address,
            "emitDeposit(address,address,uint256)",
            &[user, asset, &(i * 1000).to_string()],
        );
    }

    let shared_cursor_store = MockCursorStore::new();
    let publisher_pre_kill = MockEventPublisher::new();

    // Ingest blocks 1..=3
    {
        let mut ingestor = BlockIngestor::connect_http(&anvil.rpc_url, publisher_pre_kill.clone(), 10)
            .expect("connect ingestor")
            .with_cursor_store(shared_cursor_store.clone());

        ingestor.ingest_hint(3).await.expect("ingest blocks 1..=3");
        assert_eq!(ingestor.last_processed_block, Some(3));
    } // <-- Ingestor is "killed" here (dropped)

    // Verify cursor persisted at block 3
    let cursor = shared_cursor_store.get_last_cursor().await.unwrap();
    assert_eq!(cursor.map(|c| c.0), Some(3));

    let pre_events = publisher_pre_kill.get_indexed_events();
    // Block 2 and Block 3 each emitted 1 deposit event
    assert_eq!(pre_events.len(), 2);
    assert_eq!(pre_events[0].block_number, 2);
    assert_eq!(pre_events[1].block_number, 3);

    // 2. Mine blocks 4 and 5 while indexer is DOWN
    for i in 3..=4 {
        call_cast_send(
            &anvil.rpc_url,
            &pool_address,
            "emitDeposit(address,address,uint256)",
            &[user, asset, &(i * 1000).to_string()],
        );
    }

    // 3. Restart Indexer: recover cursor from store and resume
    let publisher_post_kill = MockEventPublisher::new();
    let mut restarted_ingestor = BlockIngestor::connect_http(&anvil.rpc_url, publisher_post_kill.clone(), 10)
        .expect("reconnect ingestor")
        .with_cursor_store(shared_cursor_store.clone());

    let recovered_block = restarted_ingestor.recover_from_cursor_store(10).await.expect("recover cursor");
    assert_eq!(recovered_block, Some(3));
    assert_eq!(restarted_ingestor.last_processed_block, Some(3));

    // Ingest up to block 5: automatically resumes from cursor + 1 (block 4)
    restarted_ingestor.ingest_hint(5).await.expect("ingest blocks 4..=5");
    assert_eq!(restarted_ingestor.last_processed_block, Some(5));

    // Verify post-restart events: only blocks 4 and 5 were ingested
    let post_events = publisher_post_kill.get_indexed_events();
    assert_eq!(post_events.len(), 2);
    assert_eq!(post_events[0].block_number, 4);
    assert_eq!(post_events[1].block_number, 5);

    // Verify final cursor
    let final_cursor = shared_cursor_store.get_last_cursor().await.unwrap();
    assert_eq!(final_cursor.map(|c| c.0), Some(5));

    // Combined stream verification: blocks 2, 3, 4, 5 in strict sequential order without gaps
    let combined_blocks: Vec<u64> = pre_events.iter().chain(post_events.iter()).map(|e| e.block_number).collect();
    assert_eq!(combined_blocks, vec![2, 3, 4, 5]);
}

#[tokio::test]
async fn test_rs03_crash_after_nats_publish_before_postgres_commit() {
    let anvil = AnvilRunner::spawn();
    // Block 1: Deployment
    let pool_address = deploy_contract(&anvil.rpc_url, "../../contracts/out/EventEmitterPool.sol/EventEmitterPool.json");

    let user = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let asset = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";

    // Block 2: deposit 1
    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitDeposit(address,address,uint256)",
        &[user, asset, "500"],
    );

    // Block 3: deposit 2
    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitDeposit(address,address,uint256)",
        &[user, asset, "1000"],
    );

    let shared_cursor_store = MockCursorStore::new();
    let shared_publisher = MockEventPublisher::new();

    // Ingest blocks 1..=2 normally
    let mut ingestor = BlockIngestor::connect_http(&anvil.rpc_url, shared_publisher.clone(), 10)
        .expect("connect ingestor")
        .with_cursor_store(shared_cursor_store.clone());

    ingestor.ingest_hint(2).await.expect("ingest blocks 1..=2");
    assert_eq!(shared_cursor_store.get_last_cursor().await.unwrap().map(|c| c.0), Some(2));

    // Simulate crash at block 3:
    // Event for block 3 is published to NATS, but crash occurs before Postgres commit completes
    // (So cursor in store remains at block 2)
    let block3 = ingestor.provider.get_block_by_number(alloy::rpc::types::eth::BlockNumberOrTag::Number(3))
        .await.unwrap().unwrap();
    let block3_hash = format!("{:?}", block3.header.hash);

    let simulated_event = common::events::IndexedEvent::new(
        "trace-crash-test",
        3,
        &block3_hash,
        "0xtx3",
        0,
        ChainEvent::Deposit {
            account: Address::new(user),
            asset: Address::new(asset),
            amount: 1000,
            block_number: 3,
            tx_hash: "0xtx3".into(),
        },
        block3.header.timestamp,
    );
    shared_publisher.publish_indexed_event(&simulated_event).await.unwrap();

    // Notice: cursor store was NOT committed for block 3 (it is still at block 2)
    assert_eq!(shared_cursor_store.get_last_cursor().await.unwrap().map(|c| c.0), Some(2));

    // --- RECOVERY / RESTART ---
    // Restart indexer: it reads cursor=2 from store, and replays block 3
    let mut restarted = BlockIngestor::connect_http(&anvil.rpc_url, shared_publisher.clone(), 10)
        .expect("reconnect ingestor")
        .with_cursor_store(shared_cursor_store.clone());

    restarted.recover_from_cursor_store(10).await.expect("recover cursor");
    assert_eq!(restarted.last_processed_block, Some(2));

    // Replay block 3
    restarted.ingest_hint(3).await.expect("re-ingest block 3");
    assert_eq!(restarted.last_processed_block, Some(3));
    assert_eq!(shared_cursor_store.get_last_cursor().await.unwrap().map(|c| c.0), Some(3));

    // Downstream verification:
    // Block 3 was published twice due to the crash, but both copies carry the EXACT SAME
    // deterministic idempotency key (block_hash:log_index)
    let all_published = shared_publisher.get_indexed_events();
    let block3_events: Vec<_> = all_published.iter().filter(|e| e.block_number == 3).collect();
    assert_eq!(block3_events.len(), 2, "Block 3 was replayed on crash");

    assert_eq!(
        block3_events[0].idempotency_key,
        block3_events[1].idempotency_key,
        "Both publications share the identical deterministic idempotency key"
    );

    // Downstream deduplication set
    let mut seen_keys = HashSet::new();
    let mut deduplicated_events = Vec::new();
    for evt in all_published {
        if seen_keys.insert(evt.idempotency_key.to_key_string()) {
            deduplicated_events.push(evt);
        }
    }

    // After deduplication, exactly 2 unique canonical events exist (block 2 and block 3) without gaps
    assert_eq!(deduplicated_events.len(), 2);
    assert_eq!(deduplicated_events[0].block_number, 2);
    assert_eq!(deduplicated_events[1].block_number, 3);
}
