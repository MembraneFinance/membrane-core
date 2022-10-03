use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use membrane::types::LiquidityInfo;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    pub osmosis_proxy: Addr,
    pub positions_contract: Addr,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const ASSETS: Map<String, LiquidityInfo> = Map::new("assets");