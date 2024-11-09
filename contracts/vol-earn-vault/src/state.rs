use cosmwasm_schema::cw_serde;

use cosmwasm_std::{Addr, Decimal, MessageInfo, Uint128};
use cw_storage_plus::Item;

use membrane::{oracle::PriceResponse, vol_earn_vault::Config};
use membrane::types::ClaimTracker;


#[cw_serde]
pub struct StateAssurance {
    pub pre_tx_ltv: Decimal,
    pub pre_btokens_per_one: Uint128,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const VAULT_TOKEN: Item<Uint128> = Item::new("vault_token");
pub const STATE_ASSURANCE: Item<StateAssurance> = Item::new("state_assurance");
pub const CLAIM_TRACKER: Item<ClaimTracker> = Item::new("claim_tracker");
pub const EXIT_MESSAGE_INFO: Item<MessageInfo> = Item::new("exit_message_info");


pub const OWNERSHIP_TRANSFER: Item<Addr> = Item::new("ownership_transfer");