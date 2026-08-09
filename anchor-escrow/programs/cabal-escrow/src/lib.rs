use anchor_lang::prelude::*;
use ephemeral_rollups_sdk::anchor::{commit, delegate, ephemeral};
use ephemeral_rollups_sdk::cpi::DelegateConfig;
use ephemeral_rollups_sdk::ephem::MagicIntentBundleBuilder;

declare_id!("7ajNjyCeMYaPNDecgxDLt5NAJVoey39DKGhcjiVRQSuq");

pub const ESCROW_SEED: &[u8] = b"cabal-escrow";

/// Status of an escrow deal, mirroring the original Escrow.sol enum.
#[account]
#[derive(Default)]
pub struct Escrow {
    pub depositor: Pubkey,
    pub payee: Pubkey,
    /// Amount locked, in lamports (1 SOL = 1e9 lamports).
    pub amount: u64,
    /// Unix timestamp in seconds; 0 means no expiry.
    pub expiry: i64,
    /// 0 = active, 1 = released, 2 = refunded.
    pub status: u8,
}

impl Escrow {
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 1;
}

#[ephemeral]
#[program]
pub mod cabal_escrow {
    use super::*;

    /// Creates a new escrow deal: the depositor (signer) locks `amount`
    /// lamports into a fresh PDA that both parties can then transact on via
    /// the Ephemeral Rollup. The PDA is funded with rent + `amount`.
    pub fn initialize_escrow(
        ctx: Context<InitializeEscrow>,
        payee: Pubkey,
        amount: u64,
        expiry: i64,
    ) -> Result<()> {
        require!(amount > 0, EscrowError::NoFunds);
        require!(payee != Pubkey::default(), EscrowError::InvalidPayee);
        require!(
            expiry == 0 || expiry > Clock::get()?.unix_timestamp,
            EscrowError::InvalidExpiry
        );

        // Transfer the locked lamports into the PDA (rent comes from the
        // `init` constraint; this is the actual escrowed value). Done before
        // the mutable borrow below so the two account accesses don't overlap.
        {
            let escrow_info = ctx.accounts.escrow.to_account_info();
            let depositor_info = ctx.accounts.depositor.to_account_info();
            anchor_lang::system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    anchor_lang::system_program::Transfer {
                        from: depositor_info,
                        to: escrow_info,
                    },
                ),
                amount,
            )?;
        }

        let escrow = &mut ctx.accounts.escrow;
        escrow.depositor = ctx.accounts.depositor.key();
        escrow.payee = payee;
        escrow.amount = amount;
        escrow.expiry = expiry;
        escrow.status = 0; // Active

        msg!("Escrow {} created: {} -> {} ({} lamports)", escrow.key(), ctx.accounts.depositor.key(), payee, amount);
        Ok(())
    }

    /// Releases the escrowed lamports to the payee. Callable by the depositor
    /// while active. This is the instruction both the base layer and the ER
    /// can execute once the escrow PDA is delegated.
    pub fn release(ctx: Context<ReleaseEscrow>) -> Result<()> {
        let escrow = &ctx.accounts.escrow;
        require!(escrow.status == 0, EscrowError::NotActive);
        require!(
            escrow.depositor == ctx.accounts.caller.key(),
            EscrowError::OnlyDepositor
        );

        let amount = escrow.amount;
        let key = escrow.key();
        drop(escrow);

        let escrow = &mut ctx.accounts.escrow;
        escrow.status = 1; // Released

        let escrow_info = ctx.accounts.escrow.to_account_info();
        let payee_info = ctx.accounts.payee.to_account_info();
        **escrow_info.try_borrow_mut_lamports()? -= amount;
        **payee_info.try_borrow_mut_lamports()? += amount;

        msg!("Escrow {} released: {} lamports to {}", key, amount, ctx.accounts.payee.key());
        Ok(())
    }

    /// Marks an escrow released inside the ER. Payout is finalized on the
    /// base layer by `settle` because a wallet payee is not delegated and
    /// Magic Router rejects mixed ER/base writable accounts.
    pub fn release_er(ctx: Context<ReleaseErEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == 0, EscrowError::NotActive);
        require!(escrow.depositor == ctx.accounts.caller.key(), EscrowError::OnlyDepositor);
        escrow.status = 1;
        msg!("Escrow {} released in ER; awaiting base settlement", escrow.key());
        Ok(())
    }

    /// Pays the wallet after the ER release state has been committed back to
    /// Solana. This keeps the ER transaction single-environment.
    pub fn settle(ctx: Context<ReleaseEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == 1, EscrowError::NotReleased);
        require!(escrow.amount > 0, EscrowError::NothingToSettle);
        require!(escrow.depositor == ctx.accounts.caller.key(), EscrowError::OnlyDepositor);

        let amount = escrow.amount;
        escrow.amount = 0;
        let escrow_info = escrow.to_account_info();
        let payee_info = ctx.accounts.payee.to_account_info();
        **escrow_info.try_borrow_mut_lamports()? -= amount;
        **payee_info.try_borrow_mut_lamports()? += amount;
        msg!("Escrow {} settled: {} lamports to {}", escrow.key(), amount, ctx.accounts.payee.key());
        Ok(())
    }

    /// Refunds the escrowed lamports to the depositor. Callable by the
    /// depositor anytime, or by anyone after expiry.
    pub fn refund(ctx: Context<RefundEscrow>) -> Result<()> {
        let escrow = &ctx.accounts.escrow;
        require!(escrow.status == 0, EscrowError::NotActive);

        let expired = escrow.expiry != 0
            && Clock::get()?.unix_timestamp >= escrow.expiry;
        require!(
            escrow.depositor == ctx.accounts.caller.key() || expired,
            EscrowError::NotAuthorized
        );

        let amount = escrow.amount;
        let key = escrow.key();
        drop(escrow);

        let escrow = &mut ctx.accounts.escrow;
        escrow.status = 2; // Refunded

        let escrow_info = ctx.accounts.escrow.to_account_info();
        let depositor_info = ctx.accounts.depositor.to_account_info();
        **escrow_info.try_borrow_mut_lamports()? -= amount;
        **depositor_info.try_borrow_mut_lamports()? += amount;

        msg!("Escrow {} refunded: {} lamports to {}", key, amount, ctx.accounts.depositor.key());
        Ok(())
    }

    /// Delegates the escrow PDA to the MagicBlock Ephemeral Rollup so the
    /// deal can be released/refunded with real-time, zero-fee latency.
    /// A specific ER validator can be pinned via remaining accounts.
    pub fn delegate(ctx: Context<DelegateInput>) -> Result<()> {
        // The escrow PDA is derived from both the static seed and the
        // depositor key. Pass the complete seed tuple so the delegation CPI
        // can sign for the PDA without a privilege-escalation failure.
        let depositor = ctx.accounts.payer.key();
        ctx.accounts.delegate_pda(
            &ctx.accounts.payer,
            &[ESCROW_SEED, depositor.as_ref()],
            DelegateConfig {
                // Optionally set a specific validator from the first remaining account
                validator: ctx.remaining_accounts.first().map(|acc| acc.key()),
                ..Default::default()
            },
        )?;
        Ok(())
    }

    /// Manually commits the escrow PDA state from the ER back to the base
    /// layer.
    pub fn commit(ctx: Context<CommitEscrow>) -> Result<()> {
        MagicIntentBundleBuilder::new(
            ctx.accounts.payer.to_account_info(),
            ctx.accounts.magic_context.to_account_info(),
            ctx.accounts.magic_program.to_account_info(),
        )
        .commit(&[ctx.accounts.escrow.to_account_info()])
        .build_and_invoke()?;
        Ok(())
    }

    /// Undelegates the escrow PDA: commits the latest state and returns
    /// ownership to this program on the base layer.
    pub fn undelegate(ctx: Context<CommitEscrow>) -> Result<()> {
        MagicIntentBundleBuilder::new(
            ctx.accounts.payer.to_account_info(),
            ctx.accounts.magic_context.to_account_info(),
            ctx.accounts.magic_program.to_account_info(),
        )
        .commit_and_undelegate(&[ctx.accounts.escrow.to_account_info()])
        .build_and_invoke()?;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeEscrow<'info> {
    #[account(
        init,
        payer = depositor,
        space = Escrow::LEN,
        seeds = [ESCROW_SEED, depositor.key().as_ref()],
        bump
    )]
    pub escrow: Account<'info, Escrow>,
    #[account(mut)]
    pub depositor: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.depositor.as_ref()],
        bump
    )]
    pub escrow: Account<'info, Escrow>,
    /// The caller — must be the depositor.
    pub caller: Signer<'info>,
    /// CHECK: The payee receiving the released funds. Its lamports are credited
    /// by the program; no account data is read or written through the type.
    #[account(mut)]
    pub payee: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct ReleaseErEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.depositor.as_ref()],
        bump
    )]
    pub escrow: Account<'info, Escrow>,
    pub caller: Signer<'info>,
}

