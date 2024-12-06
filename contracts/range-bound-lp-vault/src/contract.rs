use std::cmp::min;
use std::convert::TryInto;
use std::env;
use std::str::FromStr;

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, to_json_binary, Order, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, QuerierWrapper, Reply, Response, StdError, StdResult, SubMsg, Uint128, WasmMsg
};
use cw2::set_contract_version;
use cw_storage_plus::Bound;
use membrane::math::{decimal_multiplication, decimal_division};
use membrane::oracle::PriceResponse;

use crate::error::TokenFactoryError;
use crate::state::{IntentProp, TokenRateAssurance, RepayProp, CDP_REPAY_PROPAGATION, CLAIM_TRACKER, CONFIG, INTENT_PROPAGATION, OWNERSHIP_TRANSFER, TOKEN_RATE_ASSURANCE, USER_INTENT_STATE, VAULT_TOKEN};
use membrane::range_bound_lp_vault::{
    Config, ExecuteMsg, InstantiateMsg, LeaveTokens, MigrateMsg, QueryMsg, UserIntentResponse
};
use membrane::stability_pool_vault::{
    calculate_base_tokens, calculate_vault_tokens
};
use membrane::osmosis_proxy::ExecuteMsg as OP_ExecuteMsg;
use membrane::cdp::ExecuteMsg as CDP_ExecuteMsg;
use membrane::oracle::QueryMsg as Oracle_QueryMsg;
use membrane::types::{Asset, AssetInfo, ClaimTracker, RangeBoundUserIntents, RangePositions, UserInfo, VTClaimCheckpoint, UserIntentState};
use osmosis_std::types::osmosis::tokenfactory::v1beta1::{self as TokenFactory};
use osmosis_std::types::osmosis::poolmanager::v1beta1::MsgSwapExactAmountIn;
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::{self as CL, FullPositionBreakdown};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:range-bound-lp-vault";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_LIMIT: u32 = 32;

//Reply IDs
const SWAP_ADD_TO_FLOOR_REPLY_ID: u64 = 1u64;
const SWAP_ADD_TO_CEILING_REPLY_ID: u64 = 2u64;
const ADD_TO_FLOOR_REPLY_ID: u64 = 3u64;
const ADD_TO_CEILING_REPLY_ID: u64 = 4u64;
const CL_POSITION_CREATION_REPLY_ID: u64 = 5u64;
const SWAP_TO_FLOOR_ADD_BOTH_REPLY_ID: u64 = 6u64;
const SWAP_TO_CEILING_ADD_BOTH_REPLY_ID: u64 = 7u64;
const SWAP_FOR_CDP_REPAY_REPLY_ID: u64 = 8u64;
const SWAP_AFTER_EXIT_FOR_CDP_REPAY_REPLY_ID: u64 = 9u64;
const MANAGE_ERROR_DENIAL_REPLY_ID: u64 = 99u64;

//Reply IDs for intents
const PURCHASE_POST_EXIT_REPLY_ID: u64 = 10u64;
const PARSE_PURCHASE_INTENTS_REPLY_ID: u64 = 11u64;


//NOTE: When we add a WITHDRAWAL_PERIOD, it'll be added to the intents state object that holds User's VTs and saves their unstake time. 
//Then when its time to withdraw, the contract will use & burn the VTs in the contract and send the user their assets.

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, TokenFactoryError> {
    let config = Config {
        owner: info.sender.clone(),
        vault_token: String::from("factory/".to_owned() + env.contract.address.as_str() + "/" + msg.clone().vault_subdenom.as_str()),
        range_tokens: msg.clone().range_tokens,
        range_bounds: msg.clone().range_bounds,
        range_position_ids: RangePositions {
            ceiling: 0u64,
            floor: 0u64,
        },
        osmosis_proxy_contract_addr: deps.api.addr_validate(&msg.osmosis_proxy_contract_addr)?.to_string(),
        oracle_contract_addr: deps.api.addr_validate(&msg.oracle_contract_addr)?.to_string(),
    };
    //Create the vault's initial positions
    let mut submsgs: Vec<SubMsg> = vec![];
    //Ceiling
    let ceiling_creation_msg: CosmosMsg = CL::MsgCreatePosition { 
        pool_id: 1268u64, 
        sender: env.contract.address.to_string(), 
        lower_tick: config.range_bounds.ceiling.lower_tick, 
        upper_tick: config.range_bounds.ceiling.upper_tick, 
        //1 CDT
        tokens_provided: vec![Coin {
            denom: config.range_tokens.ceiling_deposit_token.clone(),
            amount: Uint128::new(1_000_000),
        }.into()], 
        token_min_amount0: String::from("0"), 
        token_min_amount1: String::from("0")
    }.into();
    submsgs.push(SubMsg::reply_on_success(ceiling_creation_msg, CL_POSITION_CREATION_REPLY_ID));
    //Floor
    let floor_creation_msg: CosmosMsg = CL::MsgCreatePosition { 
        pool_id: 1268u64,
        sender: env.contract.address.to_string(), 
        lower_tick: config.range_bounds.floor.lower_tick, 
        upper_tick: config.range_bounds.floor.upper_tick, 
        //1 USDC
        tokens_provided: vec![Coin {
            denom: config.range_tokens.floor_deposit_token.clone(),
            amount: Uint128::new(1_000_000),
        }.into()], 
        token_min_amount0: String::from("0"), 
        token_min_amount1: String::from("0")
    }.into(); 
    submsgs.push(SubMsg::reply_on_success(floor_creation_msg, CL_POSITION_CREATION_REPLY_ID));

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    //Save initial state
    CONFIG.save(deps.storage, &config)?;
    CLAIM_TRACKER.save(deps.storage, &ClaimTracker {
        vt_claim_checkpoints: vec![
            VTClaimCheckpoint {
                vt_claim_of_checkpoint: Uint128::new(1_000_000), //Assumes the decimal of the deposit token is 6
                time_since_last_checkpoint: 0u64,
            }
        ],
        last_updated: env.block.time.seconds(),
    })?;
    VAULT_TOKEN.save(deps.storage, &Uint128::zero())?;
    //Create Msg
    let denom_msg = TokenFactory::MsgCreateDenom { sender: env.contract.address.to_string(), subdenom: msg.vault_subdenom.clone() };
    
    //Create Response
    let res = Response::new()
        .add_attribute("method", "instantiate")
        .add_attribute("config", format!("{:?}", config))
        .add_attribute("contract_address", env.contract.address)
        .add_attribute("sub_denom", msg.clone().vault_subdenom)
    //UNCOMMENT
        .add_message(denom_msg)
        .add_submessages(submsgs);
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
        ExecuteMsg::UpdateConfig { owner, osmosis_proxy_contract_addr, oracle_contract_addr  } => update_config(deps, info, owner, osmosis_proxy_contract_addr, oracle_contract_addr),
        ExecuteMsg::EnterVault { leave_vault_tokens_in_vault } => enter_vault(deps, env, info, leave_vault_tokens_in_vault),
        ExecuteMsg::ExitVault { send_to } => exit_vault(deps, env, info, send_to),
        ExecuteMsg::ManageVault { rebalance_sale_max } => manage_vault(deps, env, info, rebalance_sale_max),
        ExecuteMsg::SetUserIntents { intents, reduce_vault_tokens } => set_intents(deps, env, info, intents, reduce_vault_tokens),
        ExecuteMsg::FulFillUserIntents { users } => fulfill_intents(deps, env, info, users),
        ExecuteMsg::RepayUserDebt { user_info, repayment } => repay_user_debt(deps, env, info, user_info, repayment),
        ExecuteMsg::CrankRealizedAPR { } => crank_realized_apr(deps, env, info),
        ExecuteMsg::RateAssurance { } => rate_assurance(deps, env, info),
        ExecuteMsg::DepositFee { } => deposit_fee(deps, env, info),
    }
}

//Accept the revenue distribution from the CDP contract
fn deposit_fee(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo
) -> Result<Response, TokenFactoryError> {
    //Load state
    let config = CONFIG.load(deps.storage)?;

    //Assert that the fee is in the ceiling asset
    if config.range_tokens.ceiling_deposit_token != info.funds[0].denom {
        return Err(TokenFactoryError::CustomError { val: format!("Wrong asset sent: {}. Should be the ceiling asset: {}", info.funds[0].denom, config.range_tokens.ceiling_deposit_token) });
    }

    //Get the amount of deposit tokens sent
    let deposit_amount = info.funds[0].amount;

    //Create response
    let res = Response::new()
        .add_attribute("method", "deposit_fee")
        .add_attribute("deposit_amount", deposit_amount)
        .add_attribute("deposit_token", config.range_tokens.ceiling_deposit_token);

    Ok(res)
}

