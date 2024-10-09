#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, to_json_binary, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Reply, Response, StdError, StdResult, SubMsg, Uint128, WasmMsg
};
use cw2::set_contract_version;
use membrane::math::{decimal_multiplication, decimal_division};

use crate::error::TokenFactoryError;
use crate::state::{CLAIM_TRACKER, TOKEN_RATE_ASSURANCE, TokenRateAssurance, CONFIG, DEPOSIT_BALANCE_AT_LAST_CLAIM, OWNERSHIP_TRANSFER, VAULT_TOKEN};
use membrane::stability_pool_vault::{
    calculate_base_tokens, calculate_vault_tokens, Config, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, APR, APRResponse, ClaimTracker, VTClaimCheckpoint
};
use membrane::stability_pool::{ExecuteMsg as StabilityPoolExecuteMsg, QueryMsg as StabilityPoolQueryMsg, ClaimsResponse};
use membrane::osmosis_proxy::ExecuteMsg as OsmosisProxyExecuteMsg;
use membrane::types::AssetPool;
use osmosis_std::types::osmosis::tokenfactory::v1beta1::{self as TokenFactory};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:stability-pool-vault";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

//Reply IDs
const COMPOUND_REPLY_ID: u64 = 1u64;

//Timeframe constants
const SECONDS_PER_DAY: u64 = 86_400u64;

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
        deposit_token: msg.clone().deposit_token,
        total_deposit_tokens: Uint128::zero(),
        percent_to_keep_liquid: Decimal::percent(10),
        stability_pool_contract: deps.api.addr_validate(&msg.stability_pool_contract)?,
        osmosis_proxy_contract: deps.api.addr_validate(&msg.osmosis_proxy_contract)?,
    };
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    //Save initial state
    CONFIG.save(deps.storage, &config)?;
    CLAIM_TRACKER.save(deps.storage, &ClaimTracker {
        vt_claim_checkpoints: vec![],
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
        .add_message(denom_msg);
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
        ExecuteMsg::UpdateConfig { owner, percent_to_keep_liquid, osmosis_proxy_contract } => update_config(deps, info, owner, percent_to_keep_liquid, osmosis_proxy_contract),
        ExecuteMsg::EnterVault { } => enter_vault(deps, env, info),
        ExecuteMsg::ExitVault {  } => exit_vault(deps, env, info),
        ExecuteMsg::Compound { } => claim_and_compound_liquidations(deps, env, info),
        ExecuteMsg::CrankTotalAPR { } => crank_total_apr(deps, env, info),
        ExecuteMsg::RateAssurance { } => rate_assurance(deps, env, info),
    }
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

    //Calc the rate of deposit tokens to vault tokens
    let vtokens_per_one = calculate_vault_tokens(
        Uint128::new(1_000_000), 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;
    //Calc the rate of vault tokens to deposit tokens
    let btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000), 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;

    //For deposit or withdraw, check that the rates are static 
    if vtokens_per_one != token_rate_assurance.pre_vtokens_per_one || btokens_per_one != token_rate_assurance.pre_btokens_per_one {
        return Err(TokenFactoryError::CustomError { val: format!("Deposit or withdraw rate assurance failed. Vtokens_per_one: {:?} --- pre-tx {:?}, BTokens_per_one: {:?} --- pre-tx: {:?}", vtokens_per_one, token_rate_assurance.pre_vtokens_per_one, btokens_per_one, token_rate_assurance.pre_btokens_per_one) });
    }

    Ok(Response::new())
}


