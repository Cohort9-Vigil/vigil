// The homepage: protocol overview, live on-chain metrics, system architecture, and markets entry.
import { useEffect, useState } from "react";
import { useProtocolOnce, useConfig } from "../state/hooks.ts";
import { computeMarket } from "../lib/rates.ts";
import { fmt, short } from "../lib/format.ts";
import { useStream } from "../state/stream.tsx";

export function LandingPage() {
  const chain = useProtocolOnce();
  const config = useConfig();
  const stream = useStream();
  const [, setLive] = useState<{ head: number } | null>(null);

  useEffect(() => {
    if (chain) setLive({ head: chain.head });
  }, [chain]);

  const market =
    chain && config
      ? computeMarket({
          debtIndex: chain.debtIndex,
          supplyIndex: chain.supplyIndex,
          totalDebtScaled: chain.totalDebtScaled,
          totalSupplyScaled: chain.totalSupplyScaled,
          rates: config.rates,
          reserveFactor: config.reserveFactor,
        })
      : null;

  const priceOf = (name: string) => {
    const p = chain?.prices.find((x) => x.name.toLowerCase().includes(name.toLowerCase()));
    return p && p.price_wad !== "frozen" ? Number(p.price_wad) / 1e18 : null;
  };

  const btc = priceOf("eight") ?? 20000;
  const eth = priceOf("fee") ?? 100;
  const usdx = priceOf("six") ?? 50;

  const usd = (n: number | null) => (n === null ? "–" : "$" + Number(n).toLocaleString("en-US", { maximumFractionDigits: 2 }));
  const debt = market ? market.totalDebt : null;
  const debtNumber = debt === null || !config ? null : Number(debt) / 10 ** config.debtDecimals;
  const block = chain ? chain.head.toLocaleString("en-US") : "–";

  return (
    <div className="landing-root">
      {/* Navigation Header */}
      <header className="landing-nav">
        <div className="wrap">
          <a className="brand" href="index.html">
            <div className="brand-logo-wrap">
              <svg className="brand-icon" viewBox="0 0 32 32" fill="none" xmlns="http://www.w3.org/2000/svg">
                <path d="M16 3L5 8V16C5 22.8 9.7 28.9 16 30C22.3 28.9 27 22.8 27 16V8L16 3Z" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" fill="rgba(14, 165, 233, 0.12)" />
                <circle cx="16" cy="14" r="3.5" fill="currentColor" />
                <path d="M16 17.5V23" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
              </svg>
            </div>
            <div className="brand-text">
              <span className="wordmark">VIGIL</span>
              <span className="brand-badge">SEPOLIA</span>
            </div>
          </a>

          <nav className="nav-links">
            <a href="#architecture">Architecture</a>
            <a href="#collaterals">Collateral Assets</a>
            <a href="live.html">Live Pipeline</a>
          </nav>

          <div className="spacer" />

          <div className="nav-right">
            <span className="network-tag">
              <span className="pulse-dot" />
              {stream.connected ? `Sepolia #${block}` : "Syncing RPC…"}
            </span>
            <a className="btn-launch" href="markets.html">
              Launch App ↗
            </a>
          </div>
        </div>
      </header>

      <main>
        {/* Hero Section */}
        <section className="landing-hero">
          <div className="wrap">
            <div className="hero-eyebrow">
              <span className="tag-pill">ETHEREUM SEPOLIA PROTOCOL</span>
              <span>Fully Deployed & Verified Contracts</span>
            </div>

            <h1 className="hero-title">
              Autonomous Lending & Liquidation Engine with Real-Time Risk
            </h1>

            <p className="hero-lead">
              Supply isolated collateral, borrow synthetic vUSD with dynamic interest compounding, and protect protocol solvency through Dutch-auction settlements and a Rust event pipeline.
            </p>

            <div className="hero-cta-row">
              <a className="btn-primary-large" href="markets.html">
                ⚡ Open Markets & Borrow
              </a>
              <a className="btn-secondary-large" href="live.html">
                📡 Inspect Live Event Stream
              </a>
            </div>

            {/* Live On-Chain Protocol Metrics Bar */}
            <div className="metrics-grid">
              <div className="metric-box">
                <span className="metric-label">Total Debt Active</span>
                <span className="metric-value font-mono">
                  {debtNumber === null ? "–" : fmt(market!.totalDebt, config!.debtDecimals, 2) + " vUSD"}
                </span>
                <span className="metric-sub">Compounding at 1e27 Ray math</span>
              </div>

              <div className="metric-box">
                <span className="metric-label">Latest Synced Block</span>
                <span className="metric-value font-mono">#{block}</span>
                <span className="metric-sub">Indexed from block #11913176</span>
              </div>

              <div className="metric-box">
                <span className="metric-label">Oracle Status</span>
                <span className="metric-value" style={{ color: chain?.frozen ? "var(--bad)" : "var(--ok)" }}>
                  {chain ? (chain.frozen ? "Frozen (Tripped)" : "Normal (Active)") : "–"}
                </span>
                <span className="metric-sub">Multi-asset Chainlink adapter</span>
              </div>

              <div className="metric-box">
                <span className="metric-label">Borrow APY</span>
                <span className="metric-value font-mono">
                  {market ? market.borrowApy.toFixed(2) + "%" : "–"}
                </span>
                <span className="metric-sub">Utilization: {market ? market.utilization.toFixed(1) + "%" : "–"}</span>
              </div>
            </div>
          </div>
        </section>

        {/* System Architecture Section (Replaces fake mobile phones) */}
        <section className="landing-section" id="architecture">
          <div className="wrap">
            <div className="section-header">
              <div className="section-tag">ENGINEERING ARCHITECTURE</div>
              <h2>How Vigil Works: Solidity Contracts & Rust Pipeline</h2>
              <p>Designed for sub-second risk monitoring and MEV-resistant liquidation auctions.</p>
            </div>

            <div className="arch-grid">
              <div className="arch-card">
                <div className="arch-num">01</div>
                <h3>Core Lending Pool</h3>
                <p>
                  Deployed at <code className="font-mono">0xecF5...3358</code> on Sepolia. Handles multi-collateral deposits, fee-on-transfer tokens, and synthetic vUSD borrowing with real-time interest index updates.
                </p>
                <ul className="arch-list">
                  <li>Isolated collateral vault accounting</li>
                  <li>Kinked utilization interest rate model</li>
                  <li>Reentrancy guard and pause circuit breakers</li>
                </ul>
              </div>

              <div className="arch-card">
                <div className="arch-num">02</div>
                <h3>Rust Event Indexer</h3>
                <p>
                  High-throughput background indexer scanning Sepolia blocks via Alloy. Decodes event logs, computes block state, and pushes to NATS JetStream with parent-hash reorg detection.
                </p>
                <ul className="arch-list">
                  <li>Zero-polling WebSocket & HTTP RPC streaming</li>
                  <li>PostgreSQL cursor persistence</li>
                  <li>Exact block-quote matching</li>
                </ul>
              </div>

              <div className="arch-card">
                <div className="arch-num">03</div>
                <h3>Autonomous Risk Engine</h3>
                <p>
                  Off-chain Rust service tracking loan health factors across all borrowing accounts. Immediately evaluates risk when oracle prices shift or borrow balances compound.
                </p>
                <ul className="arch-list">
                  <li>Sub-second health factor evaluation</li>
                  <li>Exactly-once JetStream message consumption</li>
                  <li>Automatic liquidation signal emission</li>
                </ul>
              </div>

              <div className="arch-card">
                <div className="arch-num">04</div>
                <h3>Dutch Auction Settlement</h3>
                <p>
                  Deployed at <code className="font-mono">0x488e...787B</code>. Settles liquidations through pre-signed EIP-712 borrower intents and linear-decay Dutch auctions to prevent bad debt.
                </p>
                <ul className="arch-list">
                  <li>Gasless EIP-712 borrower intent signatures</li>
                  <li>256-bit nonce bitmap replay prevention</li>
                  <li>Progressive bonus curve incentives</li>
                </ul>
              </div>
            </div>
          </div>
        </section>

        {/* Collateral Markets Section */}
        <section className="landing-section alt-bg" id="collaterals">
          <div className="wrap">
            <div className="section-header">
              <div className="section-tag">SUPPORTED ASSETS</div>
              <h2>Active Sepolia Collateral Markets</h2>
              <p>Real-time oracle valuations feeding the protocol's borrow limits.</p>
            </div>

            <div className="asset-cards-grid">
              <div className="asset-detail-card">
                <div className="asset-card-top">
                  <div className="asset-symbol vfee">vFEE</div>
                  <div>
                    <div className="asset-title">vFEE Collateral</div>
                    <div className="asset-token-name">18 Decimals · 2% Fee-on-Transfer</div>
                  </div>
                </div>
                <div className="asset-card-price font-mono">{usd(eth)}</div>
                <div className="asset-card-specs">
                  <div><span>Liquidation Threshold</span><b>80.0%</b></div>
                  <div><span>Liquidation Bonus</span><b>10.0%</b></div>
                  <div><span>Contract</span><code className="font-mono">0x91A9...e5DF</code></div>
                </div>
              </div>

              <div className="asset-detail-card">
                <div className="asset-card-top">
                  <div className="asset-symbol veight">vEIGHT</div>
                  <div>
                    <div className="asset-title">vEIGHT Collateral</div>
                    <div className="asset-token-name">8 Decimals Token</div>
                  </div>
                </div>
                <div className="asset-card-price font-mono">{usd(btc)}</div>
                <div className="asset-card-specs">
                  <div><span>Liquidation Threshold</span><b>80.0%</b></div>
                  <div><span>Liquidation Bonus</span><b>10.0%</b></div>
                  <div><span>Contract</span><code className="font-mono">0x2E30...b2ff</code></div>
                </div>
              </div>

              <div className="asset-detail-card">
                <div className="asset-card-top">
                  <div className="asset-symbol vsix">vSIX</div>
                  <div>
                    <div className="asset-title">vSIX Collateral</div>
                    <div className="asset-token-name">6 Decimals Token</div>
                  </div>
                </div>
                <div className="asset-card-price font-mono">{usd(usdx)}</div>
                <div className="asset-card-specs">
                  <div><span>Liquidation Threshold</span><b>80.0%</b></div>
                  <div><span>Liquidation Bonus</span><b>10.0%</b></div>
                  <div><span>Contract</span><code className="font-mono">0xfA10...45E3</code></div>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* CTA Banner */}
        <section className="landing-cta-banner">
          <div className="wrap">
            <div className="cta-box">
              <h2>Ready to test the protocol on Sepolia?</h2>
              <p>Connect your wallet, mint testnet collateral in one click with the built-in faucet, and explore loan positions.</p>
              <div className="cta-btns">
                <a className="btn-primary-large" href="markets.html">
                  Launch Markets App ↗
                </a>
              </div>
            </div>
          </div>
        </section>
      </main>

      {/* Footer */}
      <footer className="landing-footer">
        <div className="wrap">
          <div className="footer-content">
            <div className="footer-brand">
              <span className="wordmark">VIGIL</span>
              <p>Autonomous lending and liquidation protocol on Ethereum Sepolia.</p>
            </div>
            <div className="footer-links-col">
              <div className="footer-col-title">Contracts (Sepolia)</div>
              <a href="https://sepolia.etherscan.io/address/0xecF5622D592293c960bFec2A948dF9e8B1323358" target="_blank" rel="noreferrer">
                Pool Proxy: {short("0xecF5622D592293c960bFec2A948dF9e8B1323358")} ↗
              </a>
              <a href="https://sepolia.etherscan.io/address/0x488eB39f40D0E808dC0e5533697A18b3c0f3787B" target="_blank" rel="noreferrer">
                Settlement: {short("0x488eB39f40D0E808dC0e5533697A18b3c0f3787B")} ↗
              </a>
              <a href="https://sepolia.etherscan.io/address/0xAc134af348f29cdb174701E766C3959eA68dd649" target="_blank" rel="noreferrer">
                Oracle: {short("0xAc134af348f29cdb174701E766C3959eA68dd649")} ↗
              </a>
            </div>
            <div className="footer-links-col">
              <div className="footer-col-title">Services</div>
              <a href="live.html">Live Pipeline Monitor</a>
              <a href="markets.html">Lending Markets</a>
              <a href="/api/protocol">Protocol API (JSON)</a>
            </div>
          </div>
        </div>
      </footer>
    </div>
  );
}
