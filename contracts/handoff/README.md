# Vigil contracts: frontend handoff

ABIs in `abi/` match the contracts deployed on Sepolia (branch `feat/contracts-core`). Regenerate them whenever a contract's public functions, events or errors change:

```
forge build
forge inspect VigilPool abi --json > handoff/abi/VigilPool.json
```

## Network and addresses

Deployed and verified on **Ethereum Sepolia**, chain id **11155111**. Browse any address at `https://sepolia.etherscan.io/address/<address>`. Use your own RPC URL, the team's RPC key is private. The full list is in `../deployments/11155111.json`.

What the frontend needs:

| What | Address | Use |
|---|---|---|
| **Pool (proxy)** | `0xecF5622D592293c960bFec2A948dF9e8B1323358` | every user action and read, with `VigilPool.json` |
| **Liquidation settlement** | `0x488eB39f40D0E808dC0e5533697A18b3c0f3787B` | auctions and liquidations, with `LiquidationSettlement.json` |
| **Oracle** | `0xAc134af348f29cdb174701E766C3959eA68dd649` | read `state()` and `getPrice()`, with `OracleAdapter.json` |
| vUSD (stablecoin, 6 decimals) | `0xA0C8B0582B47D2b561Fca63d5458D0E12a0F4d7f` | supply, borrow, repay |
| vFEE (collateral, 18 decimals, 2% fee on transfer) | `0x91A9CA0Ad8f2306764ED2E04C9CD45287691e5DF` | collateral |
| vSIX (collateral, 6 decimals) | `0xfA1077D9f3A0D098b2827cAa6D4f2D01B31545E3` | collateral |
| vEIGHT (collateral, 8 decimals) | `0x2E301949D27B925FF5d166c3c6573bd6F28ab2ff` | collateral |

Always call the **proxy**, never the implementation. Not needed by the UI: the price feeds, the AMMs, the pool implementation and the deployer account.

Prices when deployed: vUSD $1, vFEE $100, vSIX $50, vEIGHT $20,000.

## Getting test tokens

The tokens are mocks and anyone can mint them. Call `mint(address to, uint256 amount)` on a token to give yourself some, in that token's own decimals (for example `1000e6` for 1,000 vUSD). `IERC20.json` does not include `mint`, add it to the token ABI. Gas needs Sepolia ETH from a faucet.

## Things to know

- **The oracle can freeze on its own.** The mock price feeds go stale after 24 hours without an update, and the oracle then reports Frozen (only repay and add collateral work). The team re-posts prices with `script/refresh-feeds.sh`. If the UI shows Frozen, that is probably the reason.
- **Prices are set by hand.** Anyone with the owner key can change a mock feed price, so a price drop is a demo action, not a market event.
- **The pool starts with 10 seeded borrowers** and 5,000,000 vUSD of liquidity, so there is something to look at on day one.
- **An auction lasts about 10 minutes.** The bonus grows over 50 blocks and Sepolia blocks take 12 seconds.
- Chain is Sepolia only. The old local anvil addresses (chain id 31337) in `../deployments/31337.json` do not exist on any real network.
- Nothing has been run against a UI yet, so expect small changes once integration starts.

## Files

| File | Use it at |
|---|---|
| `abi/VigilPool.json` | the pool **proxy** address, never the implementation |
| `abi/LiquidationSettlement.json` | the settlement address |
| `abi/OracleAdapter.json` | the oracle address (read only from the UI) |
| `abi/IERC20.json` | each token, for `approve`, `balanceOf` and `allowance` |

The ABIs include the custom errors, so viem and ethers can decode reverts into names like `HealthFactorTooLow`.

## Units

| Value | Scale |
|---|---|
| Health factor | 1e18, so `1e18` is exactly 1. A position with no debt returns `type(uint256).max` |
| Prices | 1e18 per whole token |
| Interest indexes | 1e27 |
| Bonus and thresholds | basis points, 10,000 = 100% |
| Token amounts | the token's own decimals (6, 8 or 18) |

## Pool: user actions

All of these take effect on the caller's own position.

| Function | Notes |
|---|---|
| `supply(amount)` | lend the stablecoin to earn interest, needs `approve` first |
| `withdrawSupply(amount)` | |
| `depositCollateral(asset, amount)` | needs `approve` first. Also enables the asset for the health factor |
| `withdrawCollateral(asset, amount)` | reverts with `HealthFactorTooLow` if it would leave the position unsafe |
| `setCollateralEnabled(asset, enabled)` | disabling is checked the same way |
| `borrow(amount)` | stablecoin only, so no asset argument |
| `repay(amount)` | needs `approve` first. An amount above the debt is capped to the debt |

