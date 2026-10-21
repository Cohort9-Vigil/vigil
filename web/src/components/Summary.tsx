// The four cards at the top: net worth, debt, the health factor bar, and borrow power used.
import { dollars, fmt } from "../lib/format.ts";
import { borrowCapacity, limitUsedPercent, type Position } from "../lib/position.ts";
import type { AccountView } from "../lib/types.ts";

export function Summary({
  account,
  position,
  debtDecimals,
}: {
  account: AccountView | null;
  position: Position | null;
  debtDecimals: number;
}) {
  if (!account || !position) {
    return (
      <section className="summary">
        <div className="stat">
          <div className="label">
            Net Worth <span className="info" title="Total collateral minus debt value">i</span>
          </div>
          <div className="value">$0.00</div>
          <div className="sub">Connect wallet to view position</div>
        </div>
        <div className="stat">
          <div className="label">
            Debt Balance <span className="info" title="Current synthetic debt in vUSD">i</span>
          </div>
          <div className="value">0.00 vUSD</div>
          <div className="sub">No active borrow position</div>
        </div>
        <div className="stat hf-card">
          <div className="label">
            Health Factor <span className="info" title="Ratio of collateral value to debt. Greater than 1.00 is safe.">i</span>
          </div>
          <div className="value" style={{ color: "var(--ok)" }}>
            ∞ (Safe)
          </div>
          <div className="hf-bar" aria-hidden="true">
            <div className="hf-fill" style={{ width: "100%", background: "var(--ok)" }} />
            <div className="hf-line" title="1.00 liquidation line" />
          </div>
          <div className="hf-scale">
            <span>0</span>
            <span>1.00 liquidation</span>
            <span>2+</span>
          </div>
          <div className="sub">Liquidation begins below 1.00</div>
        </div>
        <div className="stat">
          <div className="label">
            Borrow Power Used <span className="info" title="Share of your borrowing power in use. 100% is maximum capacity.">i</span>
          </div>
          <div className="value">0.0%</div>
          <div className="meter">
            <div style={{ width: "0%", background: "var(--primary)" }} />
          </div>
          <div className="meter-row">
            <span>Borrow limit left</span>
            <span>0.00 vUSD</span>
          </div>
        </div>
      </section>
    );
  }

  const netWorth = position.collateralTotal - position.debtValue;
  const used = limitUsedPercent(position);
  const capacity = borrowCapacity(position);

  // The gateway reads the health factor from the contract, so it is the official number.
  let hfText = "No debt";
  let hfNumber = Infinity;
  let colour = "var(--ok)";
  if (position.debt > 0n) {
    if (account.healthFactor === null) {
      hfText = "Calculating";
      colour = "var(--warn)";
    } else {
      hfNumber = Number(account.healthFactor) / 1e18;
      hfText = hfNumber.toFixed(2);
      colour = hfNumber >= 1.25 ? "var(--ok)" : hfNumber >= 1.0 ? "var(--warn)" : "var(--bad)";
    }
  }
  // Bar runs from 0 to 2. Infinity (no debt) fills it.
  const dash = hfNumber === Infinity ? 100 : Math.min(hfNumber / 2, 1) * 100;

  return (
    <section className="summary">
      <div className="stat">
        <div className="label">
          Net Worth <span className="info" title="Collateral value minus debt, in USD">i</span>
        </div>
        <div className="value">{dollars(netWorth)}</div>
        <div className="sub">
          Collateral {dollars(position.collateralTotal)} − debt {dollars(position.debtValue)}
        </div>
      </div>
      <div className="stat">
        <div className="label">
          Debt Balance <span className="info" title="vUSD owed, including accrued interest">i</span>
        </div>
        <div className="value">{fmt(position.debt, debtDecimals)} vUSD</div>
        <div className="sub">
          {Number(account.auctionStart) > 0 ? `Auction initiated at block ${account.auctionStart}` : "Interest accrues every block"}
        </div>
      </div>
      <div className="stat hf-card">
        <div className="label">
          Health Factor <span className="info" title="Above 1.00 is safe. At or below 1.00 the position can be liquidated.">i</span>
        </div>
        <div className="value" style={{ color: colour }}>
          {hfText}
        </div>
        <div className="hf-bar" aria-hidden="true">
          <div className="hf-fill" style={{ width: `${dash}%`, background: colour }} />
          <div className="hf-line" title="1.00 liquidation line" />
        </div>
        <div className="hf-scale">
          <span>0</span>
          <span>1.00 liquidation</span>
          <span>2+</span>
        </div>
        <div className="sub">
          {hfNumber >= 1 || hfNumber === Infinity
            ? "Position safe. Liquidation triggered below 1.00"
            : "⚠️ Critical: Position subject to liquidation"}
        </div>
      </div>
      <div className="stat">
        <div className="label">
          Borrow Power Used <span className="info" title="Share of your borrowing capacity currently used.">i</span>
        </div>
        <div className="value">{used.toFixed(1)}%</div>
        <div className="meter">
          <div
            style={{
              width: `${Math.min(used, 100)}%`,
              background: used < 70 ? "var(--primary)" : used < 85 ? "var(--warn)" : "var(--bad)",
            }}
          />
        </div>
        <div className="meter-row">
          <span>Borrow limit left</span>
          <span>{fmt(capacity, debtDecimals)} vUSD</span>
        </div>
      </div>
    </section>
  );
}
