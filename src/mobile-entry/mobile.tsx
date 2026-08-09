/**
 * Mobile entry point.
 *
 * Imports nothing from the frozen desktop tree: Tailwind and framer-motion live
 * there, and both are hostile to the design system — Tailwind compiles to the
 * raw px and hex values the adherence lint bans, and the brand forbids spring
 * physics outright.
 */
import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "../ds/index";
import { AppShell } from "../shell/AppShell";
import { Splash } from "../screens/Splash";
import { useLogStream } from "../state/useLogStream";
import { Home } from "../screens/Home";
import { Nodes } from "../screens/Nodes";
import { Intents } from "../screens/Intents";
import { Vault } from "../screens/Vault";
import { Profile } from "../screens/Profile";
import { New } from "../screens/New";
import { Detail } from "../screens/Detail";
import { Settled } from "../screens/Settled";
import { BoostMarketplace } from "../components/BoostMarketplace";
import type { Screen } from "../shell/screen";
import type { IntentDetail } from "../types/bindings";

function App() {
  // No mesh UI is rendered until an explicit Enter action succeeds.
  const [screen, setScreen] = useState<Screen>({ name: "splash" });
  const [meshRequested, setMeshRequested] = useState(false);
  const [settlementNotice, setSettlementNotice] = useState<string | null>(null);

  // Join in the background. Home reads the live mesh snapshot, so users see
  // CONNECTING / ONLINE there without being held on a separate loading page.
  useLogStream("enter_mesh", {}, () => undefined, meshRequested);

  useEffect(() => {
    let timer: number | undefined;
    let unlisten: (() => void) | undefined;
    let stopExchange: (() => void) | undefined;
    listen<{ id?: string }>("intent-updated", (event) => {
      const id = event.payload?.id;
      if (!id) return;
      invoke<IntentDetail>("get_intent", { id }).then((detail) => {
        const status = detail.view.status.status;
        const text = status === "NEGOTIATING"
          ? detail.view.status.best
            ? `AGENT-0X123..2413 DEAL · ${(Number(detail.view.status.best.cents) / 100).toFixed(2)} USDC`
            : `MATCH FOUND · NEGOTIATING TERMS`
          : status === "FINDING_ROUTE"
            ? `DEAL ACCEPTED · SETTLING ${detail.view.title}`
          : status === "SETTLED"
          ? `AUTO SETTLED · ${detail.view.title}`
          : status === "WAITING"
            ? `AUTO SETTLE QUEUED FOR RELAY · ${detail.view.title}`
            : status === "FAILED"
              ? `AUTO SETTLE FAILED · ${detail.view.title}`
              : null;
        if (!text) return;
        setSettlementNotice(text);
        if (timer) window.clearTimeout(timer);
        timer = window.setTimeout(() => setSettlementNotice(null), 6_000);
      }).catch(() => undefined);
    }).then((stop) => { unlisten = stop; }).catch(() => undefined);
    listen<{ text?: string }>("agent-exchange", (event) => {
      if (!event.payload?.text) return;
      setSettlementNotice(event.payload.text);
      if (timer) window.clearTimeout(timer);
      timer = window.setTimeout(() => setSettlementNotice(null), 6_000);
    }).then((stop) => { stopExchange = stop; }).catch(() => undefined);
    return () => {
      if (timer) window.clearTimeout(timer);
      unlisten?.();
      stopExchange?.();
    };
  }, []);

  if (screen.name === "splash") {
    return <Splash onEnter={() => {
      setMeshRequested(true);
      setScreen({ name: "home" });
    }} />;
  }

  return (
    <AppShell screen={screen} onNavigate={setScreen}>
      <>
      {screen.name === "home" ? (
        <Home />
      ) : screen.name === "nodes" ? (
        <Nodes />
      ) : screen.name === "intents" ? (
        <Intents
          tab={screen.tab}
          onTabChange={(tab) => setScreen({ name: "intents", tab })}
          onCompose={() => setScreen({ name: "new" })}
          onOpen={(id) => setScreen({ name: "detail", id })}
        />
      ) : screen.name === "vault" ? (
        <>
          {/* The ASSETS tab bar and balance rows are Vault's own section —
              the boost mesh market is a secondary panel bolted onto that tab,
              so it renders after Vault rather than pushing it down. */}
          <Vault tab={screen.tab} onTabChange={(tab) => setScreen({ name: "vault", tab })} />
          {screen.tab === "ASSETS" && <BoostMarketplace />}
        </>
      ) : screen.name === "profile" ? (
        <Profile />
      ) : screen.name === "new" ? (
        <New onBroadcast={(id) => setScreen({ name: "detail", id })} onOpenMarketplace={() => setScreen({ name: "vault", tab: "ASSETS" })} />
      ) : screen.name === "detail" ? (
        <Detail id={screen.id} onSettled={() => setScreen({ name: "settled", id: screen.id })} onBack={() => setScreen({ name: "intents", tab: "ACTIVE" })} />
      ) : screen.name === "settled" ? (
        <Settled id={screen.id} />
      ) : null}
      {settlementNotice ? (
        <div role="status" aria-live="assertive" style={{ position: "fixed", left: "var(--space-6)", right: "var(--space-6)", bottom: "calc(var(--safe-bottom) + 64px)", zIndex: "var(--z-nav)", padding: "var(--space-5)", border: "var(--border-width-thin) solid var(--accent-cyan)", background: "var(--surface-raised)", color: "var(--text-primary)", fontFamily: "var(--type-label-family)", fontSize: "var(--text-2xs)", letterSpacing: "var(--tracking-widest)", boxShadow: "0 0 24px rgba(0, 255, 255, .18)" }}>
          {settlementNotice}
        </div>
      ) : null}
      </>
    </AppShell>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
