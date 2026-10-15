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

#[tokio::test]
async fn test_rs01_live_anvil_ingestion_and_decoding() {
    let anvil = AnvilRunner::spawn();

    // 1. Deploy VigilPool and OracleAdapter contracts onto Anvil
    let pool_address = deploy_contract(
        &anvil.rpc_url,
        "../../contracts/out/EventEmitterPool.sol/EventEmitterPool.json",
    );
    let oracle_address = deploy_contract(
        &anvil.rpc_url,
        "../../contracts/out/EventEmitterOracle.sol/EventEmitterOracle.json",
    );

    assert!(pool_address.starts_with("0x"));
    assert!(oracle_address.starts_with("0x"));

    // 2. Trigger events: Deposit, Borrow, PriceUpdated
    let user1 = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let asset1 = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";

    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitDeposit(address,address,uint256)",
        &[user1, asset1, "5000000000000000000"],
    );

    call_cast_send(
        &anvil.rpc_url,
        &pool_address,
        "emitBorrow(address,uint256)",
        &[user1, "1200000000000000000"],
    );

    call_cast_send(
        &anvil.rpc_url,
        &oracle_address,
        "emitPriceUpdated(address,uint256,uint256)",
        &[asset1, "2500000000000000000", "1700000100"],
    );

    // 3. Instantiate BlockIngestor with MockEventPublisher
    let publisher = MockEventPublisher::new();
    let mut ingestor = BlockIngestor::connect_http(&anvil.rpc_url, publisher.clone(), 10)
        .expect("connect ingestor");

    let latest_block = ingestor
        .provider
        .get_block_number()
        .await
        .expect("get block number");
    println!("Latest block on Anvil: {latest_block}");

    // 4. Ingest from block 1 up to latest block
    let (_retracts, indexed) = ingestor
        .ingest_hint(latest_block)
        .await
        .expect("ingest hint");
    let published = publisher.get_indexed_events();
    println!(
        "Indexed len: {}, Published len: {}",
        indexed.len(),
        published.len()
    );
    for p in &published {
        println!("Emitted event: {:?}", p.event);
    }
    assert!(published.len() >= 3);

    let mut found_deposit = false;
    let mut found_borrow = false;
    let mut found_price = false;

    for evt in &published {
        // RS-04: Deterministic idempotency key and trace_id verification
        assert!(!evt.trace_id.is_empty(), "Trace ID must not be empty");
        assert_eq!(
            evt.idempotency_key.to_key_string(),
            format!("{}:{}", evt.block_hash, evt.log_index)
        );

        match &evt.event {
            ChainEvent::Deposit {
                account,
                asset,
                amount,
                ..
            } => {
                if account.0.eq_ignore_ascii_case(user1) && asset.0.eq_ignore_ascii_case(asset1) {
                    assert_eq!(*amount, 5_000_000_000_000_000_000);
                    found_deposit = true;
                }
            }
            ChainEvent::Borrow {
                account, amount, ..
            } if account.0.eq_ignore_ascii_case(user1) => {
                assert_eq!(*amount, 1_200_000_000_000_000_000);
                found_borrow = true;
            }
            ChainEvent::PriceUpdated(p) if p.asset.0.eq_ignore_ascii_case(asset1) => {
                assert_eq!(p.price_wad, 2_500_000_000_000_000_000);
                assert_eq!(p.timestamp, 1700000100);
                found_price = true;
            }
            _ => {}
        }
    }

    assert!(found_deposit, "Deposit event was not decoded properly");
    assert!(found_borrow, "Borrow event was not decoded properly");
    assert!(found_price, "PriceUpdated event was not decoded properly");
}