///Deposit the deposit_token to the vault & receive vault tokens in return
/// Send the deposit tokens to the yield strategy, in this case, the stability pool.
fn enter_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    let mut config = CONFIG.load(deps.storage)?;

    //Query claims from the Stability Pool.
    //Error is there are claims.
    //Catch the error if there aren't.
    //We don't let users enter the vault if the contract has claims bc the claims go to existing users.
    /////To avoid this error, compound before depositing/////
    let _claims: ClaimsResponse = match deps.querier.query_wasm_smart::<ClaimsResponse>(
        config.stability_pool_contract.to_string(),
        &StabilityPoolQueryMsg::UserClaims {
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
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    //Calc & save token rates
    let pre_vtokens_per_one = calculate_vault_tokens(
        Uint128::new(1_000_000), 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000), 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;
    TOKEN_RATE_ASSURANCE.save(deps.storage, &TokenRateAssurance {
        pre_vtokens_per_one,
        pre_btokens_per_one,
    })?;
    //Calculate the amount of vault tokens to mint
    let vault_tokens_to_distribute = calculate_vault_tokens(
        deposit_amount, 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;
    ////////////////////////////////////////////////////
    
    //Update the total deposit tokens after we mint the vault token
    config.total_deposit_tokens += deposit_amount;

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

    /////Send the deposit tokens to the yield strategy///
    let contract_balance_of_deposit_tokens = deps.querier.query_balance(env.contract.address.clone(), config.deposit_token.clone())?.amount;
    let total_balance_minus_new_deposit = contract_balance_of_deposit_tokens - deposit_amount;
    //Calculate ratio of deposit tokens in the contract to the total deposit tokens
    let ratio_of_tokens_in_contract = decimal_division(Decimal::from_ratio(total_balance_minus_new_deposit, Uint128::one()), Decimal::from_ratio(config.total_deposit_tokens, Uint128::one()))?;

    //Calculate what is sent and what is kept
    let mut deposit_sent_to_yield: Uint128 = Uint128::zero();
    let mut deposit_kept: Uint128 = Uint128::zero();
    //If the ratio is less than the percent_to_keep_liquid, calculate the amount of deposit tokens to send to the yield strategy
    if ratio_of_tokens_in_contract < config.percent_to_keep_liquid {
        //Calculate the amount of deposit tokens that would make the ratio equal to the percent_to_keep_liquid
        let desired_ratio_tokens = decimal_multiplication(Decimal::from_ratio(config.total_deposit_tokens, Uint128::one()), config.percent_to_keep_liquid)?;
        let tokens_to_fill_ratio = desired_ratio_tokens.to_uint_floor() - total_balance_minus_new_deposit;
        //How much do we send to the yield strategy
        if tokens_to_fill_ratio >= deposit_amount {
            deposit_kept = deposit_amount;
        } else {
            deposit_sent_to_yield = deposit_amount - tokens_to_fill_ratio;
            deposit_kept = tokens_to_fill_ratio;
        }
    } else
    //If the ratio to keep is past the threshold then send all the deposit tokens
    {
        deposit_sent_to_yield = deposit_amount;
    }
    println!("{}, {}, {}", ratio_of_tokens_in_contract, config.percent_to_keep_liquid, deposit_sent_to_yield);

    //Send the deposit tokens to the yield strategy
    if !deposit_sent_to_yield.is_zero() {
        let send_deposit_to_yield_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool_contract.to_string(),
            msg: to_json_binary(&StabilityPoolExecuteMsg::Deposit { user: None })?,
            funds: vec![Coin {
                denom: config.deposit_token.clone(),
                amount: deposit_sent_to_yield,
            }],
        });
        msgs.push(send_deposit_to_yield_msg);
    }

    //Add rate assurance callback msg
    msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_json_binary(&ExecuteMsg::RateAssurance { })?,
        funds: vec![],
    }));

    //Create Response
    let res = Response::new()
        .add_attribute("method", "enter_vault")
        .add_attribute("deposit_amount", deposit_amount)
        .add_attribute("vault_tokens_to_distribute", vault_tokens_to_distribute)
        .add_attribute("deposit_sent_to_yield", deposit_sent_to_yield)
        .add_attribute("deposit_kept", deposit_kept)
        .add_messages(msgs);

    Ok(res)
}

