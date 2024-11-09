
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use crate::types::{APR, DepositToken};

//What does this vault do?

//- Accept W(BTC) deposits which are deposited into the CDP
//- We mint up to an LTV & send that CDT to the SP (withdraw after depositing to start unstake)
//- When the SP gets paid, we withdraw everything (to reset our queue position in the SP) and compound profits into WBTC

//NOTES:
//- We unloop at Some(LTV) & anytime cost is negative 
//- When we compound, even though unlikely, we have to account for potential SP liquidations that take our CDT (i.e. compound those assets if necessary)
//- Withdrawals will repay a pro-rata amount of the CDP debt taken from the SP.
// --Bc this has no cost, we can withdraw at any time & ownership is calc'd in WBTC value, not debt.
// - We deposit directly into the SP instead of the autoSP vault to protect from liquidations.

#[cw_serde]
pub struct InstantiateMsg {
    pub cdt_denom: String,
    pub vault_subdenom: String,
    pub deposit_token_info: DepositToken,
    pub cdp_contract_addr: String,
    pub osmosis_proxy_contract_addr: String,
    pub oracle_contract_addr: String,
    pub stability_pool_contract_addr: String,
    pub mint_LTV: Decimal, //Keep this 1% under the max mint_LTV to allow calcs to have rounding errors
    pub repay_LTV: Decimal,
}

#[cw_serde]
pub enum ExecuteMsg {
    EnterVault { },
    ExitVault { },
    /// 1) Compound SP rewards into the deposit token
    /// 2) Compound SP liquidation rewards (we try to avoid getting these)
    /// 3) Repay if we're >= the repay_LTV or unprofitable
    ManageVault {
        //We can mint at a lower LTV if it means it'll mint without pushing the costs over the cost ceiling
        lower_mint_ltv_ceiling: Option<Decimal>,
    },
    /// Update the vault's config
    UpdateConfig {
        owner: Option<String>,
        /// Mainly for testing, we shouldn't change addrs bc we'd lose deposits
        cdp_contract_addr: Option<String>,
        stability_pool_contract_addr: Option<String>,
        //
        osmosis_proxy_contract_addr: Option<String>,
        oracle_contract_addr: Option<String>,
        swap_slippage: Option<Decimal>,
        vault_cost_index: Option<()>,
        cdp_position_id: Option<()>,
        cost_ceiling: Option<Decimal>,
        mint_LTV: Option<Decimal>,
        repay_LTV: Option<Decimal>,
    },
    ///Saves the current base token claim for 1 vault token
    CrankRealizedAPR { },
    //////////////CALLBACKS////////////////
    /// Assures that for deposits & withdrawals the conversion rate is static.
    /// We are trusting that Mars deposits will only go up.
    /// Only callable by the contract
    StateAssurance { 
        skip_LTV: bool,
        skip_cost: bool
    },

}

#[cw_serde]
pub enum QueryMsg {
    /// Return contract config
    Config {},
    VaultTokenUnderlying { vault_token_amount: Uint128 },
    ClaimTracker {},
}

#[cw_serde]
pub struct Config {
    pub owner: Addr,
    pub cdp_contract_addr: Addr,
    pub osmosis_proxy_contract_addr: Addr,
    pub oracle_contract_addr: Addr,
    pub stability_pool_contract_addr: String,
    pub cdt_denom: String,
    pub vault_token: String,
    pub deposit_token: String,
    /// Position ID of the vault's CDP position (set in instantiation)
    pub cdp_position_id: Uint128,
    pub swap_slippage: Decimal,
    pub vault_cost_index: usize,
    //We repay debt at this cdst
    pub cost_ceiling: Decimal,
    //Mint up to this LTV
    pub mint_LTV: Decimal,
    //Repay down to the mint LTV at this LTV
    pub repay_LTV: Decimal,
}

#[cw_serde]
pub struct MigrateMsg {}