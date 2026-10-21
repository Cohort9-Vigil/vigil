// Clean, authentic Web3 top navigation bar: brand, live status, network indicator, and wallet management.
import { useState, useRef, useEffect } from "react";
import { ThemeButton } from "./ThemeButton.tsx";
import { short } from "../lib/format.ts";
import type { ChainSnapshot } from "../lib/types.ts";
import { useWallet } from "../state/wallet.tsx";

export function Header(props: {
  active: "markets" | "live";
  connected: boolean;
  chain: ChainSnapshot | null;
  accounts?: string[];
  selected?: string;
  onSelect?: (address: string) => void;
  onMintFaucet?: () => void;
}) {
  const { active, connected, chain, accounts, selected, onSelect, onMintFaucet } = props;
  const wallet = useWallet();
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [inspectModalOpen, setInspectModalOpen] = useState(false);
  const [inspectInput, setInspectInput] = useState("");
  const dropdownRef = useRef<HTMLDivElement>(null);

  const activeAddress = wallet.account || selected || "";

  // Close dropdown on outside click
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setDropdownOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  return (
    <header className="topbar">
      <div className="wrap">
        {/* Brand identity */}
        <a className="brand" href="index.html" style={{ textDecoration: "none", color: "inherit" }}>
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

        {/* Primary nav links */}
        <nav className="main-nav">
          <a className={active === "markets" ? "nav-link active" : "nav-link"} href="markets.html">
            Markets
          </a>
          <a className={active === "live" ? "nav-link active" : "nav-link"} href="live.html">
            Live Pipeline
          </a>
        </nav>

        <div className="spacer" />

        {/* Status items */}
        <div className="statusbar">
          <div className={`stat-pill ${connected ? "is-online" : "is-reconnecting"}`} title="Live event stream from indexer gateway">
            <span className="status-dot" />
            <span className="stat-label">{connected ? "Live Stream" : "Reconnecting"}</span>
          </div>

          <BlockCounter head={chain ? chain.head : null} />

          <div className={`stat-pill ${!chain ? "is-reconnecting" : chain.frozen ? "is-danger" : "is-online"}`} title="Oracle Health Status">
            <span className="stat-label">Oracle</span>
            <span className="stat-val">{chain ? (chain.frozen ? "Frozen" : "Normal") : "–"}</span>
          </div>
        </div>

        {/* Network & Wallet Controls */}
        <div className="wallet-zone" ref={dropdownRef}>
          {wallet.account ? (
            <div className="wallet-connected-group">
              {!wallet.isSepolia && (
                <button className="btn-wrong-network" onClick={() => wallet.switchNetwork()}>
                  ⚠️ Switch to Sepolia
                </button>
              )}

              <button
                className="wallet-account-btn"
                onClick={() => setDropdownOpen((prev) => !prev)}
                title={wallet.account}
              >
                <span className="chain-dot-ok" />
                {wallet.ethBalance && <span className="wallet-eth-bal">{wallet.ethBalance} ETH</span>}
                <span className="wallet-addr-text">
                  {wallet.isDemoMode ? "Inspect: " : ""}
                  {short(wallet.account)}
                </span>
                <svg className="chevron-icon" width="12" height="12" viewBox="0 0 12 12" fill="none">
                  <path d="M3 4.5L6 7.5L9 4.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              </button>

              {dropdownOpen && (
                <div className="wallet-dropdown-menu">
                  <div className="dropdown-header">
                    <div className="dropdown-title">Connected Account</div>
                    <div className="dropdown-addr-full">{wallet.account}</div>
                  </div>

                  <div className="dropdown-actions">
                    <button
                      className="dropdown-action-btn"
                      onClick={() => {
                        if (wallet.account) navigator.clipboard.writeText(wallet.account);
                        setDropdownOpen(false);
                      }}
                    >
                      📋 Copy Address
                    </button>

                    <a
                      className="dropdown-action-btn"
                      href={`https://sepolia.etherscan.io/address/${wallet.account}`}
                      target="_blank"
                      rel="noreferrer"
                      onClick={() => setDropdownOpen(false)}
                    >
                      ↗ View on Sepolia Etherscan
                    </a>

                    {onMintFaucet && (
                      <button
                        className="dropdown-action-btn highlight"
                        onClick={() => {
                          setDropdownOpen(false);
                          onMintFaucet();
                        }}
                      >
                        🚰 Mint Test Tokens (Faucet)
                      </button>
                    )}

                    <button
                      className="dropdown-action-btn"
                      onClick={() => {
                        setDropdownOpen(false);
                        setInspectModalOpen(true);
                      }}
                    >
                      🔍 Inspect Another Address
                    </button>

                    <button
                      className="dropdown-action-btn danger"
                      onClick={() => {
                        setDropdownOpen(false);
                        wallet.disconnect();
                      }}
                    >
                      ⏏ Disconnect
                    </button>
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div className="wallet-disconnected-group">
              <button
                className="btn-connect-wallet"
                onClick={() => wallet.connect()}
                disabled={wallet.isConnecting}
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <rect x="2" y="4" width="20" height="16" rx="4" />
                  <circle cx="17" cy="12" r="2" />
                </svg>
                <span>{wallet.isConnecting ? "Connecting…" : "Connect Wallet"}</span>
              </button>

              <button
                className="btn-inspect-quick"
                title="Explore any address in read-only mode"
                onClick={() => setInspectModalOpen(true)}
              >
                Inspect
              </button>
            </div>
          )}

          {/* Fallback Anvil accounts selector if provided */}
          {accounts && accounts.length > 0 && onSelect && !wallet.account && (
            <select
              id="account"
              aria-label="Local Dev Account"
              value={activeAddress}
              onChange={(e) => onSelect(e.target.value)}
              className="dev-account-select"
            >
              <option value="">Choose local test account</option>
              {accounts.map((a, i) => (
                <option key={a} value={a}>
                  {i === 0 ? "Deployer" : `Account ${i}`} · {short(a)}
                </option>
              ))}
            </select>
          )}
        </div>

        <ThemeButton />
      </div>

      {/* Inspect Address Modal */}
      {inspectModalOpen && (
        <div className="modal-backdrop" onClick={() => setInspectModalOpen(false)}>
          <div className="modal-card" onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">Inspect Address (Read-Only)</div>
            <p className="modal-desc">
              View real-time loan positions, collateral deposits, and health factors for any Ethereum address on Sepolia.
            </p>
            <input
              type="text"
              placeholder="0x..."
              className="modal-input"
              value={inspectInput}
              onChange={(e) => setInspectInput(e.target.value)}
              autoFocus
            />
            <div className="modal-actions">
              <button className="btn-modal-cancel" onClick={() => setInspectModalOpen(false)}>
                Cancel
              </button>
              <button
                className="btn-modal-confirm"
                onClick={() => {
                  if (inspectInput.trim()) {
                    wallet.inspectAddress(inspectInput.trim());
                    if (onSelect) onSelect(inspectInput.trim());
                    setInspectModalOpen(false);
                    setInspectInput("");
                  }
                }}
              >
                Inspect Address
              </button>
            </div>
          </div>
        </div>
      )}
    </header>
  );
}

// Shows the latest block number synced from Sepolia
function BlockCounter({ head }: { head: number | null }) {
  return (
    <div className="stat-pill is-online" title="Latest Sepolia block verified by indexer">
      <span className="stat-label">Block</span>
      <span className="stat-val font-mono">{head ? `#${head}` : "–"}</span>
    </div>
  );
}
