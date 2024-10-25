use cosmwasm_schema::cw_serde;

use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::Item;

use membrane::{oracle::PriceResponse, stable_earn_vault::Config};
use membrane::types::ClaimTracker;


#[cw_serde]
pub struct TokenRateAssurance {
    pub pre_btokens_per_one: Uint128,
}

#[cw_serde]
pub struct UnloopProps {
    pub sender: String,
    pub owned_collateral: Uint128,
    pub debt_to_clear: Uint128,
    pub loop_count: u64,
    pub running_collateral_amount: Uint128,
    pub running_credit_amount: Uint128,
    pub vt_token_price: PriceResponse,
    pub cdt_peg_price: PriceResponse,
    pub cdt_market_price: PriceResponse,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const VAULT_TOKEN: Item<Uint128> = Item::new("vault_token");
pub const TOKEN_RATE_ASSURANCE: Item<TokenRateAssurance> = Item::new("token_rate_assurance");
pub const UNLOOP_PROPS: Item<UnloopProps> = Item::new("unloop_props");
pub const CLAIM_TRACKER: Item<ClaimTracker> = Item::new("claim_tracker");


pub const OWNERSHIP_TRANSFER: Item<Addr> = Item::new("ownership_transfer");