/// User sends vault_tokens to withdraw the deposit_token from the vault.
/// 1. We burn vault tokens
/// 2. send the withdrawn deposit token to the user at a max of the buffer + withdrawable SP stake.
/// 3. Unstake whatever was withdrawn to ensure the buffer amount.
///NOTE: Can't Withdraw more than the buffer unless something is currently unstakeable.
fn exit_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    let mut config = CONFIG.load(deps.storage)?;

    //Query claims from the Stability Pool.
    //Error is there are claims.
    //Catch the error if there aren't.
    //We don't let users exit the vault if they have claims bc they'd lose claimable rewards.
    let _claims: ClaimsResponse = match deps.querier.query_wasm_smart::<ClaimsResponse>(
        config.stability_pool_contract.to_string(),
        &StabilityPoolQueryMsg::UserClaims {
            user: env.contract.address.to_string(),
        },
    ){
        Ok(claims) => return Err(TokenFactoryError::ContractHasClaims { claims: claims.claims }),
        Err(_) => ClaimsResponse { claims: vec![] },
    };

    let total_deposit_tokens = deps.querier.query_balance(env.contract.address.clone(), config.deposit_token.clone())?.amount;
    if total_deposit_tokens.is_zero() {
        return Err(TokenFactoryError::ZeroDepositTokens {});
    }

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
    let pre_vtokens_per_one = calculate_vault_tokens(
        Uint128::new(1_000_000), 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000), 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;
    TOKEN_RATE_ASSURANCE.save(deps.storage, &TokenRateAssurance {
        pre_vtokens_per_one,
        pre_btokens_per_one,
    })?;
    //Calculate the amount of deposit tokens to withdraw
    let mut deposit_tokens_to_withdraw = calculate_base_tokens(
        vault_tokens, 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;
    ////////////////////////////////////////////////////
    
    //Burn vault tokens
    let burn_vault_tokens_msg: CosmosMsg = TokenFactory::MsgBurn {
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
            denom: config.vault_token.clone(),
            amount: vault_tokens.to_string(),
        }), 
        burn_from_address: info.sender.to_string(),
    }.into();

    //Update the total vault tokens
    let new_vault_token_supply = match total_vault_tokens.checked_sub(vault_tokens){
        Ok(v) => v,
        Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract vault token total supply: {} - {}", total_vault_tokens, vault_tokens) }),
    };
    //Update the total vault tokens
    VAULT_TOKEN.save(deps.storage, &new_vault_token_supply)?;
    //Update the total deposit tokens
    config.total_deposit_tokens = match config.total_deposit_tokens.checked_sub(deposit_tokens_to_withdraw){
        Ok(v) => v,
        Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract deposit token total supply: {} - {}", config.total_deposit_tokens, deposit_tokens_to_withdraw) }),
    };
    //Save the updated config
    CONFIG.save(deps.storage, &config)?;

    //Send withdrawn tokens to the user
    let send_deposit_tokens_msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: vec![Coin {
            denom: config.deposit_token.clone(),
            amount: deposit_tokens_to_withdraw,
        }],
    });


    ///Yield specific code//////
    //Query the SP asset pool
    let asset_pool: AssetPool = deps.querier.query_wasm_smart::<AssetPool> (
        config.stability_pool_contract.to_string(),
        &StabilityPoolQueryMsg::AssetPool { 
            user: Some(env.contract.address.to_string()),
            deposit_limit: None,
            start_after: None,
        },
    )?;
    //Parse deposits and calculate the amount of deposits that are withdrawable
    let withdrawable_amount = asset_pool.deposits.into_iter()
        .filter(|deposit| deposit.unstake_time.is_some() && deposit.unstake_time.unwrap() + SECONDS_PER_DAY <= env.block.time.seconds())
        .map(|deposit| deposit.amount)
        .sum::<Decimal>().to_uint_floor();

    //Add the withdrawable amount to the deposit tokens to withdraw
    //bc the SP withdraws & unstakes in the same msg 
    deposit_tokens_to_withdraw += withdrawable_amount;
    
    //Unstake the deposit tokens from the Stability Pool
    let unstake_tokens_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.stability_pool_contract.to_string(),
        msg: to_json_binary(&StabilityPoolExecuteMsg::Withdraw {
            amount: deposit_tokens_to_withdraw,
        })?,
        funds: vec![],
    });

    //Add rate assurance callback msg
    let assurance = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_json_binary(&ExecuteMsg::RateAssurance { })?,
        funds: vec![],
    });

    //Create Response 
    let res = Response::new()
        .add_attribute("method", "exit_vault")
        .add_attribute("vault_tokens", vault_tokens)
        .add_attribute("deposit_tokens_to_withdraw", deposit_tokens_to_withdraw)
        .add_message(burn_vault_tokens_msg)
        .add_message(unstake_tokens_msg)
        .add_message(send_deposit_tokens_msg)
        .add_message(assurance);

    Ok(res)
}

