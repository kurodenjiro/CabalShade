import React, { useMemo } from "react";

const COLORS = ["bg-nobody-primary/40", "bg-nobody-gold/50", "bg-nobody-accent/30"];

/**
 * A handful of slow-rising pixel motes drifting up the screen — continuous
 * ambient motion so the app never reads as a static screenshot.
 */
export const AmbientParticles: React.FC = () => {
    const particles = useMemo(() => {
        return Array.from({ length: 14 }, (_, i) => ({
            id: i,
            left: Math.round(Math.random() * 100),
            size: Math.random() > 0.7 ? 3 : 2,
            delay: Math.random() * 6,
            duration: 5 + Math.random() * 5,
            color: COLORS[i % COLORS.length],
        }));
    }, []);

    return (
        <div className="fixed inset-0 pointer-events-none overflow-hidden z-0">
            {particles.map((p) => (
                <span
                    key={p.id}
                    className={`absolute bottom-0 animate-rise ${p.color}`}
                    style={{
                        left: `${p.left}%`,
                        width: p.size,
                        height: p.size,
                        animationDelay: `${p.delay}s`,
                        animationDuration: `${p.duration}s`,
                    }}
                />
            ))}
        </div>
    );
};

export default AmbientParticles;
