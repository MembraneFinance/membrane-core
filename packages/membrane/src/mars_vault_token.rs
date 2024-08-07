
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};


#[cw_serde]
pub struct InstantiateMsg {
    pub vault_subdenom: String,
    pub deposit_token: String,
    pub mars_redbank_addr: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    EnterVault { },
    ExitVault { },
    // Compound { },
    UpdateConfig {
        owner: Option<String>,
        /// Mainly for testing, we shouldn't change addrs bc we'd lose deposits
        mars_redbank_addr: Option<String>,
    },
    ///APRs are calculated for every deposit and withdrawla but if you want something up to date
    /// you must crank.
    CrankAPR { },
    /// Assures that for deposits & withdrawals the conversion rate is static.
    /// We are trusting that Mars deposits will only go up.
    /// Only callable by the contract
    RateAssurance { },
}

#[cw_serde]
pub enum QueryMsg {
    /// Return contract config
    Config {},
    VaultTokenUnderlying { vault_token_amount: Uint128 },
    DepositTokenConversion { deposit_token_amount: Uint128 },
    APR {},
}

#[cw_serde]
pub struct Config {
    pub owner: Addr,
    pub mars_redbank_addr: Addr,
    pub vault_token: String,
    pub deposit_token: String,
    //Deposit token tally that includes tokens in the vault
    pub total_deposit_tokens: Uint128,
}

#[cw_serde]
pub struct MigrateMsg {}