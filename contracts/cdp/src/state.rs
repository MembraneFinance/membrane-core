use membrane::oracle::PriceResponse;

use cosmwasm_std::{Addr, Decimal, Uint128, Storage, QuerierWrapper, Env, StdResult, StdError};
use cosmwasm_schema::cw_serde;
use cw_storage_plus::{Item, Map};

use membrane::types::{cAsset, Asset, AssetInfo, Basket, CDPUserIntents, Position, RedemptionInfo, StoredPrice, UserInfo};
use membrane::cdp::Config;

use crate::ContractError;
use crate::risk_engine::update_basket_tally;


#[cw_serde]
pub struct ContractVersion {
    /// contract is the crate name of the implementing contract, eg. `crate:cw20-base`
    /// we will use other prefixes for other languages, and their standard global namespacing
    pub contract: String,
    /// version is any string that this implementation knows. It may be simple counter "1", "2".
    /// or semantic version on release tags "v0.7.0", or some custom feature flag list.
    /// the only code that needs to understand the version parsing is code that knows how to
    /// migrate from the given contract (and is tied to it's implementation somehow)
    pub version: String,
}

//This propogates liquidation info && state to reduce gas
#[cw_serde]
pub struct LiquidationPropagation {
    pub per_asset_repayment: Vec<Decimal>,//List of repayments
    pub liq_queue_repayment: Decimal, //LQ repayment
    pub stability_pool: Decimal, //SP repayment
    pub user_repay_amount: Decimal,
    pub positions_contract: Addr,
    pub sp_liq_fee: Decimal,
    pub cAsset_ratios: Vec<Decimal>, //these don't change during liquidation bc we liquidate based on the ratios
    pub cAsset_prices: Vec<PriceResponse>,
    pub target_position: Position,
    pub liquidated_assets: Vec<cAsset>, //List of assets liquidated for supply caps
    pub caller_fee_value_paid: Decimal,
    pub total_repaid: Decimal,
    pub position_owner: Addr,
    pub basket: Basket,
    pub config: Config,
}

#[cw_serde]
pub struct WithdrawPropagation {
    pub positions_prev_collateral: Vec<Asset>, //Amount of collateral in the position before the withdrawal
    pub withdraw_amounts: Vec<Uint128>,
    pub contracts_prev_collateral_amount: Vec<Uint128>,
    pub position_info: UserInfo,
}
#[cw_serde]
pub struct ClosePositionPropagation {
    pub withdrawn_assets: Vec<Asset>,
    pub position_info: UserInfo,
    pub send_to: Option<String>,
}
#[cw_serde]
pub struct Timer {
    pub start_time: u64,
    pub end_time: u64,
}
#[cw_serde]
pub struct CollateralVolatility {
    pub index: Decimal,
    pub volatility_list: Vec<Decimal>,
}

pub const CONTRACT: Item<ContractVersion> = Item::new("contract_info");

pub const CONFIG: Item<Config> = Item::new("config");
pub const BASKET: Item<Basket> = Item::new("basket"); 
pub const POSITIONS: Map<Addr, Vec<Position>> = Map::new("positions"); //owner, list of positions
//Volatility Tracker
pub const VOLATILITY: Map<String, CollateralVolatility> = Map::new("volatility");
pub const STORED_PRICES: Map<String, StoredPrice> = Map::new("stored_prices");

/// CDT redemption premium, opt-in mechanism.
/// This is the premium that the user will pay to redeem their debt token.
pub const REDEMPTION_OPT_IN: Map<u128, Vec<RedemptionInfo>> = Map::new("redemption_opt_in"); 

/// Config ownership transfer
pub const OWNERSHIP_TRANSFER: Item<Addr> = Item::new("ownership_transfer");

//Reply State Propagations
pub const WITHDRAW: Item<WithdrawPropagation> = Item::new("withdraw_propagation");
pub const LIQUIDATION: Item<LiquidationPropagation> = Item::new("repay_propagation");
pub const CLOSE_POSITION: Item<ClosePositionPropagation> = Item::new("close_position_propagation");
//Freeze Timer
pub const FREEZE_TIMER: Item<Timer> = Item::new("freeze_timer");
//Intents
pub const USER_INTENTS: Map<String, CDPUserIntents> = Map::new("user_intents");