//Includes:
// CL Positions' balances
// Yield: (so we don't need to force a compound before entering the vault)
// - Contract's balance of tokens
// - CL Positions' rewards
fn get_total_deposit_tokens(
    deps: Deps,
    env: Env,
    config: Config,
    //total_deposit_tokens, ceiling_price, floor_price, ceiling_position_coins, floor_position_coins, ceiling position liquidity, floor position liquidity
) -> StdResult<(Uint128, PriceResponse, PriceResponse, Vec<Coin>, Vec<Coin>, FullPositionBreakdown, FullPositionBreakdown)> {
    
    //Get token prices
    let (ceiling_price, floor_price) = get_range_token_prices(deps.querier, config.clone())?;

    //Init token totals
    let mut total_ceiling_tokens: Uint128 = Uint128::zero();
    let mut total_floor_tokens: Uint128 = Uint128::zero();

    //Get tokens in the contract
    let balance_of_ceiling_tokens = deps.querier.query_balance(env.contract.address.clone(), config.clone().range_tokens.ceiling_deposit_token)?.amount;
    let balance_of_floor_tokens = deps.querier.query_balance(env.contract.address.clone(), config.clone().range_tokens.floor_deposit_token)?.amount;

    //Accumulate tokens
    total_ceiling_tokens += balance_of_ceiling_tokens;
    total_floor_tokens += balance_of_floor_tokens;

    ////Add the tokens and the spread rewards from the CL positions//////
    //Create CL Querier
    let cl_querier = CL::ConcentratedliquidityQuerier::new(&deps.querier);
    //Query CL Positions
    let ceiling_position_response: CL::PositionByIdResponse = cl_querier.position_by_id(config.range_position_ids.ceiling)?;
    if ceiling_position_response.position.is_none() {
        return Err(StdError::GenericErr { msg: format!("Failed to query the ceiling position: {}", config.range_position_ids.ceiling) });
    }
    let ceiling_position = ceiling_position_response.position.unwrap();
    //
    let floor_position_response: CL::PositionByIdResponse = cl_querier.position_by_id(config.range_position_ids.floor)?;
    if floor_position_response.position.is_none() {
        return Err(StdError::GenericErr { msg: format!("Failed to query the floor position: {}", config.range_position_ids.floor) });
    }
    let floor_position = floor_position_response.position.unwrap();
    //
    //Initialize Coin propogation variables
    let mut ceiling_position_coins: Vec<osmosis_std::types::cosmos::base::v1beta1::Coin> = vec![];
    let mut floor_position_coins: Vec<osmosis_std::types::cosmos::base::v1beta1::Coin> = vec![];
    //Condense the ceiling position's possible coins into 1 array
    //Asset 0
    if let Some(coin) = ceiling_position.clone().asset0 {        
        //Add to the coins array
        ceiling_position_coins.push(coin.clone());
        //Add to the totals
        if coin.denom == config.range_tokens.ceiling_deposit_token {
            //Add to the total
            total_ceiling_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
        if coin.denom == config.range_tokens.floor_deposit_token {
            //Add to the total
            total_floor_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
    }
    //Asset 1
    if let Some(coin) = ceiling_position.clone().asset1 {
        //Add to the coins array
        ceiling_position_coins.push(coin.clone());
        //Add to the totals
        if coin.denom == config.range_tokens.ceiling_deposit_token {
            //Add to the total
            total_ceiling_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
        if coin.denom == config.range_tokens.floor_deposit_token {
            //Add to the total
            total_floor_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
    }
    //Find and accumulate the tokens in the positions
    ceiling_position.clone().claimable_spread_rewards.into_iter().for_each(|coin| {
        if coin.denom == config.range_tokens.ceiling_deposit_token {
            total_ceiling_tokens +=  Uint128::from_str(&coin.amount).unwrap();
        }
        if coin.denom == config.range_tokens.floor_deposit_token {
            total_floor_tokens +=  Uint128::from_str(&coin.amount).unwrap();
        }
    });
    //Condense the floor position's possible coins into 1 array
    //Asset 0
    if let Some(coin) = floor_position.clone().asset0 {
        //Add to the coins array
        floor_position_coins.push(coin.clone());
        //Add to the totals
        if coin.denom == config.range_tokens.ceiling_deposit_token {
            //Add to the total
            total_ceiling_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
        if coin.denom == config.range_tokens.floor_deposit_token {
            //Add to the total
            total_floor_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
    }
    //Asset 1
    if let Some(coin) = floor_position.clone().asset1 {
        //Add to the coins array
        floor_position_coins.push(coin.clone());
        //Add to the totals
        if coin.denom == config.range_tokens.ceiling_deposit_token {
            //Add to the total
            total_ceiling_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
        if coin.denom == config.range_tokens.floor_deposit_token {
            //Add to the total
            total_floor_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
    }
    //Find and accumulate the tokens in the positions
    floor_position.clone().claimable_spread_rewards.into_iter().for_each(|coin| {
        if coin.denom == config.range_tokens.ceiling_deposit_token {
            total_ceiling_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
        if coin.denom == config.range_tokens.floor_deposit_token {
            total_floor_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
    });

    //Calc value of the tokens
    let total_ceiling_value = ceiling_price.get_value(total_ceiling_tokens)?;
    let total_floor_value = floor_price.get_value(total_floor_tokens)?;
    let total_value = total_ceiling_value + total_floor_value;

    //Convert total value into total CDT (ceiling token)
    let total_deposit_tokens = ceiling_price.get_amount(total_value)?;

    //Convert the coins into the correct format
    let ceiling_position_coins: Vec<Coin> = ceiling_position_coins.iter().map(|coin| {
        Coin {
            denom: coin.denom.clone(),
            amount: Uint128::from_str(&coin.amount).unwrap(),
        }
    }).collect();
    let floor_position_coins: Vec<Coin> = floor_position_coins.iter().map(|coin| {
        Coin {
            denom: coin.denom.clone(),
            amount: Uint128::from_str(&coin.amount).unwrap(),
        }
    }).collect();

    Ok((
        total_deposit_tokens, 
        ceiling_price, 
        floor_price, 
        ceiling_position_coins,
        floor_position_coins,
        ceiling_position,
        floor_position,
    ))
}

///Rate assurance
/// Ensures that the conversion rate is static for deposits & withdrawals
fn rate_assurance(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    //Load config    
    let config = CONFIG.load(deps.storage)?;

    //Error if not the contract calling
    if info.sender != env.contract.address {
        return Err(TokenFactoryError::Unauthorized {});
    }

    //Load Token Assurance State
    let token_rate_assurance = TOKEN_RATE_ASSURANCE.load(deps.storage)?;

    //Load Vault token supply
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;

    //Get total_deposit_tokens & prices
    let (
        total_deposit_tokens,
        _, 
        _,
        _,
        _,
        _,
        _,
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

    //Calc the rate of vault tokens to deposit tokens
    let btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    //For deposit or withdraw, check that the rates are static 
    if btokens_per_one != token_rate_assurance.pre_btokens_per_one {
        return Err(TokenFactoryError::CustomError { val: format!("Deposit or withdraw rate assurance failed for base token conversion. pre: {:?} --- post: {:?}", token_rate_assurance.pre_btokens_per_one, btokens_per_one) });
    }

    Ok(Response::new())
}

fn get_range_token_prices(
    querier: QuerierWrapper,
    config: Config,
) -> StdResult<(PriceResponse, PriceResponse)> {
    
    let prices: Vec<PriceResponse> = match querier.query_wasm_smart::<Vec<PriceResponse>>(
        config.oracle_contract_addr.to_string(),
        &Oracle_QueryMsg::Prices {
            asset_infos: vec![
                AssetInfo::NativeToken{ denom: config.clone().range_tokens.ceiling_deposit_token },
                AssetInfo::NativeToken{ denom: config.clone().range_tokens.floor_deposit_token }
                ],
            twap_timeframe: 0, //We want market price
            oracle_time_limit: 10,
        },
    ){
        Ok(prices) => prices,
        Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the deposit_token_price in get_range_token_prices") }),
    };
    let ceiling_deposit_token_price: PriceResponse = prices[0].clone();
    let floor_deposit_token_price: PriceResponse = prices[1].clone();

    Ok((ceiling_deposit_token_price, floor_deposit_token_price))
}



/// Enter the vault 50:50, 50% CDT - 50% USDC. (1% margin of error)
/// Since the contract would swap to balance anyway..
/// ...accepting 50:50 allows the App to swap into deposits or accept both assets...
/// ...instead of always swapping into CDT and then the vault swapping back to balance.
/// 50:50 in VALUE. If we do amount, the CDT side would be overweight bc the range is (expected to be) underpeg.
/// 
/// //Enter 100% CDT
fn enter_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    leave_vault_tokens_in_vault: Option<LeaveTokens>
) -> Result<Response, TokenFactoryError> {
    let config = CONFIG.load(deps.storage)?;

    //Assert the sender sent both assets in the correct order
    if info.funds.len() != 1 || info.funds[0].denom != config.range_tokens.ceiling_deposit_token {
        return Err(TokenFactoryError::CustomError { val: format!("Need to send CDT only: {}",  config.clone().range_tokens.ceiling_deposit_token) });
    }
    
    //Get the amount of deposit tokens sent
    let ceiling_deposit_amount = info.funds[0].amount;

    //Get total_deposit_tokens & prices
    let (
        total_deposit_tokens,
        _, 
        _,
        _,
        _,
        _,
        _,
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;


    //Normalize the deposit amounts into the ceiling token (CDT)
    let normalized_deposit_amount = ceiling_deposit_amount;

    //////Calculate the amount of vault tokens to mint////
    //Get the total amount of vault tokens circulating
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    //Update the total deposit tokens after vault tokens are minted 
    //..because the total_deposit_tokens counts the deposit tokens in the contract
    let mut pre_deposit_total_deposit_tokens = total_deposit_tokens - normalized_deposit_amount;
    //if this is the first deposit, set the total to 0 bc it'll be non-zero due to the init deposit. This results in 0 VTs to mint.
    if total_vault_tokens.is_zero() {
        pre_deposit_total_deposit_tokens = Uint128::zero();
    }
    //Calc & save token rates
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        pre_deposit_total_deposit_tokens, 
        total_vault_tokens
    )?;
    TOKEN_RATE_ASSURANCE.save(deps.storage, &TokenRateAssurance {
        pre_btokens_per_one,
    })?;
    //Calculate the amount of vault tokens to mint
    let vault_tokens_to_distribute = calculate_vault_tokens(
        normalized_deposit_amount, 
        pre_deposit_total_deposit_tokens, 
        total_vault_tokens
    )?;
    ////////////////////////////////////////////////////
    
    

    let mut msgs = vec![];
    let mut vault_tokens_to_send = vault_tokens_to_distribute;

    //Check if we are minting any tokens to the contract
    if let Some( mut intent_info ) = leave_vault_tokens_in_vault {
        //Ensure % is less than 100
        let percent_to_send_to_vault = min(intent_info.percent_to_leave, Decimal::one());
        //Calc the amount of vault tokens to leave in the vault
        let vault_tokens_to_leave = decimal_multiplication(
            percent_to_send_to_vault,
            Decimal::from_ratio(vault_tokens_to_distribute, Uint128::one())
        )?.to_uint_floor();
        //If the user wants to leave some vault tokens in the vault
        if !vault_tokens_to_leave.is_zero() {
            
            //Calc the rate of vault tokens to deposit tokens
            let btokens_per_one = calculate_base_tokens(
                Uint128::new(1_000_000_000_000), 
                total_deposit_tokens, 
                total_vault_tokens
            )?;
            //Set conversion rate
            intent_info.intent_for_tokens.last_conversion_rate = btokens_per_one;

            //Calc the amount of vault tokens to send to the user
            vault_tokens_to_send = vault_tokens_to_distribute - vault_tokens_to_leave;
            //Mint vault tokens to the contract
            let mint_vault_tokens_to_contract_msg: CosmosMsg = TokenFactory::MsgMint {
                sender: env.contract.address.to_string(), 
                amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
                    denom: config.vault_token.clone(),
                    amount: vault_tokens_to_leave.to_string(),
                }), 
                mint_to_address: env.contract.address.to_string(),
            }.into();
            //UNCOMMENT
            msgs.push(mint_vault_tokens_to_contract_msg);
    

            //Set user.
            //if info.sender is the CDP contract, use the user from the Intents struct
            let user = if info.sender.to_string() == String::from("osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr") {
                intent_info.intent_for_tokens.user.to_string()
            } else {
                info.sender.to_string()
            };
            //Add the vault tokens to the user's state if they already have it.
            //Create or update the user's intent state
            USER_INTENT_STATE.update(deps.storage, user, |state| -> Result<UserIntentState, TokenFactoryError> {
                if let Some(mut user_intent_state) = state {
                    user_intent_state.vault_tokens += vault_tokens_to_leave;
                    return Ok(user_intent_state);
                } else {
                    let user_intent_state = UserIntentState {
                        vault_tokens: vault_tokens_to_leave,
                        intents: intent_info.intent_for_tokens,
                        //Unused until withdrawal period is added
                        unstake_time: 0u64,
                        fee_to_caller: Decimal::percent(1)
                    };
                    return Ok(user_intent_state)
                }
            })?;
        }
    }
    //Mint remaining vault tokens to the sender
    if !vault_tokens_to_send.is_zero() {
        let mint_vault_tokens_msg: CosmosMsg = TokenFactory::MsgMint {
            sender: env.contract.address.to_string(), 
            amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
                denom: config.vault_token.clone(),
                amount: vault_tokens_to_send.to_string(),
            }), 
            mint_to_address: info.sender.to_string(),
        }.into();
        //UNCOMMENT
        msgs.push(mint_vault_tokens_msg);

    }

    //Update the total vault tokens
    VAULT_TOKEN.save(deps.storage, &(total_vault_tokens + vault_tokens_to_distribute))?;

    //Save the updated config
    CONFIG.save(deps.storage, &config)?;

    //Add rate assurance callback msg
    if !total_vault_tokens.is_zero() {
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::RateAssurance { })?,
            funds: vec![],
        }));
    }

    //Add a manage vault msg that replys on error
    let manage_vault_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_json_binary(&ExecuteMsg::ManageVault { rebalance_sale_max: None })?,
        funds: vec![],
    });
    let manage_submsg = SubMsg::reply_on_error(manage_vault_msg, MANAGE_ERROR_DENIAL_REPLY_ID);

    //Create Response
    let res = Response::new()
        .add_attribute("method", "enter_vault")
        .add_attribute("deposit_amount", normalized_deposit_amount)
        .add_attribute("vault_tokens_to_distribute", vault_tokens_to_distribute)
        .add_messages(msgs)
        .add_submessage(manage_submsg);

    Ok(res)
}