//Claim and compound liquidation rewards.
//This doesn't compound distributed CDT from fees.
fn claim_and_compound_liquidations(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    let mut config = CONFIG.load(deps.storage)?;

    //Query claims from the Stability Pool
    let claims: ClaimsResponse = deps.querier.query_wasm_smart::<ClaimsResponse>(
        config.stability_pool_contract.to_string(),
        &StabilityPoolQueryMsg::UserClaims {
            user: env.contract.address.to_string(),
        },
    )?;
    //If there are no claims, the query will error//    

    //Claim rewards from Stability Pool
    let claim_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.stability_pool_contract.to_string(),
        msg: to_json_binary(&StabilityPoolExecuteMsg::ClaimRewards { })?,
        funds: vec![]
    });

    //Compound rewards by sending to the Router in the Osmosis proxy contract
    //...send as a submsg that checks that the contract has more of the deposit token than it started with
    let compound_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.osmosis_proxy_contract.to_string(),
        msg: to_json_binary(&OsmosisProxyExecuteMsg::ExecuteSwaps {
            token_out: config.deposit_token.clone(),
            max_slippage: Decimal::one(),
        })?,
        funds: claims.claims,
    });
    let compound_submsg = SubMsg::reply_on_success(compound_msg, COMPOUND_REPLY_ID);

    //Save current deposit token balance
    DEPOSIT_BALANCE_AT_LAST_CLAIM.save(deps.storage, &deps.querier.query_balance(env.contract.address.clone(), config.deposit_token.clone())?.amount)?;


    //Create Response
    let res = Response::new()
        .add_attribute("method", "claim_and_compound_liquidations")
        .add_message(claim_msg)   
        .add_submessage(compound_submsg);

    Ok(res)
}

/// Update contract configuration
/// This function is only callable by an owner with non_token_contract_auth set to true
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    percent_to_keep_liquid: Option<Decimal>,
    osmosis_proxy_contract: Option<String>,
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
    if let Some(percent) = percent_to_keep_liquid {
        config.percent_to_keep_liquid = percent;
        attrs.push(attr("percent_to_keep_liquid", percent.to_string()));
    }
    if let Some(addr) = osmosis_proxy_contract {
        config.osmosis_proxy_contract = deps.api.addr_validate(&addr)?;
        attrs.push(attr("osmosis_proxy_contract", addr));
    }

    CONFIG.save(deps.storage, &config)?;
    attrs.push(attr("updated_config", format!("{:?}", config)));

    Ok(Response::new().add_attributes(attrs))
}

fn crank_total_apr(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    //Load state
    let mut config = CONFIG.load(deps.storage)?; 
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;

    //Update Claim tracker
    let mut claim_tracker = CLAIM_TRACKER.load(deps.storage)?;
    //Calculate time since last claim
    let time_since_last_checkpoint = env.block.time.seconds() - claim_tracker.last_updated;       
    
    //Calc the rate of vault tokens to deposit tokens
    let btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000), 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;

    //If the trackers total time is over a year, remove the first instance
    if claim_tracker.vt_claim_checkpoints.len() > 0 && claim_tracker.vt_claim_checkpoints.iter().map(|claim_checkpoint| claim_checkpoint.time_since_last_checkpoint).sum::<u64>() > SECONDS_PER_DAY * 365 {
        claim_tracker.vt_claim_checkpoints.remove(0);
    }
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
        attr("method", "crank_total_apr"),
        attr("new_base_token_conversion_rate", btokens_per_one),
        attr("time_since_last_checkpoint", time_since_last_checkpoint.to_string())
    ]))
}


