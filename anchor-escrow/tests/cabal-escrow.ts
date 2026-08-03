import * as anchor from "@coral-xyz/anchor";
import { Program, web3 } from "@coral-xyz/anchor";
import { CabalEscrow } from "../target/types/cabal_escrow";
import { LAMPORTS_PER_SOL } from "@solana/web3.js";
import { GetCommitmentSignature } from "@magicblock-labs/ephemeral-rollups-sdk";

const ESCROW_SEED = "cabal-escrow";
const AMOUNT = 100_000_000; // 0.1 SOL in lamports

describe("cabal-escrow", () => {
  console.log("cabal-escrow.ts");

  const provider = new anchor.AnchorProvider(
    new anchor.web3.Connection(
      process.env.PROVIDER_ENDPOINT || "https://api.devnet.solana.com",
      {
        wsEndpoint: process.env.WS_ENDPOINT || undefined,
        commitment: "confirmed",
      },
    ),
    anchor.Wallet.local(),
  );
  anchor.setProvider(provider);

  const providerEphemeralRollup = new anchor.AnchorProvider(
    new anchor.web3.Connection(
      process.env.EPHEMERAL_PROVIDER_ENDPOINT ||
        "https://devnet-as.magicblock.app/",
      {
        wsEndpoint:
          process.env.EPHEMERAL_WS_ENDPOINT || "wss://devnet-as.magicblock.app/",
        commitment: "confirmed",
      },
    ),
    anchor.Wallet.local(),
  );
  console.log("Base Layer Connection: ", provider.connection.rpcEndpoint);
  console.log(
    "Ephemeral Rollup Connection: ",
    providerEphemeralRollup.connection.rpcEndpoint,
  );
  console.log(`Current SOL Public Key: ${anchor.Wallet.local().publicKey}`);

  const program = anchor.workspace.CabalEscrow as Program<CabalEscrow>;
  const payer = anchor.Wallet.local().publicKey;
  const payee = anchor.web3.Keypair.generate().publicKey;

  console.log("Program ID: ", program.programId.toString());

  it("Initialize escrow on Solana (base layer)", async () => {
    const [escrowPda] = web3.PublicKey.findProgramAddressSync(
      [Buffer.from(ESCROW_SEED), payer.toBuffer()],
      program.programId,
    );
    console.log("Escrow PDA: ", escrowPda.toString());

    let tx = await program.methods
      .initializeEscrow(payee, new anchor.BN(AMOUNT), new anchor.BN(0))
      .accounts({
        depositor: payer,
      })
      .transaction();

    const txHash = await provider.sendAndConfirm(tx, [provider.wallet.payer], {
      skipPreflight: true,
      commitment: "confirmed",
    });
    console.log(`(Base Layer) Initialize txHash: ${txHash}`);

    const escrow = await program.account.escrow.fetch(escrowPda);
    console.log(
      `Escrow state: depositor=${escrow.depositor.toString()}, payee=${escrow.payee.toString()}, amount=${escrow.amount.toString()}, status=${escrow.status}`,
    );
  });

  it("Delegate escrow PDA to ER", async () => {
    const [escrowPda] = web3.PublicKey.findProgramAddressSync(
      [Buffer.from(ESCROW_SEED), payer.toBuffer()],
      program.programId,
    );

    const remainingAccounts =
      providerEphemeralRollup.connection.rpcEndpoint.includes("localhost") ||
      providerEphemeralRollup.connection.rpcEndpoint.includes("127.0.0.1")
        ? [
            {
              pubkey: new web3.PublicKey(
                "mAGicPQYBMvcYveUZA5F5UNNwyHvfYh5xkLS2Fr1mev",
              ),
              isSigner: false,
              isWritable: false,
            },
          ]
        : [
            {
              pubkey: new web3.PublicKey(
                "MAS1Dt9qreoRMQ14YQuhg8UTZMMzDdKhmkZMECCzk57",
              ),
              isSigner: false,
              isWritable: false,
            },
          ];

    let tx = await program.methods
      .delegate()
      .accounts({
        payer,
        pda: escrowPda,
      })
      .remainingAccounts(remainingAccounts)
      .transaction();

    const txHash = await provider.sendAndConfirm(tx, [provider.wallet.payer], {
      skipPreflight: true,
      commitment: "confirmed",
    });
    console.log(`(Base Layer) Delegate txHash: ${txHash}`);
    await new Promise((resolve) => setTimeout(resolve, 3000));
  });

  it("Release escrow on ER (real-time, zero-fee)", async () => {
    const [escrowPda] = web3.PublicKey.findProgramAddressSync(
      [Buffer.from(ESCROW_SEED), payer.toBuffer()],
      program.programId,
    );

    const payeeBalanceBefore = await providerEphemeralRollup.connection.getBalance(
      payee,
    );

    let tx = await program.methods
      .release()
      .accounts({
        caller: payer,
        payee,
      })
      .transaction();
    tx.feePayer = providerEphemeralRollup.wallet.publicKey;
    tx.recentBlockhash = (
      await providerEphemeralRollup.connection.getLatestBlockhash()
    ).blockhash;
    tx = await providerEphemeralRollup.wallet.signTransaction(tx);
    const txHash = await providerEphemeralRollup.sendAndConfirm(tx);
    console.log(`(ER) Release txHash: ${txHash}`);

    const payeeBalanceAfter = await providerEphemeralRollup.connection.getBalance(
      payee,
    );
    const diff = payeeBalanceAfter - payeeBalanceBefore;
    console.log(`(ER) Payee received: ${diff / LAMPORTS_PER_SOL} SOL`);
  });
});
