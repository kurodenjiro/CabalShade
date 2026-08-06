import { useEffect, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { Badge, Button, Panel, StatusDot } from "../ds";
import type { IntentDetail, LogLine } from "../types/bindings";
import type { IntentId } from "../shell/screen";

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
      setError(err instanceof Error ? err.message : String(err));
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
      setError(err instanceof Error ? err.message : String(err));
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
          <Row label="CONDITION" value={detail.condition} />
          <Row label="AMOUNT" value={detail.amount} />
          <Row label="MODE" value={detail.mode} />
          <Row label="PRIVACY" value={detail.privacy} />
          <Row label="STATUS" value={(status ?? "DRAFT").replace("_", " ")} />
          <Row label="ELAPSED" value={v.elapsed} />
        </div>
      </Panel>

      {log.length > 0 && (
        <Panel label="SETTLEMENT LOG">
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)", padding: "var(--space-4)" }}>
            {log.map((line, i) => (
              <span key={i} style={{ fontFamily: "var(--type-data-family)", fontSize: "var(--text-2xs)", color: "var(--text-muted)" }}>
                &gt; {line.text}
              </span>
            ))}
          </div>
        </Panel>
      )}

      {error && (
        <Panel>
          <div style={{ color: "var(--text-alert)", fontSize: "var(--text-base)", padding: "var(--space-4)" }}>{error}</div>
        </Panel>
      )}

      {status === "DRAFT" || status === "BROADCAST" || status === "NEGOTIATING" || status === "FINDING_ROUTE" || status === "WAITING" ? (
        <Button tone="primary" size="lg" block className="cm-touch" disabled={busy} onClick={settle}>
          {busy ? "SETTLING..." : "SETTLE INTENT"}
        </Button>
      ) : null}

      {status === "DRAFT" || status === "BROADCAST" || status === "NEGOTIATING" || status === "FINDING_ROUTE" || status === "WAITING" ? (
        <Button tone="danger" size="md" block className="cm-touch" disabled={busy} onClick={cancel}>
          CANCEL INTENT
        </Button>
      ) : null}
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
