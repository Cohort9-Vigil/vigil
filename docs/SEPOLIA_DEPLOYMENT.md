# Vigil Contracts: Sepolia Deployment & Frontend Handoff

Deployed and verified on **Ethereum Sepolia**, Chain ID **11155111**.
Etherscan: `https://sepolia.etherscan.io/address/<address>`

## Network & Contract Addresses

| Component | Address | ABI / Notes |
|---|---|---|
| **Pool (proxy)** | `0xecF5622D592293c960bFec2A948dF9e8B1323358` | All user actions & state reads (`VigilPool.json`) |
| **Liquidation Settlement** | `0x488eB39f40D0E808dC0e5533697A18b3c0f3787B` | Auctions, EIP-712 intents & liquidations (`LiquidationSettlement.json`) |
| **Oracle** | `0xAc134af348f29cdb174701E766C3959eA68dd649` | Reads `state()` (0=Normal, 1=Frozen) & `getPrice(asset)` |
| **vUSD (Stablecoin)** | `0xA0C8B0582B47D2b561Fca63d5458D0E12a0F4d7f` | 6 decimals, Debt asset: supply, borrow, repay |
| **vFEE (Collateral)** | `0x91A9CA0Ad8f2306764ED2E04C9CD45287691e5DF` | 18 decimals, 2% fee-on-transfer |
| **vSIX (Collateral)** | `0xfA1077D9f3A0D098b2827cAa6D4f2D01B31545E3` | 6 decimals |
| **vEIGHT (Collateral)** | `0x2E301949D27B925FF5d166c3c6573bd6F28ab2ff` | 8 decimals |

> **IMPORTANT**: Always interact with the **Pool Proxy** (`0xecF5622D592293c960bFec2A948dF9e8B1323358`), never directly with the logic implementation (`0x0A5bbb0781A35112f2eBAd1F2b6FEeC72621C8E9`).

---

## Market Parameters on Sepolia

- **Initial Pool Liquidity**: 5,000,000 vUSD
- **Initial Seeded Borrowers**: 10
- **Collateral Prices**:
  - `vUSD`: \$1.00 ($1 \times 10^{18}$)
  - `vFEE`: \$100.00 ($100 \times 10^{18}$)
  - `vSIX`: \$50.00 ($50 \times 10^{18}$)
  - `vEIGHT`: \$20,000.00 ($20,000 \times 10^{18}$)
- **Liquidation Threshold**: 8000 bps (80%) for all 3 collaterals
- **Auction Duration**: 50 blocks (~10 minutes on Sepolia, ~12s per block)
- **Max Liquidation Bonus**: 1000 bps (10%)
- **Dust Threshold**: 10 vUSD (`10_000_000` units)

---

## Getting Test Tokens
The deployed tokens are mocks with public minting:
- Call `mint(address to, uint256 amount)` on the token contract to mint test tokens directly to your wallet.
- For gas on Sepolia, use any public Sepolia faucet (e.g. Google Cloud Web3 Faucet, Alchemy Faucet, Infura Faucet).

---

## EIP-712 Intent Signing Schema

- **Domain**:
  ```json
  {
    "name": "VigilLiquidation",
    "version": "1",
    "chainId": 11155111,
    "verifyingContract": "0x488eB39f40D0E808dC0e5533697A18b3c0f3787B"
  }
  ```
- **Type**:
  ```solidity
  struct LiquidationIntent {
      address borrower;
      address collateralAsset;
      uint256 maxRepay;
      uint256 minCollateralOut;
      uint256 nonce;
      uint256 deadline;
  }
  ```
- **Nonce Check**:
  Check with `nonceBitmap(signer, nonce >> 8)` on `0x488eB39f40D0E808dC0e5533697A18b3c0f3787B`: if bit `(nonce & 255)` is set, the nonce is spent.
