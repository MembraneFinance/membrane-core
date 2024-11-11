#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, to_json_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, QuerierWrapper, Reply, Response, StdError, StdResult, Storage, SubMsg, Uint128, WasmMsg
};
use membrane::oracle::{self, PriceResponse};
use membrane::types::{Asset, AssetInfo, AssetPool, Basket, ClaimTracker, UserInfo, VTClaimCheckpoint, APR};
use osmosis_std::types::osmosis;
use serde::de;
use std::cmp::{max, min};
use std::str::FromStr;
use std::vec;
use cw2::set_contract_version;
use membrane::math::{decimal_division, decimal_multiplication, decimal_subtraction};

use crate::error::TokenFactoryError;
use crate::state::{CLAIM_TRACKER, StateAssurance, CONFIG, EXIT_MESSAGE_INFO, OWNERSHIP_TRANSFER, STATE_ASSURANCE,VAULT_TOKEN};
use membrane::vol_earn_vault::{Config, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use membrane::mars_vault_token::{ExecuteMsg as Vault_ExecuteMsg, QueryMsg as Vault_QueryMsg};
use membrane::cdp::{BasketPositionsResponse, CollateralInterestResponse, ExecuteMsg as CDP_ExecuteMsg, InterestResponse, PositionResponse, QueryMsg as CDP_QueryMsg};
use membrane::osmosis_proxy::{ExecuteMsg as OP_ExecuteMsg};
use membrane::oracle::QueryMsg as Oracle_QueryMsg;
use membrane::stability_pool::{ClaimsResponse, ExecuteMsg as SP_ExecuteMsg, QueryMsg as SP_QueryMsg};
use membrane::stability_pool_vault::{
    calculate_base_tokens, calculate_vault_tokens
};
use membrane::range_bound_lp_vault::{ExecuteMsg as LP_ExecuteMsg, QueryMsg as LP_QueryMsg};
use osmosis_std::types::osmosis::tokenfactory::v1beta1::{self as TokenFactory};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:vol-earn-vault";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

//Reply IDs
const ENTER_VAULT_REPLY_ID: u64 = 1u64;
const CDP_REPLY_ID: u64 = 2u64;
const COMPOUND_REPLY_ID: u64 = 3u64;
const INITIATE_EXIT_REPLY_ID: u64 = 4u64;
//Constants
const SECONDS_PER_DAY: u64 = 86_400u64;

////PROCEDURAL FLOW/NOTES////
//- Accept W(BTC) deposits which are deposited into the CDP
//- We mint up to an LTV & send that CDT to the SP (withdraw after depositing to start unstake)
//- When the SP gets paid, we withdraw everything (to reset our queue position in the SP) and compound profits into WBTC

//NOTES:
//- We unloop at Some(LTV) & anytime cost is negative 
//- When we compound, even though unlikely, we have to account for potential SP liquidations that take our CDT (i.e. compound those assets if necessary)
//- Withdrawals will repay a pro-rata amount of the CDP debt taken from the SP.
// --Bc this has no cost, we can withdraw at any time & ownership is calc'd in WBTC value, not debt.
// - We deposit directly into the SP instead of the autoSP vault to reduce the cost of liquidations. We'd only pay the 1% fee & get our TVL withdrawn from the SP automatically.
// - We don't need multiple yield venues bc external yield will pull CDT from the SP &/or increase the supply, both of which increase the APR in the SP.


#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, TokenFactoryError> {
    //Set config
    let mut config = Config {
        owner: info.sender.clone(),
        cdt_denom: msg.clone().cdt_denom,
        vault_token: String::from("factory/".to_owned() + env.contract.address.as_str() + "/" + msg.clone().vault_subdenom.as_str()),
        deposit_token: msg.clone().deposit_token_info.deposit_token,
        cdp_contract_addr: deps.api.addr_validate(&msg.clone().cdp_contract_addr)?,
        osmosis_proxy_contract_addr: deps.api.addr_validate(&msg.clone().osmosis_proxy_contract_addr)?,
        oracle_contract_addr: deps.api.addr_validate(&msg.clone().oracle_contract_addr)?,
        stability_pool_contract_addr: deps.api.addr_validate(&msg.clone().stability_pool_contract_addr)?.to_string(),
        cdp_position_id: Uint128::zero(),
        swap_slippage: Decimal::from_str("0.005").unwrap(), //0.5%
        vault_cost_index: 0,
        mint_LTV: msg.clone().mint_LTV, 
        repay_LTV: msg.clone().repay_LTV,
        cost_ceiling: Decimal::percent(4),
    };
    //Query the basket to find the index of the deposit token
    let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasket { },
    ){
        Ok(basket) => basket,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP Basket") }),
    };
    //Find the index
    let mut saved_index: Option<u64> = None;
    for (index, asset) in basket.clone().collateral_types.into_iter().enumerate(){
        if asset.asset.info.to_string() == config.deposit_token.clone() {
            saved_index = Some(index as u64);
            break;
        }
    }
    if let Some(index) = saved_index {
        config.vault_cost_index = index as usize;
    } else {
        return Err(TokenFactoryError::CustomError { val: String::from("Failed to find the vault token in the CDP Basket") });
    }

    //Save initial state
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    //Set the initial vault token amount from the initial deposit that is used to get the CDP postion_id
    let vault_tokens_to_distribute = calculate_vault_tokens(
        info.funds[0].amount.clone(),
        Uint128::zero(), 
        Uint128::zero()
    )?;
    CONFIG.save(deps.storage, &config)?;
    VAULT_TOKEN.save(deps.storage, &vault_tokens_to_distribute)?;
    CLAIM_TRACKER.save(deps.storage, &ClaimTracker {
        vt_claim_checkpoints: vec![
            VTClaimCheckpoint {
                vt_claim_of_checkpoint: Uint128::new(10u64.pow(msg.clone().deposit_token_info.decimal) as u128),
                time_since_last_checkpoint: 0u64,
            }
        ],
        last_updated: env.block.time.seconds(),
    })?;
    //Create Denom Msg
    let denom_msg = TokenFactory::MsgCreateDenom { sender: env.contract.address.to_string(), subdenom: msg.vault_subdenom.clone() };
    //Create CDP deposit msg to get the position ID
    //Instantiatoor must send a vault token.
    //This initial deposit means the position should never be empty due to user withdrawals.
    let cdp_deposit_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.cdp_contract_addr.to_string(),
        msg: to_json_binary(&CDP_ExecuteMsg::Deposit { position_id: None, position_owner: None })?,
        funds: vec![
            info.funds[0].clone()
        ],
    });
    let cdp_submsg = SubMsg::reply_on_success(cdp_deposit_msg, CDP_REPLY_ID);
    //This can be done by taking the Basket's next ID before depositing as well but this logic has already worked for our other contracts.
    
    //Create Response
    let res = Response::new()
        .add_attribute("method", "instantiate")
        .add_attribute("config", format!("{:?}", config))
        .add_attribute("contract_address", env.contract.address)
        .add_attribute("sub_denom", msg.clone().vault_subdenom)
    //UNCOMMENT
        .add_message(denom_msg)
        .add_submessage(cdp_submsg);
    Ok(res)
}


