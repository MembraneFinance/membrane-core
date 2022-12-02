
use cosmwasm_std::{Addr, Uint128, Decimal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::{Asset, LockUp, DebtTokenAsset, AssetInfo};


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: Option<String>,   
    pub lock_up_ceiling: Option<u64>,
    pub basket_id: Uint128,
    pub accepted_lps: Vec<AssetInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Deposit { 
        lock_up_duration: u64, //in days
    },
    Withdraw { 
        amount: Uint128,  //in GAMM share tokens (AssetInfo::NativeToken)  
    },
    ClaimRewards { },
    UpdateConfig {
        owner: Option<String>,        
        lock_up_ceiling: Option<u64>,
    },
    EditAcceptedLPs {
        lp: AssetInfo,
        remove: bool,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    //Returns Config
    Config { },
    //Returns UserResponse
    User { 
        user: String,
    },
    //Returns Uint128
    TotalDepositsPerLP { },
    //Returns Vec<Asset>
    TotalDeposits { },
    //Returns Vec<LockUp>
    LockUpDistribution { },
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    pub lock_up_ceiling: u64, //in days
    pub accepted_lps: Vec<AssetInfo>,
    pub basket_id: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct UserResponse {
    pub user: String,
    pub premium_user_value: Decimal,
    pub deposits: Vec<Asset>,
    pub lock_up_distributions: Vec<LockUp>, 
}


