
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use crate::types::{RangeBounds, RangeTokens, RangePositions, RangeBoundUserIntents, UserIntentState, UserInfo};


#[cw_serde]
pub struct InstantiateMsg {
    pub vault_subdenom: String,
    pub range_tokens: RangeTokens,
    pub range_bounds: RangeBounds,
    pub osmosis_proxy_contract_addr: String,
    pub oracle_contract_addr: String,
}

#[cw_serde]
pub struct LeaveTokens {
    pub percent_to_leave: Decimal,
    pub intent_for_tokens: RangeBoundUserIntents,
}

#[cw_serde]
pub struct ReduceTokens {
    pub amount: Uint128,
    pub exit_vault: bool,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Enter the vault 100% CDT
    EnterVault {
        leave_vault_tokens_in_vault: Option<LeaveTokens>,
    },
    /// Exit vault in the current ratio of assets owned (LP + balances)
    /// The App can swap into a single token and give value options based on swap rate.
    ExitVault {
        send_to: Option<String>,
        swap_to_cdt: bool
    },
    /// Deposits CDT revenue into the contract. 
    /// We use a msg enum bc the CDP needs it.
    DepositFee { },
    /// 1) Takes deposited revenue from DepositFee & either adds it to the ceiling or swaps it all (or rebalance_sale_max) to add to the floor
    /// 2) Redeposits LP rewards into LP (compound) if price is out of its range.
    /// Flow: 
    /// - ClaimSpreadFees
    /// - Attempt to compound into ceiling or floor
    /// - If price is in the ceiling, swap and deposit into floor
    /// - If price is in the floor, swap and deposit into ceiling
    ManageVault { rebalance_sale_max: Option<Decimal> },
    /// Set intents for a user. They must send vault tokens or have a non-zero balance in state.
    /// NOTE: We don't use the asset price initiation.
    SetUserIntents { 
        intents: Option<RangeBoundUserIntents>,
        reduce_vault_tokens: Option<ReduceTokens>,
    },
    /// Fulfill intents for a user. Send fees to the caller.
    FulFillUserIntents { users: Vec<String> },
    /// Let CDP contract use VTs in user intents to repay debt.
    RepayUserDebt { 
        user_info: UserInfo,
        repayment: Uint128, 
    },
    /// Update the contract config
    UpdateConfig {
        owner: Option<String>,
        osmosis_proxy_contract_addr: Option<String>,
        oracle_contract_addr: Option<String>,
    },
    ///Saves the current base token claim for 1 vault token
    CrankRealizedAPR { },
    /// Assures that for deposits & withdrawals the conversion rate is static.
    /// Only callable by the contract
    RateAssurance { },
}

#[cw_serde]
pub enum QueryMsg {
    /// Return contract config
    Config {},
    VaultTokenUnderlying { vault_token_amount: Uint128 },
    DepositTokenConversion { deposit_token_value: Decimal },
    ClaimTracker {},
    TotalTVL {},
    GetUserIntent { 
        start_after: Option<String>,
        limit: Option<u32>,
        users: Vec<String> 
    },
}

#[cw_serde]
pub struct UserIntentResponse {
    pub user: String,
    pub intent: UserIntentState
}

#[cw_serde]
pub struct Config {
    pub owner: Addr,
    pub osmosis_proxy_contract_addr: String,
    pub oracle_contract_addr: String,
    pub vault_token: String,
    pub range_tokens: RangeTokens,
    pub range_bounds: RangeBounds,
    pub range_position_ids: RangePositions,
}

#[cw_serde]
pub struct MigrateMsg {}
