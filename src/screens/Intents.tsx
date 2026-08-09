import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Badge, Button, Panel, StatusDot } from "../ds";
import type { IntentTab } from "../shell/screen";
import type { IntentView } from "../types/bindings";

const TABS: IntentTab[] = ["ACTIVE", "PENDING", "HISTORY"];

/**
 * Empty copy per tab, in the board's voice: declarative, present tense,
 * fragments ending in full stops. "Nothing is queued. Nothing is stored." is
 * the prototype's own line, and the pattern generalises to the other tabs.
 */
const EMPTY: Record<IntentTab, { title: string; body: string }> = {
  ACTIVE: { title: "NO ACTIVE INTENTS", body: "Nothing is running. Nothing is stored." },
  PENDING: { title: "NO PENDING INTENTS", body: "Nothing is queued. Nothing is stored." },
  HISTORY: { title: "NO HISTORY", body: "Nothing has settled. Nothing is stored." },
};

/**
 * The intent list.
 *
 * Status drives both the text and the indicator tone from **one** enum variant.
 * The prototype passes a colour alongside the status (`dot: BLUE`), which would
 * hard-code the palette into Rust and turn a design change into a backend
 * release.
 *
 * The empty state is first-class rather than an afterthought — it is the state
 * this build is actually in, and the prototype ships a toggle for it precisely
 * because it matters.
 */
export function Intents({
  tab,
  onTabChange,
  onCompose,
  onOpen,
}: {
  tab: IntentTab;
  onTabChange: (tab: IntentTab) => void;
  onCompose: () => void;
  onOpen: (id: string) => void;
}) {
  const [intents, setIntents] = useState<IntentView[] | null>(null);

  const fetchIntents = () => {
    invoke<IntentView[]>("list_intents", { filter: tab })
      .then((next) => setIntents(next))
      .catch(() => setIntents([]));
  };

  useEffect(() => {
    let cancelled = false;
    invoke<IntentView[]>("list_intents", { filter: tab })
      .then((next) => {
        if (!cancelled) setIntents(next);
      })
      .catch(() => {
        if (!cancelled) setIntents([]);
      });
    return () => {
      cancelled = true;
    };
  }, [tab]);

  // Refresh when Rust emits `intent-updated` (broadcast / cancel / settle), so
  // the list tracks real transitions without polling.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen("intent-updated", fetchIntents)
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        /* event bus unavailable: the tab refetch still covers it */
      });
    return () => {
      if (unlisten) unlisten();
    };
  }, [tab]);

  // A peer can deliver an intent before this view has attached its event
  // listener. Polling the persisted ledger keeps the matched status visible
  // in both demo windows without relying on that timing.
  useEffect(() => {
    const timer = window.setInterval(fetchIntents, 2_000);
    return () => window.clearInterval(timer);
  }, [tab]);

  const empty = EMPTY[tab];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-6)", padding: "var(--space-6)" }}>
      {/* role=tablist plus aria-selected: the 2px underline is a visual
          selected state and reaches no assistive technology on its own. */}
      <div role="tablist" aria-label="Intent filter" style={{ display: "flex", gap: "var(--space-7)" }}>
        {TABS.map((name) => (
          <button
            key={name}
            type="button"
            role="tab"
            aria-selected={tab === name}
            className="cm-touch"
            onClick={() => onTabChange(name)}
            style={{
              background: "none",
              border: "none",
              padding: "var(--space-4) 0",
              cursor: "pointer",
              fontFamily: "var(--type-label-family)",
              fontSize: "var(--text-2xs)",
              letterSpacing: "var(--tracking-widest)",
              color: tab === name ? "var(--text-primary)" : "var(--text-muted)",
              borderBottom:
                tab === name
                  ? "var(--border-width-thick) solid var(--border-loud)"
                  : "var(--border-width-thick) solid transparent",
            }}
          >
            {name}
          </button>
        ))}
      </div>

      {intents === null ? null : intents.length === 0 ? (
        <Panel>
          <div
            style={{
              padding: "var(--space-11) var(--space-6)",
              display: "flex",
              flexDirection: "column",
              gap: "var(--space-5)",
              alignItems: "center",
              textAlign: "center",
            }}
          >
            <span
              style={{
                fontFamily: "var(--type-heading-family)",
                fontSize: "var(--text-sm)",
                letterSpacing: "var(--type-heading-tracking)",
                color: "var(--text-primary)",
              }}
            >
              {empty.title}
            </span>
            <span style={{ fontSize: "var(--text-base)", color: "var(--text-muted)" }}>{empty.body}</span>
            <Button
              tone="secondary"
              size="md"
              className="cm-touch"
              onClick={onCompose}
              style={{ marginTop: "var(--space-6)" }}
            >
              NEW INTENT
            </Button>
          </div>
        </Panel>
      ) : (
        <>
          {intents.map((intent) => (
            <IntentRow key={intent.id} intent={intent} onOpen={() => onOpen(intent.id)} />
          ))}
          <Button tone="secondary" size="lg" block className="cm-touch" onClick={onCompose}>
            + NEW INTENT
          </Button>
        </>
      )}
    </div>
  );
}

