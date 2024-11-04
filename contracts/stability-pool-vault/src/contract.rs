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
    calculate_base_tokens, calculate_vault_tokens, Config, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg
};
use membrane::stability_pool::{ExecuteMsg as StabilityPoolExecuteMsg, QueryMsg as StabilityPoolQueryMsg, ClaimsResponse};
use membrane::osmosis_proxy::ExecuteMsg as OsmosisProxyExecuteMsg;
use membrane::types::{AssetPool, ClaimTracker, VTClaimCheckpoint, APR};
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
        .add_attribute("sub_denom", msg.clone().vault_subdenom);
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
        ExecuteMsg::CrankRealizedAPR { } => crank_realized_apr(deps, env, info),
        ExecuteMsg::RateAssurance { } => rate_assurance(deps, env, info),
    }
}

fn get_total_deposit_tokens(
    deps: Deps,
    env: Env,
    config: Config,
) -> StdResult<Uint128> {
    //Get deposits in the withdrawal buffer
    let buffered_deposit_tokens = deps.querier.query_balance(env.contract.address.clone(), config.deposit_token.clone())?.amount;

    //Query for deposits in the SP asset pool
    let asset_pool: AssetPool = deps.querier.query_wasm_smart::<AssetPool> (
        config.stability_pool_contract.to_string(),
        &StabilityPoolQueryMsg::AssetPool { 
            user: Some(env.contract.address.to_string()),
            deposit_limit: None,
            start_after: None,
        },
    )?;
    let total_pool_deposits = asset_pool.deposits.into_iter().map(|deposit| deposit.amount).sum::<Decimal>().to_uint_floor();
    // println!("here {}, {}", buffered_deposit_tokens, total_pool_deposits);
    //Parse deposits and calculate the total deposits
    let total_deposit_tokens = buffered_deposit_tokens + total_pool_deposits;

    Ok(total_deposit_tokens)
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

    let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

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
        Ok(claims) => {
            if claims.claims.clone().into_iter().filter(|claim| claim.denom.to_string() != String::from("factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/umbrn")).collect::<Vec<Coin>>().len() > 0 as usize {
                return Err(TokenFactoryError::ContractHasClaims { claims: claims.claims })
            } else {
                ClaimsResponse { claims: vec![] }
            }
        },
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
    let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
    //Get the total amount of vault tokens circulating
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    //Update the total deposit tokens after vault tokens are minted
    let pre_deposit_total_deposit_tokens = total_deposit_tokens - deposit_amount;
    //Calc & save token rates
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        pre_deposit_total_deposit_tokens, 
        total_vault_tokens
    )?;
    TOKEN_RATE_ASSURANCE.save(deps.storage, &TokenRateAssurance {
        pre_vtokens_per_one: Uint128::zero(),
        pre_btokens_per_one,
    })?;
    //Calculate the amount of vault tokens to mint
    let vault_tokens_to_distribute = calculate_vault_tokens(
        deposit_amount, 
        pre_deposit_total_deposit_tokens, 
        total_vault_tokens
    )?;
    ////////////////////////////////////////////////////
    
    //Update config's total deposit tokens
    config.total_deposit_tokens = total_deposit_tokens;

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
    let ratio_of_tokens_in_contract = decimal_division(Decimal::from_ratio(total_balance_minus_new_deposit, Uint128::one()), Decimal::from_ratio(total_deposit_tokens, Uint128::one()))?;

    //Calculate what is sent and what is kept
    let mut deposit_sent_to_yield: Uint128 = Uint128::zero();
    let mut deposit_kept: Uint128 = Uint128::zero();
    //If the ratio is less than the percent_to_keep_liquid, calculate the amount of deposit tokens to send to the yield strategy
    if ratio_of_tokens_in_contract < config.percent_to_keep_liquid {
        //Calculate the amount of deposit tokens that would make the ratio equal to the percent_to_keep_liquid
        let desired_ratio_tokens = decimal_multiplication(Decimal::from_ratio(total_deposit_tokens, Uint128::one()), config.percent_to_keep_liquid)?;
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
    // println!("{}, {}, {}", ratio_of_tokens_in_contract, config.percent_to_keep_liquid, deposit_sent_to_yield);

    //Send the deposit tokens to the yield strategy
    if !deposit_sent_to_yield.is_zero() {
        //Send deopsit
        let send_deposit_to_yield_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool_contract.to_string(),
            msg: to_json_binary(&StabilityPoolExecuteMsg::Deposit { user: None })?,
            funds: vec![Coin {
                denom: config.deposit_token.clone(),
                amount: deposit_sent_to_yield,
            }],
        });
        msgs.push(send_deposit_to_yield_msg);

        //Automatically withdraw to stay unstaked & liquid
        let withdraw_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool_contract.to_string(),
            msg: to_json_binary(&StabilityPoolExecuteMsg::Withdraw { amount: deposit_sent_to_yield })?,
            funds: vec![],
        });
        msgs.push(withdraw_msg);
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
        Ok(claims) => {
            if claims.claims.clone().into_iter().filter(|claim| claim.denom.to_string() != String::from("factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/umbrn")).collect::<Vec<Coin>>().len() > 0 as usize {
                return Err(TokenFactoryError::ContractHasClaims { claims: claims.claims })
            } else {
                ClaimsResponse { claims: vec![] }
            }
        },
        Err(_) => ClaimsResponse { claims: vec![] },
    };


    //////CHECK WITHDRAWAL QUEUE///////
    /// - We burn all VTs & simply add the user to a new state object called the withdrawal queue
    /// - Withdrawals that aren't fulfilled by the buffer unstake from the SP & claim the unstake amount in the queue
    /// - Whenever a user goes to exit, we check the queue & if they're in it, we set their withdrawal amount to the saved queue'd amount
    /// -- Key here is that the queue is virtually FIFO so the withdrawals before the current user are subtracted from the contract's serviceable amount (rn this is called contract_balance_post_SP_withdrawal)
    /// - If they send VTs && they're in the queue, we simply add the VT's backing to their potential total 
    /// - All exits will need to calc the queue total and subtract it from the contract's serviceable amount 
    /// -- but queue'd exits only subtract what's in front of them.
    /// -- To make this easy we save the state as a vec! of WithdrawalQueue objects & use the enumerated index to split the array at the user's index

    let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
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
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    TOKEN_RATE_ASSURANCE.save(deps.storage, &TokenRateAssurance {
        pre_vtokens_per_one: Uint128::zero(),
        pre_btokens_per_one,
    })?;
    //Calculate the amount of deposit tokens to withdraw
    let mut deposit_tokens_to_withdraw = calculate_base_tokens(
        vault_tokens, 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    //////////////////////////////////////////////////// 
    //Query the SP asset pool
    let asset_pool: AssetPool = deps.querier.query_wasm_smart::<AssetPool> (
        config.stability_pool_contract.to_string(),
        &StabilityPoolQueryMsg::AssetPool { 
            user: Some(env.contract.address.to_string()),
            deposit_limit: None,
            start_after: None,
        },
    )?;
    //Calc total TVL in the SP
    let contract_SP_tvl: Uint128 = asset_pool.deposits.clone().into_iter()
        .map(|deposit| deposit.amount)
        .sum::<Decimal>().to_uint_floor();

    
    //Instantiate rate assurance callback msg
    let assurance = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_json_binary(&ExecuteMsg::RateAssurance { })?,
        funds: vec![],
    });    

    //Parse deposits and calculate the amount of deposits that are withdrawable
    let withdrawable_amount = asset_pool.deposits.clone().into_iter()
        .filter(|deposit| deposit.unstake_time.is_some() && deposit.unstake_time.unwrap() + SECONDS_PER_DAY <= env.block.time.seconds())
        .map(|deposit| deposit.amount)
        .sum::<Decimal>().to_uint_floor();
    
    //Check contract's balance of deposit tokens
    let contract_balance_of_deposit_tokens = deps.querier.query_balance(env.contract.address.clone(), config.deposit_token.clone())?.amount;

    //Calc the total balance of liquid deposit tokens after the withdrawal
    let contract_balance_post_SP_withdrawal = withdrawable_amount + contract_balance_of_deposit_tokens;

    //If the contract will have less deposit tokens than the amount to withdraw
    // - Send the contract's balance to the user
    // - Unstake the desired withdrawal amount or the contract's TVL from the SP
    // - Calc the amount of vault tokens that represent the deposit_tokens actually being sent to the user, burn these
    // - Send back the rest of the vault tokens
    if contract_balance_post_SP_withdrawal < deposit_tokens_to_withdraw {        
        //Add the withdrawable amount to the deposit tokens to withdraw
        //bc the SP withdraws & unstakes in the same msg 
        deposit_tokens_to_withdraw += withdrawable_amount;
        //Set unstake amount to either the SP TVL or the desired withdrawal amount
        let unstake_amount = deposit_tokens_to_withdraw.min(contract_SP_tvl);
        //Unstake 
        let unstake_tokens_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool_contract.to_string(),
            msg: to_json_binary(&StabilityPoolExecuteMsg::Withdraw {
                amount: unstake_amount,
            })?,
            funds: vec![],
        });

        //Send the contract's balance to the user
        let send_deposit_tokens_msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![Coin {
                denom: config.deposit_token.clone(),
                amount: contract_balance_post_SP_withdrawal,
            }],
        });
        
        //Calc the amount of vault tokens that back the withdrawable amount (contract_balance_post_SP_withdrawal)
        let vault_tokens_to_burn = calculate_vault_tokens(
            contract_balance_post_SP_withdrawal, 
            total_deposit_tokens, 
            total_vault_tokens
        )?;
        //Burn vault tokens
        let burn_vault_tokens_msg: CosmosMsg = TokenFactory::MsgBurn {
            sender: env.contract.address.to_string(), 
            amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
                denom: config.vault_token.clone(),
                amount: vault_tokens_to_burn.to_string(),
            }), 
            burn_from_address: env.contract.address.to_string(),
        }.into();
        //Send back the rest of the vault tokens
        let vault_tokens_to_send = match vault_tokens.checked_sub(vault_tokens_to_burn){
            Ok(v) => v,
            Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract vault tokens to send: {} - {}", vault_tokens, vault_tokens_to_burn) }),
        };
        let send_vault_tokens_msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![Coin {
                denom: config.vault_token.clone(),
                amount: vault_tokens_to_send,
            }],
        });
        //Update the total vault tokens
        let new_vault_token_supply = match total_vault_tokens.checked_sub(vault_tokens_to_burn){
            Ok(v) => v,
            Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract vault token total supply: {} - {}", total_vault_tokens, vault_tokens) }),
        };
        VAULT_TOKEN.save(deps.storage, &new_vault_token_supply)?;
        //Save the updated config
        CONFIG.save(deps.storage, &config)?;

        return Ok(Response::new()
            .add_attribute("method", "exit_vault")
            .add_attribute("vault_tokens_burnt", vault_tokens_to_burn)
            .add_attribute("deposit_tokens_withdrawn", contract_balance_post_SP_withdrawal)
            .add_message(burn_vault_tokens_msg)
            .add_message(unstake_tokens_msg)
            .add_message(send_deposit_tokens_msg)
            .add_message(send_vault_tokens_msg)
            .add_message(assurance)
        );
    }

    //Send withdrawn tokens to the user (Contract buffer has enough to naked send)
    let send_deposit_tokens_msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: vec![Coin {
            denom: config.deposit_token.clone(),
            amount: deposit_tokens_to_withdraw,
        }],
    });
    
    //Burn vault tokens
    let burn_vault_tokens_msg: CosmosMsg = TokenFactory::MsgBurn {
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
            denom: config.vault_token.clone(),
            amount: vault_tokens.to_string(),
        }), 
        burn_from_address: env.contract.address.to_string(),
    }.into();

    //Update the total vault tokens
    let new_vault_token_supply = match total_vault_tokens.checked_sub(vault_tokens){
        Ok(v) => v,
        Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract vault token total supply: {} - {}", total_vault_tokens, vault_tokens) }),
    };
    //Update the total vault tokens
    VAULT_TOKEN.save(deps.storage, &new_vault_token_supply)?;
    //Save the updated config
    CONFIG.save(deps.storage, &config)?;

    //Add the withdrawable amount to the deposit tokens to withdraw
    //bc the SP withdraws & unstakes in the same msg 
    deposit_tokens_to_withdraw += withdrawable_amount;    
    //We're withdrawing to replenish the buffer
    
    //Set unstake amount to either the SP TVL or deposit_tokens_to_withdraw
    let unstake_amount = deposit_tokens_to_withdraw.min(contract_SP_tvl);
    //Unstake the deposit tokens from the Stability Pool
    let unstake_tokens_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.stability_pool_contract.to_string(),
        msg: to_json_binary(&StabilityPoolExecuteMsg::Withdraw {
            amount: unstake_amount,
        })?,
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
    let mut msgs = vec![];

    //Query claims from the Stability Pool
    let mut claims: ClaimsResponse = deps.querier.query_wasm_smart::<ClaimsResponse>(
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
    msgs.push(claim_msg);

    
    //If the claims include MBRN, create a burn message for it & filter it out of the swap
    match claims.claims.clone()
        .into_iter()
        .enumerate()
        .find(|(_, claim)| claim.denom.to_string() == String::from("factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/umbrn")){
            Some((i, claim)) => {
                let burn_mbrn_msg = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.osmosis_proxy_contract.to_string(),
                    msg: to_json_binary(&OsmosisProxyExecuteMsg::BurnTokens { 
                        denom: String::from("factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/umbrn"),
                        amount: claim.amount,
                        burn_from_address: env.contract.address.to_string(),
                    })?,
                    funds: vec![],
                });
                msgs.push(burn_mbrn_msg);
                //Remove the MBRN claim
                claims.claims.remove(i);
            },
            None => {},
    };
    

    //Compound rewards by sending to the Router in the Osmosis proxy contract
    //...send as a submsg that checks that the contract has more of the deposit token than it started with
    let compound_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.osmosis_proxy_contract.to_string(),
        msg: to_json_binary(&OsmosisProxyExecuteMsg::ExecuteSwaps {
            token_out: config.deposit_token.clone(),
            max_slippage: Decimal::one(),
        })?,
        funds: claims.claims.clone(),
    });
    let compound_submsg = SubMsg::reply_on_success(compound_msg, COMPOUND_REPLY_ID);

    //Save current deposit token balance
    DEPOSIT_BALANCE_AT_LAST_CLAIM.save(deps.storage, &deps.querier.query_balance(env.contract.address.clone(), config.deposit_token.clone())?.amount)?;


    //Create Response
    let res = Response::new()
        .add_attribute("method", "claim_and_compound_liquidations")
        .add_messages(msgs)   
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