// fn crank_cdp_apr(
//     deps: DepsMut,
//     env: Env,
//     info: MessageInfo,
//     start_after: Option<String>,
//     limit: Option<u32>,
// ) -> Result<Response, TokenFactoryError> {
//     //Load state
//     let mut config = CONFIG.load(deps.storage)?; 
//     //Initialize avg rate
//     let mut avg_rate = Decimal::zero();

//     //Query CDP Basket 
//     let cdp_basket: CDPBasket = deps.querier.query_wasm_smart::<CDPBasket>(
//         config.cdp_contract.to_string(),
//         &CDP_QueryMsg::GetBasket {},
//     )?;
//     //Set total credit
//     let total_credit_supply = basket.credit_asset.amount;

//     //Query Basket Positions
//     let basket_positions: Vec<BasketPositionsResponse> = deps.querier.query_wasm_smart::<Vec<BasketPositionsResponse>>(
//         config.cdp_contract.to_string(),
//         &CDP_QueryMsg::GetBasketPositions {
//             start_after,
//             limit: limit.unwrap_or(basket.current_position_id),
//             user_info: None,
//             user: None,
//         },
//     )?;
//     //Initialize collateral rate list
//     let mut collateral_rates: Vec<(AssetInfo, Decimal)> = vec![];
//     //Iterate thru collateral types to pair collateral rates with denoms
//     for (index, asset) in basket.collateral_types.into_iter().enumerate() {
//         collateral_rates.push((asset.asset.info, basket.lastest_collateral_rates[index]));
//     }

//     //Iterate thru basket positions to calculate the avg rate
//     for position in basket_positions.into_iter() {
//         //Get the collateral rate of the position
//         let collateral_rate = collateral_rates.iter().find(|(asset, _)| asset.denom == position.collateral_denom).unwrap().1;
//         //Calculate the rate of the position
//         let position_rate = decimal_division(Decimal::from_ratio(position.collateral_amount, Uint128::one()), Decimal::from_ratio(position.debt_amount, Uint128::one()))?;
//         //Add the position rate to the avg rate
//         avg_rate += position_rate * collateral_rate;
//     }



//     //Save CDP_AVG_RATE
//     CDP_AVG_RATE.save(deps.storage, &avg_rate)?;

//     Ok(Response::new().add_attributes(vec![
//         attr("method", "crank_cdp_apr"),
//         attr("new_avg_rate", avg_rate),
//         attr("last_user", basket_positions[basket_positions.len()].user),
//     ]))
// }

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::VaultTokenUnderlying { vault_token_amount } => to_json_binary(&query_vault_token_underlying(deps, env, vault_token_amount)?),
        QueryMsg::APR {} => to_json_binary(&query_apr(deps, env)?),
    }
}

