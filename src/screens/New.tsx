import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Panel } from "../ds";
import type { AssetOption, FormOptions } from "../types/bindings";
import type { IntentId } from "../shell/screen";

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); } catch { return "Unknown error"; }
}

/**
 * The compose screen.
 *
 * Every option comes from `intent_form_options` — Rust is the single source of
 * the modes, assets and conditions, so the form cannot drift
 * from what `broadcast_intent` will accept. Validation happens on submit via
 * `preview_intent`, which validates the draft and returns exactly what the
 * confirm dialog shows.
 */
export function New({ onBroadcast, onOpenMarketplace }: { onBroadcast: (id: IntentId) => void; onOpenMarketplace: () => void }) {
  const [options, setOptions] = useState<FormOptions | null>(null);
  const [action, setAction] = useState("BUY");
  const [buyTarget, setBuyTarget] = useState<"SOL" | "NFT">("SOL");
  const [sellTarget, setSellTarget] = useState<"SOL" | "NFT">("SOL");
  const [asset, setAsset] = useState("SOL");
  const [condition, setCondition] = useState("Price under");
  const [price, setPrice] = useState("");
  const [amount, setAmount] = useState("");
  const [mode, setMode] = useState("SHARK MODE");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<FormOptions>("intent_form_options")
      .then((next) => {
        if (!cancelled) {
          setOptions(next);
          if (next.assets.length > 0) setAsset(next.assets[0].name);
          if (next.modes.length > 0) setMode(next.modes[0].label);
        }
      })
      .catch(() => {
        if (!cancelled) setOptions(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const confirm = async () => {
    if (busy) return;
    if (action === "SELL" && sellTarget === "NFT") {
      onOpenMarketplace();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const id = await invoke<IntentId>("broadcast_intent", {
        draft: {
          action,
          asset,
          condition: condition.startsWith("Any")
            ? { kind: "any" }
            : { kind: condition.includes("above") ? "above" : "under", price },
          amount,
          mode: mode.replace(" MODE", ""),
          // Privacy routing is not implemented yet; keep the wire format
          // compatible while hiding the non-functional control from users.
          privacy: "MEDIUM",
        },
      });
      onBroadcast(id);
    } catch (err) {
      setError(errorText(err));
      setBusy(false);
    }
  };

  if (!options) return null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-6)", padding: "var(--space-6)" }}>
      {/* I WANT TO segmented control */}
      <Segmented
        label="I WANT TO"
        values={options.actions}
        selected={action}
        onSelect={(next) => {
          setAction(next);
          if (next === "BUY") setBuyTarget("SOL");
        }}
      />

      {action === "BUY" && (
        <Field label="BUY TYPE">
          <div style={{ display: "flex", gap: "var(--space-3)" }}>
            <Pill selected={buyTarget === "SOL"} onClick={() => setBuyTarget("SOL")}>BUY SOL</Pill>
            <Pill selected={buyTarget === "NFT"} onClick={() => { setBuyTarget("NFT"); setAsset("BOOST NFT"); setAmount("1"); setCondition("Price under"); setPrice("0.0001"); }}>BUY BOOST NFT</Pill>
          </div>
          <span style={{ color: "var(--text-muted)", fontSize: "var(--text-2xs)" }}>
            {buyTarget === "SOL" ? "Broadcast a SOL buy intent through the mesh." : "Broadcast a buy intent. The matching agent negotiates against a seller's listed Boost NFT."}
          </span>
        </Field>
      )}

      {action === "SELL" && (
        <Field label="SELL TYPE">
          <div style={{ display: "flex", gap: "var(--space-3)" }}>
            <Pill selected={sellTarget === "SOL"} onClick={() => { setSellTarget("SOL"); setAsset("SOL"); }}>SELL SOL</Pill>
            <Pill selected={sellTarget === "NFT"} onClick={() => { setSellTarget("NFT"); setAsset("BOOST NFT"); setAmount("1"); }}>SELL NFT</Pill>
          </div>
          <span style={{ color: "var(--text-muted)", fontSize: "var(--text-2xs)" }}>
            {sellTarget === "SOL" ? "Broadcast a SOL sell intent through the mesh." : "Open the SPL boost NFT marketplace to list an item."}
          </span>
        </Field>
      )}

      {/* ASSET */}
      <Field label="ASSET">
        <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
          {(action === "BUY" && buyTarget === "NFT") || (action === "SELL" && sellTarget === "NFT") ? (
            <Pill selected onClick={() => setAsset("BOOST NFT")}>BOOST NFT</Pill>
          ) : options.assets.map((a: AssetOption) => (
            <Pill key={a.name} selected={asset === a.name} onClick={() => setAsset(a.name)}>
              {a.name}
            </Pill>
          ))}
        </div>
      </Field>

      {/* CONDITION */}
      <Field label="CONDITION">
        <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
          {options.conditions.map((c) => (
            <Pill key={c} selected={condition === c} onClick={() => setCondition(c)}>
              {c}
            </Pill>
          ))}
        </div>
        {!condition.startsWith("Any") && (
          <input
            className="cm-input cm-touch"
            inputMode="decimal"
            placeholder={asset === "BOOST NFT" ? "Price (SOL), e.g. 0.0001" : "Price (USDC)"}
            value={price}
            onChange={(e) => setPrice(e.target.value)}
            style={{ marginTop: "var(--space-4)", width: "100%" }}
          />
        )}
      </Field>

      {/* AMOUNT */}
      <Field label="AMOUNT">
        <input
          className="cm-input cm-touch"
          inputMode={asset === "BOOST NFT" ? "numeric" : "decimal"}
          placeholder={asset === "BOOST NFT" ? "Number of NFTs (e.g. 1)" : `Amount in ${asset}`}
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          style={{ width: "100%" }}
        />
      </Field>

      {/* MODE */}
      <Field label="MODE">
        <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
          {options.modes.map((m) => (
            <Pill key={m.label} selected={mode === m.label} onClick={() => setMode(m.label)}>
              {m.label.replace(" MODE", "")}
            </Pill>
          ))}
        </div>
      </Field>

      {error && (
        <Panel>
          <div style={{ color: "var(--text-alert)", fontSize: "var(--text-base)", padding: "var(--space-4)" }}>
            {error}
          </div>
        </Panel>
      )}

      <Button tone="primary" size="lg" block className="cm-touch" disabled={busy} onClick={confirm}>
        {busy ? "BROADCASTING..." : action === "BUY" && buyTarget === "NFT" ? "BROADCAST BUY BOOST NFT" : action === "SELL" && sellTarget === "NFT" ? "OPEN NFT MARKETPLACE" : action === "SELL" ? "BROADCAST SELL SOL" : "BROADCAST BUY INTENT"}
      </Button>
    </div>
  );
}

function Segmented({ label, values, selected, onSelect }: { label: string; values: string[]; selected: string; onSelect: (v: string) => void }) {
  return (
    <Field label={label}>
      <div role="tablist" aria-label={label} style={{ display: "flex", gap: "var(--space-3)" }}>
        {values.map((v) => (
          <button
            key={v}
            type="button"
            role="tab"
            aria-selected={selected === v}
            className="cm-touch"
            onClick={() => onSelect(v)}
            style={{
              flex: 1,
              background: "none",
              border: "none",
              borderBottom: selected === v ? "var(--border-width-thick) solid var(--border-loud)" : "var(--border-width-thick) solid transparent",
              padding: "var(--space-4) 0",
              cursor: "pointer",
              fontFamily: "var(--type-label-family)",
              fontSize: "var(--text-2xs)",
              letterSpacing: "var(--tracking-widest)",
              color: selected === v ? "var(--text-primary)" : "var(--text-muted)",
            }}
          >
            {v}
          </button>
        ))}
      </div>
    </Field>
  );
}

function Pill({ selected, onClick, children }: { selected: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      className="cm-touch"
      onClick={onClick}
      style={{
        background: selected ? "var(--surface-raised)" : "none",
        border: selected ? "var(--border-width-thick) solid var(--border-loud)" : "var(--border-width-thin) solid var(--border-subtle)",
        borderRadius: "var(--radius-sm)",
        padding: "var(--space-3) var(--space-5)",
        cursor: "pointer",
        fontFamily: "var(--type-label-family)",
        fontSize: "var(--text-2xs)",
        letterSpacing: "var(--tracking-widest)",
        color: selected ? "var(--text-primary)" : "var(--text-secondary)",
      }}
    >
      {children}
    </button>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
      <span
        style={{
          fontFamily: "var(--type-label-family)",
          fontSize: "var(--text-2xs)",
          letterSpacing: "var(--tracking-widest)",
          color: "var(--text-muted)",
        }}
      >
        {label}
      </span>
      {children}
    </div>
  );
}
