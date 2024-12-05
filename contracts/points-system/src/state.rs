use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};
use cw_storage_plus::{Item, Map};

use membrane::{points_system::{ClaimCheck, Config, UserStats}, types::Asset};

#[cw_serde]
pub struct LiquidationPropagation {
    ///CDP's Pre-Liquidation CDT SUPPLY
    pub pre_liq_CDT: Uint128,
    ///Liquidator address
    pub liquidator: Addr,
    ///Liquidatee address
    pub liquidatee: Addr,
}

#[cw_serde]
pub struct VaultConversionRate {
    ///Vault Address
    pub vault_address: String,
    ///Deposit Token Conversion Rate for 1 vault token
    pub last_conversion_rate: Uint128,
    /// Total Vault Tokens
    pub total_vault_tokens: Asset,
}

#[cw_serde]
pub struct VaultInfo {
    ///Vault Address
    pub vault_address: String,
    //Saves denom for a single vault token
    pub vault_token_denom: String,
    //Saves decimal for a single vault token
    pub single_vault_token: Uint128,
}


pub const CONFIG: Item<Config> = Item::new("config");
pub const USER_STATS: Map<Addr, UserStats> = Map::new("user_stats"); 
pub const CLAIM_CHECK: Item<ClaimCheck> = Item::new("claim_check");
pub const LIQ_PROPAGATION: Item<LiquidationPropagation> = Item::new("cdp_balances");
pub const USER_VAULT_CONVERSION_RATES: Map<Addr, Vec<VaultConversionRate>> = Map::new("user_vault_conversion_rates");
pub const VAULT_INFO: Item<Vec<VaultInfo>> = Item::new("vault_info");

pub const OWNERSHIP_TRANSFER: Item<Addr> = Item::new("ownership_transfer");