#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, TokenFactoryError> {
    match msg {
        ExecuteMsg::UpdateConfig { 
            owner, 
            cdp_contract_addr, 
            osmosis_proxy_contract_addr, 
            oracle_contract_addr,
            swap_slippage,
            vault_cost_index,
            stability_pool_contract_addr,
            cdp_position_id,
            cost_ceiling,
            mint_LTV,
            repay_LTV,
        } => update_config(deps, info, env, owner, cdp_contract_addr, stability_pool_contract_addr, osmosis_proxy_contract_addr, oracle_contract_addr, swap_slippage, vault_cost_index, cdp_position_id, cost_ceiling, mint_LTV, repay_LTV),
        ExecuteMsg::EnterVault { } => enter_vault(deps, env, info),
        ExecuteMsg::ExitVault {  } => accrue_before_exit(deps, env, info),
        ExecuteMsg::CrankRealizedAPR { } => crank_realized_apr(deps, env, info),
        ExecuteMsg::ManageVault { lower_mint_ltv_ceiling } => manage_vault(deps, env, info, lower_mint_ltv_ceiling),
        ///CALLBACKS///
        ExecuteMsg::StateAssurance { skip_LTV, skip_cost } => state_assurance(deps, env, info, skip_LTV, skip_cost),
    }
}


fn accrue_before_exit(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    //Load config
    let config = CONFIG.load(deps.storage)?;

    //Save Message Info for Exit vault
    EXIT_MESSAGE_INFO.save(deps.storage, &info)?;

    //Create accrue msg
    let accrue_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.cdp_contract_addr.to_string(),
        msg: to_json_binary(&CDP_ExecuteMsg::Accrue { 
            position_owner: None, 
            position_ids: vec![config.cdp_position_id] 
        }
        )?,
        funds: vec![],
    });
    let accrue_submsg = SubMsg::reply_on_success(accrue_msg, INITIATE_EXIT_REPLY_ID);

    //Return
    Ok(Response::new()
        .add_attributes(vec![
            attr("method","accrue_before_exit"),
            attr("position", config.cdp_position_id),
            attr("message_info", format!("{:?}", info))
        ])
        .add_submessage(accrue_submsg)
    )

}

