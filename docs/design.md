# Vigil Indexer Architecture & Design Document

## 1. System Overview

The Vigil Indexer is a high-throughput, reorg-safe blockchain indexing microservice designed to ingest EVM logs from Ethereum/Anvil nodes, decode protocol events via typed `alloy sol!` bindings, enforce deterministic idempotency, persist crash-safe cursors in PostgreSQL, and stream enriched events across NATS JetStream to downstream consumers (Risk Engine, Liquidator).

```
+----------------+      newHeads hint      +-----------------------+
|  Anvil / EVM   | ----------------------> |    Block Ingestor     |
|   JSON-RPC     | <---------------------- |  & Reorg Ring Buffer  |
+----------------+  eth_getLogs(blockHash) +-----------------------+
                                                       |
                                            1. Publish | (Await JetStream Ack)
                                                       v
                                            +---------------------+
                                            |   NATS JetStream    |
                                            | (Deduplication via  |
                                            |    Nats-Msg-Id)     |
                                            +---------------------+
                                                       |
                                            2. Commit  | Cursor (block_num, hash)
                                                       v
                                            +---------------------+
                                            | PostgreSQL Storage  |
                                            |  (indexer_cursor &  |
                                            |   cursor_history)   |
                                            +---------------------+
```

---

## 2. Division of Responsibilities

The MVP indexer build was partitioned into two parallel, non-blocking lanes:

| Lane | Lead Owner | Scope & Deliverables | Requirements |
|---|---|---|---|
| **Chain & Reorg Engine** | **Chibuikem** | RPC hint polling, gap backfilling, `alloy sol!` log decoding, 10-block sliding window ring buffer, parent-hash validation, ancestor walk-back, reverse retraction emission. | RS-01, RS-02 |
| **Infra, Bus & Persistence** | **Japheth** | Docker Compose orchestration, NATS JetStream publisher, deterministic idempotency keys (`block_hash:log_index`), distributed trace ID injection, crash-resilient PostgreSQL cursor persistence, startup recovery, Prometheus metrics, and HTTP `/health` server. | RS-03, RS-04 |

---

## 3. Core Architectural Invariants (Section 6.2)

1. **Never Trust WebSocket Subscriptions Blindly**:
   WebSocket `newHeads` streams can skip blocks during network jitter or reconnection events. Heads are treated purely as *hints* that blocks exist up to height $N$. The `BlockIngestor` detects gaps between `last_processed_block` and $N$ and actively backfills every intermediate block.

2. **Query Logs Strictly by Block Hash, Never Block Number**:
   During a chain reorganization, querying logs by block number (`fromBlock = N, toBlock = N`) can return logs from an orphaned fork. Ingesting logs filtered exclusively by `Filter::new().at_block_hash(blockHash)` guarantees that decoded logs belong to the exact block evaluated by the reorg state machine.

3. **Publish to NATS Before Updating the Postgres Cursor**:
   Events are published to NATS JetStream and the acknowledgment is awaited *before* committing the updated cursor `(block_number, block_hash)` to PostgreSQL. Publishing and then crashing causes an at-least-once replay of the block upon restart; downstream consumers deduplicate seamlessly via the deterministic idempotency key. Inverting this order (committing the cursor before publishing) causes irreversible data loss if a crash occurs between the operations.

---

## 4. Message Envelopes & Idempotency Keys (RS-04)

### 4.1 Forward Event Envelope (`IndexedEvent`)
Each protocol event (Deposit, Borrow, Repay, LiquidationCall, PriceUpdated) is wrapped in an enriched envelope:
- `trace_id`: Distributed trace identifier (`trace-b{block}-l{log_index}-{uuid}`) propagated in NATS headers and payload.
- `idempotency_key`: `IdempotencyKey { block_hash, log_index }` formatted as `"{block_hash}:{log_index}"`.
- `block_number`, `block_hash`, `tx_hash`, `log_index`: Block context.
- `event`: Typed `ChainEvent` enum.
- `timestamp`: Block header timestamp.

### 4.2 Retraction Event Envelope (`RetractEvent`)
When a chain reorganization occurs, orphaned blocks emit `RetractEvent` envelopes:
- Published to `vigil.events.retract`.
- Emitted in **strict reverse chronological order** (from newest orphaned block back to the common ancestor).
- Carries `nats_msg_id` = `"retract:{block_hash}:{log_index}"`.
- Signals downstream state engines (Risk Engine) to unwind state mutations before ingesting the newly discovered canonical branch.

### 4.3 Broker-Level Deduplication
The NATS JetStream stream `VIGIL_EVENTS` is configured with:
- Subjects: `vigil.events.chain`, `vigil.events.retract`, `vigil.events.>`
- `duplicate_window: Duration::from_secs(300)` (5-minute sliding window)
- JetStream deduplicates messages natively by evaluating the injected `Nats-Msg-Id` header against previously acknowledged message IDs.

---

## 5. Persistence & Crash Recovery (RS-03)

### 5.1 Database Schema
Migrations in `migrations/` define:
- `blocks`: Historical record of canonical blocks (`block_number`, `block_hash`, `parent_hash`, `timestamp`, `is_reorged`).
- `indexer_cursor`: Single-row checkpoint table (`id = 'default'`, `last_processed_block`, `last_processed_hash`, `updated_at`).
- `cursor_history`: Historical block checkpoints tracking reorg status (`CANONICAL` or `ORPHANED`).

