# Liquidator Microservice: Presentation Cheat-Sheet & Tutor Q&A

This cheat-sheet provides an executive summary, architectural deep-dive, technical tutor Q&A (covering RS-11 through RS-16), and terminal quick-start commands for the **Vigil Liquidator** microservice (`services/liquidator/`).

---

## 1. Executive Summary

In simple terms, the **Liquidator** is the execution arm ("the hands") of the Vigil lending protocol:
* **Purpose**: Safeguards protocol solvency by liquidating undercollateralized loans before they become bad debt.
* **How It Works**:
  1. Consumes real-time liquidation signals emitted by the **Risk Engine** over NATS JetStream.
  2. Re-verifies borrower health factor and signal freshness directly against the blockchain.
  3. Evaluates liquidation profitability via a Dutch auction strategy (`ImmediateStrike` vs. `WaitForMargin`).
  4. Pulls pre-signed borrower liquidation intents from PostgreSQL.
  5. Signs and broadcasts transactions (starting auctions or liquidating debt) using a secure local keystore.
  6. Tracks every in-flight transaction through confirmation, handling fee escalation (12.5% EIP-1559 bumps), chain reorganizations (reorgs), and nonce gap-filling.

---

## 2. Architecture Overview

The liquidator is decomposed into decoupled, testable components where pure state logic is isolated from network/IO side effects.

### Data Flow & Component Interaction

```text
       ┌────────────────────────┐
       │ NATS JetStream (Signals)│
       └───────────┬────────────┘
                   │ LiquidationSignal
                   ▼
┌────────────────────────────────────────────────────────┐
│                        main.rs                         │
│  - Event loop (select! on signals + block interval)    │
│  - NATS pull consumer (DeliverPolicy::LastPerSubject)   │
└──────────┬───────────────────────────────┬─────────────┘
           │                               │
     handle_signal()                     tick()
           ▼                               ▼
┌────────────────────────────────────────────────────────┐
│                      executor.rs                       │
│  - Signal freshness (RS-09) & health verification (RS-16)│
│  - Auction lifecycle: startAuction vs liquidate        │
│  - In-flight tracking & 10-block reorg watch (RS-15)   │
└───┬─────────────┬──────────────┬──────────────┬────────┘
    │             │              │              │
    ▼             ▼              ▼              ▼
┌──────────┐ ┌──────────┐ ┌─────────────┐ ┌──────────────┐
│strategy.rs││ nonce.rs │ │  signer.rs  │ │   store.rs   │
│Strategy  │ │NonceBook │ │KeystoreSigner││PgIntentStore │
│trait     │ │(RS-13/14)│ │(RS-12)      │ │PgTxLog       │
└──────────┘ └──────────┘ └──────┬──────┘ └──────────────┘
                                 │ signs tx
                                 ▼
                         ┌──────────────┐
                         │    rpc.rs    │
                         │ RpcTxChain   │
                         └───────┬──────┘
                                 │ JSON-RPC / eth_sendRawTransaction
                                 ▼
                         ┌──────────────┐
                         │ Ethereum /   │
                         │ Anvil Node   │
                         └──────────────┘
```

### Module Responsibilities