When the oracle is Frozen only `repay` and `depositCollateral` work. Everything else reverts with `OracleFrozen`. When the pool is paused, `repay` and `depositCollateral` still work too.

## Pool: reads

| Function | Returns |
|---|---|
| `getHealthFactor(user)` | health factor |
| `getDebt(user)` | debt including interest accrued up to now |
| `getCollateralBalance(user, asset)` | amount of that collateral |
| `getLiquidationData(user, asset)` | `(healthFactor, debt, collateralBalance)` in one call |
| `positionOf(user)` | `(scaledDebt, scaledSupply, bitmap)`. These are scaled, not real amounts |
| `poolState()` | `(borrowIndex, liquidityIndex, totalScaledDebt, totalScaledSupply, reserve, deficit, paused)` |
| `assetConfig(asset)` | `(listed, id, decimals, liquidationThresholdBps)` |
| `getPrice(asset)` | price from the oracle |
| `debtAsset()` | the stablecoin address |
| `hasCollateral(user)` | true if the user holds any collateral |

Things to know about the reads:

- **Supply balance has no getter.** Compute it as `scaledSupply * liquidityIndex / 1e27`, using `positionOf` and `poolState`.
- `getHealthFactor`, `getDebt`, `getLiquidationData` and `getPrice` revert while the pool is in the middle of a call. A normal `eth_call` never hits this.
- `getHealthFactor` and `getPrice` also revert when a price feed is invalid or stale. Show "price unavailable" rather than an error.
- There is no on-chain list of positions or collateral assets. Get the asset list from the deployment file or the `CollateralListed` event. For dashboards that list many positions, use the Risk Engine snapshot API.

## Oracle

- `state()` returns 0 for Normal and 1 for Frozen. Use it to show a banner and disable buttons.
- `getPrice(asset)` reverts with `InvalidPrice(asset)` when that asset's feed is bad.

## Liquidation

1. Anyone calls `startAuction(borrower)` once the health factor is below 1e18.
2. `currentBonusBps(borrower)` gives the live bonus. It grows linearly from 0 to `maxBonusBps()` over `auctionDuration()` blocks, counted from `auctionStartBlock(borrower)`.
3. The liquidator signs an intent. Anyone can then submit it with `executeLiquidation(intent, signature)`. The signer pays the repayment, so the signer must have approved the settlement contract for the stablecoin.
4. At most 50% of the debt can be repaid per liquidation, unless the remainder would be below `dustThreshold()`, or the position is underwater (its collateral is worth less than the debt plus the bonus). An underwater position can be cleared in one go, and whatever debt is left once the collateral is gone is written off against the reserve.

`resetAuction(borrower)` clears an auction after the position is healthy again. `settleBadDebt(borrower)` writes off debt once the collateral is gone.

### Signing the intent (EIP-712)

Domain:

```
name: "VigilLiquidation"
version: "1"
chainId: <chain id>
verifyingContract: <settlement address>
```

Type:

```
LiquidationIntent(
  address borrower,
  address collateralAsset,
  uint256 maxRepay,
  uint256 minCollateralOut,
  uint256 nonce,
  uint256 deadline
)
```

- `deadline` is a unix timestamp in seconds.
- `maxRepay` is in stablecoin units.
- `nonce` can be any number that has not been used by that signer. Check with `nonceBitmap(signer, nonce >> 8)`: if bit `nonce & 255` is set, the nonce is taken.

## Events

Pool: `Supplied`, `SupplyWithdrawn`, `CollateralDeposited`, `CollateralWithdrawn`, `CollateralEnabledChanged`, `Borrowed`, `Repaid`, `Liquidated`, `BadDebtWrittenOff`, `InterestAccrued`, `CollateralListed`, `SettlementSet`, `Paused`, `Unpaused`.

Settlement: `AuctionStarted`, `AuctionReset`, `LiquidationExecuted`, `NonceUsed`, `BadDebtSettled`.

## Errors worth showing to users

| Error | Meaning |
|---|---|
| `OracleFrozen` | prices are not trustworthy, only repay and add collateral work |
| `PoolPaused` | the pool is paused |
| `HealthFactorTooLow` | the action would leave the position unsafe |
| `InsufficientLiquidity` | not enough stablecoin in the pool to borrow or withdraw |
| `InsufficientBalance` | the user does not hold that much |
| `ZeroAmount` | amount was 0 |
| `PositionHealthy` | liquidation attempted on a healthy position |
| `IntentExpired`, `NonceAlreadyUsed`, `InvalidSignature` | a bad liquidation intent |
| `InsufficientCollateralOut` | collateral received would be below `minCollateralOut` |

## Not for the UI

`applyLiquidation`, `writeOffBadDebt`, `listCollateral`, `setSettlement`, `pause`, `unpause`, `setSource` and the upgrade functions are admin or internal.
