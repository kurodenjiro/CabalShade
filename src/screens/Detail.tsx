import { useEffect, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge, Button, Panel, StatusDot } from "../ds";
import type { IntentDetail, LogLine } from "../types/bindings";
import type { IntentId } from "../shell/screen";

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); } catch { return "Unknown error"; }
}

/**
 * The intent detail screen.
 *
 * Loads the full intent from `get_intent`, streams settlement progress via
 * `settle_intent` (a `Channel<LogLine>`), and offers cancel while the intent is
 * still live on the mesh. The status text and dot tone come from the same
 * `IntentStatus` variant the list uses, so detail and list never disagree.
 */
export function Detail({
  id,
  onSettled,
  onBack,
}: {
  id: IntentId;
  onSettled: () => void;
  onBack: () => void;
}) {
  const [detail, setDetail] = useState<IntentDetail | null>(null);
  const [log, setLog] = useState<Array<{ text: string; tone: string }>>([]);
  const [busy, setBusy] = useState(false);
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

  // The autonomous matcher reports its actual milestones separately from the
  // transaction stream, so an order opened while the worker is negotiating
  // does not look frozen at NEGOTIATING.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ id?: string; text?: string }>("agent-exchange", (event) => {
      if (event.payload?.id !== id || !event.payload.text) return;
      setLog((lines) => [...lines, { text: event.payload.text!, tone: "info" }]);
    }).then((stop) => { unlisten = stop; }).catch(() => undefined);
    return () => unlisten?.();
  }, [id]);

  // The settlement worker changes state outside this view. Re-read the
  // canonical ledger on its event so the detail screen never appears stuck
  // at NEGOTIATING while the mesh/LLM worker is already progressing.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ id?: string }>("intent-updated", (event) => {
      if (event.payload?.id !== id) return;
      invoke<IntentDetail>("get_intent", { id })
        .then(setDetail)
        .catch(() => undefined);
    }).then((stop) => { unlisten = stop; }).catch(() => undefined);
    return () => unlisten?.();
  }, [id]);

  const status = detail?.view.status.status;

  const settle = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    const channel = new Channel<LogLine>();
    channel.onmessage = (line) => setLog((lines) => [...lines, { text: line.text, tone: line.tone }]);
    try {
      // settle_intent streams verification lines over the Channel and returns
      // the subscription id; the stream closes itself on completion.
      await invoke<string>("settle_intent", { id, onLine: channel });
      // Only a genuinely settled intent has a proof to show. When the escrow
      // create was signed offline and queued for relay, the intent lands in
      // WAITING — stay here and let the resume path finish it.
      const next = await invoke<IntentDetail>("get_intent", { id }).catch(() => null);
      if (next?.view.status.status === "SETTLED") {
        setLog((lines) => [...lines, { text: "SETTLED.", tone: "ok" }]);
        setBusy(false);
        onSettled();
      } else {
        setBusy(false);
      }
    } catch (err) {
      setError(errorText(err));
      setBusy(false);
    }
  };

  const cancel = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await invoke("cancel_intent", { id });
      setBusy(false);
      onBack();
    } catch (err) {
      setError(errorText(err));
      setBusy(false);
    }
  };

  if (!detail) {
    return (
      <div style={{ padding: "var(--space-6)" }}>
        <Panel>{error ?? "LOADING..."}</Panel>
      </div>
    );
  }

  const v = detail.view;
  const isNegotiating = status === "NEGOTIATING";
  const negotiatedPrice = v.price;
  const live =
    status === "DRAFT" ||
    status === "BROADCAST" ||
    status === "NEGOTIATING" ||
    status === "FINDING_ROUTE" ||
    status === "WAITING";

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-6)", padding: "var(--space-6)" }}>
      <Panel label="INTENT">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-5)", padding: "var(--space-4)" }}>
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-4)" }}>
            <StatusDot
              tone={
                status === "SETTLED"
                  ? "online"
                  : status === "FAILED"
                    ? "alert"
                    : status === "WAITING"
                      ? "idle"
                      : "info"
              }
            />
            <span
              style={{
                fontFamily: "var(--type-heading-family)",
                fontSize: "var(--text-sm)",
                letterSpacing: "var(--type-heading-tracking)",
                color: "var(--text-primary)",
              }}
            >
              {v.title}
            </span>
            {v.badge ? (
              <Badge tone="quiet" size="sm">
                {v.badge}
              </Badge>
            ) : null}
          </div>
          <div style={{ borderTop: "1px solid var(--line-subtle)", borderBottom: "1px solid var(--line-subtle)", display: "grid", placeItems: "center", minHeight: 196, padding: "var(--space-3) 0", background: "radial-gradient(circle at center, rgba(255,255,255,.06), transparent 64%)" }}>
            <img src="/ds-assets/textures/mesh-radar.png" alt="Network map" className="cm-pixel" style={{ width: "min(100%, 214px)", aspectRatio: "1", objectFit: "cover", opacity: 0.92 }} />
          </div>
          <Row label="CONDITION" value={detail.condition} />
          <Row label="AMOUNT" value={detail.amount} />
          <Row label="MODE" value={detail.mode} />
          <Row label="STATUS" value={statusWord(status)} />
          <Row label="ELAPSED" value={v.elapsed} />
        </div>
      </Panel>

      {/* The deal itself: who, at what price, which way the asset moves, and
          the transaction that proves it. Everything a user needs to check
          that what settled is what they agreed to. */}
      {v.counterparty ? (
        <Panel label="DEAL">
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-5)", padding: "var(--space-4)" }}>
            <Row label="COUNTERPARTY" value={v.counterparty} />
            {detail.counterpartyWallet ? (
              <Row label="WALLET" value={detail.counterpartyWallet} wrap />
            ) : null}
            {v.price ? <Row label="AGREED PRICE" value={v.price} /> : null}
            {detail.direction ? <Row label="DIRECTION" value={detail.direction} /> : null}
            {v.proof ? <Row label="PROOF" value={v.proof} wrap /> : null}
            {v.explorer ? (
              <Button
                tone="secondary"
                size="md"
                block
                className="cm-touch"
                onClick={() => {
                  // A failed open is not worth an error banner over a settled
                  // trade; the signature above is still readable and copyable.
                  openUrl(v.explorer!).catch(() => undefined);
                }}
              >
                VIEW ON SOLANA EXPLORER
              </Button>
            ) : null}
          </div>
        </Panel>
      ) : null}

      {isNegotiating ? (
        <Panel label="AGENT EXCHANGE">
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)", padding: "var(--space-4)" }}>
            <AgentLine tone="done" speaker="MESH SCOUT" text="MATCHED OPPOSITE ORDER" />
            <AgentLine tone="live" speaker="LOCAL LLM" text={negotiatedPrice ? `PROPOSED ${negotiatedPrice}` : "ANALYSING USDC TERMS…"} />
            <AgentLine tone="pending" speaker="SETTLEMENT AGENT" text={negotiatedPrice ? "WAITING FOR ACCEPTANCE" : "ROUTE LOCKS AFTER QUOTE"} />
            {log.filter((line) => line.text.includes("LLM") || line.text.includes("MESH AGENT")).slice(-2).map((line, i) => (
              <AgentLine key={`${line.text}-${i}`} tone="done" speaker="LIVE EVENT" text={line.text} />
            ))}
          </div>
        </Panel>
      ) : null}

      {log.length > 0 && (
        <Panel label="SETTLEMENT LOG">
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)", padding: "var(--space-4)" }}>
            {log.map((line, i) => <SettlementLogLine key={i} text={line.text} tone={line.tone} />)}
          </div>
        </Panel>
      )}

      {error && (
        <Panel>
          <div style={{ color: "var(--text-alert)", fontSize: "var(--text-base)", padding: "var(--space-4)" }}>{error}</div>
        </Panel>
      )}

      {/* A matched pair settles through its counterparty's wallet, driven by
          the deal worker. The manual button runs the one-sided escrow to a
          fallback payee, which for a matched order would pay the wrong
          person — so it is offered only while an order is still unpaired.
          A peer's mirrored order is not this device's to act on at all. */}
      {live && !v.origin && !v.counterparty ? (
        <Button tone="primary" size="lg" block className="cm-touch" disabled={busy} onClick={settle}>
          {busy ? "SETTLING..." : "SETTLE INTENT"}
        </Button>
      ) : null}

      {live && !v.origin ? (
        <Button tone="danger" size="md" block className="cm-touch" disabled={busy} onClick={cancel}>
          CANCEL INTENT
        </Button>
      ) : null}
    </div>
  );
}