/// Exit vault in the current ratio of assets owned (LP + balances) by withdrawing pro-rata CL shares.
/// The App can swap into a single token and give value options based on swap rate.
/// 1. We burn vault tokens
/// 2. Send the withdrawn tokens to the user/send_to address
fn exit_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    send_to: Option<String>
) -> Result<Response, TokenFactoryError> {
    let config = CONFIG.load(deps.storage)?;
    let mut msgs = vec![];
    //Set the send_to address
    let send_to = match send_to {
        Some(address) => address,
        None => info.sender.to_string(),
    };
    //Get total_deposit_tokens & prices
    let (
        total_deposit_tokens,
        _,
        _,
        ceiling_position_coins,
        floor_position_coins,
        ceiling_position,
        floor_position
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
    if total_deposit_tokens.is_zero() {
        return Err(TokenFactoryError::ZeroDepositTokens {});
    }

    let ceiling_liquidity = Decimal::from_str(&ceiling_position.position.unwrap().liquidity).unwrap();
    let floor_liquidity = Decimal::from_str(&floor_position.position.unwrap().liquidity).unwrap();

    //Assert the only token sent is the vault token
    if info.funds.len() != 1 {
        return Err(TokenFactoryError::CustomError { val: format!("More than 1 asset was sent, this function only accepts the vault token: {:?}", config.clone().vault_token) });
    }
    if info.funds[0].denom != config.vault_token {
        return Err(TokenFactoryError::CustomError { val: format!("The wrong asset was sent ({:?}), this function only accepts the vault token: {:?}", info.funds[0].denom, config.clone().vault_token) });
    }

    //Get the amount of vault tokens sent
    let vault_tokens = info.funds[0].amount;
    if vault_tokens.is_zero() {
        return Err(TokenFactoryError::ZeroAmount {});
    }

    //////Calculate the amount of deposit tokens to withdraw////
    //Get the total amount of vault tokens circulating
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    //Calc & save token rates
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    TOKEN_RATE_ASSURANCE.save(deps.storage, &TokenRateAssurance {
        pre_btokens_per_one,
    })?;
    //Calculate the amount of liquidity to withdraw
    let withdrawal_ratio = decimal_division(
        Decimal::from_ratio(vault_tokens, Uint128::one()),
        Decimal::from_ratio(total_vault_tokens, Uint128::one()),
    )?;
    ///////////////////////////////////
    //Set ceiling & floor withdrawal amount
    let ceiling_liquidity_to_withdraw = (decimal_multiplication(
        ceiling_liquidity,
        withdrawal_ratio
    )? * Uint128::new(10u64.pow(18 as u32) as u128)).to_string();
    let floor_liquidity_to_withdraw = (decimal_multiplication(
        floor_liquidity,
        withdrawal_ratio
    )? * Uint128::new(10u64.pow(18 as u32) as u128)).to_string();
    //Withdraw liquidity from both positions
    let ceiling_position_withdraw_msg: CosmosMsg = CL::MsgWithdrawPosition {
        position_id: config.range_position_ids.ceiling,
        sender: env.contract.address.to_string(),
        liquidity_amount: ceiling_liquidity_to_withdraw,
    }.into();
    //Add to msgs
    msgs.push(ceiling_position_withdraw_msg);
    let floor_position_withdraw_msg: CosmosMsg = CL::MsgWithdrawPosition {
        position_id: config.range_position_ids.floor,
        sender: env.contract.address.to_string(),
        liquidity_amount: floor_liquidity_to_withdraw,
    }.into();
    //Add to msgs
    msgs.push(floor_position_withdraw_msg);
    //Calculate the amount of tokens that will be withdrawn and should be sent to the user
    let mut user_withdrawn_coins = vec![];
    //Ceiling
    for coin in ceiling_position_coins {
        //Calc amount
        let amount = decimal_multiplication(
            Decimal::from_ratio(coin.amount, Uint128::one()),
            withdrawal_ratio
        )?.to_uint_floor().to_string();
        //Add to the user's withdrawn coins
        user_withdrawn_coins.push(Coin {
            denom: coin.denom.clone(),
            amount: Uint128::from_str(&amount).unwrap(),
        });
    };
    //Floor
    for coin in floor_position_coins {
        //Calc amount
        let amount = decimal_multiplication(
            Decimal::from_ratio(coin.amount, Uint128::one()),
            withdrawal_ratio
        )?.to_uint_floor().to_string();

        //Check if the coin already exists in the withdrawn_coins array.
        //If so, add the amount to the existing coin, else add a new coin.
        let mut coin_exists = false;
        user_withdrawn_coins.iter_mut().for_each(|withdrawn_coin| {
            if withdrawn_coin.denom == coin.denom {
                withdrawn_coin.amount += Uint128::from_str(&amount).unwrap();
                coin_exists = true;
            }
        });
        //Add to the user's withdrawn coins
        if !coin_exists {
            user_withdrawn_coins.push(Coin {
                denom: coin.denom.clone(),
                amount: Uint128::from_str(&amount).unwrap(),
            });
        }
    };

    //Send the withdrawn tokens to the user
    let send_deposit_tokens_msg: CosmosMsg = BankMsg::Send {
        to_address: send_to,
        amount: user_withdrawn_coins.clone(),
    }.into();
    //Add to msgs
    msgs.push(send_deposit_tokens_msg);

    //Burn vault tokens
    let burn_vault_tokens_msg: CosmosMsg = TokenFactory::MsgBurn {
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
            denom: config.vault_token.clone(),
            amount: vault_tokens.to_string(),
        }), 
        burn_from_address: env.contract.address.to_string(),
    }.into();
    //Add to msgs
    msgs.push(burn_vault_tokens_msg);

    //Update the total vault tokens
    let new_vault_token_supply = match total_vault_tokens.checked_sub(vault_tokens){
        Ok(v) => v,
        Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract vault token total supply: {} - {}", total_vault_tokens, vault_tokens) }),
    };
    //Update the total vault tokens
    VAULT_TOKEN.save(deps.storage, &new_vault_token_supply)?;
    //Save the updated config
    CONFIG.save(deps.storage, &config)?;

    //Add rate assurance callback msg if this withdrawal leaves other depositors with tokens to withdraw
    if !new_vault_token_supply.is_zero() && withdrawal_ratio != Decimal::one() {
        //UNCOMMENT
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::RateAssurance { })?,
            funds: vec![],
        }));
    } 


    //Create Response 
    let res = Response::new()
        .add_attribute("method", "exit_vault")
        .add_attribute("vault_tokens", vault_tokens)
        .add_attribute("deposit_tokens_withdrawn", format!("{:?}", user_withdrawn_coins))
        .add_messages(msgs);

    Ok(res)
}

