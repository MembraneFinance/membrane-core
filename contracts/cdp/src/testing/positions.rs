use std::str::FromStr;
use std::vec;

use cosmwasm_std::{
    attr, coin, to_binary, Addr, Api, BankMsg, Coin, CosmosMsg, Decimal, DepsMut, Env, MessageInfo,
    QuerierWrapper, QueryRequest, Response, StdError, StdResult, Storage, SubMsg, Uint128, WasmMsg,
    WasmQuery,
};
use cosmwasm_storage::{Bucket, ReadonlyBucket};

use membrane::helpers::{router_native_to_native, pool_query_and_exit, query_stability_pool_fee, validate_position_owner, asset_to_coin, withdrawal_msg};
use membrane::positions::{Config, ExecuteMsg, EditBasket};
use membrane::oracle::{AssetResponse, PriceResponse};
use osmo_bindings::PoolStateResponse;
use membrane::liq_queue::ExecuteMsg as LQ_ExecuteMsg;
use membrane::liquidity_check::{ExecuteMsg as LiquidityExecuteMsg, QueryMsg as LiquidityQueryMsg};
use membrane::staking::{ExecuteMsg as Staking_ExecuteMsg, QueryMsg as Staking_QueryMsg, Config as Staking_Config};
use membrane::oracle::{ExecuteMsg as OracleExecuteMsg, QueryMsg as OracleQueryMsg};
use membrane::osmosis_proxy::{ExecuteMsg as OsmoExecuteMsg, QueryMsg as OsmoQueryMsg };
use membrane::stability_pool::{ ExecuteMsg as SP_ExecuteMsg, QueryMsg as SP_QueryMsg};
use membrane::math::{decimal_division, decimal_multiplication, Uint256};
use membrane::types::{
    cAsset, Asset, AssetInfo, AssetOracleInfo, Basket, LiquidityInfo, Position,
    StoredPrice, SupplyCap, UserInfo, PriceVolLimiter, equal, AssetPool
};

use crate::contract::get_contract_balances;
use crate::rates::accrue;
use crate::risk_engine::{update_basket_tally, update_basket_debt, update_debt_per_asset_in_position};
use crate::state::{CLOSE_POSITION, ClosePositionPropagation, BASKET};
use crate::{
    state::{
        WithdrawPropagation, CONFIG, POSITIONS, LIQUIDATION, WITHDRAW,
    },
    ContractError,
};

pub const WITHDRAW_REPLY_ID: u64 = 4u64;
pub const CLOSE_POSITION_REPLY_ID: u64 = 5u64;
pub const BAD_DEBT_REPLY_ID: u64 = 999999u64;

static PREFIX_PRICE: &[u8] = b"price";

//Deposit collateral to existing position. New or same collateral.
//Anyone can deposit, to any position. There will be barriers for withdrawals.
pub fn deposit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    position_owner: Option<String>,
    position_id: Option<Uint128>,
    cAssets: Vec<cAsset>,
) -> Result<Response, ContractError> {    
    let config = CONFIG.load(deps.storage)?;
    let valid_owner_addr = validate_position_owner(deps.api, info.clone(), position_owner)?;
    let mut basket: Basket = BASKET.load(deps.storage)?;

    //Set deposit_amounts to double check state storage 
    let deposit_amounts: Vec<Uint128> = cAssets.clone()
        .into_iter()
        .map(|cAsset| cAsset.asset.amount)
        .collect::<Vec<Uint128>>();

    //Initialize positions_prev_collateral & position_info for deposited assets
    //Used for to double check state storage
    let mut positions_prev_collateral = vec![];
    let mut position_info = UserInfo {
        position_id: Uint128::zero(),
        position_owner: "".to_string(),
    };
    
    //For Response
    let mut new_position_id: Uint128 = Uint128::new(0u128);    

    //For debt per asset updates
    let mut old_assets: Vec<cAsset>;
    let mut new_assets = vec![];

    if let Ok(mut positions) = POSITIONS.load(deps.storage, valid_owner_addr.clone()){
        //Add collateral to the position_id or Create a new position 
        if let Some(position_id) = position_id {
            //Find the position
            if let Some((position_index, mut position)) = positions.clone()
                .into_iter()
                .enumerate()
                .find(|(_i, position)| position.position_id == position_id){
                //Set old_assets for debt cap update
                old_assets = position.clone().collateral_assets;

                //Store position_info for reply
                position_info = UserInfo {
                    position_id,
                    position_owner: valid_owner_addr.clone().to_string(),
                };

                for deposit in cAssets.clone(){
                    //Search for cAsset in the position 
                    if let Some((collateral_index, cAsset)) = position.clone().collateral_assets
                        .into_iter()
                        .enumerate()
                        .find(|(_i, cAsset)| cAsset.asset.info.equal(&deposit.asset.info)){
                        //Store positions_prev_collateral
                        positions_prev_collateral.push(cAsset.clone().asset);

                        //Add to existing cAsset
                        position.collateral_assets[collateral_index].asset.amount += deposit.asset.amount;
                    } else { //Add new cAsset object to position
                        position.collateral_assets.push( deposit.clone() );

                        let placeholder_asset = Asset {
                            amount: Uint128::zero(),
                            ..deposit.clone().asset
                        };
                        //Store positions_prev_collateral
                        positions_prev_collateral.push(placeholder_asset.clone());

                        //Add empty asset to old_assets as a placeholder
                        old_assets.push(cAsset {
                            asset: placeholder_asset.clone(),
                            max_borrow_LTV: deposit.clone().max_borrow_LTV,
                            max_LTV: deposit.clone().max_LTV,
                            pool_info: deposit.clone().pool_info,
                            rate_index: deposit.clone().rate_index,
                        });
                    }
                }
                //Set new_assets for debt cap updates
                new_assets = position.clone().collateral_assets;
                
                //Set updated position
                positions[position_index] = position.clone();
                
                //Accrue
                accrue(
                    deps.storage,
                    deps.querier,
                    env.clone(),
                    &mut position.clone(),
                    &mut basket,
                    valid_owner_addr.clone().to_string(),
                )?;
                //Save Basket
                BASKET.save(deps.storage, &basket)?;
                //Save Updated Vec<Positions> for the user
                POSITIONS.save(deps.storage, valid_owner_addr.clone(), &positions)?;

                if !position.credit_amount.is_zero() {
                    update_debt_per_asset_in_position(
                        deps.storage,
                        env.clone(),
                        deps.querier,
                        config,
                        old_assets,
                        new_assets,
                        Decimal::from_ratio(position.credit_amount, Uint128::new(1u128)),
                    )?;
                }
            } else {                
                //If position_ID is passed but no position is found, Error. 
                //In case its a mistake, don't want to add assets to a new position.
                return Err(ContractError::NonExistentPosition {});
            }
        } else { //If user doesn't pass an ID, we create a new position
            let mut new_position =  create_position(cAssets.clone(), &mut basket)?;

            //Store position_info for reply
            position_info = UserInfo {
                position_id: new_position.clone().position_id,
                position_owner: valid_owner_addr.clone().to_string(),
            };

            //Accrue, mainly for repayment price
            accrue(
                deps.storage,
                deps.querier,
                env.clone(),
                &mut new_position,
                &mut basket,
                valid_owner_addr.clone().to_string(),
            )?;
            //Save Basket. This doesn't overwrite the save in update_debt_per_asset_in_position()
            BASKET.save(deps.storage, &basket)?;

            //For response
            new_position_id = new_position.clone().position_id;            

            //Need to add new position to the old set of positions if a new one was created.
            POSITIONS.update(
                deps.storage,
                valid_owner_addr.clone(),
                |positions| -> Result<Vec<Position>, ContractError> {
                    //We can .unwrap() here bc the initial .load() matched Ok()
                    let mut old_positions = positions.unwrap();

                    old_positions.push(new_position);

                    Ok(old_positions)
                },
            )?;
        }
    } else { //No existing positions loaded so new Vec<Position> is created
        let mut new_position = create_position(cAssets.clone(), &mut basket)?;

        //Store position_info for reply
        position_info = UserInfo {
            position_id: new_position.clone().position_id,
            position_owner: valid_owner_addr.clone().to_string(),
        };

        //Accrue, mainly for repayment price
        accrue(
            deps.storage,
            deps.querier,
            env.clone(),
            &mut new_position,
            &mut basket,
            valid_owner_addr.clone().to_string(),
        )?;
        //Save Basket. This only doesn't overwrite the save in update_debt_per_asset_in_position() bc they are certain to never happen at the same time
        BASKET.save(deps.storage, &basket)?;

        //For response
        new_position_id = new_position.clone().position_id;

        //Add new Vec of Positions to state under the user
        POSITIONS.save(
            deps.storage,
            valid_owner_addr.clone(),
            &vec![new_position],
        )?;
    }

    //Double check State storage
    check_deposit_state(deps.storage, deps.api, positions_prev_collateral, deposit_amounts, position_info.clone())?;    

    Ok(Response::new().add_attributes(vec![
        attr("method", "deposit"),
        attr("position_owner", position_info.position_owner),
        attr("position_id", position_info.position_id),
        attr("assets", format!("{:?}", cAssets.into_iter().map(|a|a.asset).collect::<Vec<Asset>>())),
    ]))
}

fn check_deposit_state(
    storage: &mut dyn Storage,  
    api: &dyn Api,   
    positions_prev_collateral: Vec<Asset>, //Amount of collateral in the position before the deposit
    deposit_amounts: Vec<Uint128>,
    position_info: UserInfo,
) -> Result<(), ContractError>{
    let (_i, target_position) = get_target_position(
        storage, 
        api.addr_validate(&position_info.position_owner)?, 
        position_info.position_id
    )?;

    for (i, asset) in positions_prev_collateral.clone().into_iter().enumerate(){

        if let Some(cAsset) = target_position.clone().collateral_assets
            .into_iter()
            .find(|cAsset| cAsset.asset.info.equal(&asset.info)){

            //Assert cAsset total is equal to the amount deposited + prev_asset_amount
            if cAsset.asset.amount != asset.amount + deposit_amounts[i] {
                return Err(ContractError::CustomError { val: String::from("Conditional 1: Possible state error") })
            }
        }
    }

    //If a deposit to a new position, asset amounts should be exactly what was deposited
    if positions_prev_collateral == vec![] {
        for (i, cAsset) in target_position.clone().collateral_assets.into_iter().enumerate() {
            if cAsset.asset.amount != deposit_amounts[i] {
                return Err(ContractError::CustomError { val: String::from("Conditional 2: Possible state error") })
            }
        }
    }

    Ok(())
}