/// 1a) Compound SP's CDT distribution into the deposit token
/// 1b) Compound SP liquidation rewards (we try to avoid getting these)
/// 2) Repay if we're >= the repay_LTV or unprofitable (above the set cost ceiling)
/// 3) Mint and deposit to SP
fn manage_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    lower_mint_ltv_ceiling: Option<Decimal>,
) -> Result<Response, TokenFactoryError>{
    //Load config
    let mut config = CONFIG.load(deps.storage)?;
    let mut msgs: Vec<SubMsg> = vec![];
    let mut attrs = vec![
        attr("method", "manage_vault")
    ];

    //Query SP TVL
    let asset_pool: AssetPool = deps.querier.query_wasm_smart::<AssetPool> (
        config.stability_pool_contract_addr.clone(),
        &SP_QueryMsg::AssetPool { 
            user: Some(env.contract.address.to_string()),
            deposit_limit: None,
            start_after: None,
        },
    )?;
    //Calc total TVL in the SP
    let contract_SP_tvl: Uint128 = asset_pool.deposits.clone().into_iter()    
        .map(|deposit| deposit.amount)
        .sum::<Decimal>().to_uint_floor();

    //Get withdrawable CDT
    let withdrawable_SP_tvl = asset_pool.deposits.clone().into_iter()
        .filter(|deposit| deposit.unstake_time.is_some() && deposit.unstake_time.unwrap() + SECONDS_PER_DAY <= env.block.time.seconds())
        .map(|deposit| deposit.amount)
        .sum::<Decimal>().to_uint_floor();

    //Query the CDP position for debt & collateral
    let vault_position: Vec<BasketPositionsResponse> = match deps.querier.query_wasm_smart::<Vec<BasketPositionsResponse>>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasketPositions { 
            start_after: None, 
            user: None,
            user_info: Some(UserInfo {
                position_owner: env.contract.address.to_string(),
                position_id: config.cdp_position_id,
            }), 
            limit: None, 
        },
    ){
        Ok(vault_position) => vault_position,
        Err(err) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP Position for the vault token amount in manage_vault:") + &err.to_string() }),
    };
    let vault_position: PositionResponse = vault_position[0].positions[0].clone();
    //Set credit amount
    let current_credit_amount = vault_position.credit_amount;
    //Set collateral amount
    let current_collateral_amount = vault_position.collateral_assets[0].asset.amount;

    
    ////////// 1) Compound ///////////////
    /// 1a) SP's CDT distribution into the deposit token/////////////
    
    //Calculate the earned_yield as the difference between the SP TVL & the CDP debt allocated to the SP
    let earned_yield = match contract_SP_tvl.checked_sub(current_credit_amount){
        Ok(v) => v,
        Err(_) => Uint128::zero(),
    };

    //If earned_yield is > withdrawable_SP_TVL, just error. Compound can wait for unstakes (~1day) as this is unlikely.
    if earned_yield > withdrawable_SP_tvl {
        return Err(TokenFactoryError::CustomError { val: format!("The withdrawable amount {:?} is less than the earned_yield, {:?}, so wait for more to unstake", withdrawable_SP_tvl, earned_yield)})
    }

    //If the earned_yield is non-zero:
    //- Withdraw everything from the SP (to reset our queue position)
    //- Compound the profits only into the deposit token
    //- Redeposit the deposit token into the CDP
    //- Deposit the CDT back into the SP
    if !earned_yield.is_zero() {
        attrs.push(attr("SP_yield", earned_yield.to_string()));

        //Withdraw everything we can from the SP
        let withdraw_all_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool_contract_addr.clone(),
            msg: to_json_binary(&SP_ExecuteMsg::Withdraw { amount: withdrawable_SP_tvl })?,
            funds: vec![],
        });
        msgs.push(SubMsg::new(withdraw_all_msg));
        //Compound the profits into the deposit token
        let compound_profits_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.osmosis_proxy_contract_addr.to_string(),
            msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                token_out: config.deposit_token.clone(),
                max_slippage: config.swap_slippage,
            })?,
            funds: vec![Coin {
                denom: config.cdt_denom.clone(),
                amount: earned_yield,
            }],
        });
        msgs.push(SubMsg::reply_on_success(compound_profits_msg, COMPOUND_REPLY_ID));
        //Redeposit the deposit token into the CDP (Happens in the Reply)        

        // Deposit the CDT back into the SP
        let deposit_cdt_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool_contract_addr.clone(),
            msg: to_json_binary(&SP_ExecuteMsg::Deposit { user: None })?,
            funds: vec![Coin {
                denom: config.cdt_denom.clone(),
                amount: withdrawable_SP_tvl - earned_yield,
            }],
        });
        msgs.push(SubMsg::new(deposit_cdt_msg));
    }
    
    ///////////// 1b) Compound SP liquidation rewards (we try to avoid getting these)//////////////
    let mut claims: ClaimsResponse = match deps.querier.query_wasm_smart::<ClaimsResponse>(
        config.clone().stability_pool_contract_addr,
        &SP_QueryMsg::UserClaims {
            user: env.contract.address.to_string(),
        },
    ){
        Ok(claims) => claims,
        Err(_) => ClaimsResponse { claims: vec![] },
    };

    //If the claims include MBRN, create a burn message for it & filter it out of the swap
    match claims.claims.clone()
        .into_iter()
        .enumerate()
        .find(|(_, claim)| claim.denom.to_string() == String::from("factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/umbrn")){
            Some((i, claim)) => {
                let burn_mbrn_msg = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                    msg: to_json_binary(&OP_ExecuteMsg::BurnTokens { 
                        denom: String::from("factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/umbrn"),
                        amount: claim.amount,
                        burn_from_address: env.contract.address.to_string(),
                    })?,
                    funds: vec![],
                });
                msgs.push(SubMsg::new(burn_mbrn_msg));
                //Remove the MBRN claim
                claims.claims.remove(i);
            },
            None => {},
    };
    //If there are non-MBRN claims left, compound them into the deposit token
    if claims.claims.len() > 0 as usize {      
        attrs.push(attr("SP_liquidations", format!("{:?}", claims.claims.clone())));  

        let compound_profits_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.osmosis_proxy_contract_addr.to_string(),
            msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                token_out: config.deposit_token.clone(),
                max_slippage: config.swap_slippage,
            })?,
            funds: claims.claims,
        });
        msgs.push(SubMsg::reply_on_success(compound_profits_msg, COMPOUND_REPLY_ID));
    }  
    

    //////2) Withdraw & Repay if we're >= then the repay_LTV or above the cost ceiling //////////
    /// 
    //Query the basket to get the cost of the deposit token collateral
    let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasket { },
    ){
        Ok(basket) => basket,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP Basket") }),
    };
    //Get cost from basket
    let cost = basket.lastest_collateral_rates[config.vault_cost_index].rate;

    /////Get LTV of the position/////
    //Get the price of the deposit token
    let prices: Vec<PriceResponse> = match deps.querier.query_wasm_smart::<Vec<PriceResponse>>(
        config.oracle_contract_addr.to_string(),
        &Oracle_QueryMsg::Price {
            asset_info: AssetInfo::NativeToken { denom: config.clone().deposit_token },
            twap_timeframe: 0, //We want current swap price
            oracle_time_limit: 0,
            basket_id: None
        },
    ){
        Ok(prices) => prices,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the deposit token price in manage_vault") }),
    };
    let deposit_token_price: PriceResponse = prices[0].clone();
    //Get the price of debt token from the Basket
    let cdt_price: PriceResponse = basket.credit_price;
    //Calc the LTV
    let ltv = decimal_division(
        cdt_price.get_value(current_credit_amount)?, 
        deposit_token_price.get_value(current_collateral_amount)?
    )?;
    ///////
    //If either cost or LTV are over the threshold, repay all of the SP TVL
    //We can redeposit more precisely in #3
    if cost > config.cost_ceiling || ltv >= config.repay_LTV {
        attrs.push(attr("repaying_loan", format!("over cost: {} or over LTV: {}",  cost > config.cost_ceiling,  ltv >= config.repay_LTV)));

        //Set withdrawable TVL
        let withdrawable_TVL = withdrawable_SP_tvl - earned_yield;
        // Withdraw (withdrawable_SP_tvl - earned_yield) from the SP
        let withdraw_all_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool_contract_addr.clone(),
            msg: to_json_binary(&SP_ExecuteMsg::Withdraw { amount: withdrawable_TVL })?,
            funds: vec![],
        });
        msgs.push(SubMsg::new(withdraw_all_msg));

        //Repay it all to the vault's CDP position        
        let repay_CDP_loan = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.cdp_contract_addr.to_string(),
            msg: to_json_binary(&CDP_ExecuteMsg::Repay { 
                position_id: config.cdp_position_id,
                position_owner: None,
                send_excess_to: None,
            })?,
            funds: vec![
                Coin {
                    denom: config.cdt_denom.clone(),
                    amount: withdrawable_TVL,
                }
            ],
        });
        msgs.push(SubMsg::new(repay_CDP_loan));
    } else {
        ///////////3) Mint and deposit to SP/////
        // - We don't do this unless we aren't repaying

        //Set mint LTV
        let mint_LTV = lower_mint_ltv_ceiling.unwrap_or_else(|| config.mint_LTV );

        //Calc mintable 
        let amount_to_mint = calc_mintable(
            config.clone().swap_slippage,
            deposit_token_price.clone(),
            current_collateral_amount,
            cdt_price,
            current_credit_amount,
            config.clone().mint_LTV
        )?;
        attrs.push(attr("minting_to_yield_venues",  amount_to_mint.to_string()));


        //Mint from CDP position
        let mint_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.cdp_contract_addr.to_string(),
            msg: to_json_binary(&CDP_ExecuteMsg::IncreaseDebt { 
                position_id: config.cdp_position_id,
                amount: Some(amount_to_mint),
                LTV: None,
                mint_to_addr: None,
            })?,
            funds: vec![],
        });
        msgs.push(SubMsg::new(mint_msg));

        //Deposit to SP
        let deposit_cdt_to_sp_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool_contract_addr.clone(),
            msg: to_json_binary(&SP_ExecuteMsg::Deposit { user: None })?,
            funds: vec![Coin {
                denom: config.cdt_denom.clone(),
                amount: amount_to_mint,
            }],
        });
        msgs.push(SubMsg::new(deposit_cdt_to_sp_msg));
    }
  
    Ok(Response::new()
        .add_attributes(attrs)
        .add_submessages(msgs)
    )


}

