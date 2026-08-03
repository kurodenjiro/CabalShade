/** Deterministic (never random/fake) helpers that turn already-real data —
 * a real price, a real wallet address — into game-flavored presentation.
 * Nothing here invents data; it only picks how to *display* data that's
 * already fetched from the chain/mesh elsewhere. */

export type Rarity = "Common" | "Uncommon" | "Rare" | "Epic";

export interface RarityInfo {
    tier: Rarity;
    textClass: string;
    borderClass: string;
}

/** Maps a real listing/voucher price to a rarity tier — purely cosmetic
 * banding over a real number, same price always yields the same tier. */
export function priceRarity(priceAvax: number): RarityInfo {
    if (priceAvax >= 1) {
        return { tier: "Epic", textClass: "text-nobody-accent", borderClass: "border-nobody-accent" };
    }
    if (priceAvax >= 0.1) {
        return { tier: "Rare", textClass: "text-nobody-gold", borderClass: "border-nobody-gold" };
    }
    if (priceAvax >= 0.01) {
        return { tier: "Uncommon", textClass: "text-nobody-primary", borderClass: "border-nobody-primary" };
    }
    return { tier: "Common", textClass: "text-slate-500", borderClass: "border-slate-300" };
}

export type CharacterClass = "knight" | "scout" | "mystic" | "forger";

const CLASSES: CharacterClass[] = ["knight", "scout", "mystic", "forger"];

/** Deterministically derives a "class" from a real wallet address — the same
 * address always renders the same class, purely a cosmetic identity cue
 * (not a claim about the person/agent behind it). */
export function addressClass(address: string): CharacterClass {
    let sum = 0;
    for (let i = 0; i < address.length; i++) sum += address.charCodeAt(i);
    return CLASSES[sum % CLASSES.length];
}

export const CLASS_LABEL: Record<CharacterClass, string> = {
    knight: "Knight",
    scout: "Scout",
    mystic: "Mystic",
    forger: "Forger",
};

export const CLASS_COLOR_CLASS: Record<CharacterClass, string> = {
    knight: "text-nobody-primary",
    scout: "text-nobody-gold",
    mystic: "text-nobody-accent",
    forger: "text-slate-600",
};
