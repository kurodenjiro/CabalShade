import React from "react";
import { motion } from "framer-motion";

interface OnboardingHintProps {
    visible: boolean;
    onDismiss: () => void;
}

/** A one-time hint pointing at the intent composer for first-time users —
 * dismissed permanently (persisted in localStorage) once seen or acted on. */
export const OnboardingHint: React.FC<OnboardingHintProps> = ({ visible, onDismiss }) => {
    if (!visible) return null;

    return (
        <motion.div
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 8 }}
            className="absolute bottom-[132px] left-6 w-[380px] z-30 bg-nobody-gold-soft border border-nobody-gold pixel-corners-sm p-3 shadow-card-lg"
        >
            <div className="flex items-start gap-2">
                <span className="text-lg shrink-0">👋</span>
                <div className="text-xs text-slate-700 leading-relaxed">
                    Type what you're looking for below, set a max AVAX you're willing to pay, then press Enter.
                    The local AI will scan Arsenal listings for a match and lock funds automatically if it finds one.
                </div>
            </div>
            <button
                onClick={onDismiss}
                className="mt-2 text-[11px] font-semibold text-nobody-gold hover:text-nobody-primary transition-colors"
            >
                Got it
            </button>
        </motion.div>
    );
};