/// Calc mintable value & return new tokens to deposit, value & amount to mint
fn calc_mintable(
    swap_slippage: Decimal,
    deposit_token_price: PriceResponse,
    deposit_tokens: Uint128,
    cdt_price: PriceResponse,
    debt: Uint128,
    mint_LTV: Decimal
) -> StdResult<Uint128>{ 
    //Calc the value of the collateral
    let collateral_value = deposit_token_price.get_value(deposit_tokens)?;
    //Calc the value of the CDT debt
    let debt_value = cdt_price.get_value(debt)?;
    //Calc LTV
    let ltv = decimal_division(debt_value, max(collateral_value, Decimal::one()))?;
    //Calc the distance of the LTV to 50%
    let ltv_space_to_mint = match mint_LTV.checked_sub(ltv){
        Ok(v) => v,
        Err(_) => Decimal::zero(),
    }.checked_mul(Decimal::percent(100_00))?.to_uint_floor(); //this is done to get rid of the 3rd decimal place. Mints were erroring right above the LTV
    //Calc the value of the debt to mint
    let mintable_value = decimal_multiplication(collateral_value, Decimal::percent(ltv_space_to_mint.u128() as u64))?;
    //Calc the amount of vault tokens to mint
    let amount_to_mint = cdt_price.get_amount(mintable_value)?;

    Ok(amount_to_mint)
}

//Withdrawable collateral for unloops
fn calc_withdrawable_collateral(
    swap_slippage: Decimal,
    vt_price: PriceResponse,
    cdt_price: PriceResponse,
    vault_tokens: Uint128,
    debt: Uint128,
    in_reply: bool, //If this is in a reply, we don't want to return an error for being under debt minimum
) -> StdResult<(Uint128, Decimal)>{ //withdrawal_amount, withdraw_value
    //If debt is 0, quick return 
    if debt.is_zero() {
        return Ok((vault_tokens, Decimal::zero())); //we don't use withdraw value 
    } 
    //Calc the value of the vault tokens
    let vault_tokens_value = vt_price.get_value(vault_tokens)?;
    //Calc the value of the CDT debt
    let debt_value = cdt_price.get_value(debt)?;
    //Calc LTV
    let ltv = decimal_division(debt_value, max(vault_tokens_value, Decimal::one()))?;
    //Calc the distance of the LTV to 89% (the max borrowable LTV is 90 so we want to leave a 1% buffer to bypass and tiny precision errors)
    let ltv_space_to_withdraw = match Decimal::percent(89).checked_sub(ltv){
        Ok(v) => v,
        Err(_) => return Err(StdError::GenericErr { msg: format!("LTV over 89%: {} > 0.89", ltv) }),
    };
    //Calc the value of the vault tokens we withdraw
    //It's either clearing the debt (accounting for the swap slippage) or using the LTV space
    let mut withdrawable_value = min(
        decimal_division(decimal_multiplication(vault_tokens_value, ltv_space_to_withdraw)?, Decimal::percent(90))?,
        decimal_multiplication(debt_value, Decimal::one() + swap_slippage)?,
    );


    //WE HAVE TO UNLOOP MORE THAN DESIRED TO ENSURE THE MINIMUM DEBT BUFFER IS ALWAYS AVAILABLE
    //so we unloop extra but only withdraw the desired amount at the end.
    // ACTION: We add the minimum LTV space into the desired withdrawal amount to ensure we can always clear the debt.
    // The reason is, the LTV will always go up (read: the buffer will shrink) unless we withdraw double since they are pulling the desired out.
    // So instead we are left with whatever they don't pull out which will be the minimum or the min + the leftover.
    // Visual Representation (Art): In this manner it would be a "withdrawal queue" where the buffer decreases as people withdraw until it rests 
    //...at the minimum debt value (MDV) where the withdrawal max is the MDV starting point + MAX_LOOPS.
    // The buffer only regenerates after redemptions or new deposits that don't get looped.


    /////If withdrawable_value puts the debt value below $100, make sure to leave the minimum debt so the CDP doesn't error
    let minimum_debt_value = Decimal::percent(101_00);
    //We've failed if debt value is ever more than withdrawable value, we don't want to reach this
    if debt_value > withdrawable_value && decimal_subtraction(debt_value, withdrawable_value)? < minimum_debt_value {
        //Calc the difference
        let difference = match decimal_subtraction(debt_value, minimum_debt_value){
            Ok(v) => v,
            Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to subtract debt_value from minimum_debt_value: {} - {}", debt_value, minimum_debt_value) }),
        };

        //Set withdrawable_value to the difference
        withdrawable_value = difference;

    } 
    //We should never get here, 
    //If this errors the CDP repay function would've errored later.
    else if !in_reply && debt_value < minimum_debt_value {
        return Err(StdError::GenericErr { msg: format!("Debt value: ({}), is less than minimum debt value: ({}), which will error in the CDP repay function anyway. Someone needs to add more capital to the contract's CDP, whose position ID is in the config, to create more withdrawal space to totally unloop.", debt_value, minimum_debt_value) })
    }

    //Set minimum withdrawn & swapped value
    let withdrawal_w_slippage = decimal_multiplication(withdrawable_value, decimal_subtraction(Decimal::one(), swap_slippage)?)?;
    
    //Return the amount of vault tokens we can withdraw
    let withdrawable_collateral = vt_price.get_amount(withdrawable_value)?;
    
    Ok((withdrawable_collateral, withdrawal_w_slippage))
}

