import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Panel } from "../ds";
import type { BoostNft } from "../boosts";
import { boostExpired, boostLabel } from "../boosts";

/**
 * SPL boost inventory and mesh marketplace affordances.
 *
 * The commands are deliberately separate from the frozen ERC-721 surface:
 * `use_boost_nft` must burn the token on-chain, while listing/buying only
 * transports a signed intent over mesh and waits for chain confirmation.
 */
export function BoostMarketplace() {
  const [items, setItems] = useState<BoostNft[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const claimDemo = async () => {
    setBusy("demo");
    setMessage(null);
    try {
      await invoke("claim_demo_boost");
      setMessage("DEMO BOOST CLAIMED. REFRESHING WALLET...");
      refresh();
    } catch {
      setMessage("DEMO CLAIM IS AVAILABLE ONLY FOR THE CONFIGURED DEVNET DEMO WALLET.");
    } finally {
      setBusy(null);
    }
  };

  const refresh = () => {
    invoke<BoostNft[]>("get_boost_nfts")
      .then(setItems)
      .catch(() => setItems([]));
  };

  useEffect(() => refresh(), []);

  const useBoost = async (mint: string) => {
    setBusy(mint);
    setMessage(null);
    try {
      await invoke("use_boost_nft", { mint });
      setMessage("BOOST USED. SPL NFT BURN CONFIRMED.");
      refresh();
    } catch {
      setMessage("BOOST BURN FAILED OR WALLET HAS NO TOKEN ACCOUNT.");
    } finally {
      setBusy(null);
    }
  };

  const listBoost = async (mint: string) => {
    setBusy(mint);
    setMessage(null);
    try {
      await invoke("list_boost_nft", { mint, priceLamports: "10000000" });
      setMessage("LISTING INTENT SENT THROUGH MESH.");
      refresh();
    } catch {
      setMessage("LISTING FAILED — CHECK MINT OWNERSHIP AND SOL BALANCE.");
    } finally {
      setBusy(null);
    }
  };

  const buyBoost = async (item: BoostNft) => {
    if (!item.seller) return;
    setBusy(item.mint);
    setMessage(null);
    try {
      await invoke("buy_boost_nft", { mint: item.mint, seller: item.seller });
      setMessage("BOOST PURCHASE CONFIRMED ON SOLANA.");
      refresh();
    } catch {
      setMessage("PURCHASE FAILED — CHECK SOL BALANCE OR LISTING EXPIRY.");
    } finally {
      setBusy(null);
    }
  };

  return (
    <Panel label="SPL BOOST NFT / MESH MARKET">
      <div style={{ display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "var(--space-5) var(--space-6)", color: "var(--text-muted)", fontSize: "var(--text-sm)" }}>
          Only boost items are tradeable. Using one burns the SPL NFT; expiry removes the effect automatically.
        </div>
        <div style={{ padding: "0 var(--space-6) var(--space-5)" }}>
          <Button tone="secondary" size="sm" className="cm-touch" disabled={busy === "demo"} onClick={claimDemo}>CLAIM DEMO BOOST</Button>
        </div>
        {items === null ? null : items.length === 0 ? (
          <div style={{ padding: "var(--space-8) var(--space-6)", textAlign: "center", color: "var(--text-muted)" }}>
            NO BOOST NFTS DETECTED.
          </div>
        ) : items.map((item) => {
          const expired = boostExpired(item);
          return (
            <div key={item.mint} className="cm-row" style={{ display: "flex", alignItems: "center", gap: "var(--space-4)", padding: "var(--space-5) var(--space-6)", borderTop: "var(--border-hairline-style)" }}>
              <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
                <span style={{ fontFamily: "var(--type-heading-family)", color: "var(--text-primary)" }}>{item.name}</span>
                <span style={{ fontFamily: "var(--type-label-family)", fontSize: "var(--text-2xs)", color: expired ? "var(--text-alert)" : "var(--text-muted)" }}>{boostLabel(item)}</span>
              </div>
              {item.owned && !item.listed && !expired && <Button tone="primary" size="sm" className="cm-touch" disabled={busy === item.mint} onClick={() => useBoost(item.mint)}>USE / BURN</Button>}
              {item.owned && !item.listed && !expired && <Button tone="secondary" size="sm" className="cm-touch" disabled={busy === item.mint} onClick={() => listBoost(item.mint)}>SELL VIA MESH</Button>}
              {item.listed && <span style={{ fontSize: "var(--text-2xs)", color: "var(--text-muted)" }}>LISTED</span>}
              {!item.owned && item.listed && <Button tone="primary" size="sm" className="cm-touch" disabled={busy === item.mint} onClick={() => buyBoost(item)}>BUY {item.priceLamports ? `${item.priceLamports} LAMPORTS` : "BOOST"}</Button>}
            </div>
          );
        })}
        {message && <div role="status" style={{ padding: "var(--space-4) var(--space-6)", color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}>{message}</div>}
      </div>
    </Panel>
  );
}