pub fn withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    position_id: Uint128,
    cAssets: Vec<cAsset>,
    send_to: Option<String>,
) -> Result<Response, ContractError> {
    let config: Config = CONFIG.load(deps.storage)?;
    let mut basket: Basket = BASKET.load(deps.storage)?;
    let mut msgs = vec![];

    //Check if frozen
    if basket.frozen { return Err(ContractError::Frozen {  }) }

    //Set recipient
    let mut recipient = info.clone().sender;
    if let Some(string) = send_to.clone() {
        recipient = deps.api.addr_validate(&string)?;
    } 

    //Set position owner
    let mut valid_position_owner = info.clone().sender;
    if info.clone().sender == env.contract.address && send_to.is_some(){
        valid_position_owner = recipient.clone();
    }//If the contract is withdrawing for a user^

    //This forces withdrawals to be done by the info.sender
    let (position_index, mut target_position) = get_target_position(deps.storage, valid_position_owner.clone(), position_id)?;
    //Accrue interest
    accrue(
        deps.storage,
        deps.querier,
        env.clone(),
        &mut target_position,
        &mut basket,
        valid_position_owner.clone().to_string()
    )?;

    //For debt cap updates
    let old_assets = target_position.clone().collateral_assets;
    let mut new_assets: Vec<cAsset> = vec![];
    let mut tally_update_list: Vec<cAsset> = vec![];
    let credit_amount = Uint128::zero();

    //Set withdrawal prop variables
    let mut prop_assets = vec![];
    let mut reply_order: Vec<usize> = vec![];
    let mut withdraw_amounts: Vec<Uint128> = vec![];

    //For Withdraw Msg
    let mut withdraw_coins: Vec<Coin> = vec![];

    //Check for expunged assets and assert they are being withdrawn
    check_for_expunged( old_assets.clone(), cAssets.clone(), basket.clone() )?;

    //Attempt to withdraw each cAsset
    for cAsset in cAssets.clone() {
        let withdraw_asset = cAsset.asset;             

        //Find cAsset in target_position
        if let Some((collateral_index, position_collateral)) = target_position.clone().collateral_assets
            .into_iter()
            .enumerate()
            .find(|(_i, cAsset)| cAsset.asset.info.equal(&withdraw_asset.info)){
            //If the cAsset is found in the position, attempt withdrawal
            //Cant withdraw more than the positions amount
            if withdraw_asset.amount > position_collateral.asset.amount {
                return Err(ContractError::InvalidWithdrawal {});
            } else {
                //Now that its a valid withdrawal and debt has accrued, we can add to tally_update_list
                //This will be used to keep track of Basket supply caps
                tally_update_list.push(cAsset {
                    asset: withdraw_asset.clone(),
                    ..position_collateral.clone()
                });

                //Withdraw Prop: Push the initial asset
                prop_assets.push(position_collateral.clone().asset);

                //Update cAsset data to account for the withdrawal
                let leftover_amount = position_collateral.asset.amount - withdraw_asset.amount;               

                //Delete asset from the position if the amount is being fully withdrawn, otherwise edit.
                if leftover_amount != Uint128::new(0u128) {
                    target_position.collateral_assets[collateral_index].asset.amount = leftover_amount;
                } else {
                    target_position.collateral_assets.remove(collateral_index);
                }

                //If resulting LTV makes the position insolvent, error. If not construct withdrawal_msg
                //This is taking max_borrow_LTV so users can't max borrow and then withdraw to get a higher initial LTV
                if insolvency_check(
                    deps.storage,
                    env.clone(),
                    deps.querier,
                    basket.clone(),
                    target_position.clone().collateral_assets,
                    Decimal::from_ratio(target_position.clone().credit_amount, Uint128::new(1u128)),
                    basket.credit_price,
                    true,
                    config.clone(),
                )?.0 {
                    return Err(ContractError::PositionInsolvent {});
                } else {
                    //Update Position list
                    POSITIONS.update(deps.storage, valid_position_owner.clone(), |positions: Option<Vec<Position>>| -> Result<Vec<Position>, ContractError>{

                        let mut updating_positions = positions.unwrap();

                        //If new position isn't empty, update
                        if !check_for_empty_position(target_position.clone().collateral_assets){
                            updating_positions[position_index] = target_position.clone();
                        } else { // remove old position
                            updating_positions.remove(position_index);
                        }

                        Ok( updating_positions )
                    
                    })?;
                }
                
                //Save for debt cap updates
                new_assets = target_position.clone().collateral_assets;

                //Push withdraw asset to list for withdraw prop
                withdraw_amounts.push(withdraw_asset.clone().amount);

                //Add to native token send list
                if let AssetInfo::NativeToken { denom: _ } = withdraw_asset.clone().info {
                    //Push to withdraw_coins
                    withdraw_coins.push(asset_to_coin(withdraw_asset)?);
                }
            }
        }         
    };
    
    //Push aggregated native coin withdrawal
    if withdraw_coins != vec![] {
        //Save # of native tokens withdrawn for the withdrawal reply propagation 
        reply_order.push(withdraw_coins.len() as usize);

        let message = CosmosMsg::Bank(BankMsg::Send {
            to_address: recipient.to_string(),
            amount: withdraw_coins,
        });
        msgs.push(SubMsg::reply_on_success(message, WITHDRAW_REPLY_ID));
    }

    //Update basket supply cap tallies after all withdrawals to improve UX by smoothing debt_cap restrictions
    update_basket_tally(
        deps.storage,
        deps.querier,
        env.clone(),
        &mut basket,
        tally_update_list,
        false,
    )?;

    //Save updated repayment price and asset tallies
    BASKET.save(deps.storage, &basket)?;

    //Update debt distribution for position assets
    if !target_position.clone().credit_amount.is_zero() {
        //Make sure lists are equal and add blank assets if not
        if old_assets.len() != new_assets.len() {
            for i in 0..old_assets.len() {
                let mut already_pushed = false;
                if i == new_assets.len() {
                    new_assets.push(cAsset {
                        asset: Asset {
                            info: old_assets[i].clone().asset.info,
                            amount: Uint128::zero(),
                        },
                        ..old_assets[i].clone()
                    });
                    already_pushed = true;
                }
                //If the index isn't equal, push a blank asset (0 amount) beforehand
                if !already_pushed {
                    if !old_assets[i].asset.info.equal(&new_assets[i].asset.info) {
                        let temp_vec = vec![cAsset {
                            asset: Asset {
                                info: old_assets[i].clone().asset.info,
                                amount: Uint128::zero(),
                            },
                            ..old_assets[i].clone()
                        }];

                        let mut left: Vec<cAsset> = vec![];
                        let mut right: Vec<cAsset> = vec![];
                        for (index, asset) in new_assets.into_iter().enumerate() {
                            if index < i {
                                left.push(asset)
                            } else {
                                right.push(asset)
                            }
                        }
                        left.extend(temp_vec);
                        left.extend(right);
                        new_assets = left;
                    }
                }
            }
        }
        //Update debt caps
        update_debt_per_asset_in_position(
            deps.storage,
            env.clone(),
            deps.querier,
            config,
            old_assets,
            new_assets,
            Decimal::from_ratio(target_position.credit_amount, Uint128::new(1u128)),
        )?;
    }

    //Set Withdrawal_Prop
    let prop_assets_info: Vec<AssetInfo> = prop_assets
        .clone()
        .into_iter()
        .map(|asset| asset.info)
        .collect::<Vec<AssetInfo>>();
    
    let withdrawal_prop = WithdrawPropagation {
        positions_prev_collateral: prop_assets,
        withdraw_amounts,
        contracts_prev_collateral_amount: get_contract_balances(
            deps.querier,
            env,
            prop_assets_info,
        )?,
        position_info: UserInfo {
            position_id,
            position_owner: info.clone().sender.to_string(),
        },
        reply_order,
    };
    WITHDRAW.save(deps.storage, &withdrawal_prop)?;

    Ok(Response::new()
        .add_attributes(vec![
            attr("method", "withdraw"),
            attr("position_id", position_id),
            attr("assets", format!("{:?}", cAssets)),
        ])
        .add_submessages(msgs))
}

pub fn repay(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    api: &dyn Api,
    env: Env,
    info: MessageInfo,
    position_id: Uint128,
    position_owner: Option<String>,
    credit_asset: Asset,
    send_excess_to: Option<String>,
) -> Result<Response, ContractError> {
    let config: Config = CONFIG.load(storage)?;
    let mut basket: Basket = BASKET.load(storage)?;

    //Validate position owner 
    let valid_owner_addr = validate_position_owner(api, info.clone(), position_owner)?;
    
    //Get target_position
    let (position_index, mut target_position) = get_target_position(storage, valid_owner_addr.clone(), position_id)?;

    //Accrue interest
    accrue(
        storage,
        querier,
        env.clone(),
        &mut target_position,
        &mut basket,
        valid_owner_addr.clone().to_string(),
    )?;
    
    //Set prev_credit_amount
    let prev_credit_amount = target_position.credit_amount;
    
    let mut messages = vec![];
    let total_loan = Uint128::zero();
    let mut excess_repayment = Uint128::zero();
    let updated_list: Vec<Position> = vec![];

    //Assert that the correct credit_asset was sent
    assert_credit_asset(basket.clone(), credit_asset.clone(), info.clone().sender)?;

    //Repay amount sent
    target_position.credit_amount = match target_position.credit_amount.checked_sub(credit_asset.amount){
        Ok(difference) => difference,
        Err(_err) => {
            //Set excess_repayment
            excess_repayment = credit_asset.amount - target_position.credit_amount;
            
            Uint128::zero()
        },
    };

    //Position's resulting debt can't be below minimum without being fully repaid
    if target_position.credit_amount * basket.clone().credit_price < config.debt_minimum
        && !target_position.credit_amount.is_zero(){
        //Router contract is allowed to.
        //We rather $1 of bad debt than $2000 and bad debt comes from swap slippage
        if let Some(router) = config.clone().dex_router {
            if info.sender != router {
                return Err(ContractError::BelowMinimumDebt {});
            }
        }
        //This would also pass for ClosePosition, but since spread is added to collateral amount this should never happen
        //Even if it does, the subsequent withdrawal would then error
    }

    //Update Position
    POSITIONS.update(storage, valid_owner_addr.clone(), |positions: Option<Vec<Position>>| -> Result<Vec<Position>, ContractError> {
        let mut updating_positions = positions.unwrap();
        updating_positions[position_index] = target_position.clone();
        
        Ok(updating_positions)
    })?;

    //Burn repayment & send revenue to stakers
    let burn_and_rev_msgs = credit_burn_msg(
        config.clone(),
        env.clone(),
        credit_asset.clone(),
        &mut basket,
    )?;
    messages.extend(burn_and_rev_msgs);

    //Send back excess repayment, defaults to the repaying address
    if !excess_repayment.is_zero() {
        if let Some(addr) = send_excess_to {
            let valid_addr = api.addr_validate(&addr)?;

            let msg = withdrawal_msg(Asset {
                amount: excess_repayment,
                ..basket.clone().credit_asset
            }, valid_addr )?;

            messages.push(msg);
        } else {
            let msg = withdrawal_msg(Asset {
                amount: excess_repayment,
                ..basket.clone().credit_asset
            }, info.clone().sender )?;

            messages.push(msg);
        }                                
    }

    //Subtract paid debt from debt-per-asset tallies
    update_basket_debt(
        storage,
        env,
        querier,
        config,
        &mut basket,
        target_position.collateral_assets,
        credit_asset.amount - excess_repayment,
        false,
        false,
    )?;

    //Save updated repayment price and debts
    BASKET.save(storage, &basket)?;

    //Check that state was saved correctly
    check_repay_state(
        storage,
        credit_asset.amount, 
        prev_credit_amount, 
        position_id, 
        valid_owner_addr
    )?;
    
    Ok(Response::new()
        .add_messages(messages)
        .add_attributes(vec![
            attr("method", "repay"),
            attr("position_id", position_id),
            attr("loan_amount", target_position.credit_amount),
    ]))
}