//Ensure LTV doesn't incrase post tx
//Ensure the vault token <> base token conversion rate is static
fn state_assurance(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    skip_LTV: bool,
    skip_cost: bool,
) -> Result<Response, TokenFactoryError> {
    //Load config    
    let config = CONFIG.load(deps.storage)?;

    //Error if not the contract calling
    if info.sender != env.contract.address {
        return Err(TokenFactoryError::Unauthorized {});
    }

    
    //Get total deposit tokens
    let (
        total_deposit_tokens,
        contract_SP_tvl, 
        current_collateral_amount, 
        current_credit_amount, 
        deposit_token_price, 
        cdt_price
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
    

    //Load State
    let state_assurance = STATE_ASSURANCE.load(deps.storage)?;
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    
    //Calc the LTV
    let post_tx_ltv = decimal_division(
        cdt_price.get_value(current_credit_amount)?, 
        deposit_token_price.get_value(current_collateral_amount)?
    )?;
    ///////


    //Calc the rate of vault tokens to deposit tokens
    let btokens_per_one = calculate_base_tokens(
        Uint128::new(100_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    //Check that the rates are static for everything other than exits.
    //Exits will show an increase bc of the entry fee & calculation logic.
    if !(btokens_per_one >= state_assurance.pre_btokens_per_one) {
        return Err(TokenFactoryError::CustomError { val: format!("Conversation rate assurance failed, should be equal or greater than. If its 1 off just try again. Deposit tokens per 1 pre-tx: {:?} --- post-tx: {:?}", state_assurance.pre_btokens_per_one, btokens_per_one) });
    }

    //Ensure LTV didn't increase
    if !skip_LTV && post_tx_ltv > state_assurance.pre_tx_ltv {
        return Err(TokenFactoryError::CustomError { val: format!("LTV increased post tx: {} > {}", post_tx_ltv, state_assurance.pre_tx_ltv) });
    }
    
    //Query the basket to find the index of the deposit token
    let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasket { },
    ){
        Ok(basket) => basket,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP Basket in state_assurance") }),
    };
    let cost = basket.lastest_collateral_rates[config.vault_cost_index].rate;
    //Ensure cost isn't above the cost ceiling
    if !skip_cost && cost > config.cost_ceiling {
        return Err(TokenFactoryError::CustomError { val: format!("Cost is above the cost ceiling: {} > {}", cost, config.cost_ceiling) });
    }

    Ok(Response::new())
}


fn enter_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    //Load State
    let mut config = CONFIG.load(deps.storage)?;
    
    //Get total deposit tokens
    let (
        total_deposit_tokens,
        contract_SP_tvl, 
        current_collateral_amount, 
        current_credit_amount, 
        deposit_token_price, 
        cdt_price
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

    /////Query the contract's Stability Pool TVL////
    //Query the SP asset pool
    let asset_pool: AssetPool = deps.querier.query_wasm_smart::<AssetPool> (
        config.stability_pool_contract_addr.clone(),
        &SP_QueryMsg::AssetPool { 
            user: Some(env.contract.address.to_string()),
            deposit_limit: None,
            start_after: None,
        },
    )?;
    
    //If TVL > minted debt, we can't enter the vault.
    //Must compound first to give the existing users their earned_yield.
    if contract_SP_tvl > current_credit_amount {
        return Err(TokenFactoryError::ContractHasClaims { claims: vec![Coin { 
            denom: config.cdt_denom,
            amount: contract_SP_tvl - current_credit_amount
        }] });
    }
    //////////
    
    /////Get LTV of the position/////
    //Calc the LTV
    let pre_tx_ltv = decimal_division(
        cdt_price.get_value(current_credit_amount)?, 
        deposit_token_price.get_value(current_collateral_amount)?
    )?;
    ///////


    //Query claims from the Stability Pool.
    //Error is there are claims.
    //Catch the error if there aren't.
    //We don't let users enter the vault if the contract has claims bc the claims go to existing users.
    /////To avoid this error, compound before depositing/////
    let _claims: ClaimsResponse = match deps.querier.query_wasm_smart::<ClaimsResponse>(
        config.clone().stability_pool_contract_addr,
        &SP_QueryMsg::UserClaims {
            user: env.contract.address.to_string(),
        },
    ){
        Ok(claims) => return Err(TokenFactoryError::ContractHasClaims { claims: claims.claims }),
        Err(_) => ClaimsResponse { claims: vec![] },
    };
 
    //Assert the only token sent is the deposit token
    if info.funds.len() != 1 {
        return Err(TokenFactoryError::CustomError { val: format!("More than 1 asset was sent, this function only accepts the deposit token: {:?}", config.clone().deposit_token) });
    }
    if info.funds[0].denom != config.deposit_token {
        return Err(TokenFactoryError::CustomError { val: format!("The wrong asset was sent ({:?}), this function only accepts the deposit token: {:?}", info.funds[0].denom, config.clone().deposit_token) });
    }
    
    //Get the amount of deposit token sent
    let deposit_amount = info.funds[0].amount;

    //////Calculate the amount of vault tokens to mint////
    //Get the total amount of vault tokens circulating
    let total_vault_tokens: Uint128 = VAULT_TOKEN.load(deps.storage)?;
    //Calc & save base token rates
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(100_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    STATE_ASSURANCE.save(deps.storage, &StateAssurance {
        pre_tx_ltv,
        pre_btokens_per_one,
    })?;

    //Calculate the amount of vault tokens to mint
    let vault_tokens_to_distribute = calculate_vault_tokens(
        deposit_amount,
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    // println!("vault_tokens_to_distribute: {:?}, {}, {}, {}", vault_tokens_to_distribute, total_deposit_tokens, decimal_multiplication(decimal_deposit_amount, decimal_subtraction(Decimal::one(), config.swap_slippage)?)?.to_uint_floor(), total_vault_tokens);
    ////////////////////////////////////////////////////

    let mut msgs: Vec<CosmosMsg> = vec![];
    //Mint vault tokens to the sender
    let mint_vault_tokens_msg: CosmosMsg = TokenFactory::MsgMint {
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
            denom: config.vault_token.clone(),
            amount: vault_tokens_to_distribute.to_string(),
        }), 
        mint_to_address: info.sender.to_string(),
    }.into();
    //UNCOMMENT
    msgs.push(mint_vault_tokens_msg);

    //Update the total token amounts
    VAULT_TOKEN.save(deps.storage, &(total_vault_tokens + vault_tokens_to_distribute))?;
    
    //Send the deposit to the CDP position
    let send_deposit_to_cdp_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.cdp_contract_addr.to_string(),
        msg: to_json_binary(&CDP_ExecuteMsg::Deposit { 
            position_id: Some(config.clone().cdp_position_id),
            position_owner: None
        })?,
        funds: vec![Coin {
            denom: config.deposit_token.clone(),
            amount: deposit_amount,
        }],
    });
    msgs.push(send_deposit_to_cdp_msg);

    //Save Updated Config
    CONFIG.save(deps.storage, &config)?;
    

    //Add rate assurance callback msg
    if !total_deposit_tokens.is_zero() && !total_vault_tokens.is_zero() {
        //UNCOMMENT
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::StateAssurance {
                skip_LTV: false,
                skip_cost: true    
            })?,
            funds: vec![],
        }));
    }

    //Create Response
    let res = Response::new()
        .add_attribute("method", "enter_vault")
        .add_attribute("deposit_amount", deposit_amount)
        .add_attribute("vault_tokens_distributed", vault_tokens_to_distribute)
        .add_messages(msgs);

    Ok(res)
}


