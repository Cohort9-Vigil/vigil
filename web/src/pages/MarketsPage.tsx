// The markets app: summary, markets, your supplies and borrows, intents, and activity.
import { useEffect, useMemo, useState } from "react";
import { provider } from "../lib/chain.ts";
import { calculatePosition, ONE, type Position } from "../lib/position.ts";
import { marketFrom } from "../lib/market.ts";
import { fmt, short } from "../lib/format.ts";
import * as actions from "../lib/actions.ts";
import type { ActionContext } from "../lib/actions.ts";
import { useConfig, useContracts, useAccount, useIntents } from "../state/hooks.ts";
import { useStream } from "../state/stream.tsx";
import { useToast } from "../state/toasts.tsx";
import { useWallet } from "../state/wallet.tsx";
import { Header } from "../components/Header.tsx";
import { Summary } from "../components/Summary.tsx";
import { SupplyMarkets, BorrowMarkets, type Tab } from "../components/Markets.tsx";
import { Supplies, Borrows, AccountDetails } from "../components/AccountTables.tsx";
import { IntentsCard } from "../components/IntentsCard.tsx";
import { Activity } from "../components/Activity.tsx";
import { ActionDialog, type DialogStep } from "../components/ActionDialog.tsx";
import { reasonOf } from "../lib/chain.ts";
import type { Collateral } from "../lib/types.ts";
import type { ActionMessage } from "./messages.ts";

