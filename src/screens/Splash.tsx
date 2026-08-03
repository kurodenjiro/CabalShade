import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Logo } from "../ds";
import type { SessionStatus } from "../types/bindings";

/**
 * First screen. Two offers, no chrome.
 *
 * Copy is the board's, verbatim: impersonal, present tense, fragments ending in
 * full stops. "ZERO IDENTITY. PRIVATE INTENTS." is three statements, not a
 * tagline to be softened.
 *
 * The offers are gated on the **real** session state from `session_status`:
 * ENTER THE MESH is disabled until bootstrap completes, and the node id shows
 * once one exists — no more offering a join the app cannot perform yet.
 */
export function Splash({ onEnter }: { onEnter: () => void }) {
  const [status, setStatus] = useState<SessionStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      invoke<SessionStatus>("session_status")
        .then((next) => {
          if (!cancelled) setStatus(next);
        })
        .catch(() => {
          /* not ready renders as disabled */
        });
    };
    poll();
    const interval = window.setInterval(poll, 2_000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  const ready = status?.ready ?? false;

  return (
    <section
      style={{
        minHeight: "100dvh",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: "var(--space-8)",
        padding: "var(--space-9) var(--space-8)",
        paddingTop: "calc(var(--safe-top) + var(--space-9))",
        paddingBottom: "calc(var(--safe-bottom) + var(--space-9))",
        textAlign: "center",
      }}
    >
      <Logo variant="minimal" size={72} basePath="/ds-assets/logo" />

      <h1
        style={{
          margin: 0,
          fontFamily: "var(--type-wordmark-family)",
          fontSize: "var(--text-xl)",
          letterSpacing: "var(--type-wordmark-tracking)",
          // The wordmark's tracking pushes it off-centre without a matching
          // indent — the board specifies both together.
          textIndent: "var(--type-wordmark-tracking)",
          color: "var(--text-primary)",
        }}
      >
        CABAL MESH
      </h1>

      <p
        style={{
          fontFamily: "var(--type-label-family)",
          fontSize: "var(--text-2xs)",
          letterSpacing: "var(--tracking-widest)",
          color: "var(--text-muted)",
          textTransform: "uppercase",
        }}
      >
        Zero identity. Private intents.
      </p>

      {status?.nodeId ? (
        <p
          style={{
            fontFamily: "var(--type-data-family)",
            fontSize: "var(--text-2xs)",
            color: "var(--text-secondary)",
          }}
        >
          NODE {status.nodeId}
        </p>
      ) : null}

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-6)",
          width: "100%",
          maxWidth: 320,
          // 16px apart: expanded hit areas that meet would steal each other's
          // taps (ds/mobile.css, C4).
          marginTop: "var(--space-8)",
        }}
      >
        <Button tone="primary" size="lg" block className="cm-touch" disabled={!ready} onClick={onEnter}>
          {ready ? "ENTER THE MESH" : "CONNECTING..."}
        </Button>
        <Button tone="ghost" size="lg" block className="cm-touch" onClick={onEnter}>
          CREATE ANONYMOUS NODE
        </Button>
      </div>

      <p
        style={{
          marginTop: "var(--space-8)",
          fontSize: "var(--text-2xs)",
          letterSpacing: "var(--tracking-widest)",
          color: "var(--text-disabled)",
          textTransform: "uppercase",
        }}
      >
        The nobody network.
      </p>
    </section>
  );
}
