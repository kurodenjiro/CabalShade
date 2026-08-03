import React, { useState, useEffect } from "react";
import { motion } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";

interface DelegationCenterProps {
    visible: boolean;
    onComplete: () => void; // Called after successful delegation
    onCancel: () => void;
}

interface IdentityView {
    alias: string;
    emoji: string;
    address: string;
}

export const DelegationCenter: React.FC<DelegationCenterProps> = ({ visible, onComplete, onCancel }) => {
    const [step, setStep] = useState<"idle" | "signing">("idle");
    const [progress, setProgress] = useState(0);
    const [identity, setIdentity] = useState<IdentityView | null>(null);

    useEffect(() => {
        if (visible) {
            invoke<IdentityView[]>("get_identity")
                .then((ids) => setIdentity(ids?.[0] || null))
                .catch((e) => console.error("Failed to fetch identity:", e));
        }
    }, [visible]);

    const handleSignAndDelegate = async () => {
        setStep("signing");
        try {
            await invoke("enable_instant_session");
            setProgress(100);
            setTimeout(onComplete, 800); // Brief pause so the "Done" state is visible before closing
        } catch (e) {
            console.error("Delegation Failed", e);
            alert("Delegation Failed: " + e);
            setStep("idle");
            setProgress(0);
        }
    };

    if (!visible) return null;

    return (
        <motion.div
            className="absolute inset-0 z-[60] flex items-center justify-center bg-black/70 backdrop-blur-sm text-sm"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
        >
            <div className="w-[650px] pixel-notch-tr border border-nobody-gold/30 bg-nobody-charcoal shadow-card-lg relative flex flex-col overflow-hidden">

                {/* Header */}
                <div className="bg-slate-50 px-5 py-3 border-b border-slate-200 flex justify-between items-center text-xs">
                    <span className="text-nobody-primary font-pixel text-[10px] tracking-wide">DELEGATION CENTER</span>
                    <span className="text-slate-400 font-mono">
                        {identity ? `${identity.emoji || "👻"} ${identity.address.slice(0, 6)}...${identity.address.slice(-4)}` : "Loading..."}
                    </span>
                    <span className="text-nobody-primary font-medium">Engine: Instant Session</span>
                </div>

                <div className="p-8 space-y-6">

                    {/* Agent Authority Setup */}
                    <div className="space-y-2">
                        <div className="text-nobody-primary font-semibold text-xs tracking-wide">🧙 Recruit an Agent Companion</div>
                        <div className="text-slate-500 text-xs leading-relaxed border-l-2 border-slate-200 pl-3">
                            This lets the app negotiate and sign matched deals for you automatically, without asking each time.
                            You can always undo this from Wallet → Erase all local data, or by restarting the app.
                        </div>
                    </div>

                    <div className="border-t border-slate-100" />

                    {/* Real, current limits — no fabricated spending cap or protocol list */}
                    <div className="space-y-2">
                        <div className="bg-slate-50 pixel-corners-sm p-3 border border-slate-200 text-xs flex justify-between items-center text-slate-500">
                            <span>Session length</span>
                            <span className="text-slate-900 font-semibold">1 hour</span>
                        </div>
                        <div className="text-[11px] text-slate-400 leading-relaxed">
                            There's currently no separate spending cap on top of this — every real transaction (buying, releasing, refunding) still uses your same wallet and shows up in this app, matched deal by matched deal.
                        </div>
                    </div>

                    <div className="border-t border-slate-100" />

                    {/* Delegation Status */}
                    <div className="bg-slate-50 pixel-corners-sm p-4 border border-slate-200 space-y-2 relative overflow-hidden">
                        <div className="text-slate-400 font-semibold text-xs tracking-wide mb-2">⚙️ Status</div>

                        <div className="flex justify-between items-center text-xs text-slate-500">
                            <span>Agent session</span>
                            <span className={progress >= 100 ? "text-nobody-primary font-medium" : "text-slate-300"}>
                                {progress >= 100 ? "Active ✓" : step === "signing" ? "Signing..." : "Not started"}
                            </span>
                        </div>

                        <div className="mt-4 border-l-2 border-nobody-primary pl-3 py-1 text-xs text-slate-500">
                            <span className="text-nobody-primary font-semibold">Agent:</span> "Once authorized, I can match and lock funds for you even while you're away — you still confirm every release yourself."
                        </div>
                    </div>

                    {/* Actions */}
                    <div className="flex gap-4 pt-2">
                        <button
                            onClick={handleSignAndDelegate}
                            disabled={step === "signing"}
                            className={`flex-1 bg-nobody-primary text-nobody-ink font-semibold py-3 pixel-corners-sm hover:brightness-125 transition-colors ${step === "signing" ? "opacity-70 cursor-wait" : ""}`}
                        >
                            {step === "signing" ? "✍️ Signing..." : "✍️ Sign & Delegate"}
                        </button>
                        <button
                            onClick={onCancel}
                            disabled={step === "signing"}
                            className="px-6 pixel-corners-sm border border-slate-200 text-slate-500 hover:text-slate-900 hover:border-slate-300 transition-colors font-semibold disabled:opacity-50"
                        >
                            ← Cancel
                        </button>
                    </div>

                </div>
            </div>
        </motion.div>
    );

};