fn check_repay_state(
    storage: &mut dyn Storage,
    repay_amount: Uint128,
    prev_credit_amount: Uint128,
    position_id: Uint128,
    position_owner: Addr,
) -> Result<(), ContractError>{

    //Get target_position
    let (_i, target_position) = get_target_position(storage, position_owner.clone(), position_id.clone())?;

    if repay_amount >= prev_credit_amount { 
        if target_position.credit_amount != Uint128::zero() {
            return Err(ContractError::CustomError { val: String::from("Conditional 1: Possible state error") })
        }
    } else {
        //Assert that credit_amount is equal to the origin - what was repayed
        if target_position.credit_amount != prev_credit_amount - repay_amount {
            return Err(ContractError::CustomError { val: String::from("Conditional 2: Possible state error") })
        }
    }

    Ok(())
}

//This is what the stability pool contract calls to repay for a liquidation and to get its collateral distribution
pub fn liq_repay(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_asset: Asset,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    
    //Fetch position info to repay for
    let liquidation_propagation = LIQUIDATION.load(deps.storage)?;

    //Can only be called by the SP contract
    if config.clone().stability_pool.is_none() || info.sender != config.clone().stability_pool.unwrap(){
        return Err(ContractError::Unauthorized {});
    }

    //These 3 checks shouldn't error we are pulling the ids from state.
    //Would have to be an issue w/ the repay_progation initialization
    let basket: Basket = BASKET.load(deps.storage)?;

    let (_i, target_position) = get_target_position(
        deps.storage, 
        deps.api.addr_validate(&liquidation_propagation.clone().position_info.position_owner)?,
        liquidation_propagation.clone().position_info.position_id,
    )?;
    
    //Position repayment
    let res = match repay(
        deps.storage,
        deps.querier,
        deps.api,
        env.clone(),
        info.clone(),
        liquidation_propagation.clone().position_info.position_id,
        Some(liquidation_propagation.clone().position_info.position_owner.to_string()),
        credit_asset.clone(),
        None,
    ) {
        Ok(res) => res,
        Err(e) => return Err(e),
    };
   
    //Set collateral_assets
    let collateral_assets = target_position.clone().collateral_assets;

    //Get position's cAsset ratios
    let (cAsset_ratios, _) = get_cAsset_ratios(
        deps.storage,
        env.clone(),
        deps.querier,
        collateral_assets.clone(),
        config.clone(),
    )?;
    //Get cAsset prices
    let (_avg_borrow_LTV, _avg_max_LTV, _total_value, cAsset_prices) = get_avg_LTV(
        deps.storage,
        env.clone(),
        deps.querier,
        config.clone(),
        basket.clone(),
        collateral_assets.clone(),
    )?;

    let repay_value = decimal_multiplication(
        Decimal::from_ratio(credit_asset.amount, Uint128::new(1u128)),
        basket.credit_price,
    );

    let mut messages = vec![];
    let mut coins: Vec<Coin> = vec![];
    let mut native_repayment = Uint128::zero();

    //Stability Pool receives pro rata assets
    //Add distribute messages to the message builder, so the contract knows what to do with the received funds
    let mut distribution_assets = vec![];

    //Query SP liq fee
    let sp_liq_fee = query_stability_pool_fee(deps.querier, config.clone().stability_pool.unwrap().to_string())?;

    //Calculate distribution of assets to send from the repaid position
    for (num, cAsset) in collateral_assets.clone().into_iter().enumerate() {

        let collateral_repay_value = decimal_multiplication(repay_value, cAsset_ratios[num]);
        let collateral_repay_amount = decimal_division(collateral_repay_value, cAsset_prices[num]);
        let collateral_w_fee = (decimal_multiplication(collateral_repay_amount, sp_liq_fee+Decimal::one())) * Uint128::new(1u128);

        let repay_amount_per_asset = credit_asset.amount * cAsset_ratios[num];

        //Remove collateral from user's position claims
        update_position_claims(
            deps.storage,
            deps.querier,
            env.clone(),
            liquidation_propagation.clone().position_info.position_id,
            deps.api.addr_validate(&liquidation_propagation.clone().position_info.position_owner)?,
            cAsset.clone().asset.info,
            collateral_w_fee,
        )?;

        //SP Distribution needs list of cAsset's and is pulling the amount from the Asset object
        match cAsset.clone().asset.info {
            AssetInfo::NativeToken { denom: _ } => {
                //Adding each native token to the list of distribution assets
                let asset = Asset {
                    amount: collateral_w_fee,
                    ..cAsset.clone().asset
                };
                //Add to the distribution_for field for native sends
                native_repayment += repay_amount_per_asset;

                distribution_assets.push(asset.clone());
                coins.push(asset_to_coin(asset)?);
            },            
            AssetInfo::Token { address } => { return Err(ContractError::CustomError { val: String::from("Collateral assets are supposed to be native") }) }
        }
    }

    //Adds Native token distribution msg to messages
    let distribution_msg = SP_ExecuteMsg::Distribute {
        distribution_assets: distribution_assets.clone(),
        distribution_asset_ratios: cAsset_ratios, //The distributions are based off cAsset_ratios so they shouldn't change
        distribute_for: native_repayment.clone(),
    };
    //Build the Execute msg w/ the full list of native tokens
    let msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.clone().stability_pool.unwrap().to_string(),
        msg: to_binary(&distribution_msg)?,
        funds: coins,
    });

    messages.push(msg);

    Ok(res
        .add_messages(messages)
        .add_attribute("method", "liq_repay")
        .add_attribute("distribution_assets", format!("{:?}", distribution_assets))
        .add_attribute("distribute_for", native_repayment.clone()))
}

pub fn increase_debt(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    position_id: Uint128,
    amount: Option<Uint128>,
    LTV: Option<Decimal>,
    mint_to_addr: Option<String>,
) -> Result<Response, ContractError> {
    let config: Config = CONFIG.load(deps.storage)?;
    let mut basket: Basket = BASKET.load(deps.storage)?;

    //Check if frozen
    if basket.frozen { return Err(ContractError::Frozen {  }) }

    //Get Target position
    let (position_index, mut target_position) = get_target_position(deps.storage, info.clone().sender, position_id)?;

    //Accrue interest
    accrue(
        deps.storage,
        deps.querier,
        env.clone(),
        &mut target_position,
        &mut basket,
        info.clone().sender.to_string(),
    )?;

    //Set prev_credit_amount
    let prev_credit_amount = target_position.credit_amount;

    //Set amount
    let amount = match amount {
        Some(amount) => amount,
        None => {
            if let Some(LTV) = LTV {
                get_amount_from_LTV(deps.storage, deps.querier, env.clone(), config.clone(), target_position.clone(), basket.clone(), LTV)?
            } else {
                return Err(ContractError::CustomError { val: String::from("If amount isn't passed, LTV must be passed") })
            }            
        }
    };

    //Add new credit_amount
    target_position.credit_amount += amount;

    //Test for minimum debt requirements
    if decimal_multiplication(
        Decimal::from_ratio(target_position.credit_amount, Uint128::new(1u128)),
        basket.credit_price,
    ) < Decimal::from_ratio(config.debt_minimum, Uint128::new(1u128))
    {
        return Err(ContractError::BelowMinimumDebt {});
    }

    let message: CosmosMsg;

    //Can't take credit before an oracle is set
    if basket.oracle_set {
        //If resulting LTV makes the position insolvent, error. If not construct mint msg
        if insolvency_check(
            deps.storage,
            env.clone(),
            deps.querier,
            basket.clone(),
            target_position.clone().collateral_assets,
            Decimal::from_ratio(target_position.credit_amount, Uint128::new(1u128)),
            basket.credit_price,
            true,
            config.clone(),
        )? .0 {
            return Err(ContractError::PositionInsolvent {});
        } else {
            //Set recipient
            let recipient = {
                if let Some(mint_to) = mint_to_addr {
                    deps.api.addr_validate(&mint_to)?
                } else {
                    info.clone().sender
                }
            };
            message = credit_mint_msg(
                config.clone(),
                Asset {
                    amount,
                    ..basket.clone().credit_asset
                },
                recipient,
            )?;

            //Add credit amount to the position
            //Update Position
            POSITIONS.update(deps.storage, info.clone().sender, |positions: Option<Vec<Position>>| -> Result<Vec<Position>, ContractError> {
                let mut updating_positions = positions.unwrap();
                updating_positions[position_index] = target_position.clone();

                Ok(updating_positions)
            })?;

            //Add new debt to debt-per-asset tallies
            update_basket_debt(
                deps.storage,
                env,
                deps.querier,
                config,
                &mut basket,
                target_position.collateral_assets,
                amount,
                true,
                false,
            )?;
            
            //Save updated repayment price and debts
            BASKET.save(deps.storage, &basket)?;
        }
    } else {
        return Err(ContractError::NoRepaymentPrice {});
    }

    //Check state changes
    check_debt_increase_state(
        deps.storage, 
        amount, 
        prev_credit_amount, 
        position_id, 
        info.sender,
    )?;

    let response = Response::new()
        .add_message(message)
        .add_attribute("method", "increase_debt")
        .add_attribute("position_id", position_id.to_string())
        .add_attribute("total_loan", target_position.credit_amount.to_string())
        .add_attribute("increased_by", amount.to_string());

    Ok(response)
}

