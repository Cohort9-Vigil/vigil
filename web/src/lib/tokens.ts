// Token metadata for Vigil Protocol on Ethereum Sepolia.
// Assets: vFEE (18 decimals, fee-on-transfer), vSIX (6 decimals), vEIGHT (8 decimals), vUSD (6 decimals debt asset).
export interface TokenInfo {
  label: string;
  ticker: string;
  note: string;
  color: string;
}

export const TOKEN_INFO: Record<string, TokenInfo> = {
  vfee: {
    label: "vFEE",
    ticker: "vFEE",
    note: "Collateral token (18 decimals) with 2% fee-on-transfer.",
    color: "#f59e0b",
  },
  fee: {
    label: "vFEE",
    ticker: "vFEE",
    note: "Collateral token (18 decimals) with 2% fee-on-transfer.",
    color: "#f59e0b",
  },
  vsix: {
    label: "vSIX",
    ticker: "vSIX",
    note: "Collateral token (6 decimals).",
    color: "#10b981",
  },
  six: {
    label: "vSIX",
    ticker: "vSIX",
    note: "Collateral token (6 decimals).",
    color: "#10b981",
  },
  veight: {
    label: "vEIGHT",
    ticker: "vEIGHT",
    note: "Collateral token (8 decimals).",
    color: "#3b82f6",
  },
  eight: {
    label: "vEIGHT",
    ticker: "vEIGHT",
    note: "Collateral token (8 decimals).",
    color: "#3b82f6",
  },
  vusd: {
    label: "vUSD",
    ticker: "vUSD",
    note: "Protocol synthetic debt asset (6 decimals).",
    color: "#0284c7",
  },
  usd: {
    label: "vUSD",
    ticker: "vUSD",
    note: "Protocol synthetic debt asset (6 decimals).",
    color: "#0284c7",
  },
};

export function tokenInfo(name: string): TokenInfo {
  const clean = name.toLowerCase();
  const normalized = clean.replace(/^v/, "");
  return (
    TOKEN_INFO[clean] ??
    TOKEN_INFO[normalized] ?? {
      label: name.toUpperCase(),
      ticker: name.toUpperCase(),
      note: "Vigil protocol asset",
      color: "#64748b",
    }
  );
}
