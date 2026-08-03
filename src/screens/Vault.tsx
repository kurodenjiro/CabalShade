import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, IconButton, Panel } from "../ds";
import type { VaultTab } from "../shell/screen";
import type { MnemonicExport, VaultRow } from "../types/bindings";

const TABS: VaultTab[] = ["ASSETS", "IDENTITIES", "KEYS"];

const COMMAND: Record<VaultTab, string> = {
  ASSETS: "vault_assets",
  IDENTITIES: "vault_identities",
  KEYS: "vault_keys",
};

/**
 * Assets, identities and key metadata.
 *
 * **The total is masked by default and only fetched on reveal.** Sending the
 * value and hiding it in CSS would put the balance in the DOM of a screen the
 * user asked not to show it on — masking is presentation, so the *value* has to
 * be absent, not merely covered. The reveal fetches the real total from
 * `vault_total`, which reads the encrypted snapshot.
 *
 * **The KEYS tab exports and imports via a BIP-39 mnemonic.** Export shows the
 * AI's story (a recall aid, never the secret), then the words on explicit
 * reveal. Import takes a mnemonic with AI-assisted fuzzy word suggestions.
 * The story is never stored or sent anywhere — it exists only in the response.
 */
export function Vault({ tab, onTabChange }: { tab: VaultTab; onTabChange: (tab: VaultTab) => void }) {
  const [rows, setRows] = useState<VaultRow[] | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [total, setTotal] = useState<string | null>(null);
  const [story, setStory] = useState<string | null>(null);
  const [exportBusy, setExportBusy] = useState(false);
  const [importing, setImporting] = useState(false);
  const [mnemonicInput, setMnemonicInput] = useState("");
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [importError, setImportError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<VaultRow[]>(COMMAND[tab])
      .then((next) => {
        if (!cancelled) setRows(next);
      })
      .catch(() => {
        if (!cancelled) setRows([]);
      });
    return () => {
      cancelled = true;
    };
  }, [tab]);

  const toggleReveal = () => {
    const next = !revealed;
    setRevealed(next);
    // Only fetch the value on reveal — it never enters the DOM while masked.
    if (next && total === null) {
      invoke<string>("vault_total")
        .then((value) => setTotal(value))
        .catch(() => setTotal("—"));
    }
  };

  const exportWallet = async () => {
    if (exportBusy) return;
    setExportBusy(true);
    setStory(null);
    try {
      const result = await invoke<MnemonicExport>("export_mnemonic");
      setStory(result.story);
    } catch {
      setStory("Could not generate a story. The words are the wallet.");
    } finally {
      setExportBusy(false);
    }
  };

  const onMnemonicInput = (value: string) => {
    setMnemonicInput(value);
    // Suggest for the current (possibly partial) word — the last token.
    const tokens = value.trim().split(/\s+/);
    const last = tokens[tokens.length - 1] ?? "";
    if (last.length >= 2 && tokens.length <= 12) {
      invoke<string[]>("suggest_mnemonic_word", { input: last })
        .then((next) => setSuggestions(next))
        .catch(() => setSuggestions([]));
    } else {
      setSuggestions([]);
    }
  };

  const applySuggestion = (word: string) => {
    const tokens = mnemonicInput.trim().split(/\s+/);
    tokens[tokens.length - 1] = word;
    setMnemonicInput(tokens.join(" "));
    setSuggestions([]);
  };

  const importWallet = async () => {
    if (importing) return;
    setImporting(true);
    setImportError(null);
    try {
      await invoke("import_mnemonic", { mnemonic: mnemonicInput.trim(), alias: "Imported Fox", emoji: "🦊" });
      setMnemonicInput("");
      setImporting(false);
      // Refresh the identity rows.
      invoke<VaultRow[]>("vault_identities")
        .then((next) => setRows(next))
        .catch(() => setRows([]));
    } catch (err) {
      setImportError(err instanceof Error ? err.message : "Invalid mnemonic.");
      setImporting(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-6)", padding: "var(--space-6)" }}>
      <div role="tablist" aria-label="Vault section" style={{ display: "flex", gap: "var(--space-7)" }}>
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

      {tab === "ASSETS" && (
        <Panel label="TOTAL VALUE (PRIVATE)">
          <div
            className="cm-row"
            style={{
              padding: "var(--space-6)",
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: "var(--space-5)",
            }}
          >
            <span
              style={{
                fontFamily: "var(--type-data-family)",
                fontSize: "var(--text-lg)",
                letterSpacing: "var(--type-data-tracking)",
                color: "var(--text-primary)",
              }}
            >
              {/* Not fetched unless revealed — see the note above. */}
              {revealed ? total ?? "—" : "✱✱✱✱✱"}
            </span>
            <IconButton
              size="md"
              tone="outline"
              className="cm-touch"
              aria-label={revealed ? "Hide total value" : "Reveal total value"}
              aria-pressed={revealed}
              onClick={toggleReveal}
            >
              {revealed ? "×" : "◎"}
            </IconButton>
          </div>
        </Panel>
      )}

      <Panel label={tab}>
        {rows === null ? null : rows.length === 0 ? (
          <Empty tab={tab} />
        ) : (
          rows.map((row) => <Row key={`${row.tag}-${row.name}`} row={row} />)
        )}
      </Panel>

      {tab === "KEYS" && (
        <>
          <Panel label="BACKUP (AI-ANCHORED)">
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-5)", padding: "var(--space-4)" }}>
              <span style={{ fontSize: "var(--text-base)", color: "var(--text-muted)" }}>
                The story helps you recall the order. The words are the wallet.
              </span>
              <Button tone="primary" size="md" className="cm-touch" disabled={exportBusy} onClick={exportWallet}>
                {exportBusy ? "WRITING STORY..." : "EXPORT RECOVERY PHRASE"}
              </Button>
              {story && (
                <div
                  style={{
                    border: "var(--border-hairline-style)",
                    padding: "var(--space-4)",
                    display: "flex",
                    flexDirection: "column",
                    gap: "var(--space-3)",
                  }}
                >
                  <span
                    style={{
                      fontFamily: "var(--type-label-family)",
                      fontSize: "var(--text-2xs)",
                      letterSpacing: "var(--tracking-widest)",
                      color: "var(--text-muted)",
                    }}
                  >
                    YOUR STORY
                  </span>
                  <span style={{ fontSize: "var(--text-base)", color: "var(--text-primary)" }}>{story}</span>
                  <Button tone="secondary" size="sm" className="cm-touch" onClick={() => invoke("copy_mnemonic").catch(() => undefined)}>
                    COPY THE WORDS
                  </Button>
                </div>
              )}
            </div>
          </Panel>

          <Panel label="IMPORT">
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)", padding: "var(--space-4)" }}>
              <textarea
                className="cm-input cm-touch"
                value={mnemonicInput}
                onChange={(e) => onMnemonicInput(e.target.value)}
                placeholder="Paste or type your 12 recovery words..."
                rows={3}
                style={{ width: "100%", fontFamily: "var(--type-data-family)", fontSize: "var(--text-xs)" }}
              />
              {suggestions.length > 0 && (
                <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
                  {suggestions.map((word) => (
                    <button
                      key={word}
                      type="button"
                      className="cm-touch"
                      onClick={() => applySuggestion(word)}
                      style={{
                        background: "var(--surface-raised)",
                        border: "var(--border-hairline-style)",
                        padding: "var(--space-2) var(--space-3)",
                        cursor: "pointer",
                        fontFamily: "var(--type-label-family)",
                        fontSize: "var(--text-2xs)",
                        color: "var(--text-secondary)",
                      }}
                    >
                      {word}
                    </button>
                  ))}
                </div>
              )}
              {importError && (
                <span style={{ fontSize: "var(--text-base)", color: "var(--text-alert)" }}>{importError}</span>
              )}
              <Button tone="primary" size="md" className="cm-touch" disabled={importing || !mnemonicInput.trim()} onClick={importWallet}>
                {importing ? "IMPORTING..." : "IMPORT WALLET"}
              </Button>
            </div>
          </Panel>
        </>
      )}
    </div>
  );
}

