/**
 * Mobile entry point.
 *
 * Imports nothing from the frozen desktop tree: Tailwind and framer-motion live
 * there, and both are hostile to the design system — Tailwind compiles to the
 * raw px and hex values the adherence lint bans, and the brand forbids spring
 * physics outright.
 */
import React, { useState } from "react";
import ReactDOM from "react-dom/client";
import "../ds/index";
import { AppShell } from "../shell/AppShell";
import { useLogStream } from "../state/useLogStream";
import { Splash } from "../screens/Splash";
import { Home } from "../screens/Home";
import { Nodes } from "../screens/Nodes";
import { Intents } from "../screens/Intents";
import { Vault } from "../screens/Vault";
import { Profile } from "../screens/Profile";
import { New } from "../screens/New";
import { Detail } from "../screens/Detail";
import { Settled } from "../screens/Settled";
import type { Screen } from "../shell/screen";

function App() {
  // Starts at splash; entering the mesh goes directly to Home while the
  // handshake continues in the background.
  const [screen, setScreen] = useState<Screen>({ name: "splash" });

  // Keep the mesh handshake alive in the background. The user lands on Home
  // immediately; the status card and log reflect readiness as it changes.
  const isTauriRuntime = Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
  useLogStream("enter_mesh", {}, () => undefined, screen.name === "home" && isTauriRuntime);

  if (screen.name === "splash") {
    return <Splash onEnter={() => setScreen({ name: "home" })} />;
  }

  return (
    <AppShell screen={screen} onNavigate={setScreen}>
      {screen.name === "home" ? (
        <Home />
      ) : screen.name === "nodes" ? (
        <Nodes />
      ) : screen.name === "intents" ? (
        <Intents
          tab={screen.tab}
          onTabChange={(tab) => setScreen({ name: "intents", tab })}
          onCompose={() => setScreen({ name: "new" })}
        />
      ) : screen.name === "vault" ? (
        <Vault tab={screen.tab} onTabChange={(tab) => setScreen({ name: "vault", tab })} />
      ) : screen.name === "profile" ? (
        <Profile onLeave={() => setScreen({ name: "splash" })} />
      ) : screen.name === "new" ? (
        <New onBroadcast={(id) => setScreen({ name: "detail", id })} />
      ) : screen.name === "detail" ? (
        <Detail id={screen.id} onSettled={() => setScreen({ name: "settled", id: screen.id })} onBack={() => setScreen({ name: "intents", tab: "ACTIVE" })} />
      ) : screen.name === "settled" ? (
        <Settled id={screen.id} />
      ) : null}
    </AppShell>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