/// Return APR for the valid durations 7, 30, 90, 365 days
fn query_apr(
    deps: Deps,
    env: Env,
) -> StdResult<APRResponse> {
    //Load config
    let config = CONFIG.load(deps.storage)?;
    //Load VT total
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    //Calc the rate of vault tokens to deposit tokens
    let btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000), 
        config.clone().total_deposit_tokens, 
        total_vault_tokens
    )?;

    let claim_tracker = CLAIM_TRACKER.load(deps.storage)?;
    let mut aprs = APRResponse {
        week_apr: None,
        month_apr: None,
        three_month_apr: None,
        year_apr: None,        
    };
    let mut running_duration = 0;
    let mut negative_apr = false;
    //Add the present duration as Checkpoint
    let mut claim_checkpoints = claim_tracker.vt_claim_checkpoints;
    claim_checkpoints.push(VTClaimCheckpoint {
        vt_claim_of_checkpoint: btokens_per_one,
        time_since_last_checkpoint: env.block.time.seconds() - claim_tracker.last_updated,
    });
    //Parse instances to allocate APRs to the correct duration
    //We reverse to get the most recent instances first
    claim_checkpoints.reverse();
    for claim_checkpoint in claim_checkpoints.into_iter() {
        running_duration += claim_checkpoint.time_since_last_checkpoint;
        

        if running_duration >= SECONDS_PER_DAY * 7 && aprs.week_apr.is_none() {
            
            /////Calc APR////
            let change_ratio = decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
             Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one()))?;

            let percent_change = match change_ratio.checked_sub(Decimal::one()){
                Ok(diff) => diff,
                //For this to happen, a compound has to be >10% slippage, a risk the vault users take
                Err(_) => {
                    negative_apr = true;
                    //Find the negative APR
                    Decimal::one() - change_ratio
                },
            };
            let apr = match percent_change.checked_div(Decimal::percent(52_00)){
                Ok(apr) => apr,
                Err(_) => return Err(StdError::GenericErr {msg: format!("Errored on the weekly APR calc using a percent change of {}", percent_change)})
            };

            aprs.week_apr = Some(APR {
                apr,
                negative: negative_apr
            });

            negative_apr = false;
        } else if running_duration >= SECONDS_PER_DAY * 30 && aprs.month_apr.is_none() {
            /////Calc APR////
            let change_ratio = decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
             Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one()))?;

            let percent_change = match change_ratio.checked_sub(Decimal::one()){
                Ok(diff) => diff,
                //For this to happen, a compound has to be >10% slippage, a risk the vault users take
                Err(_) => {
                    negative_apr = true;
                    //Find the negative APR
                    Decimal::one() - change_ratio
                },
            };
            let apr = match percent_change.checked_div(Decimal::percent(12_00)){
                Ok(apr) => apr,
                Err(_) => return Err(StdError::GenericErr {msg: format!("Errored on the monthly APR calc using a percent change of {}", percent_change)})
            };
            aprs.month_apr = Some(APR {
                apr,
                negative: negative_apr
            });
            negative_apr = false;
        } else if running_duration >= SECONDS_PER_DAY * 90 && aprs.three_month_apr.is_none() {
            /////Calc APR////
            let change_ratio = decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
             Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one()))?;

            let percent_change = match change_ratio.checked_sub(Decimal::one()){
                Ok(diff) => diff,
                //For this to happen, a compound has to be >10% slippage, a risk the vault users take
                Err(_) => {
                    negative_apr = true;
                    //Find the negative APR
                    Decimal::one() - change_ratio
                },
            };
            let apr = match percent_change.checked_div(Decimal::percent(4_00)){
                Ok(apr) => apr,
                Err(_) => return Err(StdError::GenericErr {msg: format!("Errored on the 3M APR calc using a percent change of {}", percent_change)})
            };
            aprs.three_month_apr = Some(APR {
                apr,
                negative: negative_apr
            });
            negative_apr = false;
        } else if running_duration >= SECONDS_PER_DAY * 365 && aprs.year_apr.is_none() {
            /////Calc APR////
            let change_ratio = decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
             Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one()))?;

            let percent_change = match change_ratio.checked_sub(Decimal::one()){
                Ok(diff) => diff,
                //For this to happen, a compound has to be >10% slippage, a risk the vault users take
                Err(_) => {
                    negative_apr = true;
                    //Find the negative APR
                    Decimal::one() - change_ratio
                },
            };
            let apr = percent_change;
            aprs.year_apr = Some(APR {
                apr,
                negative: negative_apr
            });   
            negative_apr = false;  
        }        
    }

    Ok(aprs)
}

