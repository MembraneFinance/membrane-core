use cosmwasm_schema::cw_serde;

use cosmwasm_std::{Uint128, Decimal};
use cw_storage_plus::{Item, Map};

use membrane::osmosis_proxy::Config;
use membrane::types::SwapRoute;

#[cw_serde]
pub struct TokenInfo {
    /// Current minted supply
    pub current_supply: Uint128,
    /// Max supply 
    pub max_supply: Option<Uint128>,
    /// Burned supply
    pub burned_supply: Uint128,
}

#[cw_serde]
pub struct PendingTokenInfo {
    /// Chosen subdenom
    pub subdenom: String,
    /// Max supply
    pub max_supply: Option<Uint128>,
}


#[cw_serde]
pub struct SwapInfo {
    pub swapper: String,
    pub token_out: String,
    pub max_slippage: Decimal,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const TOKENS: Map<String, TokenInfo> = Map::new("tokens"); //AssetInfo, TokenInfo
pub const PENDING: Item<PendingTokenInfo> = Item::new("pending_denoms");
pub const SWAP_ROUTES: Item<Vec<SwapRoute>> = Item::new("swap_routes");
pub const SWAP_INFO: Item<SwapInfo> = Item::new("swap_info");