fn crank_realized_apr(
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
    //Get the total deposit tokens
    let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
    config.total_deposit_tokens = total_deposit_tokens;
    CONFIG.save(deps.storage, &config)?;
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
        QueryMsg::ClaimTracker {} => to_json_binary(&CLAIM_TRACKER.load(deps.storage)?),
    }
}

/// Return APR for the valid durations 7, 30, 90, 365 days
// fn query_apr(
//     deps: Deps,
//     env: Env,
// ) -> StdResult<APRResponse> {
//     //Load config
//     let config = CONFIG.load(deps.storage)?;
//     //Load VT total
//     let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
//     //Calc the rate of vault tokens to deposit tokens
//     let btokens_per_one = calculate_base_tokens(
//         Uint128::new(1_000_000), 
//         config.clone().total_deposit_tokens, 
//         total_vault_tokens
//     )?;

//     let claim_tracker = CLAIM_TRACKER.load(deps.storage)?;
//     let mut aprs = APRResponse {
//         week_apr: None,
//         month_apr: None,
//         three_month_apr: None,
//         year_apr: None,        
//     };
//     let mut running_duration = 0;
//     let mut negative_apr = false;
//     //Add the present duration as Checkpoint
//     let mut claim_checkpoints = claim_tracker.vt_claim_checkpoints;
//     claim_checkpoints.push(VTClaimCheckpoint {
//         vt_claim_of_checkpoint: btokens_per_one,
//         time_since_last_checkpoint: env.block.time.seconds() - claim_tracker.last_updated,
//     });
//     //Parse instances to allocate APRs to the correct duration
//     //We reverse to get the most recent instances first
//     claim_checkpoints.reverse();
//     for claim_checkpoint in claim_checkpoints.into_iter() {
//         running_duration += claim_checkpoint.time_since_last_checkpoint;
        