| Module | File | Core Responsibilities |
|---|---|---|
| **Entry Point & Loop** | [`main.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/main.rs) | Loads configuration, initializes keystore and database connections, spins up NATS JetStream pull consumer (`liquidator-latest`), runs the async event loop balancing incoming signals and block tick intervals. |
| **Execution Engine** | [`executor.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/executor.rs) | Coordinates end-to-end execution: guards against duplicate in-flight transactions, confirms on-chain health factor $< 1.0$, triggers `startAuction` when needed, signs/sends `liquidate` calls, monitors unconfirmed transactions, and handles chain reorg recovery. |
| **Strategy Engine** | [`strategy.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/strategy.rs) | Implements the `Strategy` trait (RS-11). Evaluates `MarketView` (bonus bps, repay amount, gas cost) to return `Decision::Strike`, `Decision::Wait`, or `Decision::Skip`. |
| **Signing Isolation** | [`signer.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/signer.rs) | Enforces RS-12. Loads the encrypted keystore from disk using an env password. Derives an Alloy `EthereumWallet`. Explicitly redacts key material from `std::fmt::Debug`. |
| **Nonce State Book** | [`nonce.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/nonce.rs) | Pure in-memory state machine implementing RS-13, RS-14, and RS-15. Manages monotonic nonce allocations, 12.5% fee replacement planning, and gap-fill actions without needing a live chain to test. |
| **RPC & Chain Client** | [`rpc.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/rpc.rs) | Alloy-backed implementation of `TxChain`. Reads block headers, nonces (`pending` and `latest`), on-chain health factors, and transaction receipts; broadcasts EIP-1559 transactions with caller-specified nonces. |
| **Persistence Store** | [`store.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/store.rs) | Implements `IntentStore` (`PgIntentStore` reads signed EIP-712 borrower intents from PostgreSQL) and `TxLog` (`PgTxLog` writes durable transaction audit logs to the `liquidator_txs` table). |

---

## 3. Top Tutor Q&A: Technical Defense

### 1. Dutch Auction Strategy (RS-11)
**Q: How does the Dutch auction mechanism work in Vigil, and how do our strategies handle it?**
* **The Mechanism**:
  * When a borrower becomes undercollateralized, an auction must be started via `startAuction(borrower)`.
  * The liquidation bonus starts at 0 bps and increases over time (per block/timestamp) according to a configured curve up to a maximum cap.
  * This allows the protocol to liquidate positions with minimal collateral penalty when gas is cheap, while scaling the incentive higher if market volatility or gas prices demand it.
* **`ImmediateStrike` vs. `WaitForMargin`**:
  * `ImmediateStrike`: Strikes as soon as `bonus_bps > 0`. Useful in fast-falling markets to front-run insolvency, but risks negative net profit if gas cost outweighs the bonus.
  * `WaitForMargin`: Computes bonus value in debt units:
    $$\text{bonus\_value} = \frac{\text{repay} \times \text{bonus\_bps}}{10\,000}$$
    It strikes only when:
    $$\text{bonus\_value} \ge \frac{\text{gas\_cost\_debt} \times \text{margin\_bps}}{10\,000}$$
    With `margin_bps = 12000` (1.2x), it guarantees that the liquidator only strikes when the auction incentive yields at least a 20% margin over expected gas expenditure.
* **Two-Step Execution**:
  * If `auction_start(borrower) == 0`, the executor reserves a nonce and submits `startAuction(borrower)`.
  * Once the auction is active and the strategy returns `Decision::Strike`, the next signal or tick issues `liquidate(...)`.

---

### 2. Key Redaction & Signing Isolation (RS-12)
**Q: How does the service guarantee that private keys are never leaked, logged, or compromised?**
* **Single Module Boundary**:
  * The private key is contained entirely within `KeystoreSigner` ([`signer.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/signer.rs)). No other module has access to the raw key bytes or internal seed.
* **Encrypted Keystore Loading**:
  * Keys are never passed via CLI flags (which leak into process listings via `ps aux`).
  * The path is provided via `--keystore / KEYSTORE_PATH` and the decryption secret is retrieved strictly from the `KEYSTORE_PASSWORD` environment variable.
* **Custom Debug Implementation**:
  * `KeystoreSigner` overrides `std::fmt::Debug`:
    ```rust
    impl std::fmt::Debug for KeystoreSigner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("KeystoreSigner")
                .field("address", &self.address())
                .field("key", &"<redacted>")
                .finish()
        }
    }
    ```
  * Even when `tracing::info!(?signer)` is called in `main.rs`, secret key material is never written to logs, console output, or telemetry spans.
* **Wallet Exposure**:
  * Only `signer.wallet()` is passed directly to Alloy's `ProviderBuilder`, keeping signing isolated to the outbound RPC layer.

---