fn check_debt_increase_state(
    storage: &mut dyn Storage,
    increase_amount: Uint128,
    prev_credit_amount: Uint128,
    position_id: Uint128,
    position_owner: Addr,  
) -> Result<(), ContractError>{
    
    //Get target_position
    let (_i, target_position) = get_target_position(storage, position_owner.clone(), position_id.clone())?;

    //Assert that credit_amount is equal to the origin + what was added
    if target_position.credit_amount != prev_credit_amount + increase_amount {
        return Err(ContractError::CustomError { val: String::from("Conditional 1: Possible state error") })
    }

    Ok(())
}

//Sell position collateral to repay debts
pub fn close_position(
    deps: DepsMut, 
    env: Env,
    info: MessageInfo,
    position_id: Uint128,
    max_spread: Decimal,
    mut send_to: Option<String>,
) -> Result<Response, ContractError>{

    //Load Config
    let config: Config = CONFIG.load(deps.storage)?;

    //Load Basket
    let basket: Basket = BASKET.load(deps.storage)?;

    //Load target_position
    let (_i, target_position) = get_target_position(deps.storage, info.clone().sender, position_id)?;

    //Calc collateral to sell
    //credit_amount * credit_price * (1 + max_spread)
    let total_collateral_value_to_sell = {
        decimal_multiplication(
            Decimal::from_ratio(target_position.credit_amount, Uint128::new(1)), 
            decimal_multiplication(basket.clone().credit_price, (max_spread + Decimal::one()))
        )
    };
    //Max_spread is added to the collateral amount to ensure enough credit is purchased
    //Excess gets sent back to the position_owner during repayment

    //Get cAsset_ratios for the target_position
    let (cAsset_ratios, cAsset_prices) = get_cAsset_ratios(deps.storage, env.clone(), deps.querier, target_position.clone().collateral_assets, config.clone())?;

    let mut submessages = vec![];
    let mut lp_withdraw_messages: Vec<CosmosMsg> = vec![];
    let mut withdrawn_assets = vec![];

    //Calc collateral_amount_to_sell per asset & create router msg
    for (i, _collateral_ratio) in cAsset_ratios.clone().into_iter().enumerate(){

        //Calc collateral_amount_to_sell
        let mut collateral_amount_to_sell = {
        
            let collateral_value_to_sell = decimal_multiplication(total_collateral_value_to_sell, cAsset_ratios[i]);
            
            decimal_division(collateral_value_to_sell, cAsset_prices[i]) * Uint128::new(1u128)
        };

        //Collateral to sell can't be more than the position owns
        if collateral_amount_to_sell > target_position.collateral_assets.clone()[i].asset.amount {
            collateral_amount_to_sell = target_position.collateral_assets.clone()[i].asset.amount;
        }

        //Set collateral asset
        let collateral_asset = target_position.clone().collateral_assets[i].clone().asset;

        //Add collateral_amount to list for propagation
        withdrawn_assets.push(Asset{
            amount: collateral_amount_to_sell,
            ..collateral_asset.clone()
        });

        //If cAsset is an LP, split into pool assets to sell
        if let Some(pool_info) = target_position.clone().collateral_assets[i].clone().pool_info{

            let (msg, share_asset_amounts) = pool_query_and_exit(
                deps.querier, 
                env.clone(), 
                config.clone().osmosis_proxy.unwrap().to_string(), 
                pool_info.pool_id,
                collateral_amount_to_sell,
            )?;

            //Push LP Withdrawal Msg
            //Comment to pass tests
            lp_withdraw_messages.push(msg);
            
            //Create Router SubMsgs for each pool_asset
            for (i, pool_asset) in pool_info.asset_infos.into_iter().enumerate(){                
                let router_msg = create_router_msg_to_buy_credit_and_repay(
                    env.contract.address.to_string(), 
                    config.clone().dex_router.unwrap().to_string(), 
                    basket.clone().credit_asset.info, 
                    pool_asset.clone().info, 
                    Uint128::from_str(&share_asset_amounts[i].clone().amount).unwrap(), 
                    position_id.clone(), 
                    info.clone().sender, 
                    Some(max_spread), 
                    send_to.clone()
                )?;

                let router_sub_msg = SubMsg::reply_on_success(router_msg, CLOSE_POSITION_REPLY_ID);
                submessages.push(router_sub_msg);                
            }                  
        } else {        
            //Create router subMsg to sell and repay, reply on success
            let router_msg: CosmosMsg = create_router_msg_to_buy_credit_and_repay(
                env.clone().contract.address.to_string(), 
                config.clone().dex_router.unwrap().to_string(), 
                basket.clone().credit_asset.info, 
                collateral_asset.clone().info, 
                collateral_amount_to_sell, 
                position_id.clone(), 
                info.clone().sender, 
                Some(max_spread), 
                send_to.clone()
            )?;

            let router_sub_msg = SubMsg::reply_on_success(router_msg, CLOSE_POSITION_REPLY_ID);
            submessages.push(router_sub_msg);
        }
    }

    //Set send_to for WithdrawMsg in Reply
    if send_to.is_none() {
        send_to = Some(info.clone().sender.to_string());
    }
    
    //Save CLOSE_POSITION_PROPAGATION
    CLOSE_POSITION.save(deps.storage, &ClosePositionPropagation {
        withdrawn_assets,
        position_info: UserInfo { 
            position_id: position_id.clone(), 
            position_owner: info.clone().sender.to_string(),
        },
        send_to: send_to.clone(),
    })?;
    
    Ok(Response::new()
        .add_messages(lp_withdraw_messages)
        .add_submessages(submessages).add_attributes(vec![
        attr("position_id", position_id),
        attr("user", info.clone().sender),
    ])) //If the sale incurred slippage and couldn't repay through the debt minimum, the subsequent withdraw msg will error and revert state 
}

fn create_router_msg_to_buy_credit_and_repay(
    positions_contract: String,
    apollo_router_addr: String,
    credit_asset: AssetInfo, //Credit asset
    asset_to_sell: AssetInfo, 
    amount_to_sell: Uint128,
    position_id: Uint128,
    position_owner: Addr,
    max_spread: Option<Decimal>,
    send_to: Option<String>,
) -> StdResult<CosmosMsg>{
    let hook_msg = Some(to_binary(&ExecuteMsg::Repay {
        position_id,
        position_owner: Some(position_owner.to_string()),
        send_excess_to: send_to.clone()
    })?);

    Ok(
        router_native_to_native(
            apollo_router_addr, 
            asset_to_sell, 
            credit_asset, 
            max_spread, 
            Some(positions_contract), //Repay credit to positions contract, 
            hook_msg, 
            amount_to_sell.into()
        )?
    )  
}

pub fn create_basket(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    basket_id: Uint128,
    collateral_types: Vec<cAsset>,
    credit_asset: Asset,
    credit_price: Decimal,
    base_interest_rate: Option<Decimal>,
    credit_pool_ids: Vec<u64>,
    liquidity_multiplier_for_debt_caps: Option<Decimal>,
    liq_queue: Option<String>,
) -> Result<Response, ContractError> {
    let config: Config = CONFIG.load(deps.storage)?;

    //Only contract owner can create new baskets. This will likely be governance.
    if info.sender != config.owner {
        return Err(ContractError::NotContractOwner {});
    }
    //One basket per contract
    if let Ok(_basket) = BASKET.load(deps.storage){
        return Err(ContractError::CustomError { val: String::from("Only one basket per contract") })
    }

    let mut new_assets = collateral_types.clone();
    let mut collateral_supply_caps = vec![];
    let mut msgs: Vec<CosmosMsg> = vec![];

    let mut new_liq_queue: Option<Addr> = None;
    if liq_queue.is_some() {
        new_liq_queue = Some(deps.api.addr_validate(&liq_queue.clone().unwrap())?);
    }

    //Minimum viable cAsset parameters
    for (i, asset) in collateral_types.iter().enumerate() {
        new_assets[i].asset.amount = Uint128::zero();
        new_assets[i].rate_index = Decimal::one();

        if asset.max_borrow_LTV >= asset.max_LTV
            && asset.max_borrow_LTV
                >= Decimal::from_ratio(Uint128::new(100u128), Uint128::new(1u128))
        {
            return Err(ContractError::CustomError {
                val: "Max borrow LTV can't be greater or equal to max_LTV nor equal to 100"
                    .to_string(),
            });
        }

        //No LPs initially. Their pool asset's need to already be added as collateral so they can't come first.
        if asset.pool_info.is_some() {
            return Err(ContractError::CustomError {
                val: String::from("Can't add an LP when creating a basket"),
            });
        } else {
            //Asserting the Collateral Asset has an oracle
            if config.clone().oracle_contract.is_some() {
                //Query Asset Oracle
                deps.querier
                    .query::<AssetResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
                        contract_addr: config.clone().oracle_contract.unwrap().to_string(),
                        msg: to_binary(&OracleQueryMsg::Asset {
                            asset_info: asset.clone().asset.info,
                        })?,
                    }))?;

                //If it errors it means the oracle doesn't exist
            } else {
                return Err(ContractError::CustomError {
                    val: String::from("Need to setup oracle contract before adding assets"),
                });
            }

            //Create Liquidation Queue for basket assets
            if new_liq_queue.clone().is_some() {
                //Gets Liquidation Queue max premium.
                //The premium has to be at most 5% less than the difference between max_LTV and 100%
                //The ideal variable for the 5% is the avg caller_liq_fee during high traffic periods
                let max_premium = match Uint128::new(95u128).checked_sub( asset.max_LTV * Uint128::new(100u128) ){
                    Ok( diff ) => diff,
                    //A default to 10 assuming that will be the lowest sp_liq_fee
                    Err( _err ) => Uint128::new(10u128) 
                    ,
                };
                //We rather the LQ liquidate than the SP if possible so its max_premium will be at most the sp_liq fee...
                //..if the first subtraction fails.
                //If it failed, allowing the LQ premium to be more than the SP fee means less efficient liquidations..
                //Since we are aiming for lowest possible fee

                msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: new_liq_queue.clone().unwrap().to_string(),
                    msg: to_binary(&LQ_ExecuteMsg::AddQueue {
                        bid_for: asset.clone().asset.info,
                        max_premium,
                        bid_threshold: Uint256::from(1_000_000_000_000u128), //1 million
                    })?,
                    funds: vec![],
                }));
            }
        }

        let mut lp = false;
        if asset.pool_info.is_some() {
            lp = true;
        }
        //Push the cAsset's asset info
        collateral_supply_caps.push(SupplyCap {
            asset_info: asset.clone().asset.info,
            current_supply: Uint128::zero(),
            supply_cap_ratio: Decimal::zero(),
            debt_total: Uint128::zero(),
            lp,
            stability_pool_ratio_for_debt_cap: None,
        });
    }

    //Set Basket fields
    let base_interest_rate = base_interest_rate.unwrap_or_else(|| Decimal::percent(0));
    let liquidity_multiplier = liquidity_multiplier_for_debt_caps.unwrap_or_else(|| Decimal::one());

    let new_basket: Basket = Basket {
        basket_id: basket_id.clone(),
        current_position_id: Uint128::from(1u128),
        collateral_types: new_assets,
        collateral_supply_caps,
        multi_asset_supply_caps: vec![],
        credit_asset: credit_asset.clone(),
        credit_price,
        base_interest_rate,
        liquidity_multiplier: liquidity_multiplier.clone(),
        pending_revenue: Uint128::zero(),
        credit_last_accrued: env.block.time.seconds(),
        rates_last_accrued: env.block.time.seconds(),
        liq_queue: new_liq_queue,
        negative_rates: true,
        cpc_margin_of_error: Decimal::one(),
        oracle_set: false,
        frozen: false,
        rev_to_stakers: true,
    };

    //Denom check
    if let AssetInfo::Token { address } = credit_asset.clone().info {
        return Err(ContractError::CustomError {
            val: "Basket credit must be a native token denom".to_string(),
        });
    }

    //Add asset to liquidity check contract
    //Liquidity AddAsset Msg
    let mut msgs = vec![];
    if let Some(liquidity_contract) = config.clone().liquidity_contract {
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: liquidity_contract.to_string(),
            msg: to_binary(&LiquidityExecuteMsg::AddAsset {
                asset: LiquidityInfo {
                    asset: new_basket.clone().credit_asset.info,
                    pool_ids: credit_pool_ids,
                },
            })?,
            funds: vec![],
        }));
    }

    //Save Basket
    BASKET.save( deps.storage, &new_basket )?;

    //Response Building
    let response = Response::new();

    Ok(response
        .add_attributes(vec![
            attr("method", "create_basket"),
            attr("basket_id", basket_id),
            attr("credit_asset", credit_asset.to_string()),
            attr("credit_price", credit_price.to_string()),
            attr(
                "liq_queue",
                liq_queue.unwrap_or_else(|| String::from("None")),
            ),
        ])
        .add_messages(msgs))
}

