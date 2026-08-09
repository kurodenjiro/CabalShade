use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount, Transfer};

declare_id!("DVJ6GqkLAGwxceuMLJoKBKrfCposypoMCpBEHFea9GNa");

pub const BOOST_SEED: &[u8] = b"boost";
pub const LISTING_SEED: &[u8] = b"listing";

#[program]
pub mod cabal_boost {
    use super::*;

    pub fn register_boost(ctx: Context<RegisterBoost>, boost_bps: u16, expires_at: i64) -> Result<()> {
        require!(boost_bps > 0, BoostError::InvalidBoost);
        require!(expires_at > Clock::get()?.unix_timestamp, BoostError::InvalidExpiry);
        require!(ctx.accounts.mint.decimals == 0, BoostError::InvalidNftMint);
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
        require!(!boost.used, BoostError::BoostAlreadyUsed);
        require!(Clock::get()?.unix_timestamp < boost.expires_at, BoostError::BoostExpired);
        require!(ctx.accounts.user_tokens.amount >= 1, BoostError::MissingBoostNft);
        token::burn(CpiContext::new(ctx.accounts.token_program.to_account_info(), Burn {
            mint: ctx.accounts.mint.to_account_info(), from: ctx.accounts.user_tokens.to_account_info(), authority: ctx.accounts.user.to_account_info(),
        }), 1)?;
        boost.used = true;
        Ok(())
    }

    pub fn list_boost(ctx: Context<ListBoost>, price_lamports: u64) -> Result<()> {
        require!(price_lamports > 0, BoostError::InvalidPrice);
        require!(ctx.accounts.user_tokens.amount == 1, BoostError::MissingBoostNft);
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
        require!(Clock::get()?.unix_timestamp < ctx.accounts.listing.expiry, BoostError::BoostExpired);
        anchor_lang::system_program::transfer(CpiContext::new(ctx.accounts.system_program.to_account_info(), anchor_lang::system_program::Transfer {
            from: ctx.accounts.buyer.to_account_info(), to: ctx.accounts.seller.to_account_info(),
        }), ctx.accounts.listing.price_lamports)?;
        let seller = ctx.accounts.listing.seller;
        let mint = ctx.accounts.mint.key();
        let bump = ctx.bumps.listing;
        let seeds: &[&[u8]] = &[LISTING_SEED, seller.as_ref(), mint.as_ref(), &[bump]];
        token::transfer(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
            from: ctx.accounts.vault_tokens.to_account_info(), to: ctx.accounts.buyer_tokens.to_account_info(), authority: ctx.accounts.listing.to_account_info(),
        }, &[seeds]), 1)?;
        Ok(())
    }
}

#[account]
pub struct Boost { pub mint: Pubkey, pub issuer: Pubkey, pub boost_bps: u16, pub expires_at: i64, pub used: bool }
impl Boost { pub const LEN: usize = 8 + 32 + 32 + 2 + 8 + 1; }
#[account]
pub struct BoostListing { pub seller: Pubkey, pub mint: Pubkey, pub price_lamports: u64, pub expiry: i64 }
impl BoostListing { pub const LEN: usize = 8 + 32 + 32 + 8 + 8; }

#[derive(Accounts)]
pub struct RegisterBoost<'info> {
    #[account(init, payer = issuer, space = Boost::LEN, seeds = [BOOST_SEED, mint.key().as_ref()], bump)] pub boost: Account<'info, Boost>,
    pub mint: Account<'info, Mint>, #[account(mut)] pub issuer: Signer<'info>, pub system_program: Program<'info, System>,
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
    /// CHECK: validated against listing.seller.
    #[account(mut, address = listing.seller)] pub seller: AccountInfo<'info>,
    #[account(mut)] pub buyer: Signer<'info>, pub token_program: Program<'info, Token>, pub system_program: Program<'info, System>,
}

#[error_code]
pub enum BoostError { #[msg("Invalid boost")] InvalidBoost, #[msg("Invalid expiry")] InvalidExpiry, #[msg("NFT mint must use zero decimals")] InvalidNftMint, #[msg("Boost already used")] BoostAlreadyUsed, #[msg("Boost expired")] BoostExpired, #[msg("Missing boost NFT")] MissingBoostNft, #[msg("Invalid price")] InvalidPrice }
