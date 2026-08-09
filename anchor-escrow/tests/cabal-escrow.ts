import * as anchor from "@coral-xyz/anchor";
import { Program, web3 } from "@coral-xyz/anchor";
import { CabalEscrow } from "../target/types/cabal_escrow";
import { LAMPORTS_PER_SOL, sendAndConfirmTransaction } from "@solana/web3.js";
import {
  DELEGATION_PROGRAM_ID,
  ConnectionMagicRouter,
  delegateBufferPdaFromDelegatedAccountAndOwnerProgram,
  delegationMetadataPdaFromDelegatedAccount,
  delegationRecordPdaFromDelegatedAccount,
} from "@magicblock-labs/ephemeral-rollups-sdk";

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
    new ConnectionMagicRouter(
      process.env.EPHEMERAL_PROVIDER_ENDPOINT ||
        "https://devnet-router.magicblock.app/",
      {
        wsEndpoint:
          process.env.EPHEMERAL_WS_ENDPOINT || "wss://devnet-router.magicblock.app/",
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
  // Use a fresh depositor for every run so the deterministic escrow PDA is
  // never left over from an earlier devnet test.
  const depositor = anchor.web3.Keypair.generate();
  const payer = depositor.publicKey;
  // The provider wallet is an existing system account on both base and ER;
  // using it as payee avoids writing to a non-existent account on ER.
  const payee = anchor.Wallet.local().publicKey;

  console.log("Program ID: ", program.programId.toString());

  it("Initialize escrow on Solana (base layer)", async () => {
    let funded = false;
    for (let attempt = 0; attempt < 3 && !funded; attempt += 1) {
      try {
        const airdrop = await provider.connection.requestAirdrop(
          depositor.publicKey,
          300_000_000,
        );
        await provider.connection.confirmTransaction(airdrop, "confirmed");
        funded = true;
      } catch (error) {
        if (attempt === 2) throw error;
        await new Promise((resolve) => setTimeout(resolve, 1500));
      }
    }

    const [escrowPda] = web3.PublicKey.findProgramAddressSync(
      [Buffer.from(ESCROW_SEED), payer.toBuffer()],
      program.programId,
    );
    console.log("Escrow PDA: ", escrowPda.toString());

    let tx = await program.methods
      .initializeEscrow(payee, new anchor.BN(AMOUNT), new anchor.BN(0))
      .accounts({
        depositor: payer,
      } as any)
      .transaction();

    const txHash = await provider.sendAndConfirm(tx, [depositor], {
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
        bufferPda: delegateBufferPdaFromDelegatedAccountAndOwnerProgram(
          escrowPda,
          program.programId,
        ),
        delegationRecordPda:
          delegationRecordPdaFromDelegatedAccount(escrowPda),
        delegationMetadataPda:
          delegationMetadataPdaFromDelegatedAccount(escrowPda),
        pda: escrowPda,
        ownerProgram: program.programId,
        delegationProgram: DELEGATION_PROGRAM_ID,
        systemProgram: web3.SystemProgram.programId,
      } as any)
      .remainingAccounts(remainingAccounts)
      .transaction();

    const txHash = await provider.sendAndConfirm(tx, [depositor], {
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
        escrow: escrowPda,
        caller: payer,
        payee,
      } as any)
      .transaction();
    tx.feePayer = providerEphemeralRollup.wallet.publicKey;
    const txHash = await sendAndConfirmTransaction(
      providerEphemeralRollup.connection,
      tx,
      [depositor, providerEphemeralRollup.wallet.payer],
      { skipPreflight: true, commitment: "confirmed" },
    );
    console.log(`(ER) Release txHash: ${txHash}`);

    const payeeBalanceAfter = await providerEphemeralRollup.connection.getBalance(
      payee,
    );
    const diff = payeeBalanceAfter - payeeBalanceBefore;
    console.log(`(ER) Payee received: ${diff / LAMPORTS_PER_SOL} SOL`);
  });
});