/// Takes CDT balance (deposited revenue from DepositFee) & the Positions' spread fees to
/// .. either adds it to the ceiling or swaps it all (or rebalance_sale_max) to add to the floor
/// Flow: 
/// - ClaimSpreadFees
/// - Attempt to compound into ceiling or floor
/// - If price is in the ceiling, swap and deposit into floor. If its above, swap to floor and deposit into both.
/// - If price is in the floor, swap and deposit into ceiling. If its below, swap to ceiling and deposit into both.
fn manage_vault(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    rebalance_sale_max: Option<Decimal>
) -> Result<Response, TokenFactoryError> {

    //Load state
    let config = CONFIG.load(deps.storage)?;
    let mut msgs: Vec<SubMsg> = vec![];
    let mut position_ids: Vec<u64> = vec![];

    //Set the rebalance_sale_max
    let rebalance_sale_max = match rebalance_sale_max {
        Some(max) => min(max, Decimal::one()),
        None => Decimal::one(),
    };

    //Get total_deposit_tokens & prices
    let (
        _,
        cdt_price,
        _,
        _,
        _,
        ceiling_position,
        floor_position
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

    //CDT balance
    let balance_of_ceiling_tokens = deps.querier.query_balance(env.contract.address.clone(), config.clone().range_tokens.ceiling_deposit_token)?.amount;
    //USDC balance
    let balance_of_floor_tokens = deps.querier.query_balance(env.contract.address.clone(), config.clone().range_tokens.floor_deposit_token)?.amount;
    //Set token totals
    let mut total_ceiling_tokens = balance_of_ceiling_tokens;
    let mut total_floor_tokens = balance_of_floor_tokens;
    //Add CEILING spread rewards to the totals
    ceiling_position.claimable_spread_rewards.into_iter().for_each(|coin| {
        //If this runs at all it means the claims aren't an empty list
        position_ids.push(config.range_position_ids.ceiling);
        //Add to respective totals
        if coin.denom == config.range_tokens.ceiling_deposit_token {
            total_ceiling_tokens += Uint128::from_str(&coin.amount).unwrap();
        } else if coin.denom == config.range_tokens.floor_deposit_token {
            total_floor_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
    });
    //Add FLOOR spread rewards to the totals
    floor_position.claimable_spread_rewards.into_iter().for_each(|coin| {
        //If this runs at all it means the claims aren't an empty list
        position_ids.push(config.range_position_ids.floor);
        //Add to respective totals
        if coin.denom == config.range_tokens.ceiling_deposit_token {
            total_ceiling_tokens += Uint128::from_str(&coin.amount).unwrap();
        } else if coin.denom == config.range_tokens.floor_deposit_token {
            total_floor_tokens += Uint128::from_str(&coin.amount).unwrap();
        }
    });

    //Create claim spread fees msg
    if !position_ids.is_empty() {
        let claim_spread_fees_msg: CosmosMsg = CL::MsgCollectSpreadRewards {
            position_ids,
            sender: env.contract.address.to_string(),
        }.into();
        //Add to msgs
        msgs.push(SubMsg::new(claim_spread_fees_msg));
    }

    /////Is price in the ceiling or floor?///
    //Above the ceiling, swap to floor & add to BOTH
    if cdt_price.price > Decimal::from_str("0.993").unwrap(){
        
        //Add to BOTH position
        if !total_floor_tokens.is_zero() {
            //Split the total_floor_tokens 50/50
            let half_of_floor_token_total = total_floor_tokens / Uint128::new(2);

            //Add half to FLOOR position
            let add_to_floor: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.floor,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: String::from("0"),
                //USDC
                amount1: half_of_floor_token_total.to_string(),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            //Add to msgs
            msgs.push(SubMsg::reply_on_success(add_to_floor, ADD_TO_FLOOR_REPLY_ID));
            
            //Add half to CEILING position
            let add_to_ceiling: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.ceiling,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: String::from("0"),
                //USDC
                amount1: half_of_floor_token_total.to_string(),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            //Add to msgs
            msgs.push(SubMsg::reply_on_success(add_to_ceiling, ADD_TO_CEILING_REPLY_ID));
        }

        //Set swappable amount based on the rebalance_sale_max
        let swappable_amount = decimal_multiplication(
            rebalance_sale_max,
            Decimal::from_ratio(total_ceiling_tokens, Uint128::one())
        )?.to_uint_floor();

        //Swap ceiling (CDT) to floor (USDC)
        if !swappable_amount.is_zero() {
            let swap_to_floor: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                    token_out: config.range_tokens.clone().floor_deposit_token,
                    max_slippage: Decimal::percent(1), //we'd take whatever if this was only swapping yields but deposits get swapped as well
                })?,
                funds: vec![
                    Coin {
                        denom: config.range_tokens.ceiling_deposit_token.clone(),
                        amount: swappable_amount,
                    },
                ],
            });
            //Add to msgs as SubMsg
            msgs.push(SubMsg::reply_on_success(swap_to_floor, SWAP_TO_FLOOR_ADD_BOTH_REPLY_ID));
            //& deposit into BOTH in a submsg post swap
        }
    }
    //In the ceiling, so add to FLOOR
    else if cdt_price.price >= Decimal::percent(99) && cdt_price.price <= Decimal::from_str("0.993").unwrap(){
        //Add to FLOOR position
        if !total_floor_tokens.is_zero() {
            let add_to_floor: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.floor,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: String::from("0"),
                //USDC
                amount1: total_floor_tokens.to_string(),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            //Add to msgs
            msgs.push(SubMsg::reply_on_success(add_to_floor, ADD_TO_FLOOR_REPLY_ID));
        }

        //Set swappable amount based on the rebalance_sale_max
        let swappable_amount = decimal_multiplication(
            rebalance_sale_max,
            Decimal::from_ratio(total_ceiling_tokens, Uint128::one())
        )?.to_uint_floor();

        //Swap ceiling (CDT) to floor (USDC)
        if !swappable_amount.is_zero() {
            let swap_to_floor: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                    token_out: config.range_tokens.clone().floor_deposit_token,
                    max_slippage: Decimal::percent(1), //we'd take whatever if this was only swapping yields but deposits get swapped as well
                })?,
                funds: vec![
                    Coin {
                        denom: config.range_tokens.ceiling_deposit_token.clone(),
                        amount: swappable_amount,
                    },
                ],
            });
            //Add to msgs as SubMsg
            msgs.push(SubMsg::reply_on_success(swap_to_floor, SWAP_ADD_TO_FLOOR_REPLY_ID));
            //& deposit into floor in a submsg post swap
        }

    } 
    // if price is outside of the ceiling & not below the Floor, deposit all CDT to the ceiling
    else if cdt_price.price < Decimal::percent(99) && cdt_price.price > Decimal::from_str("0.982").unwrap() {
        //Add to CEILING position
        if !total_ceiling_tokens.is_zero() {
        
            let add_to_ceiling: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.ceiling,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: total_ceiling_tokens.to_string(),
                //USDC
                amount1: String::from("0"),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            //Add to msgs
            msgs.push(SubMsg::reply_on_success(add_to_ceiling, ADD_TO_CEILING_REPLY_ID));
        }

        //Set swappable amount based on the rebalance_sale_max
        let swappable_amount = decimal_multiplication(
            rebalance_sale_max,
            Decimal::from_ratio(total_floor_tokens, Uint128::one())
        )?.to_uint_floor();

        //Swap floor (USDC) to ceiling (CDT)
        if !swappable_amount.is_zero() {
                
            let swap_to_ceiling: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                    token_out: config.range_tokens.clone().ceiling_deposit_token,
                    max_slippage: Decimal::percent(1), //we'd take whatever if this was only swapping yields but deposits get swapped as well
                })?,
                funds: vec![
                    Coin {
                        denom: config.range_tokens.floor_deposit_token.clone(),
                        amount: swappable_amount,
                    },
                ],
            });
            //Add to msgs as SubMsg
            msgs.push(SubMsg::reply_on_success(swap_to_ceiling, SWAP_ADD_TO_CEILING_REPLY_ID));
            //& deposit into ceiling in a submsg post swap
        }

    }     
    //Below the floor, add ceiling to BOTH
    else {        
        
        //Add to BOTH position
        if !total_ceiling_tokens.is_zero() {
            //Split the total_floor_tokens 50/50
            let half_of_ceiling_token_total = total_ceiling_tokens / Uint128::new(2);

            //Add half to FLOOR position
            let add_to_floor: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.floor,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: half_of_ceiling_token_total.to_string(),
                //USDC
                amount1: String::from("0"),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            //Add to msgs
            msgs.push(SubMsg::reply_on_success(add_to_floor, ADD_TO_FLOOR_REPLY_ID));
            
            //Add half to CEILING position
            let add_to_ceiling: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.ceiling,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: half_of_ceiling_token_total.to_string(),
                //USDC
                amount1: String::from("0"),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            //Add to msgs
            msgs.push(SubMsg::reply_on_success(add_to_ceiling, ADD_TO_CEILING_REPLY_ID));
        }

        //Set swappable amount based on the rebalance_sale_max
        let swappable_amount = decimal_multiplication(
            rebalance_sale_max,
            Decimal::from_ratio(total_floor_tokens, Uint128::one())
        )?.to_uint_floor();

        //Swap floor (USDC) to ceiling (CDT)
        if !swappable_amount.is_zero() {
                
            let swap_to_ceiling: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                    token_out: config.range_tokens.clone().ceiling_deposit_token,
                    max_slippage: Decimal::percent(1), //we'd take whatever if this was only swapping yields but deposits get swapped as well
                })?,
                funds: vec![
                    Coin {
                        denom: config.range_tokens.floor_deposit_token.clone(),
                        amount: swappable_amount,
                    },
                ],
            });
            //Add to msgs as SubMsg
            msgs.push(SubMsg::reply_on_success(swap_to_ceiling, SWAP_TO_CEILING_ADD_BOTH_REPLY_ID));
            //& deposit into BOTH in a submsg post swap
        }
    }

    if msgs.is_empty() {
        return Err(TokenFactoryError::CustomError { val: String::from("Nothing to compound") })
    }

    Ok(Response::new()
    .add_attribute("method", "manage_vault")
        .add_attribute("total_ceiling_tokens", total_ceiling_tokens)
        .add_attribute("total_floor_tokens", total_floor_tokens)
        .add_submessages(msgs)
    )
}

