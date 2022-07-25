use std::fmt;

use cosmwasm_std::{Addr, Uint128, Coin, Binary, Decimal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::{ Asset, cAsset, Position, LiqAsset, SellWallDistribution, AssetInfo };

use cw20::Cw20ReceiveMsg;


//Msg Start
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub liq_fee: Decimal,
    pub stability_pool: Option<String>,
    pub dex_router: Option<String>,
    pub fee_collector: Option<String>,
    pub osmosis_proxy: Option<String>,
    pub owner: Option<String>,
    //For Basket creation
    pub collateral_types: Option<Vec<cAsset>>,
    pub credit_asset: Option<Asset>,
    pub credit_price: Option<Decimal>,
    pub credit_interest: Option<Decimal>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Receive(Cw20ReceiveMsg),
    Deposit{
        assets: Vec<AssetInfo>,
        position_owner: Option<String>,
        basket_id: Uint128,
        position_id: Option<Uint128>, //If the user wants to create a new/separate position, no position id is passed         
    },
    IncreaseDebt { //only works on open positions
        basket_id: Uint128,
        position_id: Uint128,
        amount: Uint128,
    }, 
    Withdraw {
        basket_id: Uint128,
        position_id: Uint128,
        assets: Vec<Asset>,
    },
    Repay {
        basket_id: Uint128,
        position_id: Uint128,
        position_owner: Option<String>, //If not the sender
    },
    LiqRepay {
        credit_asset: Asset,
    },
    Liquidate {  
        basket_id: Uint128,
        position_id: Uint128,
        position_owner: String,
    },
    CreateBasket {
        owner: Option<String>,
        collateral_types: Vec<cAsset>,
        credit_asset: Asset,
        credit_price: Option<Decimal>,
        credit_interest: Option<Decimal>,
    },
    EditBasket {
        basket_id: Uint128,
        added_cAsset: Option<cAsset>,
        owner: Option<String>,
        credit_interest: Option<Decimal>,
        liq_queue: Option<String>,
    }, 
    EditAdmin {
        owner: String,
    },


    
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cw20HookMsg {
    Deposit {
        basket_id: Uint128,
        position_owner: Option<String>,
        position_id: Option<Uint128>,
    },
    Repay {
        basket_id: Uint128,
        position_id: Uint128,
        position_owner: Option<String>, //If not the sender
    },
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
    GetUserPositions { //All positions from a user
        basket_id: Option<Uint128>, 
        user: String,
        limit: Option<u32>,
    },
    GetPosition { //Singular position
        position_id: Uint128, 
        basket_id: Uint128, 
        user: String 
    },
    GetBasketPositions { //All positions in a basket
        basket_id: Uint128,
        start_after: Option<String>,
        limit: Option<u32>,
    },
    GetBasket { basket_id: Uint128 }, //Singular basket
    GetAllBaskets { //All baskets
        start_after: Option<Uint128>,
        limit: Option<u32>, 
    },
    //Used internally to test state propagation
    Prop {},
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PositionResponse {
    pub position_id: String,
    pub collateral_assets: Vec<cAsset>,
    pub avg_borrow_LTV: String,
    pub avg_max_LTV: String,
    pub credit_amount: String,
    pub basket_id: String,
    
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PositionsResponse{
    pub user: String,
    pub positions: Vec<Position>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct BasketResponse{
    pub owner: String,
    pub basket_id: String,
    pub current_position_id: String,
    pub collateral_types: Vec<cAsset>, 
    pub credit_asset: Asset, 
    pub credit_price: String,
    pub credit_interest: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: String,
    pub current_basket_id: Uint128,
    pub stability_pool: String,
    pub dex_router: String, //Apollo's router, will need to change msg types if the router changes most likely.
    pub fee_collector: String,
    pub osmosis_proxy: String,
    pub liq_fee: Decimal, // 5 = 5%
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PropResponse {
    pub liq_queue_leftovers: Decimal,
    pub stability_pool: Decimal,
    pub sell_wall_distributions: Vec<SellWallDistribution>,
    pub positions_contract: String,
    //So the sell wall knows who to repay to
    pub position_id: Uint128,
    pub basket_id: Uint128,
    pub position_owner: String,
}