pub fn edit_basket(
    //Can't edit basket id, current_position_id or credit_asset.
    //Credit price can only be changed thru the accrue function.
    deps: DepsMut,
    info: MessageInfo,
    editable_parameters: EditBasket,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    let mut new_queue: Option<Addr> = None;
    if let Some(liq_queue) = editable_parameters.clone().liq_queue {
        new_queue = Some(deps.api.addr_validate(&liq_queue)?);
    }

    //Blank cAsset
    //This never gets added unless its edited. Here due to uninitialized errors.
    let mut new_cAsset = cAsset {
        asset: Asset {
            info: AssetInfo::NativeToken {
                denom: String::from("None"),
            },
            amount: Uint128::zero(),
        },
        max_borrow_LTV: Decimal::zero(),
        max_LTV: Decimal::zero(),
        pool_info: None,
        rate_index: Decimal::one(),
    };

    let mut msgs: Vec<CosmosMsg> = vec![];

    let mut basket = BASKET.load(deps.storage)?;
    //cAsset check
    if let Some(added_cAsset) = editable_parameters.clone().added_cAsset {
        let mut check = true;
        new_cAsset = added_cAsset.clone();

        //new_cAsset can't be the basket credit_asset or MBRN 
        if let Some(staking_contract) = config.clone().staking_contract {
            let mbrn_denom = deps.querier.query::<Staking_Config>(&QueryRequest::Wasm(WasmQuery::Smart { 
                contract_addr: staking_contract.to_string(), 
                msg: to_binary(&Staking_QueryMsg::Config { })? 
            }))?
            .mbrn_denom;

            if new_cAsset.asset.info.to_string() == mbrn_denom {
                return Err(ContractError::InvalidCollateral {  } )
            }
        }
        if new_cAsset.asset.info == basket.clone().credit_asset.info {
            return Err(ContractError::InvalidCollateral {  } )
        }
        ////
        
        //Each cAsset has to initialize amount as 0..
        new_cAsset.asset.amount = Uint128::zero();
        
        //..and index at 1
        new_cAsset.rate_index = Decimal::one();

        //No duplicates
        if let Some(_duplicate) = basket
            .clone()
            .collateral_types
            .into_iter()
            .find(|cAsset| cAsset.asset.info.equal(&new_cAsset.asset.info))
        {
            return Err(ContractError::CustomError {
                val: format!(
                    "Attempting to add duplicate asset: {}",
                    new_cAsset.asset.info
                ),
            });
        }

        if let Some(mut pool_info) = added_cAsset.clone().pool_info {

            //Query share asset amount
            let pool_state = match deps.querier.query::<PoolStateResponse>(&QueryRequest::Wasm(
                WasmQuery::Smart {
                    contract_addr: config.clone().osmosis_proxy.unwrap().to_string(),
                    msg: match to_binary(&OsmoQueryMsg::PoolState {
                        id: pool_info.pool_id,
                    }) {
                        Ok(binary) => binary,
                        Err(err) => {
                            return Err(ContractError::CustomError {
                                val: err.to_string(),
                            })
                        }
                    },
                },
            )) {
                Ok(resp) => resp,
                Err(err) => {
                    return Err(ContractError::CustomError {
                        val: err.to_string(),
                    })
                }
            };
            let pool_assets = pool_state.assets;

            //Set correct shares denom
            new_cAsset.asset.info = AssetInfo::NativeToken {
                denom: pool_state.shares.denom,
            };

            //Assert Asset order of pool_assets in PoolInfo object
            //Assert pool_assets are already in the basket, which confirms an oracle and adequate parameters for them
            for (i, asset) in pool_assets.iter().enumerate() {

                //Set pool assets 
                pool_info.asset_infos[i].info = AssetInfo::NativeToken { denom: asset.clone().denom };               
               
                //Asserting that its pool assets are already added as collateral types
                if let None = basket.clone().collateral_types.into_iter().find(|cAsset| {
                    cAsset.asset.info.equal(&AssetInfo::NativeToken {
                        denom: asset.clone().denom,
                    })
                }) {
                    return Err(ContractError::CustomError {
                        val: format!(
                            "Need to add all pool assets before adding the LP. Errored on {}",
                            asset.denom
                        ),
                    });
                }
            }

            //Update pool_info
            new_cAsset.pool_info = Some(pool_info);

        } else {
            //Asserting the Collateral Asset has an oracle
            if config.clone().oracle_contract.is_some() {
                //Query Asset Oracle
                deps.querier
                    .query::<AssetResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
                        contract_addr: config.clone().oracle_contract.unwrap().to_string(),
                        msg: to_binary(&OracleQueryMsg::Asset {
                            asset_info: new_cAsset.clone().asset.info,
                        })?,
                    }))?;

                //If it errors it means the oracle doesn't exist
            } else {
                return Err(ContractError::CustomError {
                    val: String::from("Need to setup oracle contract before adding assets"),
                });
            }
        }        

        //Create Liquidation Queue for the asset
        if basket.clone().liq_queue.is_some() {
            //Gets Liquidation Queue max premium.
            //The premium has to be at most 5% less than the difference between max_LTV and 100%
            //The ideal variable for the 5% is the avg caller_liq_fee during high traffic periods
            let max_premium = match Uint128::new(95u128).checked_sub( new_cAsset.max_LTV * Uint128::new(100u128) ){
                Ok( diff ) => diff,
                //A default to 10 assuming that will be the lowest sp_liq_fee
                Err( _err ) => Uint128::new(10u128) 
                ,
            };

            msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: basket.clone().liq_queue.unwrap().into_string(),
                msg: to_binary(&LQ_ExecuteMsg::AddQueue {
                    bid_for: new_cAsset.clone().asset.info,
                    max_premium,
                    bid_threshold: Uint256::from(1_000_000_000_000u128), //1 million
                })?,
                funds: vec![],
            }));
        } else if let Some(new_queue) = new_queue.clone() {
            //Gets Liquidation Queue max premium.
            //The premium has to be at most 5% less than the difference between max_LTV and 100%
            //The ideal variable for the 5% is the avg caller_liq_fee during high traffic periods
            let max_premium = match Uint128::new(95u128).checked_sub( new_cAsset.max_LTV * Uint128::new(100u128) ){
                Ok( diff ) => diff,
                //A default to 10 assuming that will be the lowest sp_liq_fee
                Err( _err ) => Uint128::new(10u128) 
                ,
            };

            msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: new_queue.clone().into_string(),
                msg: to_binary(&LQ_ExecuteMsg::AddQueue {
                    bid_for: new_cAsset.clone().asset.info,
                    max_premium,
                    bid_threshold: Uint256::from(1_000_000_000_000u128), //1 million
                })?,
                funds: vec![],
            }));
        }

        //..needs minimum viable LTV parameters
        if new_cAsset.max_borrow_LTV >= new_cAsset.max_LTV
            || new_cAsset.max_borrow_LTV
                >= Decimal::from_ratio(Uint128::new(100u128), Uint128::new(1u128))
        {
            check = false;
        }

        if !check {
            return Err(ContractError::CustomError {
                val: "Max borrow LTV can't be greater or equal to max_LTV nor equal to 100"
                    .to_string(),
            });
        }

        let mut lp = false;
        if new_cAsset.pool_info.is_some() {
            lp = true;
        }
        //Push the cAsset's asset info
        basket.collateral_supply_caps.push(SupplyCap {
            asset_info: new_cAsset.clone().asset.info,
            current_supply: Uint128::zero(),
            supply_cap_ratio: Decimal::zero(),
            debt_total: Uint128::zero(),
            lp,
            stability_pool_ratio_for_debt_cap: None,
        });
    }
    
    //Save basket's new collateral_supply_caps
    BASKET.save(deps.storage, &basket)?;

    //Send credit_asset TWAP info to Oracle Contract
    let mut oracle_set = basket.oracle_set;

    if let Some(credit_twap) = editable_parameters.clone().credit_asset_twap_price_source {
        if let Some(oracle_contract) = config.clone().oracle_contract {
            //Set the credit Oracle. Using EditAsset updates or adds.
            msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: oracle_contract.to_string(),
                msg: to_binary(&OracleExecuteMsg::EditAsset {
                    asset_info: basket.clone().credit_asset.info,
                    oracle_info: Some(AssetOracleInfo {
                        basket_id: basket.clone().basket_id,
                        osmosis_pools_for_twap: vec![credit_twap],
                        static_price: None,
                    }),
                    remove: false,
                })?,
                funds: vec![],
            }));

            oracle_set = true;
        }
    };
    let mut attrs = vec![attr("method", "edit_basket")];

    //Create EditAssetMsg for Liquidity contract
    if let Some(pool_ids) = editable_parameters.clone().credit_pool_ids {
        attrs.push(attr("new_pool_ids", format!("{:?}", pool_ids.clone())));

        if let Some(liquidity_contract) = config.clone().liquidity_contract {
            msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: liquidity_contract.to_string(),
                msg: to_binary(&LiquidityExecuteMsg::EditAsset {
                    asset: LiquidityInfo {
                        asset: basket.clone().credit_asset.info,
                        pool_ids,
                    },
                })?,
                funds: vec![],
            }));
        }
    }

    //Update Basket
    BASKET.update(deps.storage, |mut basket| -> Result<Basket, ContractError> {
        if info.sender.clone() != config.owner {
            return Err(ContractError::Unauthorized {});
        } else { //Set all optional parameters & append to Response attributes

            if editable_parameters.clone().added_cAsset.is_some() {
                basket.collateral_types.push(new_cAsset.clone());
                attrs.push(attr(
                    "added_cAsset",
                    new_cAsset.clone().asset.info.to_string(),
                ));
            }
            if editable_parameters.clone().liq_queue.is_some() {
                basket.liq_queue = new_queue.clone();
                attrs.push(attr("new_queue", new_queue.clone().unwrap().to_string()));
            }
            if let Some(collateral_supply_caps) = editable_parameters.clone().collateral_supply_caps {
                //Set new cap parameters
                for new_cap in collateral_supply_caps {
                    if let Some((index, _cap)) = basket.clone().collateral_supply_caps
                        .into_iter()
                        .enumerate()
                        .find(|(_x, cap)| cap.asset_info.equal(&new_cap.asset_info))
                    {
                        //Set supply cap ratio
                        basket.collateral_supply_caps[index].supply_cap_ratio = new_cap.supply_cap_ratio;
                        //Set stability pool based ratio
                        basket.collateral_supply_caps[index].stability_pool_ratio_for_debt_cap = new_cap.stability_pool_ratio_for_debt_cap;
                    }
                }
                attrs.push(attr("new_collateral_supply_caps", String::from("Edited")));
            }
            if let Some(multi_asset_supply_caps) = editable_parameters.clone().multi_asset_supply_caps {
                //Set new cap parameters
                for new_cap in multi_asset_supply_caps {
                    if let Some((index, _cap)) = basket.clone().multi_asset_supply_caps
                        .into_iter()
                        .enumerate()
                        .find(|(_x, cap)| equal(&cap.assets, &new_cap.assets))
                    {
                        //Set supply cap ratio
                        basket.multi_asset_supply_caps[index].supply_cap_ratio = new_cap.supply_cap_ratio;
                    } else {
                        basket.multi_asset_supply_caps.push(new_cap);
                    }
                }
                attrs.push(attr("new_collateral_supply_caps", String::from("Edited")));
            }
            if let Some(base_interest_rate) = editable_parameters.clone().base_interest_rate {
                basket.base_interest_rate = base_interest_rate.clone();
                attrs.push(attr("new_base_interest_rate",base_interest_rate.clone().to_string()));
            }
            if let Some(toggle) = editable_parameters.clone().negative_rates {
                basket.negative_rates = toggle.clone();
                attrs.push(attr("negative_rates", toggle.to_string()));
            }
            if let Some(toggle) = editable_parameters.clone().frozen {
                basket.frozen = toggle.clone();
                attrs.push(attr("frozen", toggle.to_string()));
            }
            if let Some(toggle) = editable_parameters.clone().rev_to_stakers {
                basket.rev_to_stakers = toggle.clone();
                attrs.push(attr("rev_to_stakers", toggle.to_string()));
            }
            if let Some(error_margin) = editable_parameters.clone().cpc_margin_of_error {
                basket.cpc_margin_of_error = error_margin.clone();
                attrs.push(attr("new_cpc_margin_of_error", error_margin.to_string()));
            }
            //Set basket specific multiplier
            if let Some(multiplier) = editable_parameters.clone().liquidity_multiplier {
                basket.liquidity_multiplier = multiplier.clone();
                attrs.push(attr("new_liquidity_multiplier", multiplier.to_string()));
            }

            basket.oracle_set = oracle_set;
        }

        Ok(basket)
    })?;

    Ok(Response::new().add_attributes(attrs).add_messages(msgs))
}

