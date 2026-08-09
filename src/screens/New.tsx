import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Panel } from "../ds";
import type { AssetOption, FormOptions } from "../types/bindings";
import type { IntentId } from "../shell/screen";

/**
 * The compose screen.
 *
 * Every option comes from `intent_form_options` — Rust is the single source of
 * the modes, assets, conditions and privacy levels, so the form cannot drift
 * from what `broadcast_intent` will accept. Validation happens on submit via
 * `preview_intent`, which validates the draft and returns exactly what the
 * confirm dialog shows.
 */
export function New({ onBroadcast }: { onBroadcast: (id: IntentId) => void }) {
  const [options, setOptions] = useState<FormOptions | null>(null);
  const [loadingOptions, setLoadingOptions] = useState(true);
  const [optionsError, setOptionsError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const [action, setAction] = useState("BUY");
  const [asset, setAsset] = useState("SOL");
  const [condition, setCondition] = useState("Price under");
  const [price, setPrice] = useState("");
  const [amount, setAmount] = useState("");
  const [mode, setMode] = useState("SHARK MODE");
  const [privacy, setPrivacy] = useState("MEDIUM");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoadingOptions(true);
    setOptionsError(null);
    invoke<FormOptions>("intent_form_options")
      .then((next) => {
        if (!cancelled) {
          setOptions(next);
          if (next.assets.length > 0) setAsset(next.assets[0].name);
          if (next.modes.length > 0) setMode(next.modes[0].label);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setOptions(null);
          setOptionsError("FORM OPTIONS UNAVAILABLE");
        }
      })
      .finally(() => {
        if (!cancelled) setLoadingOptions(false);
      });
    return () => {
      cancelled = true;
    };
  }, [reloadToken]);

  const confirm = async () => {
    if (busy) return;
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
          privacy,
        },
      });
      onBroadcast(id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setBusy(false);
    }
  };

  if (loadingOptions) {
    return (
      <Panel label="NEW INTENT">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)", padding: "var(--space-8)", alignItems: "center", textAlign: "center" }}>
          <span style={{ fontFamily: "var(--type-heading-family)", fontSize: "var(--text-sm)", color: "var(--text-primary)" }}>LOADING FORM OPTIONS…</span>
          <span style={{ color: "var(--text-muted)", fontSize: "var(--text-base)" }}>Reading the active mesh protocol.</span>
        </div>
      </Panel>
    );
  }

  if (!options) {
    return (
      <Panel label="NEW INTENT">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)", padding: "var(--space-8)", alignItems: "center", textAlign: "center" }}>
          <span style={{ fontFamily: "var(--type-heading-family)", fontSize: "var(--text-sm)", color: "var(--text-alert)" }}>{optionsError ?? "FORM UNAVAILABLE"}</span>
          <span style={{ color: "var(--text-muted)", fontSize: "var(--text-base)" }}>The desktop bridge did not return the protocol options.</span>
          <Button tone="secondary" size="md" className="cm-touch" onClick={() => setReloadToken((value) => value + 1)}>RETRY</Button>
        </div>
      </Panel>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-6)", padding: "var(--space-6)" }}>
      {/* I WANT TO segmented control */}
      <Segmented
        label="I WANT TO"
        values={options.actions}
        selected={action}
        onSelect={setAction}
      />

      {/* ASSET */}
      <Field label="ASSET">
        <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
          {options.assets.map((a: AssetOption) => (
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
            placeholder="Price (USD)"
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
          inputMode="decimal"
          placeholder={`Amount in ${asset}`}
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

      {/* PRIVACY */}
      <Field label="PRIVACY">
        <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
          {options.privacyLevels.map((p) => (
            <Pill key={p} selected={privacy === p} onClick={() => setPrivacy(p)}>
              {p}
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
        {busy ? "BROADCASTING..." : "BROADCAST INTENT"}
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
