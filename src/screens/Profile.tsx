import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Icon, Panel, Switch } from "../ds";
import type { GlyphName } from "../shell/screen";
import type { ActivityLogView, ProfileView } from "../types/bindings";

const ROWS: ReadonlyArray<{ label: string; icon: GlyphName }> = [
  { label: "ACHIEVEMENTS", icon: "reputation" },
  { label: "ACTIVITY LOG", icon: "log" },
  { label: "SECURITY", icon: "encrypt" },
  { label: "ABOUT CABAL MESH", icon: "mesh" },
];

/**
 * Identity, settings and leaving the mesh.
 *
 * The reputation row is derived in Rust from real demonstrated behaviour —
 * relayed transactions, relayed bytes, settled intents and observed peer
 * latency (see src/reputation.rs) — in the same place the home tile reads
 * from, so the two screens cannot disagree. With no mesh it stays an em dash —
 * there is nothing measured yet, and a constant would put one score on every
 * device.
 *
 * The network is shown plainly, with testnet marked, so nobody mistakes a test
 * balance for a real one.
 */
export function Profile({ onLeave }: { onLeave: () => void }) {
  const [profile, setProfile] = useState<ProfileView | null>(null);
  const [activity, setActivity] = useState<ActivityLogView | null>(null);
  const [expanded, setExpanded] = useState<"achievements" | "activity" | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;

    // Polled, not fetched once. A single fetch at mount races bootstrap: the
    // screen can mount before services are published, and a one-shot call then
    // leaves every field showing an em dash for the rest of the session with
    // no way to recover.
    const refresh = () =>
      invoke<ProfileView>("profile_summary")
        .then((next) => {
          if (!cancelled) setProfile(next);
        })
        .catch(() => undefined);

    refresh();
    const timer = window.setInterval(refresh, 5_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const refreshActivity = () =>
      invoke<ActivityLogView>("activity_log")
        .then((next) => {
          if (!cancelled) setActivity(next);
        })
        .catch(() => undefined);
    refreshActivity();
    const timer = window.setInterval(refreshActivity, 5_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const toggleOffline = async () => {
    if (!profile || busy) return;
    const next = !profile.offline;
    setBusy(true);
    // Optimistic, then reconciled: the switch must not feel laggy, but it also
    // must not claim a state the mesh refused.
    setProfile({ ...profile, offline: next });
    try {
      await invoke("set_offline_mode", { offline: next });
    } catch {
      setProfile({ ...profile, offline: !next });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-6)", padding: "var(--space-6)" }}>
      <Panel label="IDENTITY">
        <div style={{ padding: "var(--space-6)", display: "flex", flexDirection: "column", gap: "var(--space-5)" }}>
          <div style={{ display: "flex", justifyContent: "center", paddingBottom: "var(--space-3)" }}>
            <img
              src="/ds-assets/logo/oracle-emblem.png"
              alt="Cabal Mesh oracle emblem"
              className="cm-pixel"
              style={{ width: 124, height: 124, objectFit: "contain", opacity: 0.92 }}
            />
          </div>
          <Field label="NODE ID" value={profile?.nodeId ?? "—"} />
          <Field label="REPUTATION SCORE" value={profile?.reputation ?? "—"} />
          <Field label="MEMBER SINCE" value={profile?.memberSince ?? "—"} />
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-4)" }}>
            <Badge tone={profile?.isTestnet ? "alert" : "quiet"} size="sm">
              {profile?.network ?? "—"}
            </Badge>
            {profile?.isTestnet ? (
              <span style={{ fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)", color: "var(--text-muted)" }}>
                TEST FUNDS ONLY.
              </span>
            ) : null}
          </div>
        </div>
      </Panel>

      <Panel>
        {ROWS.map((row) => {
          const interactive = row.label === "ACHIEVEMENTS" || row.label === "ACTIVITY LOG";
          const selected = row.label === "ACHIEVEMENTS" ? expanded === "achievements" : expanded === "activity";
          return interactive ? (
          <button
            key={row.label}
            type="button"
            aria-expanded={selected}
            aria-label={row.label}
            className="cm-row"
            onClick={() => setExpanded(selected ? null : row.label === "ACHIEVEMENTS" ? "achievements" : "activity")}
            style={{
              width: "100%",
              display: "flex",
              alignItems: "center",
              gap: "var(--space-5)",
              padding: "var(--space-5) var(--space-6)",
              borderTop: "var(--border-hairline-style)",
              background: "none",
              border: "none",
              textAlign: "left",
              color: "inherit",
            }}
          >
            <Icon name={row.icon} size={20} basePath="/ds-assets/icons" />
            <span
              style={{
                flex: 1,
                fontFamily: "var(--type-label-family)",
                fontSize: "var(--text-2xs)",
                letterSpacing: "var(--tracking-widest)",
                color: "var(--text-secondary)",
              }}
            >
              {row.label}
            </span>
            <span style={{ color: "var(--text-muted)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)" }}>
              {row.label === "ACHIEVEMENTS" ? (activity ? `${activity.broadcastCount + activity.settledCount} RECORDED` : "—") : (activity ? `${activity.entries.length} EVENTS` : "—")}
            </span>
          </button>
          ) : (
          <div
            key={row.label}
            role="listitem"
            aria-label={`${row.label}: not recorded`}
            className="cm-row"
            style={{ width: "100%", display: "flex", alignItems: "center", gap: "var(--space-5)", padding: "var(--space-5) var(--space-6)", borderTop: "var(--border-hairline-style)", background: "none", border: "none", textAlign: "left" }}
          >
            <Icon name={row.icon} size={20} basePath="/ds-assets/icons" />
            <span style={{ flex: 1, fontFamily: "var(--type-label-family)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)", color: "var(--text-secondary)" }}>{row.label}</span>
            <span style={{ color: "var(--text-muted)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)" }}>NOT RECORDED</span>
          </div>
          );
        })}

        {expanded === "achievements" ? (
          <div className="cm-row" style={{ display: "grid", gap: "var(--space-3)", padding: "var(--space-4) var(--space-6)", borderTop: "var(--border-hairline-style)", color: "var(--text-muted)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)" }}>
            <span>BROADCASTS: {activity?.broadcastCount ?? "—"}</span>
            <span>SETTLED: {activity?.settledCount ?? "—"}</span>
            <span>CANCELLED: {activity?.cancelledCount ?? "—"}</span>
          </div>
        ) : null}
        {expanded === "activity" ? (
          <div className="cm-row" style={{ display: "grid", gap: "var(--space-2)", padding: "var(--space-4) var(--space-6)", borderTop: "var(--border-hairline-style)", color: "var(--text-muted)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-wide)" }}>
            {activity?.entries.length ? activity.entries.slice(0, 8).map((entry) => <span key={entry.id}>{entry.kind} · {entry.summary}</span>) : <span>NO ACTIVITY RECORDED</span>}
          </div>
        ) : null}

        <div
          className="cm-row"
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-5)",
            padding: "var(--space-5) var(--space-6)",
            borderTop: "var(--border-hairline-style)",
          }}
        >
          <span
            id="offline-label"
            style={{
              flex: 1,
              fontFamily: "var(--type-label-family)",
              fontSize: "var(--text-2xs)",
              letterSpacing: "var(--tracking-widest)",
              color: "var(--text-secondary)",
            }}
          >
            OFFLINE MODE
          </span>
          {/* role=switch with aria-checked announces the resulting state
              rather than the action, which is what a screen reader user needs. */}
          <Switch
            checked={profile?.offline ?? false}
            role="switch"
            aria-checked={profile?.offline ?? false}
            aria-labelledby="offline-label"
            className="cm-touch"
            onClick={toggleOffline}
          />
        </div>
      </Panel>

      <Button tone="danger" size="lg" block className="cm-touch" onClick={onLeave}>
        LEAVE THE MESH
      </Button>

      <p
        style={{
          textAlign: "center",
          fontFamily: "var(--type-label-family)",
          fontSize: "var(--text-2xs)",
          letterSpacing: "var(--tracking-widest)",
          color: "var(--text-disabled)",
          textTransform: "uppercase",
        }}
      >
        We leave no identity, only traces.
      </p>
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