//         if running_duration >= SECONDS_PER_DAY * 7 && aprs.week_apr.is_none() {
            
//             /////Calc APR////
//             let change_ratio = decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
//              Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one()))?;

//             let percent_change = match change_ratio.checked_sub(Decimal::one()){
//                 Ok(diff) => diff,
//                 //For this to happen, a compound has to be >10% slippage, a risk the vault users take
//                 Err(_) => {
//                     negative_apr = true;
//                     //Find the negative APR
//                     Decimal::one() - change_ratio
//                 },
//             };
//             let apr = match percent_change.checked_mul(Decimal::percent(52_00)){
//                 Ok(apr) => apr,
//                 Err(_) => return Err(StdError::GenericErr {msg: format!("Errored on the weekly APR calc using a percent change of {}", percent_change)})
//             };

//             aprs.week_apr = Some(APR {
//                 apr,
//                 negative: negative_apr
//             });

//             negative_apr = false;
//         }
//         if running_duration >= SECONDS_PER_DAY * 30 && aprs.month_apr.is_none() {
//             /////Calc APR////
//             let change_ratio = decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
//              Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one()))?;

//             let percent_change = match change_ratio.checked_sub(Decimal::one()){
//                 Ok(diff) => diff,
//                 //For this to happen, a compound has to be >10% slippage, a risk the vault users take
//                 Err(_) => {
//                     negative_apr = true;
//                     //Find the negative APR
//                     Decimal::one() - change_ratio
//                 },
//             };
//             let apr = match percent_change.checked_mul(Decimal::percent(12_00)){
//                 Ok(apr) => apr,
//                 Err(_) => return Err(StdError::GenericErr {msg: format!("Errored on the monthly APR calc using a percent change of {}", percent_change)})
//             };
//             aprs.month_apr = Some(APR {
//                 apr,
//                 negative: negative_apr
//             });
//             negative_apr = false;
//         }
//         if running_duration >= SECONDS_PER_DAY * 90 && aprs.three_month_apr.is_none() {
//             /////Calc APR////
//             let change_ratio = decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
//              Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one()))?;