/// Withdraw pro-rata debt from the SP & repay to the CDP position
/// Send user tokens back
/// Confirm that the LTV for the position hasn't gone up post withdraw in the StateAssurance callback
fn exit_vault(
    deps: DepsMut,
    env: Env,
) -> StdResult<Response> {
    //Load state
    let mut config = CONFIG.load(deps.storage)?;
    let info = EXIT_MESSAGE_INFO.load(deps.storage)?;

    let mut msgs: Vec<CosmosMsg> = vec![];
    
    //Get total deposit tokens
    let (
        total_deposit_tokens,
        _, 
        current_collateral_amount, 
        current_credit_amount, 
        deposit_token_price, 
        cdt_price
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
    
    //Assert the only token sent is the vault token
    if info.funds.len() != 1 {
        return Err(StdError::GenericErr { msg: format!("More than 1 asset was sent, this function only accepts the vault token: {:?}", config.clone().vault_token) });
    }
    if info.funds[0].denom != config.vault_token {
        return Err(StdError::GenericErr { msg: format!("The wrong asset was sent ({:?}), this function only accepts the vault token: {:?}", info.funds[0].denom, config.clone().vault_token) });
    }

    //Get the amount of vault tokens sent
    let vault_tokens = info.funds[0].amount;
    if vault_tokens.is_zero() {
        return Err(StdError::GenericErr { msg: String::from("Need to send more than 0 vault tokens") });
    }

    /////Get LTV of the position/////
    let collateral_TVL = deposit_token_price.get_value(current_collateral_amount)?;
    //Calc the LTV
    let pre_tx_ltv = decimal_division(
        cdt_price.get_value(current_credit_amount)?, 
        collateral_TVL
    )?;
    ///////
    
    //if the pre_tx_ltv is above the mint_LTV, repay down to the mint_LTV + whatever needs to be repaid to withdraw.
    //This allows withdrawals when above the mint_LTV vs. erroring out during the withdraw bc its over the LTV.
    let amount_to_reset_to_mint_LTV = match decimal_subtraction(pre_tx_ltv, config.mint_LTV) {
        Ok(ltv_diff) => {
            ///Calc the amount needed to repay to get down to the mint_LTV///
            let value_to_repay = decimal_multiplication(collateral_TVL, ltv_diff)?;
            let amount_to_repay = cdt_price.get_amount(value_to_repay)?;

            amount_to_repay

        },
        Err(_) => Uint128::zero()
    };
   
    //Get the total amount of vault tokens circulating
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    //Calc & save token rate
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(100_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    STATE_ASSURANCE.save(deps.storage, &StateAssurance {
        pre_tx_ltv,
        pre_btokens_per_one,
    })?;
    //Get amount of tokens to exit
    let mut deposit_tokens_to_withdraw = calculate_base_tokens(
        vault_tokens, 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    /// Withdraw pro-rata debt from both yield venues(SP& LP) & repay to the CDP position ////
    //Calc the user's ratio of collateral
    let user_ratio_of_collateral = decimal_division(
        Decimal::from_ratio(deposit_tokens_to_withdraw, Uint128::one()),
        Decimal::from_ratio(current_collateral_amount, Uint128::one())
    )?;
    //Calc user's ratio of debt
    //We add 1 to make sure exit's can't leave more debt due to rounding
    let user_ratio_of_debt = decimal_multiplication(user_ratio_of_collateral, Decimal::from_ratio(current_credit_amount, Uint128::one()))?.to_uint_ceil();
    //Set total debt withdrawal which includes the LTV reset amount
    let total_debt_withdrawal = user_ratio_of_debt + amount_to_reset_to_mint_LTV;
    //Withdraw this amount of debt from the SP
    let withdraw_debt_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.stability_pool_contract_addr.clone(),
        msg: to_json_binary(&SP_ExecuteMsg::Withdraw {
            amount: total_debt_withdrawal,
        })?,
        funds: vec![],
    });
    msgs.push(withdraw_debt_msg);

    //Repay this debt to the CDP position
    let repay_debt_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.cdp_contract_addr.to_string(),
        msg: to_json_binary(&CDP_ExecuteMsg::Repay { 
            position_id: config.cdp_position_id,
            position_owner: None,
            send_excess_to: None,
        })?,
        funds: vec![
            Coin {
                denom: config.cdt_denom.clone(),
                amount: total_debt_withdrawal,
            }
        ],
    });
    msgs.push(repay_debt_msg);
    /////////////////////////////////////////////////////////////////

    //Withdraw the collateral from the CDP
    let withdraw_collateral_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.cdp_contract_addr.to_string(),
        msg: to_json_binary(&CDP_ExecuteMsg::Withdraw { 
            position_id: config.cdp_position_id,
            assets: vec![
                Asset {
                    info: AssetInfo::NativeToken {
                        denom: config.clone().deposit_token,
                    },
                    amount: user_ratio_of_collateral.to_uint_floor(),
                }
            ],
            send_to: None,
        })?,
        funds: vec![],
    });
    msgs.push(withdraw_collateral_msg);    

    //Send the deposit tokens to the user
    let send_deposit_to_user_msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
        to_address: info.clone().sender.to_string(),
        amount: vec![Coin {
            denom: config.deposit_token.clone(),
            amount: deposit_tokens_to_withdraw,
        }],
    });
    msgs.push(send_deposit_to_user_msg);
    
    //Burn vault tokens
    let burn_vault_tokens_msg: CosmosMsg = TokenFactory::MsgBurn {
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
            denom: config.vault_token.clone(),
            amount: vault_tokens.to_string(),
        }), 
        burn_from_address: env.contract.address.to_string(),
    }.into();
    //UNCOMMENT
    msgs.push(burn_vault_tokens_msg);

    //Update the total vault tokens
    let new_vault_token_supply = match total_vault_tokens.checked_sub(vault_tokens){
        Ok(v) => v,
        Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to subtract vault token total supply: {} - {}", total_vault_tokens, vault_tokens) }),
    };
    VAULT_TOKEN.save(deps.storage, &new_vault_token_supply)?;
       
    //Add rate assurance callback msg if this withdrawal leaves other depositors with tokens to withdraw
    if !new_vault_token_supply.is_zero() && total_deposit_tokens > deposit_tokens_to_withdraw {
        //UNCOMMENT
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::StateAssurance { 
                skip_LTV: false,
                skip_cost: true
            })?,
            funds: vec![],
        }));
    } 

    //Create Response 
    let res = Response::new()
        .add_attribute("method", "exit_vault")
        .add_attribute("vault_tokens", vault_tokens)
        .add_attribute("deposit_tokens_withdrawn_to_exit_with", deposit_tokens_to_withdraw)
        .add_messages(msgs);

    Ok(res)
}
 