/**
 * The status word as the board says it.
 *
 * `NEGOTIATING` is the lifecycle's name for "paired, agreeing terms", which
 * reads as indecision on a row that has in fact found its counterparty —
 * MATCHED is the fact the user is waiting for.
 */
const STATUS_WORD: Record<string, string> = {
  NEGOTIATING: "MATCHED",
  FINDING_ROUTE: "ROUTING",
};

function IntentRow({ intent, onOpen }: { intent: IntentView; onOpen: () => void }) {
  const status = intent.status.status;
  const tone =
    status === "SETTLED"
      ? "online"
      : status === "FAILED"
        ? "alert"
        : status === "WAITING"
          ? "idle"
          : "info";
  // A matched row's whole point is who and at what price, so it says both
  // rather than making the user open the detail to find out.
  const dealLine = intent.counterparty
    ? `${intent.counterparty}${intent.price ? ` · ${intent.price}` : ""}`
    : null;

  return (
    <Panel>
      <button
        type="button"
        onClick={onOpen}
        className="cm-row"
        style={{ width: "100%", padding: "var(--space-6)", display: "flex", flexDirection: "column", gap: "var(--space-5)", background: "none", border: "none", cursor: "pointer", textAlign: "left" }}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
          <span
            style={{
              fontFamily: "var(--type-heading-family)",
              fontSize: "var(--text-sm)",
              letterSpacing: "var(--type-heading-tracking)",
              color: "var(--text-primary)",
            }}
          >
            {intent.title}
          </span>
          <span
            style={{
              fontFamily: "var(--type-label-family)",
              fontSize: "var(--text-2xs)",
              letterSpacing: "var(--tracking-widest)",
              color: "var(--text-muted)",
            }}
          >
            {intent.subtitle}
          </span>
          <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
            {intent.badge ? (
              <Badge tone="quiet" size="sm">
                {intent.badge}
              </Badge>
            ) : null}
            {/* A mirrored peer order sits in the same list as the user's own.
                Saying whose it is prevents reading someone else's exposure as
                one's own position. */}
            {intent.origin ? (
              <Badge tone="quiet" size="sm">
                PEER {intent.origin}
              </Badge>
            ) : null}
          </div>
        </div>

        {dealLine ? (
          <div
            aria-label="Matched counterparty"
            style={{
              display: "grid",
              gridTemplateColumns: "auto 1fr",
              gap: "var(--space-3)",
              alignItems: "center",
              padding: "var(--space-3) var(--space-4)",
              borderLeft: "var(--border-width-thick) solid var(--accent-cyan)",
              background: "linear-gradient(90deg, rgba(0,255,255,.10), transparent)",
            }}
          >
            <span aria-hidden="true" style={{ color: "var(--accent-cyan)", fontFamily: "var(--type-data-family)", fontSize: "var(--text-sm)" }}>◆</span>
            <span style={{ fontFamily: "var(--type-label-family)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)", color: "var(--text-primary)" }}>
              {status === "SETTLED" ? "SETTLED WITH" : "MATCHED WITH"} {dealLine}
            </span>
          </div>
        ) : null}

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "var(--space-5)" }}>
          <span
            style={{
              fontFamily: "var(--type-data-family)",
              fontSize: "var(--text-sm)",
              color: "var(--text-primary)",
            }}
          >
            {intent.amount}
          </span>
          <span style={{ display: "flex", alignItems: "center", gap: "var(--space-3)" }}>
            <StatusDot tone={tone} />
            {/* The status word exists as text. Colour alone reaches no screen
                reader, and the brand's accent budget forbids leaning on it. */}
            <span
              style={{
                fontFamily: "var(--type-label-family)",
                fontSize: "var(--text-2xs)",
                letterSpacing: "var(--tracking-widest)",
                color: "var(--text-secondary)",
              }}
            >
              {STATUS_WORD[status] ?? status.replace("_", " ")}
            </span>
          </span>
          <span
            style={{
              fontFamily: "var(--type-data-family)",
              fontSize: "var(--text-2xs)",
              color: "var(--text-muted)",
            }}
          >
            {intent.elapsed}
          </span>
        </div>
      </button>
    </Panel>
  );
}