/// Set intents for a user. They must send vault tokens or have a non-zero balance in state.
/// NOTE: Since the CDP will repay with these assets for you during liquidation, we don't have autoRepay intents.
/// NOTE: You either purchase or compound an asset, can't split it.
fn set_intents(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mut intents: RangeBoundUserIntents,
    reduce_vault_tokens: Option<Uint128>, 
) -> Result<Response, TokenFactoryError> {
    //Load config
    let config = CONFIG.load(deps.storage)?;
    let mut msgs = vec![];
    //Load user intents state
    let mut user_intent_state = match USER_INTENT_STATE.load(deps.storage, info.clone().sender.to_string()){
        Ok(mut state) => {
            //If the user has intents set, update them
            state.intents = intents;

            //Make sure the yield distribution isn't over 100%
            if state.intents.purchase_intents.clone().into_iter().map(|intent| intent.yield_percent).sum::<Decimal>() > Decimal::one() {
                return Err(TokenFactoryError::CustomError { val: String::from("Yield distribution cannot exceed 100%") });
            }
            //Make sure intents aren't empty
            if state.intents.purchase_intents.len() == 0 {
                return Err(TokenFactoryError::CustomError { val: String::from("Intents cannot be empty") });
            }

            //Return updated
            state
        },
        Err(_) => {
            
            let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;

            //Get the total deposit tokens
            let (
                total_deposit_tokens,
                _, 
                _, _, _, _, _
            ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
            //Calc the rate of vault tokens to deposit tokens
            let btokens_per_one = calculate_base_tokens(
                Uint128::new(1_000_000_000_000), 
                total_deposit_tokens, 
                total_vault_tokens
            )?;
            //Set conversion rate
            intents.last_conversion_rate = btokens_per_one;
            
            //Make sure the yield distribution isn't over 100%
            if intents.purchase_intents.clone().into_iter().map(|intent| intent.yield_percent).sum::<Decimal>() > Decimal::one() {
                return Err(TokenFactoryError::CustomError { val: String::from("Yield distribution cannot exceed 100%") });
            }
            //Make sure intents aren't empty
            if intents.purchase_intents.len() == 0 {
                return Err(TokenFactoryError::CustomError { val: String::from("Intents cannot be empty") });
            }
            //Return state
            UserIntentState {
                vault_tokens: Uint128::zero(),
                intents,
                //Unused until withdrawal period is added
                unstake_time: 0u64,
                fee_to_caller: Decimal::percent(1)
            }
        },
    };
    //If user vault tokens are zero, they must send vault tokens
    if user_intent_state.vault_tokens.is_zero() {
        if info.funds.len() != 1 {
            return Err(TokenFactoryError::CustomError { val: String::from("User must send vault tokens to set intents") });
        }
        if info.funds[0].denom != config.vault_token {
            return Err(TokenFactoryError::CustomError { val: String::from("User must send vault tokens to set intents") });
        }
        user_intent_state.vault_tokens = info.funds[0].amount;
    } else {
        //Add the vault tokens sent to the user's vault tokens
        if info.funds.len() == 1 && info.funds[0].denom == config.vault_token {
            user_intent_state.vault_tokens += info.funds[0].amount;
        }

        //Reduce the user's vault tokens if requested
        if let Some(reduce_amount) = reduce_vault_tokens {
            //Set reduce amount max
            let reduce_amount = min(reduce_amount, user_intent_state.vault_tokens);
            //Subtract the reduce amount from the user's vault tokens
            user_intent_state.vault_tokens -= reduce_amount;

            //Send the reduced vault tokens to the user as a BankMsg
            let send_vault_tokens_msg: CosmosMsg = BankMsg::Send {
                to_address: info.sender.clone().to_string(),
                amount: vec![Coin {
                    denom: config.vault_token.clone(),
                    amount: reduce_amount,
                }],
            }.into();
            //Add to msgs
            msgs.push(send_vault_tokens_msg);

        }
    }


    //Save user intents state
    USER_INTENT_STATE.save(deps.storage, info.sender.clone().to_string(), &user_intent_state)?;

    //If the user removed all VTs, delete their state
    if user_intent_state.vault_tokens.is_zero() {
        USER_INTENT_STATE.remove(deps.storage, info.sender.to_string());
    }

    //Create response
    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("method", "set_intents")
        .add_attribute("user", info.sender)
        .add_attribute("vault_tokens", user_intent_state.vault_tokens)
        .add_attribute("intents", format!("{:?}", user_intent_state.intents))
    )
}

/// Fulfill intents for user(s). Send fees to the caller. Update last conversion rate.
fn fulfill_intents(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    users: Vec<String>,
) -> Result<Response, TokenFactoryError> {
    //Load config
    let config = CONFIG.load(deps.storage)?;
    //init submsgs
    let mut submsgs: Vec<SubMsg> = vec![];
    
    //Get total_deposit_tokens
    let (
        total_deposit_tokens,
        _, 
        _, _, _, _, _
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

    //Load vault token total from state
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    
    ///Check current conversion rate///
    /// If the conversion rate is greater than the user's last, exit the increased % and fulfill the compound/purchase intents///    
    //Calc the rate of vault tokens to deposit tokens
    let current_conversion_rate = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    //Iterate over users
    for user in users.clone() {

        //Validate user
        let mut user_intent_state = USER_INTENT_STATE.load(deps.storage, user.clone())?;
        //Shouldn't be possible
        if user_intent_state.intents.purchase_intents.len() == 0 { 
            continue;
        }

        //If the current rate is greater than the user's last rate, exit the increased % and fulfill the compound/purchase intents
        match decimal_division(
            Decimal::from_ratio(current_conversion_rate, Uint128::one()),
            Decimal::from_ratio(user_intent_state.intents.last_conversion_rate, Uint128::one())
        ) {
            Ok(rate) => {
                if rate > Decimal::one() {
                    let profit = rate - Decimal::one();
                    let total_vault_tokens_to_exit= user_intent_state.vault_tokens * profit;
                    //Subtract the exit amount from the user's vault tokens
                    user_intent_state.vault_tokens -= total_vault_tokens_to_exit;
                    //Calc the amount that goes to the caller as a fee
                    let vault_tokens_to_caller = total_vault_tokens_to_exit * user_intent_state.fee_to_caller;
                    //Calc the amount that goes to the user
                    let vault_tokens_to_exit_for_user = total_vault_tokens_to_exit - vault_tokens_to_caller;
                           
                    //Create exit vault msg for the fee amount
                    let exit_vault_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: env.contract.address.to_string(),
                        msg: to_json_binary(&ExecuteMsg::ExitVault { send_to: Some(info.sender.clone().to_string()) })?,
                        funds: vec![
                            Coin {
                                denom: config.vault_token.clone(),
                                amount: vault_tokens_to_caller.clone(),
                            },
                        ],
                    });

                    //Exit and send the fee to the caller
                    let caller_fee_exit_submsg = SubMsg::new(exit_vault_msg);
                    //Add to submsgs
                    submsgs.push(caller_fee_exit_submsg);

                    //Exit the vault with the user's profit
                    let exit_vault_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: env.contract.address.to_string(),
                        msg: to_json_binary(&ExecuteMsg::ExitVault { send_to: None })?,
                        funds: vec![
                            Coin {
                                denom: config.vault_token.clone(),
                                amount: vault_tokens_to_exit_for_user.clone(),
                            },
                        ],
                    });

                    //Swap CDT to fulfill the intents post-exit//
                    let fulfill_submsg = SubMsg::reply_on_success(exit_vault_msg, PURCHASE_POST_EXIT_REPLY_ID);
                    //Add to submsgs
                    submsgs.push(fulfill_submsg);
                
                    //Save intent data for the reply
                    INTENT_PROPAGATION.save(deps.storage, &IntentProp {
                        intents: user_intent_state.clone().intents,
                        prev_cdt_balance: deps.querier.query_balance(env.contract.address.to_string(), config.range_tokens.ceiling_deposit_token.clone())?.amount,
                        prev_usdc_balance: deps.querier.query_balance(env.contract.address.to_string(), config.range_tokens.floor_deposit_token.clone())?.amount,
                    })?;
                    
                    //Set the user's last conversion rate
                    user_intent_state.intents.last_conversion_rate = current_conversion_rate;
                    
                    //Update the user's state
                    USER_INTENT_STATE.save(deps.storage, user.clone(), &user_intent_state)?;
                } else {
                    //Nothing to profit take, error
                    return Err(TokenFactoryError::CustomError { val: format!("Current conversion rate: {} isn't greater than the user's last rate: {}", current_conversion_rate, user_intent_state.intents.last_conversion_rate) });
                }
            },
            Err(err) => {
                //Error calculating rate
                return Err(TokenFactoryError::CustomError { val: format!("Error calculating profit percentage: {}", err) });
            }
        };
    }

    //Create response
    Ok(Response::new()
        .add_attribute("method", "fulfill_intents")
        .add_attribute("users", format!("{:?}", users))
        .add_submessages(submsgs)
    )
}