/// Update contract configuration
/// This function is only callable by an owner with non_token_contract_auth set to true
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    owner: Option<String>,
    cdp_contract_addr: Option<String>,
    stability_pool_contract_addr: Option<String>,
    osmosis_proxy_contract_addr: Option<String>,
    oracle_contract_addr: Option<String>,
    swap_slippage: Option<Decimal>,
    vault_cost_index: Option<()>,
    cdp_position_id: Option<()>,
    cost_ceiling: Option<Decimal>,
    mint_LTV: Option<Decimal>,
    repay_LTV: Option<Decimal>,
) -> Result<Response, TokenFactoryError> {
    let mut config = CONFIG.load(deps.storage)?;

    //Assert Authority
    if info.sender != config.owner {
        //Check if ownership transfer is in progress & transfer if so
        if info.sender == OWNERSHIP_TRANSFER.load(deps.storage)? {
            config.owner = info.sender;
        } else {
            return Err(TokenFactoryError::Unauthorized {});
        }
    }

    let mut attrs = vec![attr("method", "update_config")];
    //Save optionals
    if let Some(addr) = owner {
        let valid_addr = deps.api.addr_validate(&addr)?;

        //Set owner transfer state
        OWNERSHIP_TRANSFER.save(deps.storage, &valid_addr)?;
        attrs.push(attr("owner_transfer", valid_addr));  
    }
    if let Some(addr) = cdp_contract_addr {
        config.cdp_contract_addr = deps.api.addr_validate(&addr)?;
        attrs.push(attr("updated_cdp_contract_addr", addr));
    }
    if let Some(addr) = stability_pool_contract_addr {
        config.stability_pool_contract_addr = deps.api.addr_validate(&addr)?.to_string();
        attrs.push(attr("updated_stability_pool_contract_addr", addr));
    }
    if let Some(addr) = osmosis_proxy_contract_addr {
        config.osmosis_proxy_contract_addr = deps.api.addr_validate(&addr)?;
        attrs.push(attr("updated_osmosis_proxy_contract_addr", addr));
    }
    if let Some(addr) = oracle_contract_addr {
        config.oracle_contract_addr = deps.api.addr_validate(&addr)?;
        attrs.push(attr("updated_oracle_contract_addr", addr));
    }
    if let Some(cost_ceiling) = cost_ceiling {
        config.cost_ceiling = cost_ceiling;
        attrs.push(attr("updated_cost_ceiling", cost_ceiling.to_string()));
    }
    if let Some(mint_LTV) = mint_LTV {
        config.mint_LTV = mint_LTV;
        attrs.push(attr("updated_mint_LTV", mint_LTV.to_string()));
    }
    if let Some(repay_LTV) = repay_LTV {
        config.repay_LTV = repay_LTV;
        attrs.push(attr("updated_repay_LTV", repay_LTV.to_string()));
    }
    if let Some(slippage) = swap_slippage {
        config.swap_slippage = slippage;
        attrs.push(attr("updated_swap_slippage", slippage.to_string()));
    }
    if let Some(_) = vault_cost_index {
        //Query the basket to find the index of the deposit token
        let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
            config.cdp_contract_addr.to_string(),
            &CDP_QueryMsg::GetBasket { },
        ){
            Ok(basket) => basket,
            Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP Basket") }),
        };
        //Find the index
        let mut saved_index: Option<u64> = None;
        for (index, asset) in basket.clone().collateral_types.into_iter().enumerate(){
            if asset.asset.info.to_string() == config.deposit_token.clone() {
                saved_index = Some(index as u64);
                break;
            }
        }
        if let Some(index) = saved_index {
            config.vault_cost_index = index as usize;
        } else {
            return Err(TokenFactoryError::CustomError { val: String::from("Failed to find the vault token in the CDP Basket") });
        }    
    }
    
    if let Some(_) = cdp_position_id {
        let vault_position: Vec<BasketPositionsResponse> = match deps.querier.query_wasm_smart::<Vec<BasketPositionsResponse>>(
            config.cdp_contract_addr.to_string(),
            &CDP_QueryMsg::GetBasketPositions { 
                start_after: None, 
                user: Some(env.contract.address.to_string()),
                user_info: None, 
                limit: None, 
            },
        ){
            Ok(vault_position) => vault_position,
            Err(err) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP Position for the vault token amount in update_config:") + &err.to_string() }),
        };
        let vault_position: PositionResponse = vault_position[0].positions[0].clone(); 
        //Set position ID
        config.cdp_position_id = vault_position.position_id; 
    }
    CONFIG.save(deps.storage, &config)?;
    attrs.push(attr("updated_config", format!("{:?}", config)));

    Ok(Response::new().add_attributes(attrs))
}


