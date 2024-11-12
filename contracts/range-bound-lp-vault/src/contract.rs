use std::cmp::min;
use std::convert::TryInto;
use std::str::FromStr;

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, to_json_binary, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, QuerierWrapper, Reply, Response, StdError, StdResult, SubMsg, Uint128, WasmMsg
};
use cw2::set_contract_version;
use membrane::math::{decimal_multiplication, decimal_division};
use membrane::oracle::PriceResponse;

use crate::error::TokenFactoryError;
use crate::state::{CLAIM_TRACKER, TOKEN_RATE_ASSURANCE, TokenRateAssurance, CONFIG, OWNERSHIP_TRANSFER, VAULT_TOKEN};
use membrane::range_bound_lp_vault::{
    Config, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg
};
use membrane::stability_pool_vault::{
    calculate_base_tokens, calculate_vault_tokens
};
use membrane::osmosis_proxy::ExecuteMsg as OP_ExecuteMsg;
use membrane::oracle::QueryMsg as Oracle_QueryMsg;
use membrane::types::{AssetInfo, ClaimTracker, RangePositions, VTClaimCheckpoint};
use osmosis_std::types::osmosis::tokenfactory::v1beta1::{self as TokenFactory};
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::{self as CL, FullPositionBreakdown};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:range-bound-lp-vault";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

//Reply IDs
const SWAP_TO_FLOOR_REPLY_ID: u64 = 1u64;
const SWAP_TO_CEILING_REPLY_ID: u64 = 2u64;
const CL_POSITION_CREATION_REPLY_ID: u64 = 3u64;

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
        ExecuteMsg::EnterVault { } => enter_vault(deps, env, info),
        ExecuteMsg::ExitVault {  } => exit_vault(deps, env, info),
        ExecuteMsg::ManageVault { rebalance_sale_max } => manage_vault(deps, env, info, rebalance_sale_max),
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
fn enter_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    let config = CONFIG.load(deps.storage)?;

    //Assert the sender sent both assets in the correct order
    if info.funds.len() != 2 || info.funds[0].denom != config.range_tokens.ceiling_deposit_token || info.funds[1].denom != config.range_tokens.floor_deposit_token {
        return Err(TokenFactoryError::CustomError { val: format!("Need to send both range assets in the correct order. Ceiling token first: {}, Floor token second: {} ",  config.clone().range_tokens.ceiling_deposit_token,  config.clone().range_tokens.floor_deposit_token) });
    }
    
    //Get the amount of deposit tokens sent
    let ceiling_deposit_amount = info.funds[0].amount;
    let floor_deposit_amount = info.funds[1].amount;

    //Get total_deposit_tokens & prices
    let (
        total_deposit_tokens,
        ceiling_price, 
        floor_price,
        _,
        _,
        _,
        _,
    ) = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

    //Set the value sent
    let ceiling_value_sent = ceiling_price.get_value(ceiling_deposit_amount)?;
    let floor_value_sent = floor_price.get_value(floor_deposit_amount)?;
    let value_ratio = decimal_division(ceiling_value_sent, floor_value_sent)?;

    //Check that the value sent is within 1% of each other
    if value_ratio > Decimal::percent(101) || value_ratio < Decimal::percent(99) {
        return Err(TokenFactoryError::CustomError { val: format!("The value of the assets sent must be within 1% of each other. Ceiling value: {}, Floor value: {}", ceiling_value_sent, floor_value_sent) });
    }

    //Normalize the deposit amounts into the ceiling token (CDT)
    let normalized_deposit_amount = ceiling_price.get_amount(ceiling_value_sent + floor_value_sent)?;

    //////Calculate the amount of vault tokens to mint////
    //Get the total amount of vault tokens circulating
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    //Update the total deposit tokens after vault tokens are minted 
    //..because the total_deposit_tokens counts the deposit tokens in the contract
    let pre_deposit_total_deposit_tokens = total_deposit_tokens - normalized_deposit_amount;
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

    //Update the total vault tokens
    VAULT_TOKEN.save(deps.storage, &(total_vault_tokens + vault_tokens_to_distribute))?;

    //Save the updated config
    CONFIG.save(deps.storage, &config)?;

    //Add rate assurance callback msg
    msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_json_binary(&ExecuteMsg::RateAssurance { })?,
        funds: vec![],
    }));

    //Create Response
    let res = Response::new()
        .add_attribute("method", "enter_vault")
        .add_attribute("ceiling_value_sent", ceiling_value_sent.to_string())
        .add_attribute("floor_value_sent", floor_value_sent.to_string())
        .add_attribute("normalized_deposit_amount", normalized_deposit_amount)
        .add_attribute("vault_tokens_to_distribute", vault_tokens_to_distribute)
        .add_messages(msgs);

    Ok(res)
}