/**
 * `NEGOTIATING` is the lifecycle's word for "paired, agreeing terms". On the
 * screen a user is watching for a fill, MATCHED is the fact; the list uses the
 * same mapping so the two never disagree.
 */
function statusWord(status: string | undefined): string {
  if (status === "NEGOTIATING") return "MATCHED";
  return (status ?? "DRAFT").replace("_", " ");
}

function Row({ label, value, wrap = false }: { label: string; value: string; wrap?: boolean }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", gap: "var(--space-5)" }}>
      <span
        style={{
          fontFamily: "var(--type-label-family)",
          fontSize: "var(--text-2xs)",
          letterSpacing: "var(--tracking-widest)",
          color: "var(--text-muted)",
          flexShrink: 0,
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontFamily: "var(--type-data-family)",
          // A 44-character address or an 88-character signature has to break
          // somewhere, and a truncated one cannot be checked against a chain.
          fontSize: wrap ? "var(--text-2xs)" : "var(--text-sm)",
          color: "var(--text-primary)",
          textAlign: "right",
          wordBreak: wrap ? "break-all" : "normal",
          userSelect: wrap ? "text" : "auto",
        }}
      >
        {value}
      </span>
    </div>
  );
}

function SettlementLogLine({ text, tone }: { text: string; tone: string }) {
  const isLlm = text.toUpperCase().includes("LLM");
  const thinking = isLlm && /ANALYSING|ANALYZING|THINKING/.test(text.toUpperCase());
  if (!isLlm) {
    return <span style={{ fontFamily: "var(--type-data-family)", fontSize: "var(--text-2xs)", color: tone === "error" ? "var(--text-alert)" : "var(--text-muted)" }}>&gt; {text}</span>;
  }
  return (
    <div style={{ display: "grid", gridTemplateColumns: "auto minmax(0, 1fr)", gap: "var(--space-3)", alignItems: "start", padding: "var(--space-3) var(--space-4)", borderLeft: "var(--border-width-thick) solid var(--accent-cyan)", background: "linear-gradient(90deg, rgba(0,255,255,.12), transparent)" }}>
      <span aria-hidden="true" className={thinking ? "cm-pulse" : undefined} style={{ color: "var(--accent-cyan)", fontFamily: "var(--type-data-family)" }}>◆</span>
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
        <span style={{ fontFamily: "var(--type-label-family)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)", color: "var(--accent-cyan)" }}>LOCAL LLM {thinking ? <span className="cm-blink">●●●</span> : "✓"}</span>
        <span style={{ fontFamily: "var(--type-data-family)", fontSize: "var(--text-2xs)", color: "var(--text-primary)" }}>{text.replace(/^LOCAL LLM:\s*/i, "")}</span>
      </div>
    </div>
  );
}

function AgentLine({
  tone,
  speaker,
  text,
}: {
  tone: "done" | "live" | "pending";
  speaker: string;
  text: string;
}) {
  const color = tone === "live" ? "var(--accent-cyan)" : tone === "done" ? "var(--text-primary)" : "var(--text-muted)";
  return (
    <div style={{ display: "grid", gridTemplateColumns: "10px minmax(0, 1fr)", columnGap: "var(--space-3)", alignItems: "start" }}>
      <span aria-hidden="true" style={{ color, fontFamily: "var(--type-data-family)", lineHeight: 1.35 }}>{tone === "live" ? "◆" : tone === "done" ? "■" : "□"}</span>
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
        <span style={{ fontFamily: "var(--type-label-family)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)", color }}>{speaker}</span>
        <span style={{ fontFamily: "var(--type-data-family)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-wide)", color: tone === "pending" ? "var(--text-muted)" : "var(--text-primary)" }}>{text}</span>
      </div>
    </div>
  );
}