/// Repay for a user in the Positions contract
fn repay_user_debt(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    user_info: UserInfo,
    repayment: Uint128, //This is the amount of the deposit asset to repay
) -> Result<Response, TokenFactoryError> {
    //Assert sender is the Positions contract
    if info.sender.to_string() != String::from("osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr") {
        return Err(TokenFactoryError::Unauthorized {});
    }

    //Load config
    let config = CONFIG.load(deps.storage)?;
    //Load user's intent state
    let mut user_intent_state = USER_INTENT_STATE.load(deps.storage, user_info.position_owner.clone())?;

    let mut submsgs = vec![];
    let attrs = vec![
        attr("method", "repay"),
        attr("user_info", user_info.to_string()),
    ];

    //Load total vault token supply
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    
    //Get total deposit tokens
    let (
        total_deposit_tokens,
        _, 
        _,
        _, 
        _,
        _, 
        _
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

    //Get the expected_exit_amount of the user's vault token total
    let users_total_base_tokens = calculate_base_tokens(
        user_intent_state.vault_tokens, 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    
    //Calculate how much to repay
    let repayment = min(repayment, users_total_base_tokens);

    //Calculate the amount of vault tokens to exit
    let vault_tokens_to_exit = calculate_vault_tokens(
        repayment, 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    //Update user intent state
    user_intent_state.vault_tokens -= vault_tokens_to_exit;

    //Save user intent state
    USER_INTENT_STATE.save(deps.storage, user_info.position_owner.clone(), &user_intent_state)?;

    //Create exit vault msg    
    let exit_vault_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_json_binary(&ExecuteMsg::ExitVault { send_to: None })?,
        funds: vec![
            Coin {
                denom: config.vault_token.clone(),
                amount: vault_tokens_to_exit.clone(),
            },
        ],
    });
    //Add to submsgs
    submsgs.push(SubMsg::reply_on_success(exit_vault_msg, SWAP_AFTER_EXIT_FOR_CDP_REPAY_REPLY_ID));
    
    //Create response
    Ok(Response::new()
        .add_attributes(attrs)
        .add_attribute("vault_tokens_to_exit", vault_tokens_to_exit)
        .add_submessages(submsgs)
    )
}

/// Update contract configuration
/// This function is only callable by the owner
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    osmosis_proxy_contract: Option<String>,
    oracle_contract_addr: Option<String>,
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
    if let Some(addr) = osmosis_proxy_contract {
        let valid_addr = deps.api.addr_validate(&addr)?;
        config.osmosis_proxy_contract_addr = valid_addr.to_string();
        attrs.push(attr("updated_osmosis_proxy_contract_addr", valid_addr));  
    }
    if let Some(addr) = oracle_contract_addr {
        let valid_addr = deps.api.addr_validate(&addr)?;
        config.oracle_contract_addr = valid_addr.to_string();
        attrs.push(attr("updated_oracle_contract_addr", valid_addr));  
    }

    CONFIG.save(deps.storage, &config)?;
    attrs.push(attr("updated_config", format!("{:?}", config)));

    Ok(Response::new().add_attributes(attrs))
}

fn crank_realized_apr(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    //Load state
    let config = CONFIG.load(deps.storage)?; 
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;

    //Update Claim tracker
    let mut claim_tracker = CLAIM_TRACKER.load(deps.storage)?;
    //Calculate time since last claim
    let time_since_last_checkpoint = env.block.time.seconds() - claim_tracker.last_updated;
    //Get the total deposit tokens
    let (
        total_deposit_tokens,
        _, 
        _, _, _, _, _
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
    //Calc the rate of vault tokens to deposit tokens
    let btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
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
            attr("added_time_to_checkpoint", time_since_last_checkpoint.to_string())
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
        QueryMsg::DepositTokenConversion { deposit_token_value } => to_json_binary(&query_deposit_token_conversion(deps, env, deposit_token_value)?),
        QueryMsg::ClaimTracker {} => to_json_binary(&CLAIM_TRACKER.load(deps.storage)?),
        QueryMsg::TotalTVL {  } => to_json_binary(&query_vault_token_underlying(deps, env, VAULT_TOKEN.load(deps.storage)?)?),
        QueryMsg::GetUserIntent { start_after, limit, users } => to_json_binary(&query_user_intent_state(deps, env, start_after, limit, users)?),
    }
}
fn query_user_intent_state(
    deps: Deps,
    _env: Env,
    start_after: Option<String>,
    limit: Option<u32>,
    // Users
    user: Vec<String>,
)-> StdResult<Vec<UserIntentResponse>> {
    let limit = limit.unwrap_or(MAX_LIMIT) as usize;

    let start = if let Some(start) = start_after {
        Some(Bound::exclusive(start))
    } else {
        None
    };

    if user.len() > 0 {
        return user
            .into_iter()
            .map(|user| {
                let intent = USER_INTENT_STATE.load(deps.storage, user.clone())?;
                Ok(UserIntentResponse {
                    user,
                    intent,
                })
            })
            .collect();
    } else {
        return USER_INTENT_STATE
            .range(deps.storage, start, None, Order::Ascending)
            .take(limit)
            .map(|item| {
                let (k, v) = item?;
                Ok(UserIntentResponse {
                    user: k,
                    intent: v,
                })
            }).collect();
    }
}
/// Return vault token amount conversion amount for a value of deposit tokens
fn query_deposit_token_conversion(
    deps: Deps,
    env: Env,
    deposit_token_value: Decimal,
) -> StdResult<Uint128> {
    let config = CONFIG.load(deps.storage)?;
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    
    //Get total deposit tokens
    let (
        total_deposit_tokens,
        ceiling_price, 
        _,
        _, 
        _,
        _, 
        _
    ) = get_total_deposit_tokens(deps, env.clone(), config.clone())?;

    //Calc the amount of deposit tokens the user owns
    let potential_vault_tokens = calculate_vault_tokens(
        ceiling_price.get_amount(deposit_token_value)?, 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    //Return
    Ok(potential_vault_tokens)
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

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.id {
        SWAP_ADD_TO_FLOOR_REPLY_ID => handle_swap_add_to_floor(deps, env, msg),
        SWAP_ADD_TO_CEILING_REPLY_ID => handle_swap_to_ceiling(deps, env, msg),
        ADD_TO_FLOOR_REPLY_ID => handle_add_to_floor_position(deps, env, msg),
        ADD_TO_CEILING_REPLY_ID => handle_add_to_ceiling_position(deps, env, msg),
        SWAP_TO_FLOOR_ADD_BOTH_REPLY_ID => handle_swap_to_floor_add_to_both_reply(deps, env, msg),
        SWAP_TO_CEILING_ADD_BOTH_REPLY_ID => handle_swap_to_ceiling_add_to_both_reply(deps, env, msg),
        CL_POSITION_CREATION_REPLY_ID => handle_cl_position_creation_reply(deps, env, msg),
        SWAP_AFTER_EXIT_FOR_CDP_REPAY_REPLY_ID => handle_repay_and_swap_after_exit_reply(deps, env, msg),
        SWAP_FOR_CDP_REPAY_REPLY_ID => handle_repay_after_swap_reply(deps, env, msg),
        PURCHASE_POST_EXIT_REPLY_ID => handle_purchase_post_exit_reply(deps, env, msg),
        PARSE_PURCHASE_INTENTS_REPLY_ID => handle_parse_purchase_intents_reply(deps, env, msg),
        MANAGE_ERROR_DENIAL_REPLY_ID => Ok(Response::new().add_attribute("method", "handle_error_denial")),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
}

//Purchase post exit for the purchase intents
fn handle_purchase_post_exit_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            let mut submsgs = vec![];
            //Load state
            let config = CONFIG.load(deps.storage)?;

            //Load Intents propagation
            let intent_prop = INTENT_PROPAGATION.load(deps.storage)?;

            //Calc newly added balances
            let current_cdt_balance = deps.querier.query_balance(env.contract.address.to_string(), config.range_tokens.ceiling_deposit_token.clone())?.amount;
            let current_usdc_balance = deps.querier.query_balance(env.contract.address.to_string(), config.range_tokens.floor_deposit_token.clone())?.amount;
            let new_cdt_balance = current_cdt_balance - intent_prop.prev_cdt_balance;
            let new_usdc_balance = current_usdc_balance - intent_prop.prev_usdc_balance;

            //Update current balances
            INTENT_PROPAGATION.save(deps.storage, &IntentProp {
                intents: intent_prop.intents.clone(),
                prev_cdt_balance: current_cdt_balance,
                prev_usdc_balance: current_usdc_balance,
            })?;
            //Init available balance trackers
            let mut available_cdt_balance = new_cdt_balance;
            let mut available_usdc_balance = new_usdc_balance;
            //Are we buying an asset and sending it to the user?              
            for purchase_intent in intent_prop.intents.purchase_intents.clone() {
                //Calc the amount of USDC to sell
                let mut usdc_to_sell = new_usdc_balance * purchase_intent.yield_percent;
                let mut cdt_to_sell = new_cdt_balance * purchase_intent.yield_percent;
                //Update available balances
                //these swap & zero out instead of error if there is overflow///
                available_cdt_balance = match available_cdt_balance.checked_sub(cdt_to_sell){
                    Ok(diff) => diff,
                    Err(_) => {
                        cdt_to_sell = available_cdt_balance;
                        Uint128::zero()
                    }
                };
                available_usdc_balance = match available_usdc_balance.checked_sub(usdc_to_sell){
                    Ok(diff) => diff,
                    Err(_) => {
                        usdc_to_sell = available_usdc_balance;
                        Uint128::zero()
                    }
                };

                //Swap USDC to the desired asset & compound if necessary
                if !usdc_to_sell.is_zero() {
                    /////Swap USDC to the desired asset///
                    if let Some(routes) = purchase_intent.route.clone() {
                        //Set the swap route
                        let routes = routes.usdc_route;
                        //Swap USDC using given route
                        let swap_to_desired: CosmosMsg = MsgSwapExactAmountIn {
                            sender: env.contract.address.to_string(),
                            routes,
                            token_in: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
                                amount: usdc_to_sell.to_string(),
                                denom: config.range_tokens.floor_deposit_token.clone(),
                            }),
                            token_out_min_amount: "1".to_string(), //We don't limit the min amount for misc. assets (i.e. max slippage at 100%)
                            
                        }.into();
                        //Add to msgs as SubMsg
                        submsgs.push(SubMsg::new(swap_to_desired));
                        //Post swap we parse thru intents again in a reply & send or compound the desired asset


                    } else {
                        //Swap using default route
                        let swap_to_desired: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                            contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                            msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                                token_out: purchase_intent.desired_asset.clone(),
                                max_slippage: purchase_intent.slippage.unwrap_or_else(|| Decimal::percent(1)),
                            })?,
                            funds: vec![
                                Coin {
                                    denom: config.range_tokens.floor_deposit_token.clone(),
                                    amount: usdc_to_sell,
                                },
                            ],
                        });
                        //Add to msgs as SubMsg
                        submsgs.push(SubMsg::new(swap_to_desired));
                        //Post swap we parse thru intents again in a reply & send or compound the desired asset
                    }
                }
                
                //Swap CDT to the desired asset
                if !cdt_to_sell.is_zero() {  
                    /////Swap CDT to the desired asset///
                    if let Some(routes) = purchase_intent.route {
                        //Set the swap route
                        let routes = routes.cdt_route;
                        
                        //Swap CDT using given route
                        let swap_to_desired: CosmosMsg = MsgSwapExactAmountIn {
                            sender: env.contract.address.to_string(),
                            routes,
                            token_in: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
                                amount: cdt_to_sell.to_string(),
                                denom: config.range_tokens.ceiling_deposit_token.clone(),
                            }),
                            token_out_min_amount: "1".to_string(), //We don't limit the min amount for misc. assets (i.e. max slippage at 100%)
                            
                        }.into();
                        //Add to msgs as SubMsg
                        submsgs.push(SubMsg::new(swap_to_desired));
                        //Post swap we parse thru intents again in a reply & send or compound the desired asset


                    } else {
                        //Swap using default route
                        let swap_to_desired: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                            contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                            msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                                token_out: purchase_intent.desired_asset.clone(),
                                max_slippage: purchase_intent.slippage.unwrap_or_else(|| Decimal::percent(1)),
                            })?,
                            funds: vec![
                                Coin {
                                    denom: config.range_tokens.ceiling_deposit_token.clone(),
                                    amount: cdt_to_sell,
                                },
                            ],
                        });
                        //Add to msgs as SubMsg
                        submsgs.push(SubMsg::new(swap_to_desired));
                        //Post swap we parse thru intents again in a reply & send or compound the desired asset
                    }
                } 
            }
            
            
            //Pop last submsg and add PARSE_PURCHASE_INTENTS_REPLY_ID on success
            if submsgs.len() > 0 {
                //Remove last msg from msgs
                let last_submsg = match submsgs.pop(){
                    Some(msg) => msg,
                    None => return Err(StdError::GenericErr { msg: String::from("No messages to swap") })
                };
        
                //Set the last msg of the list to be a submessage with a parse purchase intents reply
                submsgs.push(SubMsg::reply_on_success(last_submsg.msg, PARSE_PURCHASE_INTENTS_REPLY_ID));
            }

            //Create response
            let res = Response::new()
                .add_submessages(submsgs)
                .add_attribute("method", "handle_purchase_post_exit_reply")
                .add_attribute("purchase_intents", format!("{:?}", intent_prop.intents.purchase_intents))
                .add_attribute("new_cdt_balance", new_cdt_balance.to_string())
                .add_attribute("new_usdc_balance", new_usdc_balance.to_string());

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