function Row({ row }: { row: VaultRow }) {
  return (
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
        aria-hidden="true"
        style={{
          fontFamily: "var(--type-label-family)",
          fontSize: "var(--text-2xs)",
          letterSpacing: "var(--tracking-wider)",
          color: "var(--text-muted)",
          border: "var(--border-hairline-style)",
          padding: "var(--space-2) var(--space-3)",
        }}
      >
        {row.tag}
      </span>

      <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
        <span
          style={{
            fontFamily: "var(--type-heading-family)",
            fontSize: "var(--text-sm)",
            letterSpacing: "var(--type-heading-tracking)",
            color: "var(--text-primary)",
          }}
        >
          {row.name}
        </span>
        {row.detail ? (
          <span
            style={{
              fontFamily: "var(--type-label-family)",
              fontSize: "var(--text-2xs)",
              letterSpacing: "var(--tracking-widest)",
              color: "var(--text-muted)",
            }}
          >
            {row.detail}
          </span>
        ) : null}
      </div>

      <span
        style={{
          fontFamily: "var(--type-data-family)",
          fontSize: "var(--text-sm)",
          letterSpacing: "var(--type-data-tracking)",
          color: "var(--text-secondary)",
        }}
      >
        {row.amount}
      </span>
    </div>
  );
}

function Empty({ tab }: { tab: VaultTab }) {
  const body =
    tab === "ASSETS"
      ? "Nothing is held. Nothing is stored."
      : tab === "IDENTITIES"
        ? "No identity exists yet."
        : "No key material is held.";

  return (
    <div style={{ padding: "var(--space-9) var(--space-6)", textAlign: "center" }}>
      <span style={{ fontSize: "var(--text-base)", color: "var(--text-muted)" }}>{body}</span>
    </div>
  );
}