### 5.2 Commit Sequence & Transaction Boundary
For each canonical block $N$:
1. Fetch logs by `blockHash`.
2. Decode logs into typed enums.
3. Publish all `IndexedEvent`s to NATS JetStream with headers `Nats-Msg-Id` and `trace_id`.
4. Await JetStream broker publish acknowledgments.
5. In a PostgreSQL ACID transaction:
   - Upsert `indexer_cursor` with `(N, block_hash)`.
   - Upsert `blocks` table with `is_reorged = FALSE`.
   - Upsert `cursor_history` with status `'CANONICAL'`.
6. Push block into the in-memory 10-block sliding window ring buffer.
7. Update in-memory `last_processed_block` and telemetry gauge.

### 5.3 Startup Recovery Routine
On boot:
1. `PostgresCursorStore::get_last_cursor()` loads the latest committed `(block_number, block_hash)`.
2. `PostgresCursorStore::load_recent_window(10)` loads the last 10 canonical blocks in chronological order.
3. The in-memory `ReorgRingBuffer` is seeded with this recent block window, ensuring the reorg detection engine is active immediately without requiring a warm-up period.
4. `BlockIngestor` sets `last_processed_block = cursor.block_number` and resumes ingestion strictly from `cursor.block_number + 1`.

---

## 6. Section 6.6 Instructor Code-Review Defense

### Point 1: Crash safety between any two lines
> **Question**: *What happens if the Indexer process crashes or is killed between any two arbitrary lines of code?*
- **Crash before NATS publish**: No message was sent to JetStream. PostgreSQL cursor has not advanced. Upon restart, the Indexer reads the previous cursor and re-processes the block. Zero events lost.
- **Crash after NATS publish, but before PostgreSQL cursor commit**: The block's events were published to NATS JetStream and acknowledged by the broker. However, PostgreSQL cursor still points to block $N - 1$. Upon restart, the Indexer reads cursor $N - 1$ and re-processes block $N$. When block $N$ is published again:
  1. JetStream rejects duplicates during its 5-minute deduplication window via `Nats-Msg-Id: {block_hash}:{log_index}`.
  2. Even if outside the broker window, downstream consumers (Risk Engine, Liquidator) store previously processed idempotency keys `(block_hash, log_index)` in local caches or database constraints, safely dropping the redundant message.
  3. No gaps exist, and no permanent duplicate state mutations occur.
- **Crash after PostgreSQL cursor commit, but before in-memory state update**: PostgreSQL is the durable source of truth. Upon restart, the Indexer loads block $N$ from PostgreSQL and resumes from $N + 1$. Zero events lost.

### Point 2: Why at-least-once delivery?
> **Question**: *Why must we use at-least-once delivery rather than committing the PostgreSQL cursor before publishing to NATS?*
- In distributed systems, exactly-once delivery across two independent state stores (PostgreSQL and NATS JetStream) without two-phase commit (2PC) is impossible.
- If we commit the PostgreSQL cursor first, a crash occurring before NATS publication completes permanently records the block as "processed" in PostgreSQL. Upon restart, the Indexer resumes from $N + 1$, permanently skipping the events of block $N$. In DeFi liquidation protocols, skipping an Oracle price update or a borrower position update leads to undercollateralized debt and insolvency.
- By enforcing **publish-then-commit**, any crash results in at-least-once delivery (replay), which is rendered strictly safe and idempotent by our deterministic `(block_hash, log_index)` keys.

---

## 7. Observability & Telemetry (RS-04)

The Indexer microservice includes a non-blocking asynchronous HTTP server running on port `8080`:
- **`/health`**: Returns HTTP 200 JSON with status and `last_indexed_block`:
  ```json
  {"status": "healthy", "last_indexed_block": 42}
  ```
  Integrated with Docker Compose container healthcheck probes (`wget -q --spider http://127.0.0.1:8080/health`).
- **`/metrics`**: Exposes Prometheus metrics in standard exposition format (0.0.4):
  - `events_processed_total`: Cumulative counter of decoded and published chain events.
  - `reorgs_handled_total`: Cumulative counter of detected and resolved reorganizations.
  - `retractions_emitted_total`: Cumulative counter of emitted retraction messages.
  - `last_indexed_block`: Gauge of the latest canonical block processed.
- **Distributed Tracing**: Spans carry root `trace_id` headers across NATS messages, linking the Indexer, Risk Engine, and Liquidator end-to-end.

---

## 8. Verification Matrix & Test Coverage

| Test File | Target Requirement | Scope Verified | Status |
|---|---|---|---|
| `rs01_integration.rs` | RS-01 | Live Anvil block hint ingestion, contract deployment, `alloy sol!` event decoding for Deposit, Borrow, PriceUpdated. | PASS |
| `rs02_reorg_chaos.rs` | RS-02 | Anvil `evm_snapshot` / `evm_revert` 2-block chain reorganization, ancestor walk-back, and reverse-order Retract publication. | PASS |
| `rs03_crash_recovery.rs` | RS-03 | Kill-and-restart continuity across Anvil mining, restart from `cursor + 1` without gaps, and crash safety with idempotent deduplication. | PASS |
| `rs04_idempotency_telemetry.rs` | RS-04 | Deterministic `(block_hash, log_index)` uniqueness, JSON schema round-trips, NATS header injection (`Nats-Msg-Id`, `trace_id`), Prometheus text format, and HTTP `/health` server. | PASS |
