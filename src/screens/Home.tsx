import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Panel, StatBlock, StatusDot, Terminal } from "../ds";
import { useLogStream } from "../state/useLogStream";
import type { LogLine, MeshSnapshotView } from "../types/bindings";
import type { BoostNft } from "../boosts";
import { boostExpired, boostLabel } from "../boosts";

/** Visible ticker lines. The rest are retained but scrolled. */
const VISIBLE = 4;
const RETAINED = 200;

/**
 * Mesh status and the live ticker.
 *
 * Every figure here is pre-formatted in Rust: the brand demands exact separated
 * numbers, so implementing the separator rules per screen would guarantee they
 * drift.
 *
 * The reputation tile is derived in Rust from real demonstrated behaviour —
 * relayed transactions, relayed bytes, settled intents and observed peer
 * latency (see src/reputation.rs). The delta is measured against a persisted
 * baseline so it stays stable across the five-second poll. Nothing here knows
 * any of that — the tile renders whatever Rust sends, like every other figure
 * on the screen.
 */
export function Home() {
  const [snapshot, setSnapshot] = useState<MeshSnapshotView | null>(null);
  const [lines, setLines] = useState<LogLine[]>([]);
  const [boost, setBoost] = useState<BoostNft | null>(null);
  const isTauriRuntime = Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);

  useEffect(() => {
    let cancelled = false;

    const refresh = () => {
      invoke<MeshSnapshotView>("mesh_snapshot")
        .then((next) => {
          if (!cancelled) setSnapshot(next);
        })
        // Not ready yet is normal during bootstrap, and the empty state below
        // already renders it.
        .catch(() => undefined);
    };

    refresh();
    const refreshBoost = () => invoke<BoostNft[]>("get_boost_nfts")
      .then((items) => setBoost(items.find((item) => item.owned && !boostExpired(item)) ?? null))
      .catch(() => setBoost(null));
    refreshBoost();
    const boostTimer = window.setInterval(refreshBoost, 5_000);
    const timer = window.setInterval(refresh, 5_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.clearInterval(boostTimer);
    };
  }, []);

  useLogStream("subscribe_mesh_log", {}, (line) => {
    setLines((previous) => {
      const next = [...previous, line];
      return next.length > RETAINED ? next.slice(-RETAINED) : next;
    });
  }, isTauriRuntime);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-6)",
        padding: "var(--space-6)",
      }}
    >
      <Panel label="MESH STATUS">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-5)", padding: "var(--space-6)" }}>
          <div style={{ display: "flex", justifyContent: "flex-end", minHeight: 96 }}>
            <img
              src="/ds-assets/logo/oracle-emblem.png"
              alt="Cabal Mesh oracle emblem"
              className="cm-pixel"
              style={{ width: 108, height: 108, objectFit: "contain", opacity: 0.92 }}
            />
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-4)" }}>
            <StatusDot tone={snapshot?.connected ? "online" : "offline"} pulse={snapshot?.connected} />
            {/* The status word exists as text, not only as a colour — a
                coloured square alone reaches no screen reader. */}
            <span
              style={{
                fontFamily: "var(--type-label-family)",
                fontSize: "var(--text-2xs)",
                letterSpacing: "var(--tracking-widest)",
                color: "var(--text-primary)",
                textTransform: "uppercase",
              }}
            >
              {snapshot?.connected ? "You are connected" : "Mesh unreachable · operating offline"}
            </span>
          </div>

          <Field label="NODE ID" value={snapshot?.nodeId ?? "—"} />
          <Field label="UPTIME" value={snapshot?.uptime ?? "—"} />
          <Field label="RELAY BOOST" value={boost ? boostLabel(boost) : "NONE"} />
        </div>
      </Panel>

      <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
        {(snapshot?.stats ?? []).map((tile) => (
          <StatBlock
            key={tile.label}
            label={tile.label}
            value={tile.value}
            delta={tile.delta}
            deltaTone={tile.deltaTone}
          />
        ))}
      </div>

      <Terminal
        label="MESH LOG"
        role="log"
        aria-live="polite"
        lines={lines.slice(-VISIBLE).map((line) => ({ text: line.text, tone: line.tone }))}
      />
    </div>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div className="cm-row" style={{ display: "flex", justifyContent: "space-between", gap: "var(--space-5)" }}>
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
          letterSpacing: "var(--type-data-tracking)",
          color: "var(--text-primary)",
        }}
      >
        {value}
      </span>
    </div>
  );
}
