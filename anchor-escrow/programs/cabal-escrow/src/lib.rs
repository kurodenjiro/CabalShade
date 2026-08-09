use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Burn, CloseAccount, Mint, Token, TokenAccount, Transfer};
use ephemeral_rollups_sdk::anchor::{commit, delegate, ephemeral};
use ephemeral_rollups_sdk::cpi::DelegateConfig;
use ephemeral_rollups_sdk::ephem::MagicIntentBundleBuilder;

declare_id!("7ajNjyCeMYaPNDecgxDLt5NAJVoey39DKGhcjiVRQSuq");

// V2 deliberately uses a fresh namespace so stale, terminal PDAs created by
// the original one-escrow-per-wallet demo cannot block the live settlement
// flow after an upgrade.
pub const ESCROW_SEED: &[u8] = b"cabal-escrow-v2";
pub const BOOST_SEED: &[u8] = b"cabal-boost";
pub const LISTING_SEED: &[u8] = b"cabal-boost-listing";
pub const USDC_ESCROW_SEED: &[u8] = b"cabal-usdc-escrow-v1";
/// One PDA per matched mesh deal. Unlike the legacy one-sided escrows, this
/// account records and atomically releases both legs of a SOL/USDC swap.
pub const TRADE_ESCROW_SEED: &[u8] = b"cabal-trade-v1";
pub const CIRCLE_USDC_DEVNET_MINT: Pubkey = pubkey!("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");

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

/// Circle USDC is held in this PDA's associated token account until release.
#[account]
pub struct UsdcEscrow {
    pub depositor: Pubkey,
    pub payee: Pubkey,
    pub amount: u64,
    pub expiry: i64,
    pub status: u8,
}
impl UsdcEscrow { pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 1; }