fn get_amount_from_LTV(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    env: Env,
    config: Config,
    position: Position,
    basket: Basket,
    target_LTV: Decimal,
) -> Result<Uint128, ContractError>{
    //Get avg_borrow_LTV & total_value
    let (avg_borrow_LTV, _avg_max_LTV, total_value, _cAsset_prices) = get_avg_LTV(
        storage, 
        env, 
        querier, 
        config, 
        basket.clone(), 
        position.clone().collateral_assets
    )?;

    //Target LTV can't be greater than possible borrowable LTV for the Position
    if target_LTV > avg_borrow_LTV {
        return Err(ContractError::InvalidLTV { target_LTV })
    }

    //Calc current LTV
    let current_LTV = {
        let credit_value = decimal_multiplication(Decimal::from_ratio(position.credit_amount, Uint128::new(1)), basket.clone().credit_price);

        decimal_division(credit_value, total_value)
    };

    //If target_LTV is <= current_LTV there is no room to increase
    if target_LTV <= current_LTV {
        return Err(ContractError::InvalidLTV { target_LTV })
    }

    //Calculate amount of credit to get to target_LTV
    let credit_amount: Uint128 = {        
        //Calc spread between current LTV and target_LTV
        let LTV_spread = target_LTV - current_LTV;

        //Calc the value LTV_spread represents
        let increased_credit_value = decimal_multiplication(total_value, LTV_spread);
        
        decimal_division(increased_credit_value, basket.clone().credit_price) * Uint128::new(1)
    };

    Ok( credit_amount )
}

pub fn update_position(
    storage: &mut dyn Storage,
    valid_position_owner: Addr,
    new_position: Position,
) -> StdResult<()>{

    POSITIONS.update(
        storage,
        valid_position_owner.clone(),
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
                                //Save stored_positon
                                stored_position
                            }
                        })
                        .collect::<Vec<Position>>();

                    Ok(new_positions)
                },
                None => {
                    return Err(StdError::GenericErr {
                        msg: "Invalid position owner".to_string(),
                    })
                }
            }
        },
    )?;

    Ok(())
}


//Checks if any Basket caps are set to 0
//If so the withdrawal assets have to either fully withdraw the asset from the position or only withdraw said asset
//Otherwise users could just fully withdrawal other assets and create a new position
//In a LUNA situation this would leave debt backed by an asset whose solvency Membrane has no faith in
fn check_for_expunged(
    position_assets: Vec<cAsset>,
    withdrawal_assets: Vec<cAsset>,
    basket: Basket
)-> StdResult<()>{
    //Extract the Asset from the cAssets
    let position_assets: Vec<Asset> = position_assets
        .into_iter()
        .map(|cAsset| cAsset.asset)
        .collect::<Vec<Asset>>();

    let withdrawal_assets: Vec<Asset> = withdrawal_assets
        .into_iter()
        .map(|cAsset| cAsset.asset)
        .collect::<Vec<Asset>>();

    let mut passed = true;
    let mut invalid_withdraws = vec![];

    //For any supply cap at 0
    for cap in basket.clone().collateral_supply_caps {

        if cap.supply_cap_ratio.is_zero(){

            //If in the position
            if let Some( asset ) = position_assets.clone().into_iter().find(|asset| asset.info.equal(&cap.asset_info)){

                //Withdraw asset has to either..
                //1) Only withdraw the asset
                if withdrawal_assets[0].info.equal(&asset.info) && withdrawal_assets.len() == 1 as usize{
                    passed = true;
                
                //2) Fully withdraw the asset
                } else if let Some( withdrawal_asset ) = withdrawal_assets.clone().into_iter().find(|w_asset| w_asset.info.equal(&asset.info)){

                    if withdrawal_asset.amount == asset.amount {
                        passed = true;
                    }else {
                        passed = false;
                        invalid_withdraws.push( asset.info.to_string() );
                    } 
                } else {
                    passed = false;
                    invalid_withdraws.push( asset.info.to_string() );
                }
            }
        }
    }
    if !passed {
        return Err( StdError::GenericErr { msg: format!("These assets need to be expunged from the positon: {:?}", invalid_withdraws) } )
    }

    Ok(())
}

//Create Position instance
pub fn create_position(
    cAssets: Vec<cAsset>, //Assets being added into the position
    basket: &mut Basket,
) -> Result<Position, ContractError> {   

    let new_position = Position {
        position_id: basket.current_position_id,
        collateral_assets: cAssets.clone(),
        credit_amount: Uint128::zero(),
    };

    //increment position id
    basket.current_position_id += Uint128::from(1u128);

    return Ok(new_position);
}


