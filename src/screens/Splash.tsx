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
 * Session status is observed in the background for diagnostics; ENTER THE MESH
 * remains available while Home reflects readiness as it arrives.
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
          /* Browser preview may not expose Tauri IPC; keep the splash actionable. */
        });
    };
    poll();
    const interval = window.setInterval(poll, 2_000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);


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
      <Logo variant="hero" size={288} basePath="/ds-assets/logo" />


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
        <Button tone="primary" size="lg" block className="cm-touch" onClick={onEnter}>
          ENTER THE MESH
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
