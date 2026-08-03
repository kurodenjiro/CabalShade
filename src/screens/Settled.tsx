import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Panel } from "../ds";
import type { IntentDetail } from "../types/bindings";
import type { IntentId } from "../shell/screen";

/**
 * The settled proof screen.
 *
 * Loads the intent from `get_intent` and renders the terminal `SETTLED` state:
 * the proof hash, the price it filled at, and the elapsed time. If the intent
 * is not settled, the screen says so rather than fabricating a proof.
 */
export function Settled({ id }: { id: IntentId }) {
  const [detail, setDetail] = useState<IntentDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<IntentDetail>("get_intent", { id })
      .then((next) => {
        if (!cancelled) setDetail(next);
      })
      .catch(() => {
        if (!cancelled) setError("Intent not found.");
      });
    return () => {
      cancelled = true;
    };
  }, [id]);

  if (!detail) {
    return (
      <div style={{ padding: "var(--space-6)" }}>
        <Panel>{error ?? "LOADING PROOF..."}</Panel>
      </div>
    );
  }

  const status = detail.view.status;
  if (status.status !== "SETTLED") {
    return (
      <div style={{ padding: "var(--space-6)" }}>
        <Panel>
          <div style={{ color: "var(--text-muted)", fontSize: "var(--text-base)", padding: "var(--space-4)" }}>
            This intent has not settled yet.
          </div>
        </Panel>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-6)", padding: "var(--space-6)" }}>
      <Panel label="PROOF">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-5)", padding: "var(--space-4)" }}>
          <Row label="INTENT" value={detail.view.title} />
          <Row label="FILLED AT" value={`${status.filled_at}`} />
          <Row label="ELAPSED" value={detail.view.elapsed} />
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
            <span
              style={{
                fontFamily: "var(--type-label-family)",
                fontSize: "var(--text-2xs)",
                letterSpacing: "var(--tracking-widest)",
                color: "var(--text-muted)",
              }}
            >
              VERIFICATION HASH
            </span>
            <span
              style={{
                fontFamily: "var(--type-data-family)",
                fontSize: "var(--text-xs)",
                color: "var(--text-primary)",
                wordBreak: "break-all",
              }}
            >
              {status.proof}
            </span>
          </div>
        </div>
      </Panel>

      <Panel>
        <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-base)", padding: "var(--space-4)" }}>
          Settlement confirmed on-chain. No identity was attached.
        </div>
      </Panel>
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", gap: "var(--space-5)" }}>
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
      <span
        style={{
          fontFamily: "var(--type-data-family)",
          fontSize: "var(--text-sm)",
          color: "var(--text-primary)",
          textAlign: "right",
        }}
      >
        {value}
      </span>
    </div>
  );
}