/// Return underlying deposit token amount for an amount of vault tokens
fn query_vault_token_underlying(
    deps: Deps,
    _env: Env,
    vault_token_amount: Uint128,
) -> StdResult<Uint128> {
    let config = CONFIG.load(deps.storage)?;
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;

    //Query the Stability Pool for its total funds in state    
    let asset_pool: AssetPool = deps.querier.query_wasm_smart::<AssetPool> (
        config.stability_pool_contract.to_string(),
        &StabilityPoolQueryMsg::AssetPool { 
            user: None,
            deposit_limit: Some(1),
            start_after: None,
        },
    )?;
    let asset_pool_deposit_tokens = asset_pool.credit_asset.amount;
    //Query the Stability Pool balanace for its total deposit tokens
    let total_deposit_tokens = deps.querier.query_balance(config.stability_pool_contract.clone(), config.deposit_token.clone())?.amount;
    
    // If the Stability Pool has less deposit tokens than it thinks it does in state, return a discounted amount
    /////This is hack insurance & guarantees that underlying queries return less if the SP has been exploited////////
    let mut deposit_discount = Decimal::one();
    if total_deposit_tokens < asset_pool_deposit_tokens {
        deposit_discount = Decimal::from_ratio(total_deposit_tokens, asset_pool_deposit_tokens);
    }
    //Calc the amount of deposit tokens the user owns pre-discount
    let users_base_tokens = calculate_base_tokens(
        vault_token_amount,
        config.total_deposit_tokens,
        total_vault_tokens
    )?;
    //Apply the discount
    let discounted_base_tokens: Decimal = decimal_multiplication(Decimal::from_ratio(users_base_tokens, Uint128::one()), deposit_discount)?;

    //Return the discounted amount
    Ok(discounted_base_tokens.to_uint_floor())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.id {
        COMPOUND_REPLY_ID => handle_compound_reply(deps, env, msg),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
}


/// Find & save created full denom
fn handle_compound_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(result) => {
            //Load state
            let mut config = CONFIG.load(deps.storage)?; 
            let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;

            //Load previous deposit token balance
            let prev_balance = DEPOSIT_BALANCE_AT_LAST_CLAIM.load(deps.storage)?;
            
            //Load current balance of deposit token
            let current_balance = deps.querier.query_balance(env.contract.address.clone(), config.deposit_token.clone())?.amount;

            //If the contract has less of the deposit token than it started with, error.
            // if current_balance - config.compound_activation_fee <= prev_balance {
            //     return Err(StdError::GenericErr { msg: "Contract needs to compound more than the compound fee".to_string() });
            // }
            
            //^The reason we don't error here is bc if the contract swaps past a 10% slippage and it errors
            //, the contract will be stuck with depreciating assets. So its better to offload them and make up for the loss later.
            //This will be a risk communicated to users in the UI.
            
            //Calc the amount of deposit tokens that were compounded
            let compounded_amount = current_balance - prev_balance;
            //Update the total deposit tokens
            config.total_deposit_tokens += compounded_amount;
            //Update Claim tracker
            let mut claim_tracker = CLAIM_TRACKER.load(deps.storage)?;
            //Calculate time since last claim
            let time_since_last_checkpoint = env.block.time.seconds() - claim_tracker.last_updated;       
            
            //Calc the rate of vault tokens to deposit tokens
            let btokens_per_one = calculate_base_tokens(
                Uint128::new(1_000_000), 
                config.clone().total_deposit_tokens, 
                total_vault_tokens
            )?;

            //If the trackers total time is over a year, remove the first instance
            if claim_tracker.vt_claim_checkpoints.len() > 0 && claim_tracker.vt_claim_checkpoints.iter().map(|claim_checkpoint| claim_checkpoint.time_since_last_checkpoint).sum::<u64>() > SECONDS_PER_DAY * 365 {
                claim_tracker.vt_claim_checkpoints.remove(0);
            }
            //Push new instance
            claim_tracker.vt_claim_checkpoints.push(VTClaimCheckpoint {
                vt_claim_of_checkpoint: btokens_per_one,
                time_since_last_checkpoint,
            });
            //Update last updated time
            claim_tracker.last_updated = env.block.time.seconds();
            //Save Claim Tracker
            CLAIM_TRACKER.save(deps.storage, &claim_tracker)?;

            //Save Updated Config
            CONFIG.save(deps.storage, &config)?;
            
            //Send everything to the yield strategy
            let send_deposit_to_yield_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.stability_pool_contract.to_string(),
                msg: to_json_binary(&StabilityPoolExecuteMsg::Deposit { user: None })?,
                funds: vec![Coin {
                    denom: config.deposit_token.clone(),
                    amount: compounded_amount,
                }],
            });

            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_compound_reply")
                .add_attribute("compounded_amount", compounded_amount)
                .add_message(send_deposit_to_yield_msg);

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, env: Env, _msg: MigrateMsg) -> Result<Response, TokenFactoryError> {
    Ok(Response::default())
}