fn crank_realized_apr(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    //Load state
    let mut config = CONFIG.load(deps.storage)?; 
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;    
    //Get total deposit tokens
    let (
        total_deposit_tokens,
        _, 
        _, 
        _, 
        _, 
        _
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

    //Update Claim tracker
    let mut claim_tracker = CLAIM_TRACKER.load(deps.storage)?;
    //Calculate time since last claim
    let time_since_last_checkpoint = env.block.time.seconds() - claim_tracker.last_updated;   
    
    //Calc the rate of vault tokens to deposit tokens
    let btokens_per_one = calculate_base_tokens(
        Uint128::new(100_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    //If the current rate is the same as the last rate, update the time since last checkpoint & return 
    if claim_tracker.vt_claim_checkpoints.len() > 0 && claim_tracker.vt_claim_checkpoints.last().unwrap().vt_claim_of_checkpoint == btokens_per_one {
        //Update time since last checkpoint
        claim_tracker.vt_claim_checkpoints.last_mut().unwrap().time_since_last_checkpoint += time_since_last_checkpoint;               
        //Update last updated time
        claim_tracker.last_updated = env.block.time.seconds();
        //Save Claim Tracker
        CLAIM_TRACKER.save(deps.storage, &claim_tracker)?;

        return Ok(Response::new().add_attributes(vec![
            attr("method", "crank_realized_apr"),
            attr("no_change_to_conversion_rate", btokens_per_one),
            attr("added_time_to__checkpoint", time_since_last_checkpoint.to_string())
        ]));
    }


    //If the trackers total time is over a year, remove the first instance
    // if claim_tracker.vt_claim_checkpoints.len() > 0 && claim_tracker.vt_claim_checkpoints.iter().map(|claim_checkpoint| claim_checkpoint.time_since_last_checkpoint).sum::<u64>() > SECONDS_PER_DAY * 365 {
    //     claim_tracker.vt_claim_checkpoints.remove(0);
    // }
    //Push new instance
    claim_tracker.vt_claim_checkpoints.push(VTClaimCheckpoint {
        vt_claim_of_checkpoint: btokens_per_one,
        time_since_last_checkpoint,
    });
    //Update last updated time
    claim_tracker.last_updated = env.block.time.seconds();
    //Save Claim Tracker
    CLAIM_TRACKER.save(deps.storage, &claim_tracker)?;

    Ok(Response::new().add_attributes(vec![
        attr("method", "crank_realized_apr"),
        attr("new_base_token_conversion_rate", btokens_per_one),
        attr("time_since_last_checkpoint", time_since_last_checkpoint.to_string())
    ]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::VaultTokenUnderlying { vault_token_amount } => to_json_binary(&query_vault_token_underlying(deps, env, vault_token_amount)?),
        QueryMsg::ClaimTracker {} => to_json_binary(&CLAIM_TRACKER.load(deps.storage)?),
    }
}

/// Return underlying deposit token amount for an amount of vault tokens
fn query_vault_token_underlying(
    deps: Deps,
    env: Env,
    vault_token_amount: Uint128,
) -> StdResult<Uint128> {
    let config = CONFIG.load(deps.storage)?;
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    
    //Get total deposit tokens
    let (
        total_deposit_tokens,
        _, 
        _, 
        _, 
        _, 
        _
    ) = get_total_deposit_tokens(deps, env.clone(), config.clone())?;
    //Calc the amount of deposit tokens the user owns
    let users_base_tokens = calculate_base_tokens(
        vault_token_amount, 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    // println!("total_deposit_tokens: {:?}, total_vault_tokens: {:?}, vault_token_amount: {:?}, users_base_tokens: {:?}", total_deposit_tokens, total_vault_tokens, vault_token_amount, users_base_tokens);

    //Return
    Ok(users_base_tokens)
}

//This calcs the value of tokens backing the CDP minus any debt-based losses.
// - A loss is defined as debt higher than outstanding minted debt due to accrued interest rates.
fn get_total_deposit_tokens(
    deps: Deps,
    env: Env,
    config: Config,
) -> StdResult<(Uint128, Uint128, Uint128, Uint128, PriceResponse, PriceResponse)> { //total deposit tokens, contract_SP_tvl, collateral_amount, debt_amount, deposit_token_price, cdt_peg_price
    //Get CDT price
    let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasket {  },
    ){
        Ok(basket) => basket,
        Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the CDP basket in get_total_deposit_tokens") }),
    };
    let cdt_peg_price: PriceResponse = basket.credit_price;

    //Get deposit token price
    let prices: Vec<PriceResponse> = match deps.querier.query_wasm_smart::<Vec<PriceResponse>>(
        config.oracle_contract_addr.to_string(),
        &Oracle_QueryMsg::Price {
            asset_info: AssetInfo::NativeToken{ denom: config.clone().deposit_token },
            twap_timeframe: 60, //We want the price the CDP will use
            oracle_time_limit: 600,
            basket_id: None
        },
    ){
        Ok(prices) => prices,
        Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the deposit_token_price in get_total_deposit_tokens") }),
    };
    let deposit_token_price: PriceResponse = prices[0].clone();
    //Query the CDP position
    let vault_position: Vec<BasketPositionsResponse> = match deps.querier.query_wasm_smart::<Vec<BasketPositionsResponse>>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasketPositions { 
            start_after: None, 
            user: None,
            user_info: Some(UserInfo {
                position_owner: env.contract.address.to_string(),
                position_id: config.cdp_position_id,
            }), 
            limit: None, 
        },
    ){
        Ok(vault_position) => vault_position,
        Err(err) => return Err(StdError::GenericErr { msg: String::from("Failed to query the CDP Position for the vault token amount in get_total_deposit_tokens:") + &err.to_string() }),
    };
    let vault_position: PositionResponse = vault_position[0].positions[0].clone();
    //Set current collateral
    let collateral_amount = vault_position.collateral_assets[0].asset.amount;
    let debt_amount = vault_position.credit_amount;

    //////How much CDT is in the SP////////
    //Query the SP asset pool
    let asset_pool: AssetPool = deps.querier.query_wasm_smart::<AssetPool> (
        config.stability_pool_contract_addr.clone(),
        &SP_QueryMsg::AssetPool { 
            user: Some(env.contract.address.to_string()),
            deposit_limit: None,
            start_after: None,
        },
    )?;
    //Calc total TVL in the SP
    let contract_SP_tvl: Uint128 = asset_pool.deposits.clone().into_iter()    
        .map(|deposit| deposit.amount)
        .sum::<Decimal>().to_uint_floor();

    //Set total CDT 
    let total_outstanding_debt_tokens = contract_SP_tvl;

    //If debt amount is more than total outstanding CDT, 
    //...subtract the value from the total collateral to calc total deposit tokens
    match debt_amount.checked_sub(total_outstanding_debt_tokens){
        Ok(current_unrealized_cost) => {
            
            //Calc value of the value lost
            let loss_value = cdt_peg_price.get_value(current_unrealized_cost)?;
            //Calc value of the collateral
            let collateral_value = deposit_token_price.get_value(collateral_amount)?;
            //Calc the value of the collateral minus the debt
            let liquid_value = match collateral_value.checked_sub(loss_value){
                Ok(v) => v,
                Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to subtract the excess debt from the collateral in get_total_deposit_tokens, collateral value: {}, excess debt value: {}", collateral_value, loss_value) }),
            };
            
            //Calc the amount of vaulted deposit tokens
            let total_deposit_tokens = deposit_token_price.get_amount(liquid_value)?;

            return Ok((total_deposit_tokens, contract_SP_tvl, collateral_amount, debt_amount, deposit_token_price, cdt_peg_price))
        },
        Err(_) => return Ok((collateral_amount, contract_SP_tvl, collateral_amount, debt_amount, deposit_token_price, cdt_peg_price))
    }

    
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.id {
        CDP_REPLY_ID => handle_cdp_reply(deps, env, msg),
        COMPOUND_REPLY_ID => handle_compound_reply(deps, env, msg),
        INITIATE_EXIT_REPLY_ID => exit_vault(deps, env),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
} 

//1) Deposit the deposit tokens into the CDP position
fn handle_compound_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_result) => {
            //Load config
            let config = CONFIG.load(deps.storage)?;  
               
            //Query balance for the deposit token received from the exit vault
            let deposit_token_balance = deps.querier.query_balance(env.contract.address.to_string(), config.clone().deposit_token)?.amount;   

            //Redeposit the deposit token into the CDP
            let redeposit_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.cdp_contract_addr.to_string(),
                msg: to_json_binary(&CDP_ExecuteMsg::Deposit { 
                    position_id: Some(config.cdp_position_id),
                    position_owner: None,
                })?,
                funds: vec![
                    Coin {
                        denom: config.deposit_token.clone(),
                        amount: deposit_token_balance,
                    }
                ],
            });

            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_compound_reply")
                .add_attribute("deposit_token_swapped_for", deposit_token_balance)
                .add_message(redeposit_msg);

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

fn handle_cdp_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(result) => {
            let cdp_event = result
                .events
                .into_iter()
                .find(|e| e.attributes.iter().any(|attr| attr.key == "position_id"))
                .ok_or_else(|| StdError::GenericErr {  msg: String::from("unable to find cdp deposit event")})?;

                let position_id = &cdp_event
                .attributes
                .iter()
                .find(|attr| attr.key == "position_id")
                .unwrap()
                .value;
                let position_id = Uint128::from_str(position_id)?;
            //Load config
            let mut config = CONFIG.load(deps.storage)?;  
            //Save the position ID
            config.cdp_position_id = position_id;
            //Save Updated Config
            CONFIG.save(deps.storage, &config)?;

            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_initial_cdp_deposit_reply")
                .add_attribute("vault_position_id", position_id);  

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}


#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, env: Env, _msg: MigrateMsg) -> Result<Response, TokenFactoryError> {

    Ok(Response::default())
}