//             let percent_change = match change_ratio.checked_sub(Decimal::one()){
//                 Ok(diff) => diff,
//                 //For this to happen, a compound has to be >10% slippage, a risk the vault users take
//                 Err(_) => {
//                     negative_apr = true;
//                     //Find the negative APR
//                     Decimal::one() - change_ratio
//                 },
//             };
//             let apr = match percent_change.checked_mul(Decimal::percent(4_00)){
//                 Ok(apr) => apr,
//                 Err(_) => return Err(StdError::GenericErr {msg: format!("Errored on the 3M APR calc using a percent change of {}", percent_change)})
//             };
//             aprs.three_month_apr = Some(APR {
//                 apr,
//                 negative: negative_apr
//             });
//             negative_apr = false;
//         } 
//         if running_duration >= SECONDS_PER_DAY * 365 && aprs.year_apr.is_none() {
//             /////Calc APR////
//             let change_ratio = decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
//              Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one()))?;

//             let percent_change = match change_ratio.checked_sub(Decimal::one()){
//                 Ok(diff) => diff,
//                 //For this to happen, a compound has to be >10% slippage, a risk the vault users take
//                 Err(_) => {
//                     negative_apr = true;
//                     //Find the negative APR
//                     Decimal::one() - change_ratio
//                 },
//             };
//             let apr = percent_change;
//             aprs.year_apr = Some(APR {
//                 apr,
//                 negative: negative_apr
//             });   
//             negative_apr = false;  
//         }        
//     }

