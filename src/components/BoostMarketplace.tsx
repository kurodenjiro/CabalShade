import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Panel } from "../ds";
import type { BoostNft } from "../boosts";
import { boostExpired, boostLabel } from "../boosts";

const errorText = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); } catch { return "Unknown error"; }
};

const solPrice = (lamports: string | null): string | null => {
  if (!lamports) return null;
  try {
    const raw = BigInt(lamports);
    const whole = raw / 1_000_000_000n;
    const fraction = (raw % 1_000_000_000n).toString().padStart(9, "0").replace(/0+$/, "");
    return `${whole}${fraction ? `.${fraction}` : ""} SOL`;
  } catch {
    return null;
  }
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
      // Demo listing price: 0.0001 SOL, matching the Buy Boost intent's
      // default ceiling and preserving nine-decimal SOL precision.
      await invoke("list_boost_nft", { mint, priceLamports: "100000" });
      const id = await invoke<string>("broadcast_intent", {
        draft: {
          action: "SELL",
          asset: "BOOST NFT",
          condition: { kind: "any", price: null },
          amount: "1",
          mode: "SHARK",
          privacy: "MEDIUM",
          boostMint: mint,
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

  return (
    <Panel label="SPL BOOST NFT / MESH MARKET">
      <div style={{ display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "var(--space-5) var(--space-6)", color: "var(--text-muted)", fontSize: "var(--text-sm)" }}>
          Only boost items are tradeable. Using one burns the SPL NFT. Sellers list here, then buyers create a BUY BOOST NFT intent in New so both agents can match and relay the signed purchase.
        </div>
        <div style={{ padding: "0 var(--space-6) var(--space-5)", display: "flex", gap: "var(--space-3)" }}>
          <Button tone="secondary" size="sm" className="cm-touch" disabled={busy === "demo"} onClick={claimDemo}>CLAIM DEMO BOOST</Button>
          <Button tone="secondary" size="sm" className="cm-touch" disabled={busy !== null} onClick={refresh}>REFRESH MARKET</Button>
        </div>
        {items === null ? null : items.length === 0 ? (
          <div style={{ padding: "var(--space-8) var(--space-6)", textAlign: "center", color: "var(--text-muted)" }}>
            NO BOOST NFTS DETECTED. ASK THE OTHER PEER TO SELL VIA MESH, THEN REFRESH MARKET.
          </div>
        ) : (
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
              gap: "var(--space-4)",
              padding: "var(--space-5) var(--space-6)",
              borderTop: "var(--border-hairline-style)",
            }}
          >
            {items.map((item) => {
              const expired = boostExpired(item);
              const listingPrice = solPrice(item.priceLamports);
              return (
                <div
                  key={item.mint}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "var(--space-3)",
                    padding: "var(--space-4)",
                    border: "var(--border-hairline-style)",
                    minWidth: 0,
                  }}
                >
                  <div
                    style={{
                      display: "grid",
                      placeItems: "center",
                      aspectRatio: "1",
                      background: "var(--surface-page)",
                      border: "var(--border-hairline-style)",
                      opacity: expired ? 0.4 : 1,
                    }}
                  >
                    <img
                      src="/ds-assets/intent/boost-nft.png"
                      alt=""
                      className="cm-pixel"
                      style={{ width: "60%", height: "60%", objectFit: "contain" }}
                    />
                  </div>
                  <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
                    <span
                      style={{
                        fontFamily: "var(--type-heading-family)",
                        color: "var(--text-primary)",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                    >
                      {item.name}
                    </span>
                    <span
                      style={{
                        fontFamily: "var(--type-label-family)",
                        fontSize: "var(--text-2xs)",
                        color: expired ? "var(--text-alert)" : "var(--text-muted)",
                      }}
                    >
                      {boostLabel(item)}
                    </span>
                    {item.listed && listingPrice && (
                      <span style={{ fontFamily: "var(--type-label-family)", fontSize: "var(--text-2xs)", color: "var(--accent-cyan)" }}>
                        LIST PRICE · {listingPrice}
                      </span>
                    )}
                    {item.listed && (
                      <span style={{ fontSize: "var(--text-2xs)", color: item.owned ? "var(--text-muted)" : "var(--accent-cyan)" }}>
                        {item.owned ? "LISTED" : "LISTED · CREATE BUY INTENT"}
                      </span>
                    )}
                  </div>
                  {item.owned && !item.listed && !expired && (
                    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
                      <Button tone="primary" size="sm" block className="cm-touch" disabled={busy === item.mint} onClick={() => useBoost(item.mint)}>
                        BURN
                      </Button>
                      <Button tone="secondary" size="sm" block className="cm-touch" disabled={busy === item.mint} onClick={() => listBoost(item.mint)}>
                        SELL VIA MESH
                      </Button>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
        {message && <div role="status" style={{ padding: "var(--space-4) var(--space-6)", color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}>{message}</div>}
      </div>
    </Panel>
  );
}