//Send or compound the desired asset post swap in line with the user's intent
fn handle_parse_purchase_intents_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            let mut submsgs = vec![];
            let mut attrs = vec![];
            //Load state
            // let config = CONFIG.load(deps.storage)?;

            //Load Intents propagation
            let intent_prop = INTENT_PROPAGATION.load(deps.storage)?;
                        
            //Parse thru intents & send or compound the desired asset
            for intent in intent_prop.intents.purchase_intents.clone() {
                //Get the balance of the desired asset
                let desired_asset_balance = deps.querier.query_balance(env.contract.address.to_string(), intent.desired_asset.clone())?.amount;

                //If the intent has a position_id compound
                if let Some(position_id) = intent.position_id {
                    //Add asset & amount to attrs for response
                    attrs.push(attr("compounded_asset", Asset {
                        info: AssetInfo::NativeToken { denom: intent.desired_asset.clone() },
                        amount: desired_asset_balance.clone(),
                    }.to_string()));
                    //Create position deposit message
                    let position_deposit_msg: CosmosMsg = CosmosMsg::Wasm(
                        WasmMsg::Execute {
                            contract_addr: "osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr".to_string(),
                            msg: to_json_binary(&CDP_ExecuteMsg::Deposit {
                                position_id: Some(Uint128::new(position_id as u128)),
                                position_owner: Some(intent_prop.intents.user.clone()),
                            })?,
                            funds: vec![
                                Coin {
                                    denom: intent.desired_asset.clone(),
                                    amount: desired_asset_balance,
                                }
                            ],
                        }
                    );
                    //Add to msgs
                    submsgs.push(SubMsg::new(position_deposit_msg));
                } 
                //Otherwise send the desired asset to the user
                else {
                    //Add asset & amount to attrs for response
                    attrs.push(attr("purchased_and_sent_asset", Asset {
                        info: AssetInfo::NativeToken { denom: intent.desired_asset.clone() },
                        amount: desired_asset_balance.clone(),
                    }.to_string()));
                    //Send the desired asset to the user
                    let send_to_user: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
                        to_address: intent_prop.intents.user.clone(),
                        amount: vec![Coin {
                            denom: intent.desired_asset.clone(),
                            amount: desired_asset_balance,
                        }],
                    });
                    //Add to msgs
                    submsgs.push(SubMsg::new(send_to_user));
                }
            }
            
            //Create response
            let res = Response::new()
                .add_submessages(submsgs)
                .add_attribute("method", "handle_parse_purchase_intents_reply")
                .add_attribute("purchase_intents", format!("{:?}", intent_prop.intents.purchase_intents))
                .add_attributes(attrs);

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

/// Swap any exited USDC to CDT & repay any new CDT to the intended position.
/// This happens for the repay() fn which is called by the Positions contract during liquidations.
fn handle_repay_and_swap_after_exit_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            let mut submsgs = vec![];
            //Load state
            let config = CONFIG.load(deps.storage)?;

            //Load Repay propagation
            let repay_prop = CDP_REPAY_PROPAGATION.load(deps.storage)?;

            //Calc newly added balances
            let current_cdt_balance = deps.querier.query_balance(env.contract.address.to_string(), config.range_tokens.ceiling_deposit_token.clone())?.amount;
            let current_usdc_balance = deps.querier.query_balance(env.contract.address.to_string(), config.range_tokens.floor_deposit_token.clone())?.amount;
            let new_cdt_balance = current_cdt_balance - repay_prop.prev_cdt_balance;
            let new_usdc_balance = current_usdc_balance - repay_prop.prev_usdc_balance;

            //Update current balances
            CDP_REPAY_PROPAGATION.save(deps.storage, &RepayProp {
                user_info: repay_prop.user_info.clone(),
                prev_cdt_balance: current_cdt_balance,
                prev_usdc_balance: current_usdc_balance,
            })?;

            //If new USDC balance is not 0, swap to CDT
            if !new_usdc_balance.is_zero() {
                //Swap USDC to CDT
                let swap_to_ceiling: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                    msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps {
                        token_out: config.range_tokens.clone().ceiling_deposit_token,
                        max_slippage: Decimal::percent(90), //We can't have this error out for liquidations
                    })?,
                    funds: vec![
                        Coin {
                            denom: config.range_tokens.floor_deposit_token.clone(),
                            amount: new_usdc_balance,
                        },
                    ],
                });
                //Add to msgs as SubMsg
                let swap_to_ceiling_submsg = SubMsg::reply_on_success(swap_to_ceiling, SWAP_FOR_CDP_REPAY_REPLY_ID);
                submsgs.push(swap_to_ceiling_submsg);
            }
            
            if !new_cdt_balance.is_zero() {                
                //Create position repay message
                let position_repay_msg: CosmosMsg = CosmosMsg::Wasm(
                    WasmMsg::Execute {
                        contract_addr: "osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr".to_string(),
                        msg: to_json_binary(&CDP_ExecuteMsg::Repay {
                            position_id: repay_prop.user_info.position_id,
                            position_owner: Some(repay_prop.user_info.position_owner.clone()),
                            send_excess_to: Some(repay_prop.user_info.position_owner.clone()),
                        })?,
                        funds: vec![
                            Coin {
                                denom: config.range_tokens.ceiling_deposit_token.clone(),
                                amount: new_cdt_balance,
                            }
                        ],
                    }
                );
                //Add to msgs
                submsgs.push(SubMsg::new(position_repay_msg));
            }       


            //Create response
            let res = Response::new()
                .add_submessages(submsgs)
                .add_attribute("method", "handle_repay_and_swap_after_exit_reply")
                .add_attribute("position_id", repay_prop.user_info.position_id)
                .add_attribute("repay_amount", new_cdt_balance.to_string())
                .add_attribute("swap_amount", new_usdc_balance.to_string());


            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

