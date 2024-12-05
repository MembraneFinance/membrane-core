use cosmwasm_schema::cw_serde;

use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Item, Map};

use membrane::range_bound_lp_vault::{Config};
use membrane::types::{ClaimTracker, RangeBoundUserIntents, UserInfo, UserIntentState};


#[cw_serde]
pub struct TokenRateAssurance {
    pub pre_btokens_per_one: Uint128,
}


#[cw_serde]
pub struct IntentProp {
    pub intents: RangeBoundUserIntents,
    pub prev_usdc_balance: Uint128,
    pub prev_cdt_balance: Uint128,
}

#[cw_serde]
pub struct RepayProp {
    pub user_info: UserInfo,
    pub prev_usdc_balance: Uint128,
    pub prev_cdt_balance: Uint128,
}


pub const CONFIG: Item<Config> = Item::new("config");
pub const VAULT_TOKEN: Item<Uint128> = Item::new("vault_token");
pub const CLAIM_TRACKER: Item<ClaimTracker> = Item::new("claim_tracker");
pub const TOKEN_RATE_ASSURANCE: Item<TokenRateAssurance> = Item::new("token_rate_assurance");
pub const USER_INTENT_STATE: Map<String, UserIntentState> = Map::new("user_intent_state");
pub const INTENT_PROPAGATION: Item<IntentProp> = Item::new("intent_propagation");
pub const CDP_REPAY_PROPAGATION: Item<RepayProp> = Item::new("repay_propagation");

pub const OWNERSHIP_TRANSFER: Item<Addr> = Item::new("ownership_transfer");