export function MarketsPage() {
  const config = useConfig();
  const contracts = useContracts(config);
  const stream = useStream();
  const toast = useToast();
  const wallet = useWallet();

  const [accounts, setAccounts] = useState<string[]>([]);
  const [selected, setSelected] = useState("");
  const [onlyMe, setOnlyMe] = useState(false);
  const [dialog, setDialog] = useState<{ open: boolean; tab: Tab; assetIndex: number }>({
    open: false,
    tab: "deposit",
    assetIndex: 0,
  });
  const [messages, setMessages] = useState<Record<string, ActionMessage>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [steps, setSteps] = useState<DialogStep[]>([]);
  const [now, setNow] = useState(Date.now());
  const [tab, setTab] = useState<"markets" | "positions" | "intents" | "activity">("markets");
  const [query, setQuery] = useState("");

  // Sync selected address with wallet state
  useEffect(() => {
    if (wallet.account) {
      setSelected(wallet.account);
    }
  }, [wallet.account]);

  // Read accounts if local Anvil node is active
  useEffect(() => {
    provider
      .send("eth_accounts", [])
      .then((list: string[]) => {
        if (list && list.length > 0) {
          setAccounts(list);
          if (!wallet.account) setSelected((s) => s || list[0] || "");
        }
      })
      .catch(() => {});
  }, [wallet.account]);

  // Keep the "updated N seconds ago" ticker moving
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);

  const activeAddress = wallet.account || selected || "";
  const { account, walletVusd, walletCollateral, reload } = useAccount(contracts, activeAddress);
  const intents = useIntents(contracts, activeAddress);
  const market = marketFrom(stream.chain, config);

  // Position calculation following gateway data
  const position: Position | null = useMemo(() => {
    if (!account) return null;
    const fallback = stream.chain && stream.chain.debtPriceWad !== "frozen" ? BigInt(stream.chain.debtPriceWad) : ONE;
    return calculatePosition(account, fallback);
  }, [account, stream.chain]);

  // Executes an on-chain transaction action
  async function run(id: string, label: string, fn: (ctx: ActionContext) => Promise<string>) {
    if (!activeAddress) {
      toast.push("Wallet Required", "Please connect your Web3 wallet first.", "error");
      wallet.connect();
      return;
    }
    if (busy || !contracts) return;
    setBusy(id);
    setMessages((m) => ({ ...m, [id]: { id, text: label + "…", kind: "busy" } }));
    toast.push(label, "Confirm transaction in your wallet…", "busy");
    try {
      const ctx: ActionContext = {
        who: activeAddress,
        contracts,
        account: account ?? {
          borrower: activeAddress,
          debt: "0",
          supplied: "0",
          healthFactor: null,
          auctionStart: "0",
          debtPriceWad: null,
          debtDecimals: config?.debtDecimals ?? 6,
          collaterals: [],
          riskEngineTracked: false,
        },
        walletVusd,
        walletCollateral,
        position: position ?? {
          debt: 0n,
          debtDecimals: config?.debtDecimals ?? 6,
          debtPrice: ONE,
          debtValue: 0n,
          weighted: 0n,
          complete: true,
          assets: [],
          collateralTotal: 0n,
        },
        fallbackDebtPrice: stream.chain && stream.chain.debtPriceWad !== "frozen" ? BigInt(stream.chain.debtPriceWad) : ONE,
        reload,
      };
      const extra = await fn(ctx);
      setMessages((m) => ({ ...m, [id]: { id, text: label + " done." + extra, kind: "good" } }));
      toast.push(label + " complete", extra.trim() || "Confirmed on Sepolia.", "good");
    } catch (err) {
      const text = reasonOf(err);
      setMessages((m) => ({ ...m, [id]: { id, text: label + " failed: " + text, kind: "error" } }));
      toast.push(label + " failed", text, "error");
    } finally {
      setBusy(null);
      await reload();
    }
  }

  function openDialog(assetIndex: number, t: Tab) {
    if (!activeAddress) {
      toast.push("Connect Wallet", "Connect your wallet on Sepolia to supply or borrow.", "busy");
      wallet.connect();
      return;
    }
    setDialog({ open: true, tab: t, assetIndex });
  }

  let stepId = 0;
  const addStep = (text: string) => {
    const id = ++stepId;
    setSteps((s) => [...s, { id, text, done: false }]);
    return () => setSteps((s) => s.map((x) => (x.id === id ? { ...x, done: true } : x)));
  };

  const supplies: Collateral[] = config?.collaterals ?? [];
  const updated = stream.lastBlockAt ? Math.round((now - stream.lastBlockAt) / 1000) : null;
  const chain = stream.chain;

  return (
    <>
      <Header
        active="markets"
        connected={stream.connected}
        chain={chain}
        accounts={accounts}
        selected={activeAddress}
        onSelect={(a) => {
          setSelected(a);
          wallet.inspectAddress(a);
        }}
        onMintFaucet={() => run("mint", "Mint Test Tokens", (ctx) => actions.mintTestTokens(ctx))}
      />

      <main className="wrap">
        {/* Wallet Connection Onboarding Banner (if disconnected) */}
        {!activeAddress && (
          <div className="onboarding-card">
            <div className="onboarding-content">
              <div className="onboarding-badge">Ethereum Sepolia Testnet</div>
              <h2>Connect your Web3 Wallet to interact with Vigil</h2>
              <p>
                Deposit test collateral (vFEE, vSIX, vEIGHT), borrow synthetic vUSD, and sign automated liquidation intents. You can also explore any Sepolia address in read-only mode.
              </p>
              <div className="onboarding-buttons">
                <button className="btn-primary" onClick={() => wallet.connect()} disabled={wallet.isConnecting}>
                  ⚡ Connect Web3 Wallet
                </button>
                <button
                  className="btn-secondary"
                  onClick={() => {
                    const deployer = config?.collaterals[0]?.address ? "0x91A9CA0Ad8f2306764ED2E04C9CD45287691e5DF" : "0xecF5622D592293c960bFec2A948dF9e8B1323358";
                    wallet.inspectAddress(deployer);
                  }}
                >
                  🔍 Inspect Deployer Address
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Connected Wallet Quick-Action Banner */}
        {activeAddress && (
          <div className="account-banner">
            <div className="account-info">
              <span className="account-badge">
                {wallet.isDemoMode ? "👀 Read-Only Inspection" : "🟢 Connected"}
              </span>
              <span className="account-address font-mono">{activeAddress}</span>
              {wallet.ethBalance && <span className="account-balance">({wallet.ethBalance} Sepolia ETH)</span>}
            </div>

            <div className="account-actions">
              <button
                className="btn-faucet"
                disabled={busy !== null}
                onClick={() => run("mint", "Mint Test Tokens", (ctx) => actions.mintTestTokens(ctx))}
                title="Mint 1000 vFEE, 1000 vSIX, 1000 vEIGHT and 100 vUSD directly to your wallet"
              >
                🚰 Faucet: Mint 1,000 Test Tokens
              </button>
              <a
                className="btn-etherscan"
                href={`https://sepolia.etherscan.io/address/${activeAddress}`}
                target="_blank"
                rel="noreferrer"
              >
                ↗ Sepolia Etherscan
              </a>
            </div>
          </div>
        )}

        <div className="page-head">
          <div>
            <h1>Lending Markets</h1>
            <p>Collateralized borrowing with autonomous liquidation protection on Ethereum Sepolia.</p>
          </div>
        </div>

        <Summary account={account} position={position} debtDecimals={config?.debtDecimals ?? 6} />

        {/* Live Protocol Metric Bar */}
        <section className="totals">
          <span>
            Total Borrowed:{" "}
            <b>
              {chain && config
                ? fmt((BigInt(chain.totalDebtScaled) * BigInt(chain.debtIndex)) / 10n ** 27n, config.debtDecimals) + " vUSD"
                : "–"}
            </b>
          </span>
          <span>
            Protocol Reserves: <b>{chain && config ? fmt(BigInt(chain.reserves), config.debtDecimals) + " vUSD" : "–"}</b>
          </span>
          <span>
            Bad Debt: <b>{chain && config ? fmt(BigInt(chain.deficit), config.debtDecimals) + " vUSD" : "–"}</b>
          </span>
          <span>
            Debt Index: <b>{chain ? (Number(chain.debtIndex) / 1e27).toFixed(6) : "–"}</b>
          </span>
          <span className="spacer" />
          <span className="updated">
            Synced {updated === null ? "–" : updated < 2 ? "just now" : `${updated}s ago`}
          </span>
        </section>

        {/* Navigation Tabs */}
        <div className="tabbar" role="tablist">
          <button role="tab" aria-selected={tab === "markets"} onClick={() => setTab("markets")}>
            Markets Overview
          </button>
          <button role="tab" aria-selected={tab === "positions"} onClick={() => setTab("positions")}>
            Your Positions
          </button>
          <button role="tab" aria-selected={tab === "intents"} onClick={() => setTab("intents")}>
            Liquidation Intents <span className="count">{intents.rows.length}</span>
          </button>
          <button role="tab" aria-selected={tab === "activity"} onClick={() => setTab("activity")}>
            Live Activity
          </button>
        </div>

        {tab === "markets" && (
          <>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12, marginBottom: 12, flexWrap: "wrap" }}>
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search collateral assets (vFEE, vSIX, vEIGHT)..."
                aria-label="Search assets"
                style={{ maxWidth: 340 }}
              />
              <span className="muted" style={{ fontSize: 13 }}>
                Asset prices grounded in deployed Chainlink/Sepolia Oracle Adapter
              </span>
            </div>

            <SupplyMarkets
              config={config}
              chain={chain}
              history={stream.priceHistory}
              query={query}
              onOpen={(asset, t) => openDialog(config?.collaterals.indexOf(asset) ?? 0, t)}
            />

            <BorrowMarkets
              config={config}
              chain={chain}
              market={market}
              onBorrow={() => openDialog(0, "borrow")}
              onRepay={() => openDialog(0, "repay")}
            />
          </>
        )}

        {tab === "positions" && (
          <>
            <div className="split">
              <Supplies
                position={position}
                walletCollateral={walletCollateral}
                collaterals={supplies}
                onOpen={(asset, t) => openDialog(supplies.indexOf(asset), t)}
              />
              <Borrows
                account={account}
                position={position}
                market={market}
                debtDecimals={config?.debtDecimals ?? 6}
                onRepay={() => openDialog(0, "repay")}
              />
            </div>
            <AccountDetails
              account={account}
              position={position}
              walletVusd={walletVusd}
              debtDecimals={config?.debtDecimals ?? 6}
              riskTracked={account?.riskEngineTracked ?? false}
            />
          </>
        )}

        {tab === "intents" && (
          <IntentsCard
            who={activeAddress}
            contracts={contracts}
            config={config}
            rows={intents.rows}
            used={intents.used}
            reload={intents.reload}
            onMessage={(m) => setMessages((x) => ({ ...x, [m.id]: m }))}
            busy={busy !== null}
            setBusy={(b) => setBusy(b ? "intent" : null)}
          />
        )}

        {tab === "activity" && (
          <Activity
            activity={stream.activity}
            config={config}
            onlyMe={onlyMe}
            setOnlyMe={setOnlyMe}
            selected={activeAddress}
          />
        )}

        {/* Professional Web3 Footer */}
        <footer className="footer-bar">
          <div className="footer-left">
            <span>Vigil Protocol · Ethereum Sepolia Testnet</span>
            <span className="footer-sep">•</span>
            <a
              href={`https://sepolia.etherscan.io/address/${config?.pool}`}
              target="_blank"
              rel="noreferrer"
              className="footer-link"
            >
              Pool: {config ? short(config.pool) : "0xecF5...3358"} ↗
            </a>
            <span className="footer-sep">•</span>
            <span>Rust Backend & JetStream Event Pipeline</span>
          </div>
          <div className="footer-right">
            <button
              className="pill-btn highlight"
              disabled={busy !== null || !activeAddress}
              onClick={() => run("mint", "Mint Test Tokens", (ctx) => actions.mintTestTokens(ctx))}
            >
              🚰 Faucet: Get Test Tokens
            </button>
          </div>
        </footer>
      </main>

      <ActionDialog
        open={dialog.open}
        tab={dialog.tab}
        assetIndex={dialog.assetIndex}
        config={config}
        position={position}
        market={market}
        walletVusd={walletVusd}
        walletCollateral={walletCollateral}
        messages={messages}
        steps={steps}
        busy={busy}
        onTab={(t) => setDialog((d) => ({ ...d, tab: t }))}
        onAsset={(assetIndex) => setDialog((d) => ({ ...d, assetIndex }))}
        onClose={() => setDialog((d) => ({ ...d, open: false }))}
        onSupply={(asset, text) => run("deposit", "Supply", (ctx) => actions.supply(ctx, asset, text))}
        onBorrow={(text) => run("borrow", "Borrow", (ctx) => actions.borrow(ctx, text))}
        onRepay={(text) => run("repay", "Repay", (ctx) => actions.repay(ctx, text))}
        onRepayAll={() => {
          setSteps([]);
          run("repayAll", "Repay and return", (ctx) => actions.repayAllAndReturn(ctx, addStep));
        }}
        onWithdraw={(asset, text) => run("withdraw", "Withdraw", (ctx) => actions.withdraw(ctx, asset, text))}
      />
    </>
  );
}
