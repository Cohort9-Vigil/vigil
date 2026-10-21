// The asset badge with token ticker (vFEE, vSIX, vEIGHT, vUSD) and metadata.
import { tokenInfo } from "../lib/tokens.ts";

export function assetCell(name: string) {
  const info = tokenInfo(name);
  const clean = name.toLowerCase();
  return (
    <div className="asset" title={info.note}>
      <span className={`coin ${clean}`}>{info.ticker}</span>
      <div>
        <div className="name">{info.ticker}</div>
        <div className="ticker">{info.note.split(".")[0]}</div>
      </div>
    </div>
  );
}
