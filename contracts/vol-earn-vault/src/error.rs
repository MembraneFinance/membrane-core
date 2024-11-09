use cosmwasm_std::{Coin, StdError};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum TokenFactoryError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized {},
    
    #[error("amount was zero, must be positive")]
    ZeroAmount {},

    #[error("The contract must compound all rewards before entering or exiting the vault: {claims:?}")]
    ContractHasClaims { claims: Vec<Coin> },

    #[error("No liquid tokens available, currently unstaking from the yield strategy.")]
    ZeroDepositTokens {},

    #[error("Custom Error val: {val:?}")]
    CustomError { val: String },
}