//Helper functions
/// Update asset claims a Position has
pub fn update_position_claims(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    env: Env,
    config: Config,
    position_id: Uint128,
    position_owner: Addr,
    liquidated_asset: AssetInfo,
    liquidated_amount: Uint128,
) -> StdResult<()> {
    let mut credit_amount: Uint128 = Uint128::zero();

    let mut target_position = None;

    POSITIONS.update(
        storage,
        position_owner,
        |old_positions| -> StdResult<Vec<Position>> {
            if let Some(old_positions) = old_positions {
                let new_positions = old_positions
                    .into_iter()
                    .map(|mut position| {
                        //Find position
                        if position.position_id == position_id {
                            //Set target_position
                            target_position = Some(position.clone());
                            //Set credit_amount
                            credit_amount = position.credit_amount;

                            //Find asset in position
                            position.collateral_assets = position
                                .collateral_assets
                                .into_iter()
                                .map(|mut c_asset| {
                                    //Subtract amount liquidated from claims
                                    if c_asset.asset.info.equal(&liquidated_asset) {                                    
                                        c_asset.asset.amount -= liquidated_amount;
                                    }

                                    c_asset
                                })
                                .collect::<Vec<cAsset>>();
                        }
                        position
                    })
                    .collect::<Vec<Position>>();

                Ok(new_positions)
            } else {
                Err(StdError::GenericErr {
                    msg: String::from("Invalid position owner"),
                })
            }
        },
    )?;

    //Subtract liquidated amount from total asset tally
    let collateral_assets = vec![cAsset {
        asset: Asset {
            info: liquidated_asset,
            amount: liquidated_amount,
        },
        max_borrow_LTV: Decimal::zero(),
        max_LTV: Decimal::zero(),
        pool_info: None,
        rate_index: Decimal::one(),
        hike_rates: Some(false),
    }];

    //If there is no credit, basket tallies were updated in the repay function
    if credit_amount.is_zero() {
        return Ok(());
    }

    let mut basket = BASKET.load(storage)?;
    match update_basket_tally(storage, querier, env, &mut basket, collateral_assets, target_position.unwrap().collateral_assets, false, config, false) {
        Ok(_res) => {
            BASKET.save(storage, &basket)?;
        }
        Err(err) => {
            return Err(StdError::GenericErr {
                msg: err.to_string(),
            })
        }
    };

    Ok(())
}

/// Returns Position & index of Position in User's list
pub fn get_target_position(
    storage: &dyn Storage,
    valid_position_owner: Addr,
    position_id: Uint128,
) -> Result<(usize, Position), ContractError> {
    let positions: Vec<Position> = match POSITIONS.load(
        storage, valid_position_owner
    ){
        Err(_) => return Err(ContractError::NoUserPositions {}),
        Ok(positions) => positions,
    };

    match positions.into_iter().enumerate().find(|(_i, x)| x.position_id == position_id) {
        Some(position) => Ok(position),
        None => Err(ContractError::NonExistentPosition { id: position_id }),
    }
}


/// Replace Position data in state
pub fn update_position(
    storage: &mut dyn Storage,
    valid_position_owner: Addr,
    new_position: Position,
) -> StdResult<()>{

    POSITIONS.update(
        storage,
        valid_position_owner,
        |old_positions| -> StdResult<Vec<Position>> {
            match old_positions {
                Some(old_positions) => {
                    let new_positions = old_positions
                        .into_iter()
                        .map(|stored_position| {
                            //Find position
                            if stored_position.position_id == new_position.position_id {
                                //Swap to target_position 
                                new_position.clone()
                            } else {
                                //Don't override
                                stored_position
                            }
                        })
                        .collect::<Vec<Position>>();

                    Ok(new_positions)
                },
                None => {
                    Err(StdError::GenericErr {
                        msg: String::from("Invalid position owner"),
                    })
                }
            }
        },
    )?;

    Ok(())
}
