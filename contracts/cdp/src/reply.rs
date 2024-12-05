use std::str::FromStr;

use cosmwasm_std::{attr, to_binary, CosmosMsg, Decimal, DepsMut, Env, Reply, Response, StdError, StdResult, Uint128, WasmMsg};

use membrane::cdp::{ExecuteMsg, Config};
use membrane::types::{cAsset, Asset, AssetInfo, Basket};
use membrane::helpers::{asset_to_coin, get_contract_balances, withdrawal_msg};

use crate::risk_engine::update_basket_tally;
use crate::state::{get_target_position, update_position, update_position_claims, LiquidationPropagation, BASKET, LIQUIDATION, WITHDRAW, ClosePositionPropagation, CLOSE_POSITION, CONFIG};

//Signify the revenue destination that errored without halting the msg flow
#[allow(unused_variables)]
pub fn handle_revenue_reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_result) => {
            //Its reply on error only
            Ok(Response::new())
        }        
        Err(string) => {
            Ok(Response::new()
                .add_attribute("error_while_distributing_revenue", string))
        }
    }
}

/// Validate withdrawls by asserting that the amount withdrawn is less than or equal to the amount of the asset in the contract.
/// Assert new cAssets amount was saved correctly.
pub fn handle_withdraw_reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    //Initialize Response Attributes
    let mut attrs = vec![];

    //Match on msg.result
    match msg.result.into_result() {
        Ok(_result) => {
            let withdraw_prop = WITHDRAW.load(deps.storage)?;

            //Assert valid withdrawal for each asset this reply is
            for (i, prev_collateral) in withdraw_prop.clone().positions_prev_collateral.into_iter().enumerate(){
                let asset_info: AssetInfo = prev_collateral.info.clone();
                let position_amount: Uint128 = prev_collateral.amount;
                let withdraw_amount: Uint128 = withdraw_prop.withdraw_amounts[i];

                let current_asset_balance = match get_contract_balances(
                    deps.querier,
                    env.clone(),
                    vec![asset_info.clone()],
                ) {
                    Ok(balances) => balances[0],
                    Err(err) => {
                        return Err(StdError::GenericErr {
                            msg: err.to_string(),
                        })
                    }
                };

                //If balance differnce is more than what they tried to withdraw, error
                if withdraw_prop.contracts_prev_collateral_amount[i] - current_asset_balance > withdraw_amount {
                    return Err(StdError::GenericErr {
                        msg: format!(
                            "Conditional 1: Invalid withdrawal, possible bug found by {}",
                            withdraw_prop.position_info.position_owner
                        ),
                    });
                }

                match get_target_position(
                    deps.storage,
                    deps.api.addr_validate(&withdraw_prop.position_info.position_owner)?,
                    withdraw_prop.position_info.position_id,
                ){
                    Ok((_i, user_position)) => {
                        //Assert the withdrawal was correctly saved to state
                        if let Some(cAsset) = user_position
                        .collateral_assets
                        .into_iter()
                        .find(|cAsset| cAsset.asset.info.equal(&asset_info))
                        {
                            if cAsset.asset.amount != (position_amount - withdraw_amount) {
                                return Err(StdError::GenericErr {
                                    msg: format!(
                                        "Conditional 2: Invalid withdrawal, possible bug found by {}",
                                        withdraw_prop.position_info.position_owner
                                    ),
                                });
                            }
                        }
                    },
                    Err(err) => {
                        //Error means the position was deleted from state, assert that collateral was supposed to be completely withdrawn
                        if !(position_amount - withdraw_amount).is_zero(){
                            return Err(StdError::GenericErr {
                                msg: err.to_string(),
                            })
                        }
                    }
                };                

                //Add Success attributes
                attrs.push(attr(
                    "successful_withdrawal",
                    Asset {
                        info: asset_info,
                        amount: withdraw_amount,
                    }
                    .to_string(),
                ));
            }
        //We can go by first entries for these fields bc the replies will come in FIFO in terms of assets sent
        
        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
    
    Ok(Response::new().add_attributes(attrs))
}

/// Send the liquidation queue its collateral reward.
/// Send leftovers to the SP.
/// Note: We send collateral here bc the LQ queries have returned less debt than the executed msg before so we want to give the LQ exactly what its expecting.
#[allow(unused_variables)]
pub fn handle_liq_queue_reply(deps: DepsMut, msg: Reply, env: Env) -> StdResult<Response> {
    let mut attrs = vec![attr("method", "handle_liq_queue_reply")];

    match msg.result.into_result() {
        Ok(result) => {        
            //1) Parse potential repaid_amount and substract from running total
            //2) Send collateral to the Queue

            let liq_event = result
                .events
                .into_iter()
                .find(|e| e.attributes.iter().any(|attr| attr.key == "repay_amount"))
                .ok_or_else(|| StdError::GenericErr {  msg: String::from("unable to find liq-queue event")})?;

            let repay = &liq_event
                .attributes
                .iter()
                .find(|attr| attr.key == "repay_amount")
                .unwrap()
                .value;
            let repay_amount = Uint128::from_str(repay)?;

            let mut prop: LiquidationPropagation = LIQUIDATION.load(deps.storage)?;
            let mut basket = prop.clone().basket;
            let config = prop.clone().config;

            //Send successfully liquidated amount
            let amount = &liq_event
                .attributes
                .iter()
                .find(|attr| attr.key == "collateral_amount")
                .unwrap()
                .value;

            let send_amount = Uint128::from_str(amount)?;

            let token = &liq_event
                .attributes
                .iter()
                .find(|attr| attr.key == "collateral_token")
                .unwrap()
                .value;

            let asset_info = &liq_event
                .attributes
                .iter()
                .find(|attr| attr.key == "collateral_info")
                .unwrap()
                .value;

            let token_info: AssetInfo = if asset_info.eq(&String::from("token")) {
                AssetInfo::Token {
                    address: deps.api.addr_validate(token)?,
                }
            } else {
                AssetInfo::NativeToken {
                    denom: token.to_string(),
                }
            };
        
            let msg = withdrawal_msg(
                Asset {
                    info: token_info.clone(),
                    amount: send_amount,
                },
                basket.liq_queue.clone().unwrap(),
            )?;
            
            //Subtract repaid amount from total repay amount that is held in liq_queue_leftovers.
            if repay_amount != Uint128::zero() {

                //Update credit amount based on liquidation's total repaid amount
                prop.target_position.credit_amount -= repay_amount;
                
                //Update position claims in prop.target_position
                prop.target_position.collateral_assets
                    .iter_mut()
                    .find(|cAsset| cAsset.asset.info.equal(&token_info))
                    .unwrap()
                    .asset
                    .amount -= send_amount;
                
                //update liquidated assets
                prop.liquidated_assets.push(
                    cAsset {
                        asset: Asset {
                            amount: send_amount,
                            info: token_info.clone()
                        },
                        max_borrow_LTV: Decimal::zero(),
                        max_LTV: Decimal::zero(),
                        pool_info: None,
                        rate_index: Decimal::one(),
                        hike_rates: Some(false),
                    }
                );

                //Add to total repaid
                prop.total_repaid += Decimal::from_ratio(repay_amount, Uint128::new(1u128));
            }
        
            //If this is the last asset left to send and nothing was sent to the SP, update the position here instead of in liq_repay
            //We use 1 as our 0 to account for LQ rounding errors
            if prop.per_asset_repayment.len() == 1 && prop.stability_pool <= Decimal::one() {

                //Update supply caps
                if prop.clone().target_position.credit_amount.is_zero(){                
                    //Remove all assets from Supply caps 
                    match update_basket_tally(
                        deps.storage, 
                        deps.querier, 
                        env.clone(), 
                        &mut basket, 
                       [prop.clone().target_position.clone().collateral_assets, prop.clone().liquidated_assets].concat(),
                        prop.clone().target_position.clone().collateral_assets,
                        false, 
                        config.clone(),
                        true,
                    ){
                        Ok(_) => {},
                        Err(err) => return Err(StdError::GenericErr { msg: err.to_string() }),
                    };
                } else {
                    //Remove liquidated assets from Supply caps
                    match update_basket_tally(
                        deps.storage, 
                        deps.querier, 
                        env.clone(), 
                        &mut basket,
                        prop.clone().liquidated_assets,
                        prop.clone().target_position.clone().collateral_assets,
                        false,
                        config.clone(),
                        true,
                    ){
                        Ok(_) => {},
                        Err(err) => return Err(StdError::GenericErr { msg: err.to_string() }),
                    };
                }            
                //Update Basket
                BASKET.save(deps.storage, &basket)?;

                //Update position w/ new credit amount
                update_position(deps.storage, prop.clone().position_owner, prop.clone().target_position)?;                
            }

            //Remove Asset
            prop.per_asset_repayment.remove(0);
            LIQUIDATION.save(deps.storage, &prop)?;

            attrs.extend(vec![
                attr("repay_amount", repay_amount),
                attr("reward_amount", send_amount),
                attr("reward_info", token_info.to_string()),
            ]);

            Ok(Response::new()
                .add_message(msg)
                .add_attributes(attrs)
            )
        }
        Err(string) => {
            //Only reply on success
            Ok(Response::new()
                .add_attribute("error", string))
        }
    }
}

/// On success, update position claims & attempt to withdraw leftover using a WithdrawMsg
pub fn handle_close_position_reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_result) => {
            //Load Close Position Prop
            let state_propagation: ClosePositionPropagation = CLOSE_POSITION.load(deps.storage)?;

            //Create user info variables
            let valid_position_owner = deps.api.addr_validate(&state_propagation.position_info.position_owner)?;
            let position_id = state_propagation.position_info.position_id;             

            //Load State
            let basket: Basket = BASKET.load(deps.storage)?;
            let config: Config = CONFIG.load(deps.storage)?;

            //Query contract balance of the basket credit_asset
            let credit_asset_balance = get_contract_balances(
                deps.querier, 
                env.clone(), 
                vec![basket.credit_asset.info.clone()]
            )?[0];

            //Create repay_msg
            let repay_msg = ExecuteMsg::Repay { 
                position_id, 
                position_owner: Some(valid_position_owner.clone().to_string()),
                send_excess_to: Some(valid_position_owner.clone().to_string()),
            };

            //Create repay_msg with queried funds
            //This works because the contract doesn't hold excess credit_asset, all repayments are burned & revenue isn't minted
            let repay_msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: env.contract.address.to_string(), 
                msg: to_binary(&repay_msg)?, 
                funds: vec![asset_to_coin(
                    Asset { 
                        info: basket.credit_asset.info.clone(),
                        amount: credit_asset_balance.clone(),
                    })?]
            });

            //Update position claims for each asset withdrawn + sold
            for withdrawn_collateral in state_propagation.clone().withdrawn_assets {

                update_position_claims(
                    deps.storage, 
                    deps.querier, 
                    env.clone(), 
                    config.clone(),
                    position_id,
                    valid_position_owner.clone(), 
                    withdrawn_collateral.info, 
                    withdrawn_collateral.amount
                )?;
            }

            //Load position
            let (_i, target_position) = match get_target_position(
                deps.storage, 
                valid_position_owner.clone(), 
                position_id, 
            ){
                Ok(position) => position,
                Err(err) => return Err(StdError::GenericErr { msg: err.to_string() })
            };

            //Withdrawing everything thats left
            let assets_to_withdraw: Vec<Asset> = target_position.collateral_assets
                .into_iter()
                .filter(|cAsset| cAsset.asset.amount > Uint128::zero())
                .map(|cAsset| cAsset.asset)
                .collect::<Vec<Asset>>();

            if assets_to_withdraw.len() > 0 {     
                //Create WithdrawMsg
                let withdraw_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: env.contract.address.to_string(), 
                    msg: to_binary(& ExecuteMsg::Withdraw { 
                        position_id, 
                        assets: assets_to_withdraw, 
                        send_to: state_propagation.send_to, 
                    })?, 
                    funds: vec![],
                });

                //Response 
                Ok(Response::new()
                    .add_message(repay_msg)
                    .add_attribute("amount_repaid", credit_asset_balance)
                    .add_message(withdraw_msg)
                    .add_attribute("sold_assets", format!("{:?}", state_propagation.withdrawn_assets))            
                )
            } else {
                //Response 
                Ok(Response::new()
                    .add_message(repay_msg)
                    .add_attribute("amount_repaid", credit_asset_balance)
                    .add_attribute("sold_assets", format!("{:?}", state_propagation.withdrawn_assets))            
                )
            }
        },

        Err(err) => {
            //Its reply on success only
            Ok(Response::new().add_attribute("error", err))
        }
    }
}

