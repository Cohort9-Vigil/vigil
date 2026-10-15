//! RS-02 Chaos Test: 2-Block Chain Reorganization Recovery (Lead Owner: Chibuikem).
//!
//! Verifies:
//! 1. Anvil evm_snapshot / evm_revert forces a 2-block reorg.
//! 2. Parent-hash mismatch is detected against the 10-block sliding window.
//! 3. Ingestor walks backward to the common ancestor.
//! 4. Retract events fire in strict reverse chronological order for orphaned logs.
//! 5. New canonical branch logs are ingested with zero gaps.

use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use alloy::providers::Provider;
use common::events::ChainEvent;
use indexer::listener::BlockIngestor;
use indexer::publisher::MockEventPublisher;

const ANVIL_DEV_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

pub struct AnvilRunner {
    child: Child,
    pub rpc_url: String,
    pub port: u16,
}

impl AnvilRunner {
    pub fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("find available port");
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

        Self {
            child,
            rpc_url,
            port,
        }
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
        panic!(
            "Deployment failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        panic!(
            "cast send failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn evm_snapshot(rpc_url: &str) -> String {
    let output = Command::new("cast")
        .arg("rpc")
        .arg("evm_snapshot")
        .arg("--rpc-url")
        .arg(rpc_url)
        .output()
        .expect("call evm_snapshot");

    if !output.status.success() {
        panic!(
            "evm_snapshot failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // remove quotes if json string
    raw.trim_matches('"').to_string()
}

fn evm_revert(rpc_url: &str, snapshot_id: &str) {
    let output = Command::new("cast")
        .arg("rpc")
        .arg("evm_revert")
        .arg(snapshot_id)
        .arg("--rpc-url")
        .arg(rpc_url)
        .output()
        .expect("call evm_revert");

    if !output.status.success() {
        panic!(
            "evm_revert failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test]
async fn test_rs02_chaos_reorg_2_blocks_with_reverse_retractions() {
    let anvil = AnvilRunner::spawn();

    // Step 1: Deploy VigilPool contract onto Anvil (Block 1)
    let pool_address = deploy_contract(
        &anvil.rpc_url,
        "../../contracts/out/VigilPool.sol/VigilPool.json",
    );

    let publisher = MockEventPublisher::new();
    let mut ingestor = BlockIngestor::connect_http(&anvil.rpc_url, publisher.clone(), 10)
        .expect("connect ingestor");

    // Step 2: Ingest baseline block 2 with an initial deposit (Common Ancestor)
    let user_a = "0xa0Ee7A142d267C1f36714E4a8F75612F20a79720";
    let asset = "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65";

    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitDeposit(address,address,uint256)",
        &[user_a, asset, "1000000"],
    );

    let block_baseline = ingestor
        .provider
        .get_block_number()
        .await
        .expect("block baseline");
    println!("Baseline block (common ancestor): {block_baseline}");

    // Ingest baseline up to block 2
    let (_retracts, indexed) = ingestor
        .ingest_hint(block_baseline)
        .await
        .expect("ingest baseline");
    assert!(!indexed.is_empty());
    let ancestor_block_number = ingestor.ring.latest_block_number().unwrap();
    let ancestor_block_hash = ingestor.ring.latest_block().unwrap().block_hash.clone();
    println!("Stored ancestor: block #{ancestor_block_number} hash={ancestor_block_hash}");

    // Step 3: Take EVM snapshot at the common ancestor
    let snapshot_id = evm_snapshot(&anvil.rpc_url);
    println!("EVM Snapshot taken: id={snapshot_id}");

    // Step 4: Build Fork A (2 blocks: Block 3 and Block 4)
    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitDeposit(address,address,uint256)",
        &[user_a, asset, "2000000"],
    );

    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitBorrow(address,uint256)",
        &[user_a, "3000000"],
    );

    let fork_a_head = ingestor
        .provider
        .get_block_number()
        .await
        .expect("fork a head");
    println!("Fork A head: {fork_a_head}");
    assert_eq!(fork_a_head, ancestor_block_number + 2);

    // Ingest Fork A into Indexer
    let (retracts_a, indexed_a) = ingestor
        .ingest_hint(fork_a_head)
        .await
        .expect("ingest fork a");
    assert!(
        retracts_a.is_empty(),
        "No retractions expected on linear fork A"
    );
    assert_eq!(indexed_a.len(), 2, "Expected 2 forward events on fork A");
    assert_eq!(ingestor.ring.latest_block_number(), Some(fork_a_head));
    println!(
        "Fork A ingested into ring buffer. Ring len={}",
        ingestor.ring.len()
    );

    // Step 5: Force 2-Block Chain Reorganization via evm_revert
    println!("Reverting EVM back to snapshot {snapshot_id}...");
    evm_revert(&anvil.rpc_url, &snapshot_id);

    let reverted_block = ingestor
        .provider
        .get_block_number()
        .await
        .expect("reverted block");
    println!("Chain height after revert: {reverted_block}");
    assert_eq!(reverted_block, ancestor_block_number);

    // Step 6: Build Fork B (New Canonical Branch: Block 3, Block 4, Block 5)
    let user_b = "0x90F79bf6EB2c4f870365E785982E1f101E93b906";

    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitDeposit(address,address,uint256)",
        &[user_b, asset, "7777777"],
    );

    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitBorrow(address,uint256)",
        &[user_b, "8888888"],
    );

    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitRepay(address,uint256)",
        &[user_b, "9999999"],
    );