#[account]
pub struct TradeEscrow {
    pub seller: Pubkey,
    pub buyer: Pubkey,
    pub trade_id: [u8; 32],
    pub sol_amount: u64,
    /// Circle USDC base units (6 decimals).
    pub usdc_amount: u64,
    pub expiry: i64,
    pub sol_locked: bool,
    pub usdc_locked: bool,
}
impl TradeEscrow { pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 1 + 1; }

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

    pub fn initialize_usdc_escrow(
        ctx: Context<InitializeUsdcEscrow>, payee: Pubkey, amount: u64, expiry: i64,
    ) -> Result<()> {
        require!(amount > 0, EscrowError::NoFunds);
        require!(payee != Pubkey::default(), EscrowError::InvalidPayee);
        require!(expiry == 0 || expiry > Clock::get()?.unix_timestamp, EscrowError::InvalidExpiry);
        token::transfer(CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
            from: ctx.accounts.depositor_tokens.to_account_info(),
            to: ctx.accounts.escrow_tokens.to_account_info(),
            authority: ctx.accounts.depositor.to_account_info(),
        }), amount)?;
        let escrow = &mut ctx.accounts.escrow;
        escrow.depositor = ctx.accounts.depositor.key(); escrow.payee = payee;
        escrow.amount = amount; escrow.expiry = expiry; escrow.status = 0;
        Ok(())
    }

    pub fn release_usdc(ctx: Context<ReleaseUsdcEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == 0, EscrowError::NotActive);
        require!(escrow.depositor == ctx.accounts.caller.key(), EscrowError::OnlyDepositor);
        let amount = escrow.amount;
        let depositor = escrow.depositor;
        let bump = ctx.bumps.escrow;
        let signer: &[&[u8]] = &[USDC_ESCROW_SEED, depositor.as_ref(), &[bump]];
        token::transfer(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
            from: ctx.accounts.escrow_tokens.to_account_info(), to: ctx.accounts.payee_tokens.to_account_info(), authority: escrow.to_account_info(),
        }, &[signer]), amount)?;
        // The escrow token account has a PDA authority. Close it after its
        // balance has been transferred so this deal can be garbage-collected
        // completely, including its rent reserve.
        token::close_account(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), CloseAccount {
            account: ctx.accounts.escrow_tokens.to_account_info(),
            destination: ctx.accounts.caller.to_account_info(),
            authority: escrow.to_account_info(),
        }, &[signer]))?;
        escrow.status = 1;
        Ok(())
    }

    /// Seller opens a matched deal and locks the native SOL leg. `trade_id`
    /// is a hash agreed by the two mesh peers, so both sides address exactly
    /// the same PDA without trusting a server-side coordinator.
    pub fn open_trade(
        ctx: Context<OpenTrade>,
        trade_id: [u8; 32],
        buyer: Pubkey,
        sol_amount: u64,
        usdc_amount: u64,
        expiry: i64,
    ) -> Result<()> {
        require!(sol_amount > 0 && usdc_amount > 0, EscrowError::NoFunds);
        require!(buyer != ctx.accounts.seller.key(), EscrowError::InvalidPayee);
        require!(expiry > Clock::get()?.unix_timestamp, EscrowError::InvalidExpiry);
        anchor_lang::system_program::transfer(CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.seller.to_account_info(),
                to: ctx.accounts.trade.to_account_info(),
            },
        ), sol_amount)?;
        let trade = &mut ctx.accounts.trade;
        trade.seller = ctx.accounts.seller.key();
        trade.buyer = buyer;
        trade.trade_id = trade_id;
        trade.sol_amount = sol_amount;
        trade.usdc_amount = usdc_amount;
        trade.expiry = expiry;
        trade.sol_locked = true;
        trade.usdc_locked = false;
        Ok(())
    }

    /// Buyer deposits the Circle USDC leg. The exact mint is constrained by
    /// the account context, preventing arbitrary SPL tokens from settling a
    /// trade that claims to be USDC.
    pub fn lock_trade_usdc(ctx: Context<LockTradeUsdc>) -> Result<()> {
        let trade = &mut ctx.accounts.trade;
        require!(trade.sol_locked && !trade.usdc_locked, EscrowError::NotActive);
        require!(trade.buyer == ctx.accounts.buyer.key(), EscrowError::OnlyBuyer);
        require!(Clock::get()?.unix_timestamp < trade.expiry, EscrowError::InvalidExpiry);
        token::transfer(CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
            from: ctx.accounts.buyer_tokens.to_account_info(),
            to: ctx.accounts.trade_tokens.to_account_info(),
            authority: ctx.accounts.buyer.to_account_info(),
        }), trade.usdc_amount)?;
        trade.usdc_locked = true;
        Ok(())
    }

    /// Atomically releases the two locked legs: SOL to buyer and Circle USDC
    /// to seller. Any caller can execute once both deposits are present;
    /// Solana rolls back both transfers if either side cannot complete.
    pub fn release_trade(ctx: Context<ReleaseTrade>) -> Result<()> {
        let trade = &ctx.accounts.trade;
        require!(trade.sol_locked && trade.usdc_locked, EscrowError::TradeNotFunded);
        let sol_amount = trade.sol_amount;
        let usdc_amount = trade.usdc_amount;
        let trade_id = trade.trade_id;
        let bump = ctx.bumps.trade;
        let signer: &[&[u8]] = &[TRADE_ESCROW_SEED, trade_id.as_ref(), &[bump]];
        token::transfer(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
            from: ctx.accounts.trade_tokens.to_account_info(),
            to: ctx.accounts.seller_tokens.to_account_info(),
            authority: trade.to_account_info(),
        }, &[signer]), usdc_amount)?;
        let trade_info = ctx.accounts.trade.to_account_info();
        let buyer_info = ctx.accounts.buyer.to_account_info();
        **trade_info.try_borrow_mut_lamports()? -= sol_amount;
        **buyer_info.try_borrow_mut_lamports()? += sol_amount;
        token::close_account(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), CloseAccount {
            account: ctx.accounts.trade_tokens.to_account_info(),
            destination: ctx.accounts.caller.to_account_info(),
            authority: trade_info,
        }, &[signer]))?;
        Ok(())
    }

    /// Refunds every funded leg after expiry in the same transaction. Before
    /// the buyer deposits USDC only the seller's SOL is returned; afterwards
    /// both users are returned exactly what they locked.
    pub fn refund_trade(ctx: Context<RefundTrade>) -> Result<()> {
        let trade = &ctx.accounts.trade;
        require!(Clock::get()?.unix_timestamp >= trade.expiry, EscrowError::NotAuthorized);
        let sol_amount = trade.sol_amount;
        let usdc_amount = trade.usdc_amount;
        let trade_id = trade.trade_id;
        let usdc_locked = trade.usdc_locked;
        let bump = ctx.bumps.trade;
        let signer: &[&[u8]] = &[TRADE_ESCROW_SEED, trade_id.as_ref(), &[bump]];
        let trade_info = trade.to_account_info();
        let seller_info = ctx.accounts.seller.to_account_info();
        **trade_info.try_borrow_mut_lamports()? -= sol_amount;
        **seller_info.try_borrow_mut_lamports()? += sol_amount;
        if usdc_locked {
            token::transfer(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.trade_tokens.to_account_info(),
                to: ctx.accounts.buyer_tokens.to_account_info(),
                authority: trade_info.clone(),
            }, &[signer]), usdc_amount)?;
            token::close_account(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), CloseAccount {
                account: ctx.accounts.trade_tokens.to_account_info(),
                destination: ctx.accounts.caller.to_account_info(),
                authority: trade_info,
            }, &[signer]))?;
        }
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
        close = caller,
        seeds = [ESCROW_SEED, escrow.depositor.as_ref()],
        bump
    )]
    pub escrow: Account<'info, Escrow>,
    /// The caller — must be the depositor.
    #[account(mut)]
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
        close = depositor,
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

