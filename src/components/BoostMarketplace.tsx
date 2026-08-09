import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Panel } from "../ds";
import type { BoostNft } from "../boosts";
import { boostExpired, boostLabel } from "../boosts";

const formatLamportsToSol = (lamports: string): string => {
  const value = Number(lamports);
  return Number.isFinite(value) ? (value / 1_000_000_000).toFixed(4) : "0.0000";
};

const errorText = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); } catch { return "Unknown error"; }
};

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
      const mint = await invoke<string>("claim_demo_boost");
      setMessage(`DEMO BOOST MINTED — ${mint.slice(0, 8)}…${mint.slice(-6)}.`);
      // The transaction is confirmed before the command returns. Keep a short
      // retry for devnet RPC propagation, but never invent an asset locally.
      refresh();
      window.dispatchEvent(new Event("boost-inventory-updated"));
      window.setTimeout(refresh, 1200);
    } catch (error) {
      const detail = errorText(error);
      setMessage(`DEMO FAUCET FAILED — ${detail.slice(0, 180)}`);
    } finally {
      setBusy(null);
    }
  };

  const refresh = () => {
    invoke<BoostNft[]>("get_boost_nfts")
      .then(setItems)
      .catch(() => setItems([]));
  };

  useEffect(() => {
    refresh();
    const timer = window.setInterval(refresh, 5_000);
    return () => window.clearInterval(timer);
  }, []);

  const useBoost = async (mint: string) => {
    setBusy(mint);
    setMessage(null);
    try {
      await invoke("use_boost_nft", { mint });
      setMessage("BOOST USED. SPL NFT BURN CONFIRMED.");
      setItems((current) => (current ?? []).filter((item) => item.mint !== mint));
      window.dispatchEvent(new Event("boost-inventory-updated"));
      window.setTimeout(refresh, 1200);
    } catch (error) {
      const detail = errorText(error);
      setMessage(`BOOST BURN FAILED — ${detail.slice(0, 180)}`);
    } finally {
      setBusy(null);
    }
  };

  const listBoost = async (mint: string) => {
    setBusy(mint);
    setMessage(null);
    try {
      await invoke("list_boost_nft", { mint, priceLamports: "10000000" });
      const id = await invoke<string>("broadcast_intent", {
        draft: {
          action: "SELL",
          asset: "BOOST NFT",
          condition: { kind: "any", price: null },
          amount: "1",
          mode: "SHARK",
          privacy: "MEDIUM",
        },
      });
      setMessage(`NFT LISTED + MESH INTENT BROADCAST — ${id}.`);
      refresh();
    } catch (error) {
      setMessage(`LISTING FAILED — ${errorText(error).slice(0, 180)}`);
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
    } catch (error) {
      setMessage(`PURCHASE FAILED — ${errorText(error).slice(0, 180)}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <Panel label="SPL BOOST NFT / MESH MARKET">
      <div style={{ display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "var(--space-5) var(--space-6)", color: "var(--text-muted)", fontSize: "var(--text-sm)" }}>
          Only boost items are tradeable. Using one burns the SPL NFT; inventory syncs automatically every 5 seconds.
        </div>
        <div style={{ padding: "0 var(--space-6) var(--space-5)", display: "flex", gap: "var(--space-3)" }}>
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
              {item.owned && !item.listed && !expired && <Button tone="primary" size="sm" className="cm-touch" disabled={busy === item.mint} onClick={() => useBoost(item.mint)}>BURN</Button>}
              {item.owned && !item.listed && !expired && <Button tone="secondary" size="sm" className="cm-touch" disabled={busy === item.mint} onClick={() => listBoost(item.mint)}>SELL VIA MESH</Button>}
              {item.listed && <span style={{ fontSize: "var(--text-2xs)", color: "var(--text-muted)" }}>LISTED</span>}
              {!item.owned && item.listed && <Button tone="primary" size="sm" className="cm-touch" disabled={busy === item.mint} onClick={() => buyBoost(item)}>BUY {item.priceLamports ? `${formatLamportsToSol(item.priceLamports)} SOL` : "BOOST"}</Button>}
            </div>
          );
        })}
        {message && <div role="status" style={{ padding: "var(--space-4) var(--space-6)", color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}>{message}</div>}
      </div>
    </Panel>
  );
}