pub fn credit_mint_msg(
    config: Config,
    credit_asset: Asset,
    recipient: Addr,
) -> StdResult<CosmosMsg> {
    match credit_asset.clone().info {
        AssetInfo::Token { address: _ } => {
            return Err(StdError::GenericErr {
                msg: "Credit has to be a native token".to_string(),
            })
        }
        AssetInfo::NativeToken { denom } => {
            if config.osmosis_proxy.is_some() {
                let message = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.osmosis_proxy.unwrap().to_string(),
                    msg: to_binary(&OsmoExecuteMsg::MintTokens {
                        denom,
                        amount: credit_asset.amount,
                        mint_to_address: recipient.to_string(),
                    })?,
                    funds: vec![],
                });
                Ok(message)
            } else {
                return Err(StdError::GenericErr {
                    msg: "No proxy contract setup".to_string(),
                });
            }
        }
    }
}

pub fn credit_burn_msg(
    config: Config, 
    env: Env, 
    credit_asset: Asset,
    basket: &mut Basket,
) -> StdResult<Vec<CosmosMsg>> {

    //Calculate the amount to burn
    let (burn_amount, revenue_amount) = {
        //Is revenue being sent to stakers? If so, calculate
        if !basket.rev_to_stakers {
            (credit_asset.clone().amount, Uint128::zero())
        } else if !basket.pending_revenue.is_zero(){
            
            if basket.pending_revenue >= credit_asset.amount {
                (Uint128::zero(), credit_asset.clone().amount)
            } else {
                let burn = credit_asset.amount - basket.pending_revenue;
                (burn, basket.pending_revenue)
            }

        } else {
            (credit_asset.clone().amount, Uint128::zero())
        }
        
    };
    //Update pending_revenue
    basket.pending_revenue -= revenue_amount;

    //Initialize messages
    let mut messages = vec![];
    
    if let AssetInfo::NativeToken { denom } = credit_asset.clone().info {
        if let Some(addr) = config.osmosis_proxy{
            //Create burn msg
            let burn_message = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: addr.to_string(),
                msg: to_binary(&OsmoExecuteMsg::BurnTokens {
                    denom,
                    amount: burn_amount,
                    burn_from_address: env.contract.address.to_string(),
                })?,
                funds: vec![],
            });
            messages.push(burn_message);

            //Create DepositFee Msg
            if !revenue_amount.is_zero() && config.staking_contract.is_some(){
                let rev_message = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.staking_contract.unwrap().to_string(),
                    msg: to_binary(&Staking_ExecuteMsg::DepositFee { })?,
                    funds: vec![ asset_to_coin(Asset {
                        amount: revenue_amount,
                        ..credit_asset.clone()
                    })? ],
                });
                messages.push(rev_message);
            }

            Ok(messages)
        } else {
            return Err(StdError::GenericErr { msg: "No proxy contract setup".to_string()});
        }
    } else { return Err(StdError::GenericErr { msg: "Cw20 assets aren't allowed".to_string() }) }
}

pub fn get_avg_LTV(
    storage: &mut dyn Storage,
    env: Env,
    querier: QuerierWrapper,
    config: Config,
    basket: Basket,
    collateral_assets: Vec<cAsset>,
) -> StdResult<(Decimal, Decimal, Decimal, Vec<Decimal>)> {
    //Get total value
    let (cAsset_values, cAsset_prices) = get_asset_values(
        storage,
        env,
        querier,
        collateral_assets.clone(),
        config,
    )?;
    let total_value: Decimal = cAsset_values.iter().sum();

    //getting each cAsset's % of total value
    let mut cAsset_ratios: Vec<Decimal> = vec![];
    for cAsset in cAsset_values {
        if total_value == Decimal::zero() {
            cAsset_ratios.push(Decimal::zero());
        } else {
            cAsset_ratios.push(decimal_division(cAsset, total_value));
        }
    }

    //Converting % of value to avg_LTV by multiplying collateral LTV by % of total value
    let mut avg_max_LTV: Decimal = Decimal::zero();
    let mut avg_borrow_LTV: Decimal = Decimal::zero();

    if cAsset_ratios.len() == 0 {
        return Ok((
            Decimal::percent(0),
            Decimal::percent(0),
            Decimal::percent(0),
            vec![],
        ));        
    }

    //Skip unecessary calculations if length is 1
    if cAsset_ratios.len() == 1 {
        return Ok((
            collateral_assets[0].max_borrow_LTV,
            collateral_assets[0].max_LTV,
            total_value,
            cAsset_prices,
        ));
    }

    for (i, _cAsset) in collateral_assets.clone().iter().enumerate() {
        avg_borrow_LTV +=
            decimal_multiplication(cAsset_ratios[i], collateral_assets[i].max_borrow_LTV);
    }

    for (i, _cAsset) in collateral_assets.clone().iter().enumerate() {
        avg_max_LTV += decimal_multiplication(cAsset_ratios[i], collateral_assets[i].max_LTV);
    }

    Ok((avg_borrow_LTV, avg_max_LTV, total_value, cAsset_prices))
}

//Gets position cAsset ratios
pub fn get_cAsset_ratios(
    storage: &mut dyn Storage,
    env: Env,
    querier: QuerierWrapper,
    collateral_assets: Vec<cAsset>,
    config: Config,
) -> StdResult<(Vec<Decimal>, Vec<Decimal>)> {
    let (cAsset_values, cAsset_prices) = get_asset_values(
        storage,
        env,
        querier,
        collateral_assets.clone(),
        config,
    )?;
    let total_value: Decimal = cAsset_values.iter().sum();

    //getting each cAsset's % of total value
    let mut cAsset_ratios: Vec<Decimal> = vec![];
    for cAsset in cAsset_values {
        if total_value == Decimal::zero() {
            cAsset_ratios.push(Decimal::zero());
        } else {
            cAsset_ratios.push(decimal_division(cAsset, total_value));
        }
    }

    Ok((cAsset_ratios, cAsset_prices))
}

pub fn insolvency_check(
    //Returns true if insolvent, current_LTV and available fee (in %) to the caller if insolvent
    storage: &mut dyn Storage,
    env: Env,
    querier: QuerierWrapper,
    basket: Basket,
    collateral_assets: Vec<cAsset>,
    credit_amount: Decimal,
    credit_price: Decimal,
    max_borrow: bool, //Toggle for either over max_borrow or over max_LTV (liquidatable), ie taking the minimum collateral ratio into account.
    config: Config,
) -> StdResult<(bool, Decimal, Uint128)> {//insolvent, current_LTV, available_fee

    //No assets but still has debt, return insolvent and skip other checks
    let total_assets: Uint128 = collateral_assets
        .iter()
        .map(|asset| asset.asset.amount)
        .collect::<Vec<Uint128>>()
        .iter()
        .sum();
    if total_assets.is_zero() && !credit_amount.is_zero() {
        return Ok((true, Decimal::percent(100), Uint128::zero()));
    }

    let avg_LTVs: (Decimal, Decimal, Decimal, Vec<Decimal>) =
        get_avg_LTV(storage, env, querier, config, basket, collateral_assets)?;

    let asset_values: Decimal = avg_LTVs.2; //pulls total_asset_value

    let check: bool;
    let current_LTV = if asset_values.is_zero() {
        Decimal::percent(100)
    } else {
        decimal_division(
            decimal_multiplication(credit_amount, credit_price),
            asset_values,
        )
    };

    match max_borrow {
        true => {
            //Checks max_borrow
            check = if asset_values.is_zero() && credit_amount.is_zero() {
                false
            } else {
                current_LTV > avg_LTVs.0
            };
        }
        false => {
            //Checks max_LTV
            check = if asset_values.is_zero() && credit_amount.is_zero() {
                false
            } else {
                current_LTV > avg_LTVs.1
            };
        }
    }

    let available_fee = if check {
        //Checks max_LTV
        current_LTV.checked_sub(avg_LTVs.1).unwrap_or_else(|_| Decimal::zero()) * Uint128::new(1u128)
    } else {
        Uint128::zero()
    };

    Ok((check, current_LTV, available_fee))
}

pub fn store_price(
    storage: &mut dyn Storage,
    env: Env, 
    asset_token: &AssetInfo,
    mut price: &mut StoredPrice,
) -> StdResult<()> {
    let mut price_bucket: Bucket<StoredPrice> = Bucket::new(storage, PREFIX_PRICE);   
    
    //Set price_vol_limiter
    let time_elapsed = env.block.time.seconds() - price.price_vol_limiter.last_time_updated;
        
    //Store prive_vol_limiter if 5 mins have passed
    if time_elapsed >= 300 {

        price.price_vol_limiter = 
        PriceVolLimiter {
                price: price.clone().price,
                last_time_updated: env.block.time.seconds(),                  
        };
    }
    //Save bucket
    price_bucket.save(&to_binary(asset_token)?, price)
}

pub fn read_price(
    storage: &dyn Storage,
    asset_token: &AssetInfo
) -> StdResult<StoredPrice> {
    let price_bucket: ReadonlyBucket<StoredPrice> = ReadonlyBucket::new(storage, PREFIX_PRICE);
    price_bucket.load(&to_binary(asset_token)?)  
}

fn query_price(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    env: Env,
    config: Config,
    asset_info: AssetInfo,
) -> StdResult<Decimal> {
    //Set variables
    let mut twap_timeframe: u64 = config.collateral_twap_timeframe;
    let basket = BASKET.load(storage)?;
    let price: Decimal; 
    let mut error_or_over_time_limit = false;

    //if AssetInfo is the basket.credit_asset change timeframe
    if asset_info.equal(&basket.credit_asset.info) {
        twap_timeframe = config.credit_twap_timeframe;
    }

    //Try and use a stored price, if within the oracle_time_limit
    let res = read_price(storage, &asset_info);

    if let Ok(stored_price) = res {
        let time_elapsed: u64 = env.block.time.seconds() - stored_price.last_time_updated;
        if time_elapsed <= config.oracle_time_limit {
            return Ok( stored_price.price )
        } 
    } 
    
    //Query new price   
    price = match querier.query::<PriceResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: config.clone().oracle_contract.unwrap().to_string(),
        msg: to_binary(&OracleQueryMsg::Price {
            asset_info: asset_info.clone(),
            twap_timeframe,
            basket_id: None,
        })?,
    })) {
        Ok(res) => {
            //Read price from storage
            if let Ok(stored_price) = read_price(storage, &asset_info){
                ////Make sure price hasn't changed by 20%+ in the oracle_time_limit, if so Error.                
                //Upside
                if decimal_multiplication(stored_price.price, Decimal::percent(120)) <= res.price {
                    return Err(StdError::GenericErr { msg: format!("Oracle price moved >= 20% to the upside in {} minutes, possible bug/manipulation", config.oracle_time_limit) })
                }//Downside
                else if decimal_multiplication(stored_price.price, Decimal::percent(80)) >= res.price {
                    return Err(StdError::GenericErr { msg: format!("Oracle price moved >= 20% to the downside in {} minutes, possible bug/manipulation", config.oracle_time_limit) })
                }
                
                //Store new price
                store_price(
                    storage,
                    env.clone(),
                    &asset_info,
                    &mut StoredPrice {
                        price: res.price,
                        last_time_updated: env.block.time.seconds(),
                        ..stored_price
                    },
                )?;
            }
                        
            //Store new price
            store_price(
                storage,
                env.clone(),
                &asset_info,
                &mut StoredPrice {
                    price: res.price,
                    last_time_updated: env.block.time.seconds(),
                    price_vol_limiter: PriceVolLimiter { 
                        price: res.price, 
                        last_time_updated: env.block.time.seconds(),
                    }
                },
            )?;

            res.price
        }
        Err(_err) => {
            return Err(StdError::GenericErr {
                msg: String::from("Oracle price invalid"),
            });            
        }    
    };
    Ok(price)
}

