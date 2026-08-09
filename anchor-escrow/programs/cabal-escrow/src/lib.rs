use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount, Transfer};
use ephemeral_rollups_sdk::anchor::{commit, delegate, ephemeral};
use ephemeral_rollups_sdk::cpi::DelegateConfig;
use ephemeral_rollups_sdk::ephem::MagicIntentBundleBuilder;

declare_id!("7ajNjyCeMYaPNDecgxDLt5NAJVoey39DKGhcjiVRQSuq");

pub const ESCROW_SEED: &[u8] = b"cabal-escrow";
pub const BOOST_SEED: &[u8] = b"cabal-boost";
pub const LISTING_SEED: &[u8] = b"cabal-boost-listing";

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

#[account]
pub struct Boost {
    pub mint: Pubkey,
    pub issuer: Pubkey,
    pub boost_bps: u16,
    pub expires_at: i64,
    pub used: bool,
}
impl Boost { pub const LEN: usize = 8 + 32 + 32 + 2 + 8 + 1; }

#[account]
pub struct BoostListing {
    pub seller: Pubkey,
    pub mint: Pubkey,
    pub price_lamports: u64,
    pub expiry: i64,
}
impl BoostListing { pub const LEN: usize = 8 + 32 + 32 + 8 + 8; }

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

    pub fn register_boost(ctx: Context<RegisterBoost>, boost_bps: u16, expires_at: i64) -> Result<()> {
        require!(boost_bps > 0, EscrowError::InvalidBoost);
        require!(expires_at > Clock::get()?.unix_timestamp, EscrowError::InvalidExpiry);
        require!(ctx.accounts.mint.decimals == 0, EscrowError::InvalidNftMint);
        let boost = &mut ctx.accounts.boost;
        boost.mint = ctx.accounts.mint.key();
        boost.issuer = ctx.accounts.issuer.key();
        boost.boost_bps = boost_bps;
        boost.expires_at = expires_at;
        boost.used = false;
        Ok(())
    }

    pub fn use_boost(ctx: Context<UseBoost>) -> Result<()> {
        let boost = &mut ctx.accounts.boost;
        require!(!boost.used, EscrowError::BoostAlreadyUsed);
        require!(Clock::get()?.unix_timestamp < boost.expires_at, EscrowError::BoostExpired);
        require!(ctx.accounts.user_tokens.amount >= 1, EscrowError::MissingBoostNft);
        token::burn(CpiContext::new(ctx.accounts.token_program.to_account_info(), Burn {
            mint: ctx.accounts.mint.to_account_info(), from: ctx.accounts.user_tokens.to_account_info(), authority: ctx.accounts.user.to_account_info(),
        }), 1)?;
        boost.used = true;
        Ok(())
    }

    pub fn list_boost(ctx: Context<ListBoost>, price_lamports: u64) -> Result<()> {
        require!(price_lamports > 0, EscrowError::InvalidPrice);
        require!(ctx.accounts.user_tokens.amount == 1, EscrowError::MissingBoostNft);
        token::transfer(CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
            from: ctx.accounts.user_tokens.to_account_info(), to: ctx.accounts.vault_tokens.to_account_info(), authority: ctx.accounts.seller.to_account_info(),
        }), 1)?;
        let listing = &mut ctx.accounts.listing;
        listing.seller = ctx.accounts.seller.key();
        listing.mint = ctx.accounts.mint.key();
        listing.price_lamports = price_lamports;
        listing.expiry = ctx.accounts.boost.expires_at;
        Ok(())
    }

    pub fn buy_boost(ctx: Context<BuyBoost>) -> Result<()> {
        require!(ctx.accounts.listing.price_lamports > 0, EscrowError::InvalidPrice);
        require!(Clock::get()?.unix_timestamp < ctx.accounts.listing.expiry, EscrowError::BoostExpired);
        anchor_lang::system_program::transfer(CpiContext::new(ctx.accounts.system_program.to_account_info(), anchor_lang::system_program::Transfer {
            from: ctx.accounts.buyer.to_account_info(), to: ctx.accounts.seller.to_account_info(),
        }), ctx.accounts.listing.price_lamports)?;
        let seller_key = ctx.accounts.listing.seller;
        let mint_key = ctx.accounts.mint.key();
        let bump = ctx.bumps.listing;
        let seeds: &[&[u8]] = &[LISTING_SEED, seller_key.as_ref(), mint_key.as_ref(), &[bump]];
        token::transfer(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
            from: ctx.accounts.vault_tokens.to_account_info(), to: ctx.accounts.buyer_tokens.to_account_info(), authority: ctx.accounts.listing.to_account_info(),
        }, &[seeds]), 1)?;
        Ok(())
    }

}

#[derive(Accounts)]
pub struct RegisterBoost<'info> {
    #[account(init, payer = issuer, space = Boost::LEN, seeds = [BOOST_SEED, mint.key().as_ref()], bump)] pub boost: Account<'info, Boost>,
    pub mint: Account<'info, Mint>,
    #[account(mut)] pub issuer: Signer<'info>, pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct UseBoost<'info> {
    #[account(mut, seeds = [BOOST_SEED, mint.key().as_ref()], bump)] pub boost: Account<'info, Boost>,
    #[account(mut, address = boost.mint)] pub mint: Account<'info, Mint>,
    #[account(mut, constraint = user_tokens.mint == mint.key(), constraint = user_tokens.owner == user.key())] pub user_tokens: Account<'info, TokenAccount>,
    pub user: Signer<'info>, pub token_program: Program<'info, Token>,
}
#[derive(Accounts)]
pub struct ListBoost<'info> {
    #[account(mut, seeds = [BOOST_SEED, mint.key().as_ref()], bump)] pub boost: Account<'info, Boost>,
    #[account(mut, address = boost.mint)] pub mint: Account<'info, Mint>,
    #[account(init, payer = seller, space = BoostListing::LEN, seeds = [LISTING_SEED, seller.key().as_ref(), mint.key().as_ref()], bump)] pub listing: Account<'info, BoostListing>,
    #[account(mut, constraint = user_tokens.mint == mint.key(), constraint = user_tokens.owner == seller.key())] pub user_tokens: Account<'info, TokenAccount>,
    #[account(init, payer = seller, associated_token::mint = mint, associated_token::authority = listing)] pub vault_tokens: Account<'info, TokenAccount>,
    #[account(mut)] pub seller: Signer<'info>, pub token_program: Program<'info, Token>, pub associated_token_program: Program<'info, AssociatedToken>, pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct BuyBoost<'info> {
    #[account(mut, close = buyer, seeds = [LISTING_SEED, listing.seller.as_ref(), mint.key().as_ref()], bump)] pub listing: Account<'info, BoostListing>,
    #[account(address = listing.mint)] pub mint: Account<'info, Mint>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = listing)] pub vault_tokens: Account<'info, TokenAccount>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = buyer)] pub buyer_tokens: Account<'info, TokenAccount>,
    /// CHECK: the listing records the seller payee.
    #[account(mut, address = listing.seller)] pub seller: AccountInfo<'info>,
    #[account(mut)] pub buyer: Signer<'info>, pub token_program: Program<'info, Token>, pub system_program: Program<'info, System>,
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
    #[msg("Boost amount is invalid")]
    InvalidBoost,
    #[msg("Boost NFT mint must use zero decimals")]
    InvalidNftMint,
    #[msg("Boost has already been used")]
    BoostAlreadyUsed,
    #[msg("Boost has expired")]
    BoostExpired,
    #[msg("User does not own the boost NFT")]
    MissingBoostNft,
    #[msg("Listing price must be greater than zero")]
    InvalidPrice,
}