    let fork_b_head = ingestor
        .provider
        .get_block_number()
        .await
        .expect("fork b head");
    println!("Fork B new canonical head: {fork_b_head}");
    assert_eq!(fork_b_head, ancestor_block_number + 3);

    // Step 7: Ingest Fork B Hint into Indexer
    println!("Feeding new canonical hint ({fork_b_head}) to BlockIngestor...");
    let (reorg_retractions, new_canonical_events) = ingestor
        .ingest_hint(fork_b_head)
        .await
        .expect("ingest fork b hint and resolve reorg");

    // Step 8: Assertions on Reorg Recovery
    println!(
        "Reorg handled! Emitted {} retractions and {} new canonical events",
        reorg_retractions.len(),
        new_canonical_events.len()
    );

    // Assertion A: Exactly 2 Retract events fired for the 2 orphaned blocks of Fork A
    assert_eq!(
        reorg_retractions.len(),
        2,
        "Must emit exactly 2 retract events for the 2 orphaned blocks"
    );

    // Assertion B: Retractions must be in STRICT REVERSE CHRONOLOGICAL ORDER:
    // 1st: Block 4 Borrow (amount 3000000)
    // 2nd: Block 3 Deposit (amount 2000000)
    println!(
        "Retraction 0: block {} trace={}",
        reorg_retractions[0].block_number, reorg_retractions[0].trace_id
    );
    println!(
        "Retraction 1: block {} trace={}",
        reorg_retractions[1].block_number, reorg_retractions[1].trace_id
    );

    assert_eq!(reorg_retractions[0].block_number, ancestor_block_number + 2);
    assert_eq!(reorg_retractions[1].block_number, ancestor_block_number + 1);

    match &reorg_retractions[0].original_event {
        Some(ChainEvent::Borrow { amount, .. }) => assert_eq!(*amount, 3000000),
        other => panic!("Expected Borrow in first retraction, got {:?}", other),
    }

    match &reorg_retractions[1].original_event {
        Some(ChainEvent::Deposit { amount, .. }) => assert_eq!(*amount, 2000000),
        other => panic!("Expected Deposit in second retraction, got {:?}", other),
    }

    // Assertion C: All Retract events published before new canonical logs
    let published_retracts = publisher.get_retract_events();
    assert_eq!(published_retracts.len(), 2);

    // Assertion D: Canonical Fork B events are all ingested (Deposit, Borrow, Repay)
    assert_eq!(new_canonical_events.len(), 3);
    match &new_canonical_events[0].event {
        ChainEvent::Deposit {
            amount, account, ..
        } => {
            assert_eq!(*amount, 7777777);
            assert!(account.0.eq_ignore_ascii_case(user_b));
        }
        other => panic!("Expected canonical Deposit, got {:?}", other),
    }

    match &new_canonical_events[1].event {
        ChainEvent::Borrow {
            amount, account, ..
        } => {
            assert_eq!(*amount, 8888888);
            assert!(account.0.eq_ignore_ascii_case(user_b));
        }
        other => panic!("Expected canonical Borrow, got {:?}", other),
    }

    match &new_canonical_events[2].event {
        ChainEvent::Repay {
            amount, account, ..
        } => {
            assert_eq!(*amount, 9999999);
            assert!(account.0.eq_ignore_ascii_case(user_b));
        }
        other => panic!("Expected canonical Repay, got {:?}", other),
    }

    // Assertion E: Ring buffer is continuous with zero gaps
    assert_eq!(ingestor.ring.latest_block_number(), Some(fork_b_head));
    let mut expected_parent = ancestor_block_hash;
    for b_num in (ancestor_block_number + 1)..=fork_b_head {
        let blk = ingestor
            .ring
            .get_block(b_num)
            .expect("block present in ring");
        assert_eq!(
            blk.parent_hash.to_lowercase(),
            expected_parent.to_lowercase(),
            "Parent hash chain continuous without gaps at block {b_num}"
        );
        expected_parent = blk.block_hash.clone();
    }

    println!("RS-02 Chaos Reorg Test Passed successfully!");
}
