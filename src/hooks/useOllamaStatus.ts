import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** Polls the real local Ollama health check (src-tauri/src/ollama_manager.rs)
 * so the UI shows whether the Shark Agent / AI matcher can actually respond,
 * instead of silently assuming it's ready because it auto-starts on boot. */
export function useOllamaStatus() {
    const [ready, setReady] = useState(false);

    useEffect(() => {
        const poll = () => {
            invoke<boolean>("get_ollama_status")
                .then(setReady)
                .catch(() => setReady(false));
        };

        poll();
        const interval = setInterval(poll, 5000);
        return () => clearInterval(interval);
    }, []);

    return ready;
}