#[derive(Accounts)]
pub struct RefundEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.depositor.as_ref()],
        bump
    )]
    pub escrow: Account<'info, Escrow>,
    /// The caller — depositor, or anyone once expired.
    #[account(mut)]
    pub caller: Signer<'info>,
    /// CHECK: The depositor receiving the refund. Its lamports are credited by
    /// the program; no account data is read or written through the type.
    #[account(mut)]
    pub depositor: AccountInfo<'info>,
}

/// Delegation context — the `#[delegate]` macro injects the delegation
/// accounts and the `delegate_pda` helper.
#[delegate]
#[derive(Accounts)]
pub struct DelegateInput<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: The pda to delegate (ownership moves to the delegation program).
    #[account(mut, del)]
    pub pda: AccountInfo<'info>,
}

/// Commit / undelegate context — the `#[commit]` macro injects the
/// `magic_context` and `magic_program` accounts.
#[commit]
#[derive(Accounts)]
pub struct CommitEscrow<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.depositor.as_ref()],
        bump
    )]
    pub escrow: Account<'info, Escrow>,
}

#[error_code]
pub enum EscrowError {
    #[msg("No funds sent")]
    NoFunds,
    #[msg("Invalid payee")]
    InvalidPayee,
    #[msg("Invalid expiry")]
    InvalidExpiry,
    #[msg("Escrow is not active")]
    NotActive,
    #[msg("Only the depositor can release")]
    OnlyDepositor,
    #[msg("Not authorized or not expired")]
    NotAuthorized,
    #[msg("Escrow has not been released in the ER")]
    NotReleased,
    #[msg("Escrow has already been settled")]
    NothingToSettle,
}
