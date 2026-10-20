# Vigil contracts: interface for the Indexer, Risk Engine and Liquidator

What the other teams need from the contracts to build their part. Everything here comes from the code in `contracts/src` and was checked against a fresh local deployment. The contracts are the source of truth; if this file and the code ever disagree, the code wins and this file is the bug.

**Start with your team's part.** Each of the next three parts says what that team gets from the contracts, what it has to do, and what is still open. The numbered sections after them are the reference those parts point into.

- [Part A: Indexer team](#part-a-indexer-team)
- [Part B: Risk Engine team](#part-b-risk-engine-team)
- [Part C: Liquidator team](#part-c-liquidator-team)
- [Agreements still needed between teams](#agreements-still-needed-between-teams)

Reference:

1. [Deployment and addresses](#1-deployment-and-addresses) (everyone)
2. [Events](#2-events) (Indexer, Risk Engine)
3. [Rebuilding a position from events](#3-rebuilding-a-position-from-events) (Risk Engine)
4. [Math to mirror bit for bit](#4-math-to-mirror-bit-for-bit) (Risk Engine, Liquidator)
5. [Oracle and the Frozen state](#5-oracle-and-the-frozen-state) (Risk Engine, Indexer)
6. [Liquidation flow and signing](#6-liquidation-flow-and-signing) (Liquidator)
7. [Reading state on chain](#7-reading-state-on-chain) (Risk Engine, Liquidator)
8. [What is blocked when](#8-what-is-blocked-when) (everyone)
9. [Known behaviours to design around](#9-known-behaviours-to-design-around) (everyone)

---

## Part A: Indexer team

You watch the chain and publish what the contracts emit. You do not interpret it: you forward decoded events, with an idempotency key, and you retract them when blocks are orphaned.

**What you get from the contracts team**

| You need | Where it is |
|---|---|
| Where to start scanning | `deployBlock` in `deployments/<chainId>.json` (section 1). It is a lower bound, so start there and do not assume the first event is in that block |
| Which addresses to watch | `pool` (the proxy) and `settlement`, from the same file. Pool events come from the proxy, never the implementation |
| The exact events to decode | The tables in section 2, with field types, which fields are indexed, and topic0 hashes |
| Machine-readable ABIs | `handoff/abi/VigilPool.json` and `handoff/abi/LiquidationSettlement.json` |
| A chain to test against | `script/local.sh` on a fresh anvil (section 1). Deterministic addresses; the chain is at about block 500 once deploy and seed finish |

**What you have to do**

1. **Replace your event definitions.** `services/indexer/src/abi.rs` declares `Deposit`, `Borrow`, `Repay`, `LiquidationCall` and `PriceUpdated`. None of them exist on our contracts, so as written the Indexer decodes nothing. The real events are `Supplied`, `SupplyWithdrawn`, `CollateralDeposited`, `CollateralWithdrawn`, `CollateralEnabledChanged`, `Borrowed`, `Repaid`, `Liquidated`, `BadDebtWrittenOff`, `InterestAccrued`, `CollateralListed` (pool) and `AuctionStarted`, `AuctionReset`, `LiquidationExecuted`, `NonceUsed`, `BadDebtSettled` (settlement). Generate your `sol!` bindings from the ABI files rather than typing them, so they cannot drift.
2. **Add a test against the real contracts.** Your current decode tests build an event and decode it back, which cannot catch this. Deploy with `script/local.sh`, make a real deposit and borrow, and assert that your Indexer publishes `CollateralDeposited` and `Borrowed`.
3. **Publish every pool and settlement event, including `InterestAccrued`.** The Risk Engine needs it to track the borrow index, and it carries no account address, so it is easy to filter out by mistake.
4. **Keep log order within a block.** The Risk Engine's rules depend on `(blockNumber, logIndex)` order (section 3). `InterestAccrued` comes before the event that uses its index.
5. **Fetch logs by block hash**, as the brief requires, and key every message on `(block_hash, log_index)`.
6. **Retractions:** when you retract an orphaned log, send the same key and enough content for the consumer to undo it without a lookup.

**What the contracts do *not* give you**

- **No event when the oracle goes Frozen or Normal.** The state is computed from feed age and the feed/TWAP gap on each read, so it can change with no transaction (section 5). You cannot learn it from logs.
- **No price event from the oracle.** `OracleAdapter` only emits `SourceSet`. The mock feeds emit `AnswerSet(int128 answer, uint32 roundId)` and `UpdatedAtSet(uint96)` from each `<token>Feed` address in the deployment file, if you want to publish price changes. These are mock-only events.
- **No event for an account's interest.** Debt grows through the index, not through per-account logs.

**Reorg testing.** On anvil every transaction is its own block. `evm_snapshot` then `evm_revert` produces a reorg, and mining new blocks afterwards gives the replacement chain. The scenario injects a 2-block reorg at crash block 8. Sepolia cannot do this.

**Open for you and the Risk Engine team:** who watches the oracle state. See the agreements section.

---

## Part B: Risk Engine team

You hold the positions, mirror the protocol math exactly, and decide who is liquidatable. Your results have to match the contracts bit for bit (RS-07), so most of what you need is the precise rules.

**What you get from the contracts team**

| You need | Where it is |
|---|---|
| How events change a position | Section 3, event by event, including the bitmap and index rules |
| The math, step by step with rounding | Section 4: accrual, `compoundedGrowth`, rate model, health factor, auction bonus |
| The deploy constants | Section 1: rate parameters, reserve factor, thresholds, auction length, bonus, dust |
| Oracle rules and state | Section 5, plus `oracle.sources(asset)` to read thresholds from the chain |
| Reference values to test against | Section 7: `getHealthFactor`, `getDebt`, `positionOf`, `poolState`, `getLiquidationData` |
| Test setup | Anvil with `script/local.sh`. Prices, balances and time are yours to drive (below) |

**What you have to do**

1. **Follow the operation order exactly.** Copy the formulas in section 4 rather than simplifying them. The brief warns that interest accrual is where the first parity run diverges.
2. **Track both indexes yourself.** Start at RAY and update from each `InterestAccrued`. If a transaction has no `InterestAccrued`, the index is unchanged (section 3).
3. **Fill the event gaps** listed in section 3: the collateral enable bit is never announced on deposit, full withdrawal or full seizure; `Liquidated` carries no index; `BadDebtWrittenOff` zeroes the debt; `CollateralListed` has no decimals.
4. **Use the rate constants from section 1, or read them from storage.** `reserveFactorBps` and the four rate parameters have no getter and no event. Section 7 lists their storage slots, checked against a live deployment. A parity failure on interest points here first.
5. **Do not emit signals while Frozen.** Read `oracle.state()` with an `eth_call` each block; it cannot be derived from logs. Also treat a failed `getPrice` as "not liquidatable right now".
6. **Stamp each signal with the block it was computed at** (RS-09). Your reference health factor for that block is `getHealthFactor` called at that block.
7. **Snapshot API (RS-10):** the frontend will read it. Keep amounts in raw token units and say which scale each field uses (health factor in WAD, prices in WAD, indexes in RAY).

**How to set up parity tests.** Everything you need is public on the test deployment:

- `MockERC20.mint(to, amount)` is open, so a test account can mint any token.
- `MockChainlinkFeed.setAnswer(int128)` and `setUpdatedAt(uint96)` are open, so you can set any price (8 feed decimals) or make a feed stale.
- Open a position with real calls: `approve`, `depositCollateral`, `borrow`. Advance time with `evm_increaseTime` and `evm_mine`, then compare your Rust numbers to `getDebt` and `getHealthFactor` at that block.
- Keep feeds fresh while time moves (re-post prices, or the oracle freezes and `getHealthFactor` reverts).

**Open for you:** which signal fields the Liquidator needs (see agreements), and who watches the oracle state.

---

## Part C: Liquidator team

You pick the moment to strike, sign the intent, and get the transaction mined exactly once.

**What you get from the contracts team**

| You need | Where it is |
|---|---|
| The sequence of calls | Section 6, steps 1 to 5 |
| The EIP-712 domain, struct, typehash and signature format | Section 6. A working Solidity signer is `signIntent` in `test/Base.t.sol` |
| Nonce rules | Section 6: Permit2-style unordered bitmap, spent only on success |
| The bonus curve and conversion rate | Sections 4 and 6, for the "wait until the bonus beats gas" strategy |
| Error names to branch on | Section 6, decoded from `handoff/abi/LiquidationSettlement.json` |
| A funded signer on local anvil | The Seed script funds and approves the deployer account (anvil's first account, 1,000,000 vUSD) |

**What you have to do**

1. **Fund and approve the right contract.** The signer pays the repayment in vUSD and receives the collateral, whoever submits the transaction. Approve the **settlement** contract, not the pool.
2. **Start the auction.** `startAuction(borrower)` must be called before any liquidation, by anyone. It reverts with `PositionHealthy` if health factor is 1e18 or more, and with `OracleFrozen` when frozen. Nothing else in the system calls it today (see agreements).
3. **Model the bonus in Rust.** `bonusBps = maxBonusBps * (block.number - startBlock) / auctionDuration`, capped, in blocks. With 50 blocks and 1000 bps the bonus rises by 20 bps per block. A liquidation costs about 257,000 gas today.
4. **Dry-run before signing.** There is no on-chain quote function. The exact result comes from simulating `executeLiquidation` with `eth_call` just before you sign. It returns the real outcome or the revert reason, so it doubles as your last-moment check (RS-16).
5. **Two different nonces.** The intent nonce (in the signed struct) is checked by the contract. The transaction nonce belongs to your wallet. They are independent. A reverted execution does not spend the intent nonce, so if your transaction is reorged out you can resubmit the same signed intent while its deadline is valid. The wallet nonce still has to be filled (RS-13 to RS-15).
6. **Decide what each error means:**

   | Error | What to do |
   |---|---|
   | `OracleFrozen` | wait; do not retry until the state is Normal |
   | `PositionHealthy`, `NoActiveAuction` | drop it; someone else got there, or the position recovered |
   | `NonceAlreadyUsed` | pick a new intent nonce |
   | `IntentExpired` | re-sign with a later `deadline` (Unix seconds, not blocks) |
   | `InsufficientCollateralOut` | the price or bonus moved; re-quote |
   | `HealthFactorDecreased`, `NothingToRepay` | re-quote with a different `maxRepay` |

7. **Leave margin for the fee token.** `minCollateralOut` is checked against the amount taken from the position. For vFEE you receive about 2% less after the token's own fee.
8. **Keep the key isolated.** The contracts see only the signature; the Signer trait and keystore handling (RS-12) are yours.

**What is not provided**

- No quote or preview function on chain (the dry run replaces it).
- No automatic reset of an auction when a position recovers. Call `resetAuction(borrower)`, and be aware that a position that recovers and falls again before a reset keeps its old start block and its larger bonus (section 9).

---

## Agreements still needed between teams

These are not decided by the contracts and not in the brief. They need an answer before integration.

1. **Who watches the oracle state?** It cannot come from logs. Recommended: the Risk Engine polls `oracle.state()` each block, since it is the consumer that must stay silent while Frozen. The Indexer then does not publish it.
2. **Who calls `startAuction`?** Recommended: the Liquidator calls it when it receives a signal for a position with no running auction. The Risk Engine can use `AuctionStarted` and `AuctionReset` to know the auction state.
3. **What a signal carries.** The Liquidator needs at least: borrower, the block it was computed at, health factor, debt, and which collateral asset to take. Agree the schema in the shared `common` crate.
4. **Who owns the shared message schemas and the `common` crate.**
5. **How the services learn the addresses.** Recommended: they read `deployments/<chainId>.json` from a shared volume in Compose, with the RPC URL from an environment variable.

The contracts team will tell you before changing any event, public function or the intent format, and will regenerate the ABI files when it does.

Units used throughout: WAD = 1e18, RAY = 1e27, BPS = 10,000 (= 100%). Health factor is a WAD, so `1e18` is the liquidation line. A position with no debt has health factor `type(uint256).max`. Prices are WAD per whole token. Token amounts are in the token's own decimals (6, 8 or 18).

---

## 1. Deployment and addresses

`script/Deploy.s.sol` writes `contracts/deployments/<chainId>.json`. Read addresses from there, never hardcode them.

| Field | Meaning |
|---|---|
| `chainId` | chain the deployment is on |
| `deployBlock` | head of the chain just before the first deployment transaction. **Start scanning logs from here.** It is a lower bound, not the exact block of the first event |
| `pool` | the VigilPool **proxy**. Always talk to this, never the implementation |
| `settlement` | LiquidationSettlement |
| `oracle` | OracleAdapter |
| `stablecoin`, `stablecoinFeed` | debt asset (vUSD, 6 decimals) and its feed |
| `feeToken` / `sixDecimalToken` / `eightDecimalToken` | the three collateral assets (vFEE 18 decimals with a 2% transfer fee, vSIX 6 decimals, vEIGHT 8 decimals) |
| `<token>Feed`, `<token>Amm` | each collateral's mock price feed and mock AMM |

### Local anvil (chain 31337)

Addresses are deterministic: a fresh anvil, the default first account and `script/local.sh` always produce these. If anvil has already run other transactions the nonce shifts and the addresses change, so restart anvil before deploying.

| | Address |
|---|---|
| pool (proxy) | `0x5FC8d32690cc91D4c39d9d3abcBD16989F875707` |
| settlement | `0x59b670e9fA9D0A427751Af201D676719a970857b` |
| oracle | `0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0` |
| vUSD | `0x5FbDB2315678afecb367f032d93F642f64180aa3` |
| vFEE | `0x0165878A594ca255338adfa4d48449f69242Eb8F` |
| vSIX | `0xB7f8BC63BbcaD18155201308C8f3540b07f84F5e` |
| vEIGHT | `0x959922bE3CAee4b8Cd9a407cc3ac1C251C2007B1` |

`script/local.sh` deploys, advances chain time by an hour (the TWAP needs history before the oracle reports Normal) and seeds 100 borrowers with health factors between 1.05 and 1.6. Sepolia (chain 11155111) is deployed and its addresses are in `deployments/11155111.json`; it is for the frontend and demos, not for chaos tests, because it has no `evm_snapshot`, `evm_revert` or paused mining.

### Deploy constants

These are set in `Deploy.s.sol`. Several have no getter on the pool, so take them from here (or read storage, section 7).

| Parameter | Value |
|---|---|
| Interest rate model (annual, RAY) | base `0.02e27`, slope1 `0.04e27`, slope2 `0.75e27`, optimal utilization `0.8e27` |
| Reserve factor | 1000 bps (10% of interest goes to the reserve) |
| Liquidation threshold, all three collateral assets | 8000 bps |
| Auction duration | 50 blocks |
| Max liquidation bonus | 1000 bps |
| Dust threshold | `10e6` (10 vUSD) |
| Close factor | 5000 bps (constant in the contract) |
| Feed max staleness | 1 day |
| TWAP window / max deviation | 30 minutes / 500 bps |
| Prices at deploy | vUSD $1, vFEE $100, vSIX $50, vEIGHT $20,000 |

ABIs are in `handoff/abi/` (`VigilPool.json`, `LiquidationSettlement.json`, `OracleAdapter.json`, `IERC20.json`) and match the current source. Regenerate them if any public function, event or error changes.

---

## 2. Events

Pool events are emitted by the **pool proxy address**, settlement events by the **settlement address**. Order logs by `(blockNumber, logIndex)`; several of the rules in section 3 depend on that order.

> **Check your `sol!` definitions against this table.** The Indexer's `abi.rs` currently declares `Deposit`, `Borrow`, `Repay`, `LiquidationCall` and `PriceUpdated`. None of those exist on these contracts, so nothing would decode. The real events are below.

### Pool

| Event | Fields | topic0 |
|---|---|---|
| `Supplied` | `address indexed account, uint256 amount, uint256 scaledAmount` | `0x5c2c0d2616a06b35bb159b4d7e227972b59bb33f8d5229ca0e5e438259bfd5d3` |
| `SupplyWithdrawn` | `address indexed account, uint256 amount, uint256 scaledAmount` | `0x95d9b0eedff011278be3f9c7b8f6dbf6a8fc050915b2e4f846a4645962479acc` |
| `CollateralDeposited` | `address indexed account, address indexed asset, uint256 amount` | `0xf1c0dd7e9b98bbff859029005ef89b127af049cd18df1a8d79f0b7e019911e56` |
| `CollateralWithdrawn` | `address indexed account, address indexed asset, uint256 amount` | `0x45892a46e6cef329bb642da6d69846d324db43d19008edc141ed82382eda1bee` |
| `CollateralEnabledChanged` | `address indexed account, address indexed asset, bool enabled` | `0x38bcf0a4eb206825868971b17d514e872a391347661ae8a67c6d98595bfd2ecd` |
| `Borrowed` | `address indexed account, uint256 amount, uint256 scaledAmount` | `0xeae9cfbc77fdd40ca899f36b608256063b2bc9d8178b0220f7ad513e178d6730` |
| `Repaid` | `address indexed account, uint256 amount, uint256 scaledAmount` | `0x1b8cd61ed43bec7c6bdad3a18ffee613f99c853d16c50678d248d879e1b43438` |
| `Liquidated` | `address indexed borrower, address indexed liquidator, address indexed collateralAsset, uint256 repayAmount, uint256 collateralAmount` | `0xd1ef587d13fefe63f9bc582e973aad46877ef08cfe790838721bd521c53caf56` |
| `BadDebtWrittenOff` | `address indexed borrower, uint256 coveredByReserve, uint256 deficit` | `0x0b5d4f0ee7d8bb73705f18602a40d65c7f75bdd55299fd895b3f2b3332ae97fa` |
| `InterestAccrued` | `uint256 borrowIndex, uint256 liquidityIndex, uint256 reserveIncrease` | `0xe9d95e658b0c714da9de2e12fbed253a24a632058a074dc7c90b8b6618675c7c` |
| `CollateralListed` | `address indexed asset, uint8 id, uint16 liquidationThresholdBps` | `0x66fc777825a3c626a0e6877229ebad60edd8da2cd297d21b4cddfd6f31694525` |
| `SettlementSet` | `address indexed settlement` | `0x9f266baf9d71b9257e2d4938694b1264393222f7d373cca75af0998bb5fcca99` |
| `Paused` / `Unpaused` | none | `0x9e87fac8…e752` / `0xa45f47fd…6933` |

The proxy also emits `Initialized`, `OwnershipTransferred` and `Upgraded`; ignore them unless you track upgrades.

### Settlement

| Event | Fields | topic0 |
|---|---|---|
| `AuctionStarted` | `address indexed borrower, uint256 startBlock` | `0xc087c4cdb16906b5ff7016a6bf927618f24ff32eb1de5e86eeaeb08b94831a68` |
| `AuctionReset` | `address indexed borrower` | `0x8ee0804d11a6abbf6dc8fb64806817b89d5ddc14765e0f4a4304a11482abf65a` |
| `LiquidationExecuted` | `address indexed borrower, address indexed liquidator, address indexed collateralAsset, uint256 repayAmount, uint256 collateralSeized, uint256 bonusBps` | `0x77dabbf924a6f11f55b1042aa168a422b93d6d1b6db5d812f2c479c16a512da7` |
| `NonceUsed` | `address indexed liquidator, uint256 nonce` | `0x2af71f10069c28afc67c3752e87e0e4616a97948d33c8f404a856c0c334b3e01` |
| `BadDebtSettled` | `address indexed borrower, uint256 coveredByReserve, uint256 deficit` | `0x04b726a3a5fa1cd5cfea26ac16b68ad0db406d6cbba9fbad67d243a026f5aaa8` |

One liquidation emits both the pool's `Liquidated` and the settlement's `LiquidationExecuted`. They describe the same event from two contracts; do not apply both to a position.

### Price and oracle events

- `OracleAdapter` emits only `SourceSet(address indexed asset, address feed, address twapSource)` (`0x209492b9b4394e2332dc4cda15cbd45fd61fcbdac058fb0ebe5807d24f3df88d`).
- The **mock feeds** emit `AnswerSet(int128 answer, uint32 roundId)` and `UpdatedAtSet(uint96 updatedAt)`, so price changes are visible in logs from each `<token>Feed` address. These are mock-only events.
- The mock AMMs emit `Swap`, `ReservesSet` and `ObservationRecorded`.
- **The Frozen/Normal state is never logged.** It is computed on every read from feed age and the feed/TWAP deviation, so it can change with no transaction at all (a feed simply gets older). See section 5.

---

## 3. Rebuilding a position from events

A position has `scaledDebt`, `scaledSupply`, a `collateral[asset]` balance per asset and a `collateralBitmap` (bit `i` set means collateral asset `i` counts towards the health factor; ids are assigned in listing order and come from `CollateralListed.id`).

Keep one running `borrowIndex` and `liquidityIndex`, starting at RAY, updated from each `InterestAccrued` event.

| Event | Effect on the account |
|---|---|
| `Supplied` | `scaledSupply += scaledAmount` |
| `SupplyWithdrawn` | `scaledSupply -= scaledAmount` (already capped to the balance in the event) |
| `CollateralDeposited` | `collateral[asset] += amount`, **set bit `id`**. `amount` is what the pool actually received, so for the fee token it is 2% less than the transfer amount |
| `CollateralWithdrawn` | `collateral[asset] -= amount`; **clear bit `id` if the balance is now zero** |
| `CollateralEnabledChanged` | set or clear bit `id` |
| `Borrowed` | `scaledDebt += scaledAmount` |
| `Repaid` | `scaledDebt -= scaledAmount` (already capped to the debt in the event) |
| `Liquidated` | `scaledDebt -= min(rayDivDown(repayAmount, borrowIndex), scaledDebt)`; `collateral[collateralAsset] -= collateralAmount`; clear the bit if the balance is now zero. Use the borrow index current at that point in the transaction |
| `BadDebtWrittenOff` | `scaledDebt = 0` |

Gaps in the events that the engine has to fill:

- **The enable bit is not announced** when collateral is deposited, fully withdrawn or fully seized. Derive it as above. `CollateralEnabledChanged` only fires for explicit enable/disable.
- **No borrow index on `Liquidated`.** `InterestAccrued` fires once per transaction at the first accrual, before the state changes, and only when the block timestamp moved. If a transaction has no `InterestAccrued`, the index is unchanged from the last one you saw.
- **`BadDebtWrittenOff` does not say how much scaled debt was cleared.** Zero the borrower's debt.
- **`CollateralListed` has no decimals.** Read `assetConfig(asset)` (returns `listed, id, decimals, liquidationThresholdBps`) or the token's `decimals()`.
- **Interest is not per-account.** Nothing is emitted when an account's debt "grows". Balances are scaled; the real debt is `scaledDebt` times the index, computed at read time.

After applying a block, you can confirm any account with `positionOf(account)` (scaled debt, scaled supply, bitmap) and `getCollateralBalance(account, asset)` through `eth_call` at that block hash.

---

## 4. Math to mirror bit for bit

The Rust implementation must follow the same operation order and rounding as `src/libraries/FixedPointMath.sol`. Division truncates; "Up" means `(a*b - 1) / d + 1` for a non-zero product.

```
mulDivDown(a, b, d) = (a * b) / d
mulDivUp(a, b, d)   = a*b == 0 ? 0 : (a*b - 1) / d + 1
wadDivDown(a, b)    = mulDivDown(a, WAD, b)
rayMulDown(a, b)    = mulDivDown(a, b, RAY)
rayMulUp(a, b)      = mulDivUp(a, b, RAY)
rayDivDown(a, b)    = mulDivDown(a, RAY, b)
rayDivUp(a, b)      = mulDivUp(a, RAY, b)
```

### Rounding by operation

| Operation | Rounding |
|---|---|
| Borrow: `scaledDebt += rayDivUp(amount, borrowIndex)` | up (debt) |
| Repay: reduces `scaledDebt` by `rayDivDown(received, borrowIndex)` | down |
| Debt amount: `rayMulUp(scaledDebt, borrowIndex)` | up |
| Supply: `scaledSupply += rayDivDown(received, liquidityIndex)` | down |
| Withdraw supply: burns `rayDivUp(amount, liquidityIndex)` (capped to balance) | up |
| Supply balance: `rayMulDown(scaledSupply, liquidityIndex)` | down |
| Liquidation repay in scaled terms: `rayDivDown(repayAmount, borrowIndex)` | down |

### Interest accrual (`_previewAccrual`)

Runs on every state-changing call and in every view that needs current indexes. `elapsed = block.timestamp - lastAccrual`. If `elapsed == 0` or `totalScaledDebt == 0` nothing changes.

```
totalDebt   = rayMulUp(totalScaledDebt, borrowIndex)
totalSupply = rayMulDown(totalScaledSupply, liquidityIndex)
utilization = totalSupply == 0 ? RAY : min(rayDivDown(totalDebt, totalSupply), RAY)

annual = utilization <= optimal
    ? baseRate + mulDivDown(slope1, utilization, optimal)
    : baseRate + slope1 + mulDivDown(slope2, utilization - optimal, RAY - optimal)
ratePerSecond = annual / 31_536_000          // 365 days, integer division

growth      = compoundedGrowth(ratePerSecond, elapsed)
borrowIndex = rayMulUp(borrowIndex, growth)

interest    = rayMulUp(totalScaledDebt, newBorrowIndex) - totalDebt
reserveCut  = interest * reserveFactorBps / 10_000
if totalSupply == 0:  reserve += interest;  liquidityIndex unchanged
else:                 reserve += reserveCut
                      liquidityIndex += mulDivDown(liquidityIndex, interest - reserveCut, totalSupply)
```

### Compounded growth (3-term Taylor, rounds down)

```
second = elapsed < 2 ? 0 : elapsed * (elapsed - 1) / 2
third  = elapsed < 3 ? 0 : elapsed * (elapsed - 1) * (elapsed - 2) / 6
r2 = rayMulDown(ratePerSecond, ratePerSecond)
r3 = rayMulDown(r2, ratePerSecond)
growth = RAY + ratePerSecond * elapsed + r2 * second + r3 * third
```

The brief warns the first parity run usually diverges here. The order above is the contract's; do not "simplify" it.

### Health factor (`_healthFactor`)

```
if scaledDebt == 0: return u256::MAX
debt      = rayMulUp(scaledDebt, currentBorrowIndex)
debtValue = mulDivUp(debt, price(debtAsset), 10^debtDecimals)

power = 0
for each set bit id in collateralBitmap (lowest first):
    value  = mulDivDown(collateral[id], price(asset), 10^assetDecimals)
    power += value * liquidationThresholdBps / 10_000      // plain integer division
return wadDivDown(power, debtValue)                        // power * 1e18 / debtValue
```

`currentBorrowIndex` is the index as it would be if accrual ran now (what `getHealthFactor` uses), not the stored one. `price()` is the oracle's WAD price (section 5).

### Auction bonus

```
elapsed = block.number - auctionStartBlock
bonusBps = elapsed >= auctionDuration ? maxBonusBps : maxBonusBps * elapsed / auctionDuration
```

It is measured in **blocks**, not seconds, is 0 in the start block and caps at `maxBonusBps`.

---

## 5. Oracle and the Frozen state

For each asset the `OracleAdapter` stores a source, readable with `sources(asset)`: `feed`, `feedDecimals`, `maxStaleness`, `twapWindow`, `maxDeviationBps`, `twapSource`, `minAnswer`, `maxAnswer`. `assets(i)` / `assetCount()` list them. The Risk Engine can read all thresholds from there instead of hardcoding them.

An asset's price is **valid** only if all of these hold:

1. `feed.latestRoundData()` does not revert and `answer > 0`.
2. `minAnswer <= answer <= maxAnswer`.
3. `block.timestamp <= updatedAt + maxStaleness`.
4. If a TWAP source is set: `consult(twapWindow)` does not revert, is non-zero, and `|price - twap| * 10_000 <= price * maxDeviationBps`.

Price is `answer * 1e18 / 10^feedDecimals`. `getPrice(asset)` reverts with `InvalidPrice(asset)` if that asset is invalid. **`state()` is `Frozen` if *any* listed asset is invalid** (including the stablecoin), else `Normal`. So one stale feed freezes the whole protocol.

`MockAMM.consult` also reverts with `InsufficientHistory` when there are not yet `window` seconds of observations, which freezes the oracle: right after deployment the oracle is Frozen until the TWAP window has passed, which is why `local.sh` advances time by an hour.

Consequences for the services:

- There is **no log for a state change.** Staleness is a function of time, so the state can flip from Normal to Frozen with no transaction. Poll `oracle.state()` (an `eth_call`, ideally pinned to the block you are processing) each block.
- Feed price changes are visible through `AnswerSet` on each feed, but the deviation check also depends on the AMM, so do not recompute the state from logs; call `state()`.
- Anvil time only moves when blocks are mined, but `evm_increaseTime` and the one-hour warp in `local.sh` can age feeds. If the oracle reports Frozen unexpectedly, refresh feeds with `script/refresh-feeds.sh`.

---

## 6. Liquidation flow and signing

1. **Start an auction.** Anyone calls `settlement.startAuction(borrower)`. It reverts with `PositionHealthy` if health factor >= 1e18, `OracleFrozen` if the oracle is Frozen, and `AuctionAlreadyActive` if one is running. It records `auctionStartBlock[borrower] = block.number`.
2. **The bonus grows** with blocks (section 4). `currentBonusBps(borrower)` returns it live.
3. **Execute.** Anyone can submit `executeLiquidation(intent, signature)`. The **signer** is the liquidator: it pays the repayment and receives the collateral, whoever submits the transaction.
4. **Finish.** The auction is cleared when the debt is fully repaid or the position is healthy again. If a healthy position's auction is left open, call `resetAuction(borrower)`.
5. **Bad debt.** When collateral runs out, `settleBadDebt(borrower)` (or the end of `executeLiquidation`) writes the rest off against the reserve; any shortfall is added to `deficit`.

### Intent and signature

EIP-712 domain: `name = "VigilLiquidation"`, `version = "1"`, `chainId`, `verifyingContract = settlement`.

```
LiquidationIntent(
  address borrower,
  address collateralAsset,
  uint256 maxRepay,          // debt asset units, upper bound on what you will repay
  uint256 minCollateralOut,  // reverts InsufficientCollateralOut below this
  uint256 nonce,
  uint256 deadline           // unix timestamp in SECONDS, not a block number
)
```

Typehash: `0xffb7ebd3582b01891fa87b9aa6d538746f6cbd1d83c57c79df3e2b532dbad6f1`. The signature is 65 bytes, `r || s || v`. `test/Base.t.sol` has a working `signIntent` as a reference.

### Nonces

Permit2-style, unordered, per signer. Nonce `n` lives in `nonceBitmap(signer, n >> 8)` at bit `n & 0xff`. A nonce is spent only when the transaction succeeds; a reverted execution does not burn it. Any unused number is valid, so concurrent submissions need no ordering.

### Funding

The signer needs the debt asset (vUSD) and must have `approve`d the **settlement** contract (not the pool) for it. The Seed script funds and approves the deployer account for this.

### What a liquidation does

```
debt, available = getLiquidationData(borrower, collateralAsset)   // debt in debt-asset units
debtPrice, collPrice = oracle.getPrice(...)
rate = 10^collDecimals * debtPrice * (10_000 + bonusBps)  /  (collPrice * 10_000 * 10^debtDecimals)
underwater = debt * rate > available                      // mulDiv(debt, num, den) > available

if underwater: repay = min(maxRepay, debt)                // close factor lifted
else:          cap = debt * 5000 / 10_000
               if debt - cap < dustThreshold: cap = debt
               repay = min(maxRepay, cap)
seized = min(mulDiv(repay, num, den), available)           // rounds down
require seized >= minCollateralOut
```

For a non-underwater position the health factor must not fall (`HealthFactorDecreased`). The contract does this inside one `eth_call`-able function, so **the cheapest exact preview is to simulate `executeLiquidation` with `eth_call` just before signing**; it returns the real outcome or the revert reason. That is also what the last-moment freshness check wants.

Revert reasons you will see (decode them from the ABI): `OracleFrozen`, `PositionHealthy`, `NoActiveAuction`, `AuctionAlreadyActive`, `IntentExpired`, `InvalidSignature`, `NonceAlreadyUsed`, `NothingToRepay`, `InsufficientCollateralOut`, `HealthFactorDecreased`, `CollateralRemaining`, `NoDebt`, `ReentrantCall`.

Gas: a single-asset liquidation measures about 257k gas today (the brief's target is 220k, still open on the contracts side). Use ~260k when your strategy compares bonus to gas cost.

---

## 7. Reading state on chain

All are `eth_call`s on the pool proxy unless noted.

| Function | Returns |
|---|---|
| `getHealthFactor(account)` | WAD, using the indexes as of now |
| `getDebt(account)` | debt incl. accrued interest, rounded up |
| `getLiquidationData(account, asset)` | `(healthFactor, debt, collateralBalance)` in one call |
| `getCollateralBalance(account, asset)` | collateral held |
| `positionOf(account)` | `(scaledDebt, scaledSupply, bitmap)` |
| `poolState()` | `(borrowIndex, liquidityIndex, totalScaledDebt, totalScaledSupply, reserve, deficit, paused)`. Stored values as of the last accrual, not previewed |
| `assetConfig(asset)` | `(listed, id, decimals, liquidationThresholdBps)` |
| `getPrice(asset)`, `hasCollateral(account)`, `debtAsset()` | as named |
| settlement: `auctionStartBlock(a)`, `currentBonusBps(a)`, `nonceBitmap(signer, word)`, `auctionDuration()`, `maxBonusBps()`, `dustThreshold()` | |
| oracle: `state()` (0 = Normal, 1 = Frozen), `getPrice(asset)`, `sources(asset)` | |

`getHealthFactor`, `getDebt`, `getLiquidationData` and `getPrice` revert with `ReentrantCall` while the pool is executing (read-only reentrancy guard). A normal `eth_call` never hits it.

### Values with no getter

`reserveFactorBps` and the four rate parameters are not exposed by any function or event. Use the constants in section 1, or read the namespaced storage directly. The base slot is `0x8fd6f20a40f8005f935934dc5768fdf271775914f110500764ba439dfca05a00`; offsets below were checked against a live local deployment with `cast storage`.

| Slot | Contents |
|---|---|
| base + 0 | oracle |
| base + 1 | debtAsset |
| base + 2 | settlement, then `debtDecimals`, `paused`, `collateralCount` packed above it |
| base + 3 | `reserveFactorBps` |
| base + 4 / 5 / 6 / 7 | `baseRate` / `slope1` / `slope2` / `optimalUtilization` |
| base + 8 / 9 | `borrowIndex` / `liquidityIndex` |
| base + 10 | `lastAccrual` |
| base + 11 / 12 | `totalScaledDebt` / `totalScaledSupply` |
| base + 13 / 14 | `reserve` / `deficit` |

A parity test against `getDebt` and `getHealthFactor` will immediately show if a constant is wrong.

---

## 8. What is blocked when

| Action | Normal | Frozen oracle | Paused |
|---|---|---|---|
| `supply`, `withdrawSupply` | yes | no (`OracleFrozen`) | no |
| `borrow`, `withdrawCollateral`, `setCollateralEnabled` | yes | no | no |
| `depositCollateral` | yes | **yes** | **yes** |
| `repay` | yes | **yes** | **yes** |
| `startAuction`, `executeLiquidation` | yes | no (`OracleFrozen`) | yes |

`unpause` reverts with `OracleNotNormal` unless the oracle is Normal. While Frozen, total debt can only grow through interest accrual, and the Risk Engine should emit no liquidation signals.

---

## 9. Known behaviours to design around

- **Auctions do not clear themselves.** If a position recovers, someone must call `resetAuction`. If it recovers and falls again before that, it keeps the old start block and the old, larger bonus. Treat `auctionStartBlock` as authoritative and do not assume a fresh auction.
- **Fee-on-transfer collateral and `minCollateralOut`.** The contract checks `minCollateralOut` against the amount taken from the position. The liquidator receives about 2% less for vFEE because of the token's fee, so leave margin.
- **`Liquidated` and `LiquidationExecuted` both fire** for one liquidation (section 2).
- **Gas is 257k, not 220k.** A known gap against the brief's SC-25.
- **Block-based auctions, timestamp-based deadlines.** Bonus uses `block.number`; `deadline` and feed staleness use `block.timestamp`. On Sepolia a block is ~12 seconds, so the 50-block auction is about 10 minutes. On anvil it is whatever you mine.
- **Underwater positions** can be cleared in one liquidation and leave bad debt; after that the reserve may cover it and `deficit` records any shortfall.