/// Repay any exited USDC that was swapped to CDT to the intended position for CDP liquidations.
fn handle_repay_after_swap_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            let mut submsgs = vec![];
            //Load state
            let config = CONFIG.load(deps.storage)?;

            //Load Repay propagation
            let repay_prop = CDP_REPAY_PROPAGATION.load(deps.storage)?;

            //Calc newly added balances
            let current_cdt_balance = deps.querier.query_balance(env.contract.address.to_string(), config.range_tokens.ceiling_deposit_token.clone())?.amount;
            let new_cdt_balance = current_cdt_balance - repay_prop.prev_cdt_balance;
            
            if !new_cdt_balance.is_zero() {                
                //Create position repay message
                let position_repay_msg: CosmosMsg = CosmosMsg::Wasm(
                    WasmMsg::Execute {
                        contract_addr: "osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr".to_string(),
                        msg: to_json_binary(&CDP_ExecuteMsg::Repay {
                            position_id: repay_prop.user_info.position_id,
                            position_owner: Some(repay_prop.user_info.position_owner.clone()),
                            send_excess_to: Some(repay_prop.user_info.position_owner.clone()),
                            
                        })?,
                        funds: vec![
                            Coin {
                                denom: config.range_tokens.ceiling_deposit_token.clone(),
                                amount: new_cdt_balance,
                            }
                        ],
                    }
                );
                //Add to msgs
                submsgs.push(SubMsg::new(position_repay_msg));
            }       


            //Create response
            let res = Response::new()
                .add_submessages(submsgs)
                .add_attribute("method", "handle_repay_after_swap_reply")
                .add_attribute("position_id", repay_prop.user_info.position_id)
                .add_attribute("repay_amount", new_cdt_balance.to_string());

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}


/// Get the tokens swapped to the floor & add them to the floor position
fn handle_swap_add_to_floor(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            //Load state
            let config = CONFIG.load(deps.storage)?;

            //Get balance of floor tokens just swapped for
            let balance_of_floor_tokens = deps.querier.query_balance(env.contract.address.clone(), config.range_tokens.floor_deposit_token)?.amount;
            
            //Add to FLOOR position
            if balance_of_floor_tokens.is_zero() {
                return Err(StdError::GenericErr { msg: String::from("No balance of floor tokens received from the swap") });
            }
            let add_to_floor: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.floor,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: String::from("0"),
                //USDC
                amount1: balance_of_floor_tokens.to_string(),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            let add_to_floor_submsg = SubMsg::reply_on_success(add_to_floor, ADD_TO_FLOOR_REPLY_ID);

            //Create Response
            let res = Response::new()
                .add_submessage(add_to_floor_submsg)
                .add_attribute("method", "handle_swap_add_to_floor_reply")
                .add_attribute("balance_of_floor_tokens_added_to_CL_position", balance_of_floor_tokens.to_string());

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

/// Get the tokens swapped to the floor & add them to BOTH positions
fn handle_swap_to_floor_add_to_both_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            //Load state
            let config = CONFIG.load(deps.storage)?;

            //Get balance of floor tokens just swapped for
            let balance_of_floor_tokens = deps.querier.query_balance(env.contract.address.clone(), config.range_tokens.floor_deposit_token)?.amount;
            
            //Add to FLOOR position
            if balance_of_floor_tokens.is_zero() {
                return Err(StdError::GenericErr { msg: String::from("No balance of floor tokens received from the swap") });
            }
            //Split the total_floor_tokens 50/50
            let half_of_floor_token_total = balance_of_floor_tokens / Uint128::new(2);
            //Add half to FLOOR position
            let add_to_floor: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.floor,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: String::from("0"),
                //USDC
                amount1: half_of_floor_token_total.to_string(),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            let add_to_floor_submsg = SubMsg::reply_on_success(add_to_floor, ADD_TO_FLOOR_REPLY_ID);

            //Add half to CEILING position
            let add_to_ceiling: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.ceiling,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: String::from("0"),
                //USDC
                amount1: half_of_floor_token_total.to_string(),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            let add_to_ceiling_submsg = SubMsg::reply_on_success(add_to_ceiling, ADD_TO_CEILING_REPLY_ID);

            //Create Response
            let res = Response::new()
                .add_submessage(add_to_floor_submsg)
                .add_submessage(add_to_ceiling_submsg)
                .add_attribute("method", "handle_swap_to_floor_add_to_both_reply")
                .add_attribute("balance_of_floor_tokens_added_to_CL_position", balance_of_floor_tokens.to_string());

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}


/// Set the new position ID for the floor position
fn handle_add_to_floor_position(
    deps: DepsMut,
    _env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(result) => {
            //Load state
            let mut config = CONFIG.load(deps.storage)?;

            //Parse response
            if let Some(b) = result.data {
                let res: CL::MsgAddToPositionResponse = match b.try_into().map_err(TokenFactoryError::Std){
                    Ok(res) => res,
                    Err(err) => return Err(StdError::GenericErr { msg: String::from(err.to_string()) })
                };
                //Save position ID
                config.range_position_ids.floor = res.position_id;
                
            } else {
                return Err(StdError::GenericErr { msg: String::from("No data in reply") })
            }

            //Save State
            CONFIG.save(deps.storage, &config)?;

            //Create Response
            return Ok(Response::new()
                .add_attribute("method", "handle_add_to_floor_position_reply")
                .add_attribute("floor_position_id", config.range_position_ids.floor.to_string()))
            

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}


/// Get the tokens swapped to the ceiling & add them to the ceiling position
fn handle_swap_to_ceiling(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_result) => {
            //Load state
            let config = CONFIG.load(deps.storage)?;

            //Get balance of ceiling tokens just swapped for
            let balance_of_ceiling_tokens = deps.querier.query_balance(env.contract.address.clone(), config.range_tokens.ceiling_deposit_token)?.amount;
            
            if balance_of_ceiling_tokens.is_zero() {
                return Err(StdError::GenericErr { msg: String::from("No balance of ceiling tokens received from the swap") });
            }

            //Add to CEILING position
            let add_to_ceiling: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.ceiling,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: balance_of_ceiling_tokens.to_string(),
                //USDC
                amount1: String::from("0"),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            let add_to_ceiling_submsg = SubMsg::reply_on_success(add_to_ceiling, ADD_TO_CEILING_REPLY_ID);

            //Create Response
            let res = Response::new()
                .add_submessage(add_to_ceiling_submsg)
                .add_attribute("method", "handle_swap_to_ceiling_reply")
                .add_attribute("balance_of_ceiling_tokens_added_to_CL_position", balance_of_ceiling_tokens.to_string());

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

/// Get the tokens swapped to the ceiling & add them to BOTH positions
fn handle_swap_to_ceiling_add_to_both_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            //Load state
            let config = CONFIG.load(deps.storage)?;

            //Get balance of ceiling tokens just swapped for
            let balance_of_ceiling_tokens = deps.querier.query_balance(env.contract.address.clone(), config.range_tokens.ceiling_deposit_token)?.amount;
            
            //Add to FLOOR position
            if balance_of_ceiling_tokens.is_zero() {
                return Err(StdError::GenericErr { msg: String::from("No balance of ceiling_deposit_token tokens received from the swap") });
            }
            //Split the total_ceiling_tokens 50/50
            let half_of_ceiling_token_total = balance_of_ceiling_tokens / Uint128::new(2);
            //Add half to FLOOR position
            let add_to_floor: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.floor,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: half_of_ceiling_token_total.to_string(),
                //USDC
                amount1: String::from("0"),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            let add_to_floor_submsg = SubMsg::reply_on_success(add_to_floor, ADD_TO_FLOOR_REPLY_ID);

            //Add half to CEILING position
            let add_to_ceiling: CosmosMsg = CL::MsgAddToPosition {
                position_id: config.range_position_ids.ceiling,
                sender: env.contract.address.to_string(),
                //CDT
                amount0: half_of_ceiling_token_total.to_string(),
                //USDC
                amount1: String::from("0"),
                token_min_amount0: String::from("0"),
                token_min_amount1: String::from("0"),
            }.into();
            let add_to_ceiling_submsg = SubMsg::reply_on_success(add_to_ceiling, ADD_TO_CEILING_REPLY_ID);

            //Create Response
            let res = Response::new()
                .add_submessage(add_to_floor_submsg)
                .add_submessage(add_to_ceiling_submsg)
                .add_attribute("method", "handle_swap_to_ceiling_add_to_both_reply")
                .add_attribute("balance_of_ceiling_tokens_added_to_CL_position", balance_of_ceiling_tokens.to_string());

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}


/// Set the new position ID for the ceiling position
fn handle_add_to_ceiling_position(
    deps: DepsMut,
    _env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(result) => {
            //Load state
            let mut config = CONFIG.load(deps.storage)?;

            //Parse response
            if let Some(b) = result.data {
                let res: CL::MsgAddToPositionResponse = match b.try_into().map_err(TokenFactoryError::Std){
                    Ok(res) => res,
                    Err(err) => return Err(StdError::GenericErr { msg: String::from(err.to_string()) })
                };
                //Save position ID
                config.range_position_ids.ceiling = res.position_id;
                
            } else {
                return Err(StdError::GenericErr { msg: String::from("No data in reply") })
            }

            //Save State
            CONFIG.save(deps.storage, &config)?;

            //Create Response
            return Ok(Response::new()
                .add_attribute("method", "handle_add_to_ceiling_position_reply")
                .add_attribute("ceiling_position_id", config.range_position_ids.ceiling.to_string()))
            

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

/// Handle the CL POSITION CREATION response from Osmosis
fn handle_cl_position_creation_reply(
    deps: DepsMut,
    _env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(result) => {

            //Load state
            let mut config = CONFIG.load(deps.storage)?;
            //Parse response
            if let Some(b) = result.data {
                let res: CL::MsgCreatePositionResponse = match b.try_into().map_err(TokenFactoryError::Std){
                    Ok(res) => res,
                    Err(err) => return Err(StdError::GenericErr { msg: String::from(err.to_string()) })
                };
                //Save position ID
                if config.range_position_ids.ceiling == 0 {
                    config.range_position_ids.ceiling = res.position_id;
                } else {
                    config.range_position_ids.floor = res.position_id;
                }
            } else {
                return Err(StdError::GenericErr { msg: String::from("No data in reply") })
            }

            //Save State
            CONFIG.save(deps.storage, &config)?;

            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_creation_reply")
                .add_attribute("cl_position_ids", format!("{:?}", config.range_position_ids));

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, TokenFactoryError> {
    Ok(Response::default())
}