//Get Asset values / query oracle
pub fn get_asset_values(
    storage: &mut dyn Storage,
    env: Env,
    querier: QuerierWrapper,
    assets: Vec<cAsset>,
    config: Config,
) -> StdResult<(Vec<Decimal>, Vec<Decimal>)> {
    //Getting proportions for position collateral to calculate avg LTV
    //Using the index in the for loop to parse through the assets Vec and collateral_assets Vec
    //, as they are now aligned due to the collateral check w/ the Config's data
    let mut cAsset_values: Vec<Decimal> = vec![];
    let mut cAsset_prices: Vec<Decimal> = vec![];

    if config.clone().oracle_contract.is_some() {
        for (_i, cAsset) in assets.iter().enumerate() {
            //If an Osmosis LP
            if let Some(pool_info) = cAsset.clone().pool_info {
                let mut asset_prices = vec![];

                for (pool_asset) in pool_info.clone().asset_infos {
                    let price = query_price(
                        storage,
                        querier,
                        env.clone(),
                        config.clone(),
                        pool_asset.info,
                    )?;
                    //Append price
                    asset_prices.push(price);
                }

                //Calculate share value
                let cAsset_value = {
                    //Query share asset amount
                    let share_asset_amounts = querier
                        .query::<PoolStateResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
                            contract_addr: config.clone().osmosis_proxy.unwrap().to_string(),
                            msg: to_binary(&OsmoQueryMsg::PoolState {
                                id: pool_info.pool_id,
                            })?,
                        }))?
                        .shares_value(cAsset.asset.amount);

                    //Calculate value of cAsset
                    let mut value = Decimal::zero();
                    for (i, price) in asset_prices.into_iter().enumerate() {
                        //Assert we are pulling asset amount from the correct asset
                        let asset_share =
                            match share_asset_amounts.clone().into_iter().find(|coin| {
                                AssetInfo::NativeToken {
                                    denom: coin.denom.clone(),
                                } == pool_info.clone().asset_infos[i].info
                            }) {
                                Some(coin) => coin,
                                None => {
                                    return Err(StdError::GenericErr {
                                        msg: format!(
                                            "Invalid asset denom: {}",
                                            pool_info.clone().asset_infos[i].info
                                        ),
                                    })
                                }
                            };
                        //Normalize Asset amounts to native token decimal amounts (6 places: 1 = 1_000_000)
                        let exponent_difference = pool_info.clone().asset_infos[i]
                            .decimals
                            .checked_sub(6u64)
                            .unwrap();
                        let asset_amount = asset_share.amount
                            / Uint128::new(10u64.pow(exponent_difference as u32) as u128);
                        let decimal_asset_amount =
                            Decimal::from_ratio(asset_amount, Uint128::new(1u128));

                        //Price * # of assets in LP shares
                        value += decimal_multiplication(price, decimal_asset_amount);
                    }

                    value
                };

                //Calculate LP price
                let cAsset_price = {
                    let share_amount =
                        Decimal::from_ratio(cAsset.asset.amount, Uint128::new(1u128));
                    if !share_amount.is_zero() {
                        decimal_division(cAsset_value, share_amount)
                    } else {
                        Decimal::zero()
                    }
                };

                //Push to price and value list
                cAsset_prices.push(cAsset_price);
                cAsset_values.push(cAsset_value);
            } else {
                let price = query_price(
                    storage,
                    querier,
                    env.clone(),
                    config.clone(),
                    cAsset.clone().asset.info,
                )?;

                cAsset_prices.push(price);
                let collateral_value = decimal_multiplication(
                    Decimal::from_ratio(cAsset.asset.amount, Uint128::new(1u128)),
                    price,
                );
                cAsset_values.push(collateral_value);
            }
        }
    }

    Ok((cAsset_values, cAsset_prices))
}

pub fn update_position_claims(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    env: Env,
    position_id: Uint128,
    position_owner: Addr,
    liquidated_asset: AssetInfo,
    liquidated_amount: Uint128,
) -> StdResult<()> {
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
                return Err(StdError::GenericErr {
                    msg: "Invalid position owner".to_string(),
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
    }];

    let mut basket = BASKET.load(storage)?;
    match update_basket_tally(storage, querier, env, &mut basket, collateral_assets, false) {
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

//Get total pooled amount for an asset
pub fn get_stability_pool_liquidity(
    querier: QuerierWrapper,
    config: Config,
    pool_asset: AssetInfo,
) -> StdResult<Uint128> {
    if let Some(sp_addr) = config.clone().stability_pool {
        //Query the SP Asset Pool
        Ok(querier
            .query::<AssetPool>(&QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: sp_addr.to_string(),
                msg: to_binary(&SP_QueryMsg::AssetPool { })?,
            }))?
            .credit_asset
            .amount)
    } else {
        Ok(Uint128::zero())
    }
}

pub fn get_asset_liquidity(
    querier: QuerierWrapper,
    config: Config,
    asset_info: AssetInfo,
) -> StdResult<Uint128> {
    if config.clone().liquidity_contract.is_some() {
        let total_pooled: Uint128 = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: config.clone().liquidity_contract.unwrap().to_string(),
            msg: to_binary(&LiquidityQueryMsg::Liquidity { asset: asset_info })?,
        }))?;

        Ok(total_pooled)
    } else {
        return Err(StdError::GenericErr {
            msg: "No proxy contract setup".to_string(),
        });
    }
}

pub fn get_target_position(
    storage: &dyn Storage,
    valid_position_owner: Addr,
    position_id: Uint128,
) -> Result<(usize, Position), ContractError> {
    let positions: Vec<Position> = match POSITIONS.load(
        storage, valid_position_owner.clone()
    ){
        Err(_) => return Err(ContractError::NoUserPositions {}),
        Ok(positions) => positions,
    };

    match positions.into_iter().enumerate().find(|(i, x)| x.position_id == position_id) {
        Some(position) => Ok(position),
        None => return Err(ContractError::NonExistentPosition {}),
    }
}

pub fn mint_revenue(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    send_to: Option<String>,
    repay_for: Option<UserInfo>,
    amount: Option<Uint128>,
) -> Result<Response, ContractError> {
    
    //Can't send_to and repay_for at the same time
    if send_to.is_some() && repay_for.is_some() || send_to.is_none() && repay_for.is_none(){
        return Err(ContractError::CustomError {
            val: String::from("Destination address is required"),
        });
    }

    let config = CONFIG.load(deps.storage)?;
    let mut basket = BASKET.load(deps.storage)?;

    if info.sender != config.owner { return Err(ContractError::Unauthorized {}) }

    if basket.pending_revenue.is_zero() {
        return Err(ContractError::CustomError {
            val: String::from("No revenue to mint"),
        });
    }

    //Set amount
    let amount = amount.unwrap_or_else(|| basket.pending_revenue);

    //Subtract amount from pending revenue
    basket.pending_revenue = match basket.pending_revenue.checked_sub(amount) {
        Ok(new_balance) => new_balance,
        Err(err) => {
            return Err(ContractError::CustomError {
                val: err.to_string(),
            })
        }
    }; //Save basket
    BASKET.save(deps.storage, &basket)?;

    let mut message: Vec<CosmosMsg> = vec![];
    let mut repay_attr = String::from("None");

    //If send to is_some
    if let Some(send_to) = send_to.clone() {
        message.push(credit_mint_msg(
            config.clone(),
            Asset {
                amount,
                ..basket.credit_asset.clone()
            }, //Send_to or interest_collector or config.owner
            deps.api.addr_validate(&send_to.clone())?
        )?);
    } else if let Some(repay_for) = repay_for {
        repay_attr = repay_for.clone().to_string();

        //Need to mint credit to the contract
        message.push(credit_mint_msg(
            config.clone(),
            Asset {
                amount,
                ..basket.credit_asset.clone()
            },
            env.clone().contract.address,
        )?);

        //and then send it for repayment
        let msg = ExecuteMsg::Repay {
            position_id: repay_for.clone().position_id,
            position_owner: Some(repay_for.position_owner),
            send_excess_to: Some(env.contract.address.to_string()),
        };

        message.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_binary(&msg)?,
            funds: vec![coin(amount.u128(), basket.credit_asset.info.to_string())],
        }));
    } 

    Ok(Response::new().add_messages(message).add_attributes(vec![
        attr("amount", amount.to_string()),
        attr("repay_for", repay_attr),
        attr("send_to", send_to.unwrap_or(String::from("None"))),
    ]))
}

fn assert_credit_asset(
    basket: Basket,
    credit_asset: Asset,
    msg_sender: Addr,
)-> Result<(), ContractError>{
    match credit_asset.clone().info {
        AssetInfo::Token { address: _ } => { return Err(ContractError::InvalidCredit {}) },
        AssetInfo::NativeToken {
            denom: submitted_denom,
        } => {
            if let AssetInfo::NativeToken { denom } = basket.clone().credit_asset.info {
                if submitted_denom != denom {
                    return Err(ContractError::InvalidCredit {})
                }
            } else {
                return Err(ContractError::InvalidCredit {})
            }
        }
    }

    Ok(())
}

pub fn check_for_empty_position( collateral_assets: Vec<cAsset> )-> bool{
    //Checks if each cAsset's amount is zero
    for asset in collateral_assets {    
        if !asset.asset.amount.is_zero(){
            return false
        }
    }
    true 
}