### 3. Nonce Synchronization & Concurrency (RS-13)
**Q: Why does a single-wallet liquidator need a dedicated `NonceBook`, and how does startup reconciliation work?**
* **Why `AtomicU64` Fails**:
  * Simple atomic counters fail during network failures: if a transaction fails to broadcast, reverts, or is rejected by the mempool, an atomic counter leaves an unrecoverable gap.
  * Calling `eth_getTransactionCount` immediately before every send also fails during concurrent async tasks, causing nonce collisions.
* **Dedicated In-Flight Bookkeeper**:
  * [`NonceBook`](file:///home/progidy/Desktop/vigil/services/liquidator/src/nonce.rs) manages sequential reservations (`reserve()`) and stores `InFlight` records (nonce, hash, fee, sent_block, purpose, trace_id).
* **Startup Reconciliation**:
  * On boot or crash recovery, `reconcile(latest, pending)` is executed:
    1. Fetches on-chain confirmed nonce count: `eth_getTransactionCount(addr, "latest")`.
    2. Fetches on-chain pending nonce count: `eth_getTransactionCount(addr, "pending")`.
    3. Sets `self.next = max(self.next, latest, pending)`.
    4. Purges all in-flight entries strictly below `latest` (as they are already confirmed and mined).
    5. Clears or settles lingering unconfirmed records in the database via `settle_pending()`.
  * Guarantees nonces never regress and startup resumes cleanly without collisions or stalls.

---

### 4. 12.5% Fee Bumping (RS-14)
**Q: Why is fee bumping set to 12.5%, and how does the replacement mechanism work?**
* **The 12.5% Rule**:
  * Standard Ethereum node implementations (Geth, Nethermind, Besu, Anvil) enforce EIP-1559 mempool replacement rules: to replace a pending transaction with the exact same nonce, the new transaction must offer at least a **10% to 12.5%** bump in both `max_fee_per_gas` and `max_priority_fee_per_gas`.
  * Submitting any lower increase results in a node rejection (`replacement transaction underpriced`).
* **Implementation (`bump_fee`)**:
  * In [`nonce.rs`](file:///home/progidy/Desktop/vigil/services/liquidator/src/nonce.rs):
    ```rust
    pub const MIN_FEE_BUMP_BPS: u128 = 1250; // +12.5%
    pub fn bump_fee(fee: u128) -> u128 {
        let factor = 10_000 + MIN_FEE_BUMP_BPS;
        let bumped = (fee * factor).div_ceil(10_000);
        bumped.max(fee + 1)
    }
    ```
  * Uses ceiling division (`div_ceil`) and adds `.max(fee + 1)` to guarantee positive integer escalation even at very low fee levels.
* **Replacement Trigger**:
  * During each block `tick()`, `NonceBook::plan` checks if `current_block >= sent_block + replace_after_blocks` (default 3 blocks).
  * If unconfirmed after 3 blocks, the executor issues an `Action::Replace`, rebroadcasting the same payload and nonce with bumped fees.

---

### 5. Reorg Gap-Filling & Recovery (RS-15)
**Q: What happens if a confirmed liquidation transaction gets reorged out, especially if the position is no longer liquidatable?**
* **The 10-Block Watch Window**:
  * Confirmed transactions are not immediately discarded; they enter a `watching` sliding window for `REORG_WATCH_BLOCKS = 10` blocks.
  * Every block tick queries `status(hash)`. If a previously confirmed transaction's status reverts to `TxStatus::Unknown`, it has been dropped by a chain reorganization.
* **Decision: Resubmit vs. Gap-Fill**:
  * When a dropped/reorged transaction is detected, `recover_dropped(nonce)` checks borrower health on the new chain branch:
    1. **Position Still Unhealthy ($HF < 1.0$)**: The liquidation is still needed. Re-encodes the call and re-broadcasts using the **exact same nonce**.
    2. **Position Now Healthy ($HF \ge 1.0$)**: (e.g., borrower repaid on the winning fork or prices rebounded). The liquidation call would revert!
* **The Zero-Value Self-Transfer (`GapFill`)**:
  * Ethereum transactions must be mined in strictly contiguous nonce order ($N, N+1, N+2, \dots$).
  * If nonce $N$ were simply abandoned, all subsequent transactions ($N+1, N+2, \dots$) would be permanently stuck in the mempool forever.
  * **Solution**: The liquidator issues a zero-value transfer to its own address (`to = self.address`, `value = 0`, `gas_limit = 21_000`) at nonce $N$.
  * This "plugs" the nonce gap cheaply, allowing downstream transactions to execute without stalling the pipeline.

---

### 6. Signal Freshness & Health Factor Re-Check (RS-16)
**Q: How do we prevent acting on stale NATS signals or double-liquidating?**
* **Freshness Check (RS-09)**:
  * In `handle_signal()`, if `current_block > signal.block_number + 2`, the signal is dropped as stale.
* **Live Health Check (RS-16)**:
  * The health factor in the signal is treated only as a hint. The executor immediately calls `chain.health_factor(borrower)` before signing.
  * If $HF \ge 1.0$, the signal is skipped (`Outcome::Skipped("healthy on chain")`).
* **In-Flight Deduplication**:
  * If any transaction for `signal.borrower` is already in the `NonceBook` inflight map, subsequent signals are skipped (`"already in flight"`), eliminating double-liquidation races.

---

## 4. Terminal Quick-Start

### 4.1 Running Local Unit Tests

The liquidator features 24 isolated unit tests verifying the strategy math, nonce state machine, fee bumping, reorg gap-filling, and executor flows without needing external services.

From the workspace root (`/home/progidy/Desktop/vigil`):

```bash
# Run all unit tests for the liquidator microservice
cargo test --manifest-path services/Cargo.toml -p vigil-liquidator

# Run only the pure NonceBook tests (RS-13, RS-14, RS-15)
cargo test --manifest-path services/Cargo.toml -p vigil-liquidator --lib nonce::tests

# Run only the Dutch Auction Strategy tests (RS-11)
cargo test --manifest-path services/Cargo.toml -p vigil-liquidator --lib strategy::tests

# Run only the Executor lifecycle & reorg tests (RS-15, RS-16)
cargo test --manifest-path services/Cargo.toml -p vigil-liquidator --lib executor::tests
```

Or from the `services/` directory:

```bash
cd services
cargo test -p vigil-liquidator
```

---

### 4.2 Running via Docker Compose

The entire Vigil platform (contracts, PostgreSQL, NATS JetStream, Anvil, Indexer, Risk Engine, and Liquidator) can be launched using Docker Compose.

#### 1. Spin Up the Stack
From the repository root (`/home/progidy/Desktop/vigil`):

```bash
# Start all dependencies and build the microservices
docker compose up --build
```

#### 2. Run Liquidator Standalone (with Dependencies)
If you wish to spin up dependencies first and run only the liquidator container:

```bash
# Start infrastructure containers
docker compose up -d postgres nats anvil

# Build and start the liquidator
docker compose up --build -d liquidator
```

#### 3. Inspect Service Logs
Follow real-time structured logs from the liquidator (filtered by trace ID or borrower address):

```bash
# Follow logs
docker compose logs -f liquidator

# Check container health and status
docker compose ps liquidator
```

#### 4. Useful Runtime Environment Flags
When debugging or testing custom strategy parameters, pass environment variables directly:

| Variable | Default | Purpose |
|---|---|---|
| `STRATEGY` | `margin` | Strategy selection: `margin` or `immediate`. |
| `MARGIN_BPS` | `12000` | Profitability margin in basis points (12,000 = 1.2x gas cost). |
| `GAS_COST_DEBT` | `1000000` | Estimated debt gas cost (6 decimals, e.g., 1 USDC). |
| `REPLACE_AFTER_BLOCKS` | `3` | Block threshold before applying 12.5% fee bump (RS-14). |
| `KEYSTORE_PASSWORD` | `vigil-dev-password` | Keystore decryption secret (RS-12). |
| `RUST_LOG` | `info` | Logging verbosity (e.g. `info,vigil_liquidator=debug`). |