//     Ok(aprs)
// }

/// Return underlying deposit token amount for an amount of vault tokens
fn query_vault_token_underlying(
    deps: Deps,
    env: Env,
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
    let sp_total_deposit_tokens = deps.querier.query_balance(config.stability_pool_contract.clone(), config.deposit_token.clone())?.amount;

    // If the Stability Pool has less deposit tokens than it thinks it does in state, return a discounted amount
    /////This is hack insurance & guarantees that underlying queries return less if the SP has been exploited////////
    let mut deposit_discount = Decimal::one();
    if sp_total_deposit_tokens < asset_pool_deposit_tokens {
        deposit_discount = Decimal::from_ratio(sp_total_deposit_tokens, asset_pool_deposit_tokens);
    }
    
    //Get contract's total deposit tokens
    let total_deposit_tokens = get_total_deposit_tokens(deps, env.clone(), config.clone())?;
    // println!("{}, {}, {}, {}", sp_total_deposit_tokens, asset_pool_deposit_tokens, deposit_discount, total_deposit_tokens);

    //Calc the amount of deposit tokens the user owns pre-discount
    let users_base_tokens = calculate_base_tokens(
        vault_token_amount,
        total_deposit_tokens,
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
            let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

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
            //Update the config's total deposit tokens
            config.total_deposit_tokens = total_deposit_tokens;
            //Update Claim tracker
            let mut claim_tracker = CLAIM_TRACKER.load(deps.storage)?;
            //Calculate time since last claim
            let time_since_last_checkpoint = env.block.time.seconds() - claim_tracker.last_updated;       
            
            //Calc the rate of vault tokens to deposit tokens
            let btokens_per_one = calculate_base_tokens(
                Uint128::new(1_000_000_000_000), 
                total_deposit_tokens, 
                total_vault_tokens
            )?;

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
    let config = CONFIG.load(deps.storage)?;
    //Mint vault tokens to the sender
    let mint_vault_tokens_msg: CosmosMsg = TokenFactory::MsgMint {
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
            denom: config.vault_token.clone(),
            amount:"487728827211044".to_string(),
        }), 
        mint_to_address: String::from("osmo1wjjg0mvsfgnskjj7qq28uaxqwq5h38q68enshj"),
    }.into();

    Ok(Response::default().add_message(mint_vault_tokens_msg))
}