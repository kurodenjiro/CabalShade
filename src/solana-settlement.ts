import { Connection } from "@solana/web3.js";

/** Solana lamports per SOL. */
const LAMPORTS_PER_SOL = 1_000_000_000;

/** Solana devnet RPC for read-only balance lookups. */
const DEFAULT_RPC_URL = "https://api.devnet.solana.com";

/**
 * Read-only, frontend-side view of Solana chain state. All signing
 * (identity generation, escrow create/release/refund) happens in the Rust
 * backend, which is the only place the wallet's private key lives.
 */
export class SolanaSettlement {
    private connection: Connection;

    constructor(rpcUrl: string = DEFAULT_RPC_URL) {
        this.connection = new Connection(rpcUrl, "confirmed");
    }

    /** Formats a lamport string as a SOL number string. */
    static formatLamportsToSol(lamports: string | number): string {
        const lamportsNum =
            typeof lamports === "string" ? parseFloat(lamports) : lamports;
        return (lamportsNum / LAMPORTS_PER_SOL).toFixed(9);
    }

    /** Converts a lamport count to a SOL number. */
    static lamportsToSol(lamports: number): number {
        return lamports / LAMPORTS_PER_SOL;
    }

    async getBalancePrivately(address: string): Promise<string> {
        const balance = await this.connection.getBalance(
            new (await import("@solana/web3.js")).PublicKey(address),
        );
        return SolanaSettlement.formatLamportsToSol(balance);
    }

    async monitorTransaction(txHash: string) {
        return this.connection.getSignatureStatus(txHash);
    }
}

export default SolanaSettlement;
