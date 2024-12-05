use cosmwasm_std::{Addr, Coin, Decimal, Uint128};
use cosmwasm_schema::cw_serde;

use crate::types::UserInfo;
use crate::liq_queue::ClaimsResponse;

#[cw_serde]
pub struct InstantiateMsg {
    pub cdt_denom: String,
    pub oracle_contract: String,
    pub positions_contract: String,
    pub stability_pool_contract: String,
    pub liq_queue_contract: String,
    pub governance_contract: String,
    pub osmosis_proxy_contract: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    UpdateConfig {
        owner: Option<String>,
        cdt_denom: Option<String>,
        oracle_contract: Option<String>,
        positions_contract: Option<String>,
        stability_pool_contract: Option<String>,
        liq_queue_contract: Option<String>,
        governance_contract: Option<String>,
        osmosis_proxy_contract: Option<String>,
        mbrn_per_point: Option<Decimal>,
        max_mbrn_distribution: Option<Uint128>,
        points_per_dollar: Option<Decimal>,
    },
    /// Queries contracts to confirm & save current claims for the user
    CheckClaims {
        cdp_repayment: Option<UserInfo>,
        sp_claims: bool,
        lq_claims: bool,
        /// Proposal ID
        vote: Option<Vec<u64>>,
    }, 
    /// Recheck claims & give points for checked claims
    GivePoints {
        cdp_repayment: bool,
        sp_claims: bool,
        lq_claims: bool,
        /// Proposal ID
        vote: Option<Vec<u64>>,
    },
    /// Liquidate & send fees to caller (Points for liquidator and liquidatee)
    Liquidate {
        /// Position ID
        position_id: Uint128,
        /// Position owner
        position_owner: String,
    },
    /// Claim MBRN from level ups
    ClaimMBRN {},
}
//Position Repayments can be done on the the base Positions contract

#[cw_serde]
pub enum QueryMsg {
    /// Return contract config
    Config {},
    // Return current claim check
    ClaimCheck {},
    // Return user's stats
    UserStats { 
        user: Option<String>,
        limit: Option<u64>,
        start_after: Option<String>,
     },
}


#[cw_serde]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// CDT Denom
    pub cdt_denom: String,
    /// Oracle contract address
    pub oracle_contract: Addr,
    /// Positions contract address
    pub positions_contract: Addr,
    /// Stability Pool contract address
    pub stability_pool_contract: Addr,
    /// Liq Queue contract address
    pub liq_queue_contract: Addr,
    /// Gov contract address
    pub governance_contract: Addr,
    /// Osmosis Proxy contract address
    pub osmosis_proxy_contract: Addr,
    ///MBRN distribution per point
    pub mbrn_per_point: Decimal,
    ///Total MBRN distributon from the contract
    pub total_mbrn_distribution: Uint128,
    ///Maximum MBRN distributon for the contract
    pub max_mbrn_distribution: Uint128,
    ///Points per $1
    pub points_per_dollar: Decimal,
}

#[cw_serde]
pub struct ClaimCheck {
    /// Block time of the last check
    pub check_time: u64,
    /// User address
    pub user: Addr,
    ///Basket's last pending_revenue value
    pub cdp_pending_revenue: Uint128,
    ///LQ's Pending Claims
    pub lq_pending_claims: Vec<ClaimsResponse>,
    ///SP's Pending Claims
    pub sp_pending_claims: Vec<Coin>,
    ///Proposal IDs that the user hadn't voted in during the check
    pub vote_pending: Vec<u64>,
}

#[cw_serde]
pub struct UserStats {
    /// Total points
    pub total_points: Decimal,
    /// Claimable points
    pub claimable_points: Decimal,
}

#[cw_serde]
pub struct UserStatsResponse {
    /// User address
    pub user: Addr,
    ///Stats
    pub stats: UserStats,
}
