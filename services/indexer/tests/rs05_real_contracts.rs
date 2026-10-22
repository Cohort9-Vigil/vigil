extern crate vigil_indexer as indexer;

use std::process::Command;

use alloy::providers::Provider;
use common::events::ChainEvent;
use indexer::listener::BlockIngestor;
use indexer::publisher::MockEventPublisher;

#[tokio::test]
async fn test_real_contracts_deposit_and_borrow() {
    let rpc_url = "http://127.0.0.1:8545";

    // 1. Verify or deploy against Anvil
    // If anvil is not at block ~500+, run script/local.sh
    let provider = alloy::providers::ProviderBuilder::new()
        .connect_http(rpc_url.parse().expect("valid rpc url"));

    let block_res = provider.get_block_number().await;
    if block_res.is_err() {
        eprintln!("Anvil node not running on 8545; skipping live anvil test");
        return;
    }

    let current_block = block_res.unwrap();
    if current_block < 10 {
        // Deploy and seed using script/local.sh
        let deploy_status = Command::new("./script/local.sh")
            .current_dir("../../contracts")
            .env("RPC_URL", rpc_url)
            .status()
            .expect("run local.sh");
        assert!(deploy_status.success(), "local.sh should succeed");
    }

    // 2. Load deployment addresses from deployments/31337.json
    let deployment_path = std::path::Path::new("../../contracts/deployments/31337.json");
    assert!(deployment_path.exists(), "deployments/31337.json must exist");

    let dep_str = std::fs::read_to_string(deployment_path).expect("read deployments/31337.json");
    let dep_json: serde_json::Value = serde_json::from_str(&dep_str).expect("parse json");

    let pool_str = dep_json["pool"].as_str().expect("pool address");
    let settlement_str = dep_json["settlement"].as_str().expect("settlement address");
    let collateral_str = dep_json["sixDecimalToken"].as_str().expect("sixDecimalToken address");

    let pool_addr: alloy::primitives::Address = pool_str.parse().expect("valid pool address");
    let settlement_addr: alloy::primitives::Address = settlement_str.parse().expect("valid settlement address");

    // 3. Set up Indexer with MockEventPublisher
    let publisher = MockEventPublisher::new();
    let mut ingestor = BlockIngestor::new(provider.clone(), publisher.clone(), 20)
        .with_target_addresses(vec![pool_addr, settlement_addr]);

    let start_block = provider.get_block_number().await.expect("current block");

    // 4. Perform a real deposit and borrow using cast
    let test_user_key = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
    let test_user_addr = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Mint collateral to user
    let s1 = Command::new("cast")
        .args([
            "send",
            collateral_str,
            "mint(address,uint256)",
            test_user_addr,
            "5000000000",
            "--private-key",
            test_user_key,
            "--rpc-url",
            rpc_url,
        ])
        .status()
        .expect("mint collateral");
    assert!(s1.success(), "mint must succeed");

    // Approve pool
    let s2 = Command::new("cast")
        .args([
            "send",
            collateral_str,
            "approve(address,uint256)",
            pool_str,
            "5000000000",
            "--private-key",
            test_user_key,
            "--rpc-url",
            rpc_url,
        ])
        .status()
        .expect("approve collateral");
    assert!(s2.success(), "approve must succeed");

    // Real depositCollateral
    let s3 = Command::new("cast")
        .args([
            "send",
            pool_str,
            "depositCollateral(address,uint256)",
            collateral_str,
            "2000000000",
            "--private-key",
            test_user_key,
            "--rpc-url",
            rpc_url,
        ])
        .status()
        .expect("depositCollateral");
    assert!(s3.success(), "depositCollateral must succeed");

    // Real borrow
    let s4 = Command::new("cast")
        .args([
            "send",
            pool_str,
            "borrow(uint256)",
            "5000000",
            "--private-key",
            test_user_key,
            "--rpc-url",
            rpc_url,
        ])
        .status()
        .expect("borrow");
    assert!(s4.success(), "borrow must succeed");

    // 5. Ingest all new blocks since start_block
    let end_block = provider.get_block_number().await.expect("end block");
    assert!(end_block > start_block, "Blocks must have progressed");

    for block_num in (start_block + 1)..=end_block {
        let _ = ingestor.process_block(block_num).await.expect("process block");
    }

    // 6. Assert that published events contain CollateralDeposited and Borrowed
    let published = publisher.get_indexed_events();
    assert!(!published.is_empty(), "Events must be published");

    let has_deposit = published.iter().any(|evt| match &evt.event {
        ChainEvent::CollateralDeposited { account, asset, amount, .. } => {
            account.0.eq_ignore_ascii_case(test_user_addr)
                && asset.0.eq_ignore_ascii_case(collateral_str)
                && *amount == 2_000_000_000
        }
        _ => false,
    });
    assert!(has_deposit, "Indexer must publish real CollateralDeposited event");

    let has_borrow = published.iter().any(|evt| match &evt.event {
        ChainEvent::Borrowed { account, amount, .. } => {
            account.0.eq_ignore_ascii_case(test_user_addr) && *amount == 5_000_000
        }
        _ => false,
    });
    assert!(has_borrow, "Indexer must publish real Borrowed event");

    // Verify InterestAccrued was also published
    let has_interest = published.iter().any(|evt| matches!(&evt.event, ChainEvent::InterestAccrued { .. }));
    assert!(has_interest, "Indexer must publish InterestAccrued event");

    // Verify ordering: Within the borrow block, InterestAccrued logIndex < Borrowed logIndex
    let borrow_event = published.iter().find(|evt| matches!(&evt.event, ChainEvent::Borrowed { .. })).unwrap();
    let interest_event = published.iter().find(|evt| {
        evt.block_number == borrow_event.block_number && matches!(&evt.event, ChainEvent::InterestAccrued { .. })
    });
    if let Some(interest) = interest_event {
        assert!(
            interest.log_index < borrow_event.log_index,
            "InterestAccrued (logIndex={}) must precede Borrowed (logIndex={}) in the same block",
            interest.log_index,
            borrow_event.log_index
        );
    }
}
