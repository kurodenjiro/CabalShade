/** Transferable SPL boost NFT shown by the mobile vault. */
export type BoostNft = {
  mint: string;
  name: string;
  boostBps: number;
  expiresAt: number;
  owned: boolean;
  listed: boolean;
  priceLamports: string | null;
  seller?: string | null;
};

export function boostExpired(boost: BoostNft, now = Date.now()): boolean {
  return boost.expiresAt * 1000 <= now;
}

export function boostLabel(boost: BoostNft, now = Date.now()): string {
  if (boostExpired(boost, now)) return "EXPIRED — NO BOOST";
  const remaining = Math.max(0, boost.expiresAt * 1000 - now);
  const hours = Math.floor(remaining / 3_600_000);
  return `+${(boost.boostBps / 100).toFixed(2)}% · ${hours}H LEFT`;
}
