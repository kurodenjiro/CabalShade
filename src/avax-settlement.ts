/**
 * Read-only, frontend-side view of Avalanche C-Chain state. All signing
 * (identity generation, escrow, import/export) happens in the Rust backend,
 * which is the only place the wallet's private key lives.
 */

/** One native AVAX has 1e18 wei, the EVM standard. */
export const WEI_PER_AVAX = 1_000_000_000_000_000_000;

/** Formats a wei amount as an AVAX decimal string. */
export function formatWeiToAvax(wei: string | number): string {
  const weiNum = typeof wei === "string" ? parseFloat(wei) : wei;
  return (weiNum / WEI_PER_AVAX).toFixed(18);
}

/** Converts a wei count to an AVAX number. */
export function weiToAvax(wei: number): number {
  return wei / WEI_PER_AVAX;
}

/** Formats an AVAX number with up to 5 decimals for display. */
export function formatAvax(avax: number): string {
  return avax.toFixed(5);
}