/// Locks Circle USDC devnet in an ATA owned by the escrow PDA. The mint is
/// pinned on-chain: a caller cannot substitute a lookalike SPL token.
#[derive(Accounts)]
pub struct InitializeUsdcEscrow<'info> {
    #[account(
        init,
        payer = depositor,
        space = UsdcEscrow::LEN,
        seeds = [USDC_ESCROW_SEED, depositor.key().as_ref()],
        bump
    )]
    pub escrow: Account<'info, UsdcEscrow>,
    #[account(address = CIRCLE_USDC_DEVNET_MINT)]
    pub mint: Account<'info, Mint>,
    #[account(
        mut,
        constraint = depositor_tokens.mint == mint.key(),
        constraint = depositor_tokens.owner == depositor.key()
    )]
    pub depositor_tokens: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = depositor,
        associated_token::mint = mint,
        associated_token::authority = escrow
    )]
    pub escrow_tokens: Account<'info, TokenAccount>,
    #[account(mut)]
    pub depositor: Signer<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReleaseUsdcEscrow<'info> {
    #[account(
        mut,
        close = caller,
        seeds = [USDC_ESCROW_SEED, escrow.depositor.as_ref()],
        bump
    )]
    pub escrow: Account<'info, UsdcEscrow>,
    #[account(address = CIRCLE_USDC_DEVNET_MINT)]
    pub mint: Account<'info, Mint>,
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = escrow
    )]
    pub escrow_tokens: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = payee
    )]
    pub payee_tokens: Account<'info, TokenAccount>,
    #[account(mut)]
    pub caller: Signer<'info>,
    /// CHECK: constrained to the recipient recorded at initialization.
    #[account(address = escrow.payee)]
    pub payee: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(trade_id: [u8; 32])]
pub struct OpenTrade<'info> {
    #[account(
        init,
        payer = seller,
        space = TradeEscrow::LEN,
        seeds = [TRADE_ESCROW_SEED, trade_id.as_ref()],
        bump
    )]
    pub trade: Account<'info, TradeEscrow>,
    #[account(address = CIRCLE_USDC_DEVNET_MINT)]
    pub mint: Account<'info, Mint>,
    // Created up front so both the deposit and expiry-refund paths have a
    // deterministic account list and remain atomic.
    #[account(
        init,
        payer = seller,
        associated_token::mint = mint,
        associated_token::authority = trade
    )]
    pub trade_tokens: Account<'info, TokenAccount>,
    #[account(mut)]
    pub seller: Signer<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(trade_id: [u8; 32])]
pub struct LockTradeUsdc<'info> {
    #[account(mut, seeds = [TRADE_ESCROW_SEED, trade_id.as_ref()], bump)]
    pub trade: Account<'info, TradeEscrow>,
    #[account(address = CIRCLE_USDC_DEVNET_MINT)]
    pub mint: Account<'info, Mint>,
    #[account(mut, constraint = buyer_tokens.mint == mint.key(), constraint = buyer_tokens.owner == buyer.key())]
    pub buyer_tokens: Account<'info, TokenAccount>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = trade)]
    pub trade_tokens: Account<'info, TokenAccount>,
    #[account(mut)]
    pub buyer: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ReleaseTrade<'info> {
    #[account(mut, close = caller, seeds = [TRADE_ESCROW_SEED, trade.trade_id.as_ref()], bump)]
    pub trade: Account<'info, TradeEscrow>,
    #[account(address = CIRCLE_USDC_DEVNET_MINT)]
    pub mint: Account<'info, Mint>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = trade)]
    pub trade_tokens: Account<'info, TokenAccount>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = seller)]
    pub seller_tokens: Account<'info, TokenAccount>,
    /// CHECK: constrained to the buyer committed to the trade.
    #[account(mut, address = trade.buyer)]
    pub buyer: AccountInfo<'info>,
    /// CHECK: constrained to the seller committed to the trade.
    #[account(address = trade.seller)]
    pub seller: AccountInfo<'info>,
    #[account(mut)]
    pub caller: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RefundTrade<'info> {
    #[account(mut, close = caller, seeds = [TRADE_ESCROW_SEED, trade.trade_id.as_ref()], bump)]
    pub trade: Account<'info, TradeEscrow>,
    #[account(address = CIRCLE_USDC_DEVNET_MINT)]
    pub mint: Account<'info, Mint>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = trade)]
    pub trade_tokens: Account<'info, TokenAccount>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = buyer)]
    pub buyer_tokens: Account<'info, TokenAccount>,
    /// CHECK: constrained to the seller committed to the trade.
    #[account(mut, address = trade.seller)]
    pub seller: AccountInfo<'info>,
    /// CHECK: constrained to the buyer committed to the trade.
    #[account(address = trade.buyer)]
    pub buyer: AccountInfo<'info>,
    #[account(mut)]
    pub caller: Signer<'info>,
    pub token_program: Program<'info, Token>,
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
    #[msg("Only the matched buyer can lock Circle USDC")]
    OnlyBuyer,
    #[msg("Both SOL and Circle USDC must be locked before release")]
    TradeNotFunded,
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