/// Exit vault in the current ratio of assets owned (LP + balances) by withdrawing pro-rata CL shares.
/// The App can swap into a single token and give value options based on swap rate.
/// 1. We burn vault tokens
/// 2. Send the withdrawn tokens to the user
fn exit_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    let config = CONFIG.load(deps.storage)?;
    let mut msgs = vec![];

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

    let ceiling_liquidity = ceiling_position.position.unwrap().liquidity;
    let floor_liquidity = floor_position.position.unwrap().liquidity;

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
    let ceiling_liquidity_to_withdraw = decimal_multiplication(
        Decimal::from_str(&ceiling_liquidity).unwrap(),
        withdrawal_ratio
    )?.to_uint_floor().to_string();
    let floor_liquidity_to_withdraw = decimal_multiplication(
        Decimal::from_str(&floor_liquidity).unwrap(), 
        withdrawal_ratio
    )?.to_uint_floor().to_string();
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
        to_address: info.sender.to_string(),
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
/// - If price is in the ceiling, swap and deposit into floor
/// - If price is in the floor, swap and deposit into ceiling
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
    //In the ceiling, so add to FLOOR
    if cdt_price.price >= Decimal::percent(99) {
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
            msgs.push(SubMsg::new(add_to_floor));
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
                    max_slippage: Decimal::one(), //bc its yield we're not going to harp on slippage
                })?,
                funds: vec![
                    Coin {
                        denom: config.range_tokens.ceiling_deposit_token.clone(),
                        amount: swappable_amount,
                    },
                ],
            });
            //Add to msgs as SubMsg
            msgs.push(SubMsg::reply_on_success(swap_to_floor, SWAP_TO_FLOOR_REPLY_ID));
            //& deposit into floor in a submsg post swap
        }

    } 
    //Price is outside of the ceiling, so we deposit all CDT there
    else {
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
            msgs.push(SubMsg::new(add_to_ceiling));
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
                    max_slippage: Decimal::one(), //bc its yield we're not going to harp on slippage
                })?,
                funds: vec![
                    Coin {
                        denom: config.range_tokens.floor_deposit_token.clone(),
                        amount: swappable_amount,
                    },
                ],
            });
            //Add to msgs as SubMsg
            msgs.push(SubMsg::reply_on_success(swap_to_ceiling, SWAP_TO_CEILING_REPLY_ID));
            //& deposit into ceiling in a submsg post swap
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
        SWAP_TO_FLOOR_REPLY_ID => handle_swap_to_floor(deps, env, msg),
        SWAP_TO_CEILING_REPLY_ID => handle_swap_to_ceiling(deps, env, msg),
        CL_POSITION_CREATION_REPLY_ID => handle_cl_position_creation_reply(deps, env, msg),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
}

/// Get the tokens swapped to the floor & add them to the floor position
fn handle_swap_to_floor(
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

            //Create Response
            let res = Response::new()
                .add_message(add_to_floor)
                .add_attribute("method", "handle_swap_to_floor_reply")
                .add_attribute("balance_of_floor_tokens_added_to_CL_position", balance_of_floor_tokens.to_string());

            return Ok(res);

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

            //Create Response
            let res = Response::new()
                .add_message(add_to_ceiling)
                .add_attribute("method", "handle_swap_to_ceiling_reply")
                .add_attribute("balance_of_ceiling_tokens_added_to_CL_position", balance_of_ceiling_tokens.to_string());

            return Ok(res);

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
pub fn migrate(deps: DepsMut, env: Env, _msg: MigrateMsg) -> Result<Response, TokenFactoryError> {

    Ok(Response::default())
}