use std::cmp::min;

use cosmwasm_std::{
    entry_point, to_binary, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, WasmMsg,
    Response, StdResult, Uint128, Reply, StdError, CosmosMsg, SubMsg, Addr, coin, attr, Storage, Empty, WasmQuery, QueryRequest,
};
use cw2::set_contract_version;

use membrane::helpers::{withdrawal_msg, get_contract_balances};
use membrane::launch::{Config, ExecuteMsg, InstantiateMsg, QueryMsg, UpdateConfig, MigrateMsg};
use membrane::math::{decimal_division, decimal_multiplication};
use membrane::staking::ExecuteMsg as StakingExecuteMsg;
use membrane::osmosis_proxy::ExecuteMsg as OPExecuteMsg;
use membrane::types::{AssetInfo, Asset, UserRatio, Lockdrop, LockedUser, Lock, Owner, VestingPeriod};

use membrane::vesting::{QueryMsg as VestingQueryMsg, ExecuteMsg as VestingExecuteMsg, RecipientsResponse};
use membrane::cdp::{ExecuteMsg as CDPExecuteMsg, UpdateConfig as CDPUpdateConfig};
use membrane::oracle::ExecuteMsg as OracleExecuteMsg;
use membrane::liq_queue::ExecuteMsg as LQExecuteMsg;
use membrane::liquidity_check::ExecuteMsg as LCExecuteMsg;
use membrane::auction::{ExecuteMsg as DAExecuteMsg, UpdateConfig as AuctionUpdateConfig};
use membrane::discount_vault::ExecuteMsg as DiscountVaultExecuteMsg;

use osmosis_std::types::cosmos::base::v1beta1::Coin;
use osmosis_std::types::osmosis::gamm::poolmodels::balancer::v1beta1::MsgCreateBalancerPool;
use osmosis_std::types::osmosis::gamm::v1beta1::PoolParams;
use osmosis_std::types::osmosis::gamm::v1beta1::PoolAsset;


use crate::error::ContractError;
use crate::state::{CONFIG, ADDRESSES, LaunchAddrs, OSMO_POOL_ID, LOCKDROP, INCENTIVE_RATIOS, LOCKED_USERS};
use crate::reply::{handle_auction_reply, handle_cdp_reply, handle_create_denom_reply, handle_gov_reply, handle_lc_reply, handle_lq_reply, handle_op_reply, handle_oracle_reply, handle_sp_reply, handle_staking_reply, handle_vesting_reply, handle_discount_vault_reply, handle_system_discounts_reply, handle_balancer_reply};

// Contract name and version used for migration.
const CONTRACT_NAME: &str = "launch";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

//Reply ID
pub const OSMOSIS_PROXY_REPLY_ID: u64 = 1;
pub const ORACLE_REPLY_ID: u64 = 2;
pub const STAKING_REPLY_ID: u64 = 3;
pub const VESTING_REPLY_ID: u64 = 4;
pub const GOVERNANCE_REPLY_ID: u64 = 5;
pub const POSITIONS_REPLY_ID: u64 = 6;
pub const STABILITY_POOL_REPLY_ID: u64 = 7;
pub const LIQ_QUEUE_REPLY_ID: u64 = 8;
pub const LIQUIDITY_CHECK_REPLY_ID: u64 = 9;
pub const DEBT_AUCTION_REPLY_ID: u64 = 10;
pub const CREATE_DENOM_REPLY_ID: u64 = 12;
pub const SYSTEM_DISCOUNTS_REPLY_ID: u64 = 13;
pub const DISCOUNT_VAULT_REPLY_ID: u64 = 14;
pub const BALANCER_POOL_REPLY_ID: u64 = 15;
pub const NO_ACTION_ID: u64 = 16;

//Constants
pub const SECONDS_PER_DAY: u64 = 86_400u64;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {

    //Need 20 OSMO for CreateDenom Msgs
    // if deps.querier.query_balance(env.clone().contract.address, "uosmo")?.amount < Uint128::new(20_000_000){ return Err(ContractError::NeedOsmo {}) }

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let config = Config {
        mbrn_denom: String::from(""),
        credit_denom: String::from(""),
        pre_launch_contributors: deps.api.addr_validate(&msg.pre_launch_contributors)?,
        pre_launch_community: msg.pre_launch_community,
        apollo_router: deps.api.addr_validate(&msg.apollo_router)?,
        mbrn_launch_amount: Uint128::new(10_000_000_000_000),
        osmosis_proxy_id: msg.osmosis_proxy_id,
        oracle_id: msg.oracle_id,
        staking_id: msg.staking_id,
        vesting_id: msg.vesting_id,
        governance_id: msg.governance_id,
        positions_id: msg.positions_id,
        stability_pool_id: msg.stability_pool_id,
        liq_queue_id: msg.liq_queue_id,
        liquidity_check_id: msg.liquidity_check_id,
        mbrn_auction_id: msg.mbrn_auction_id,
        system_discounts_id: msg.system_discounts_id,
        discount_vault_id: msg.discount_vault_id,
        atom_denom: String::from("ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2"), //testnet: ibc/A8C2D23A1E6F95DA4E48BA349667E322BD7A6C996D8A4AAE8BA72E190F3D1477
        osmo_denom: String::from("uosmo"),
        usdc_denom: String::from("ibc/D189335C6E4A68B513C10AB227BF1C1D38C746766278BA3EEB4FB14124F1D858"),  //axl wrapped usdc //testnet: 6F34E1BD664C36CE49ACC28E60D62559A5F96C4F9A6CCE4FC5A67B2852E24CFE
        atomosmo_pool_id: 1, //testnet is 12
        osmousdc_pool_id: 678, //axl wrapped usdc, testnet is 5
    };
    CONFIG.save(deps.storage, &config)?;

    ADDRESSES.save(deps.storage, &LaunchAddrs {
        osmosis_proxy: Addr::unchecked(""),
        oracle: Addr::unchecked(""),
        staking: Addr::unchecked(""),
        vesting: Addr::unchecked(""),
        governance: Addr::unchecked(""),
        positions: Addr::unchecked(""),
        stability_pool: Addr::unchecked(""),
        liq_queue: Addr::unchecked(""),
        liquidity_check: Addr::unchecked(""),
        mbrn_auction: Addr::unchecked(""),
        discount_vault: Addr::unchecked(""),
        system_discounts: Addr::unchecked(""),
    })?;

    let msg = CosmosMsg::Wasm(WasmMsg::Instantiate { 
        admin: Some(env.clone().contract.address.to_string()),
        code_id: config.clone().osmosis_proxy_id,
        msg: to_binary(&Empty {})?,
        funds: vec![],
        label: String::from("osmosis_proxy") 
    });
    let sub_msg = SubMsg::reply_on_success(msg, OSMOSIS_PROXY_REPLY_ID);

    //Instantiate Lockdrop 
    let lockdrop = Lockdrop {
        num_of_incentives: Uint128::new(10_000_000_000_000),
        locked_asset: AssetInfo::NativeToken { denom: String::from("uosmo") },
        lock_up_ceiling: 365,
        start_time: env.block.time.seconds(),
        deposit_end: env.block.time.seconds() + (5 * SECONDS_PER_DAY), //5 days 
        withdrawal_end: env.block.time.seconds() + (7 * SECONDS_PER_DAY), //2 day after the deposit
        launched: false,
    };
    LOCKDROP.save(deps.storage, &lockdrop)?;

    //Instantiate Incentive Ratios
    INCENTIVE_RATIOS.save(deps.storage, &vec![])?;

    //Instantiate Pool ID
    OSMO_POOL_ID.save(deps.storage, &0)?;

    Ok(Response::new()
        .add_submessage(sub_msg)
        .add_attribute("config", format!("{:?}", config))
        .add_attribute("contract_address", env.contract.address)
    )
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Lock { lock_up_duration } => lock(deps, env, info, lock_up_duration),
        ExecuteMsg::ChangeLockDuration { uosmo_amount, old_lock_up_duration, new_lock_up_duration } => change_lockup_duration(deps, env, info, uosmo_amount, old_lock_up_duration, new_lock_up_duration),
        ExecuteMsg::Withdraw { withdrawal_amount, lock_up_duration } => withdraw(deps, env, info, withdrawal_amount, lock_up_duration),
        ExecuteMsg::Claim { } => claim(deps, env, info),
        ExecuteMsg::Launch{ } => end_of_launch(deps, env),
        ExecuteMsg::UpdateConfig(update) => update_config(deps, info, update),
        ExecuteMsg::UpdateContractConfigs { new_governance_contract } => update_contract_configs(deps, info, new_governance_contract),
    }
}

/// Deposit OSMO into the lockdrop & elect to lock MBRN rewards for a certain duration
fn lock(    
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    lock_up_duration: u64,
) -> Result<Response, ContractError>{
    let lockdrop = LOCKDROP.load(deps.storage)?;

    //Assert Lockdrop is in deposit period
    if env.block.time.seconds() > lockdrop.deposit_end { return Err(ContractError::DepositsOver {  }) }
    //Validate lockup duration
    if lock_up_duration > lockdrop.lock_up_ceiling { return Err(ContractError::CustomError { val: String::from("Can't lock that long")}) }

    let valid_asset = validate_lockdrop_asset(info.clone(), lockdrop.clone().locked_asset)?;

    //Find & add to User
    if let Ok(mut locked_user) = LOCKED_USERS.load(deps.storage, info.clone().sender){

        //Check if user has already locked up for this duration && if so, add to it
        if let Some((i, _)) = locked_user.deposits.clone().into_iter().enumerate().find(|(_, lock)| lock.lock_up_duration == lock_up_duration) {
            //Add to existing
            locked_user.deposits[i].deposit += valid_asset.amount;
            
        } else {
            //Add a new lock
            locked_user.deposits.push(
                Lock { 
                    deposit: valid_asset.amount, 
                    lock_up_duration: lock_up_duration.clone(),
                }
            );
        } 
        LOCKED_USERS.save(deps.storage, info.clone().sender, &locked_user)?; 

    } else {
        //Add a User
        let user = LockedUser { 
            user: info.clone().sender, 
            deposits: vec![Lock { 
                deposit: valid_asset.amount, 
                lock_up_duration: lock_up_duration.clone(),
            }],
            total_tickets: Uint128::zero(),
            incentives_withdrawn: Uint128::zero(),
        };
            
        LOCKED_USERS.save(deps.storage, info.clone().sender, &user)?;

    } 

    Ok(Response::new()
        .add_attributes(vec![
            attr("method", "deposit"),
            attr("user", info.clone().sender),
            attr("lock_up_duration", lock_up_duration.to_string()),
            attr("deposit", valid_asset.to_string()),
        ]))
}

/// Edit lockup duration of a locked deposit
fn change_lockup_duration(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    uosmo_amount: Option<Uint128>,
    old_lock_up_duration: u64,
    new_lock_up_duration: u64,
) -> Result<Response, ContractError>{    
    let lockdrop = LOCKDROP.load(deps.storage)?;
    let attributes;

    //Assert Lockdrop is in deposit period
    if env.block.time.seconds() > lockdrop.deposit_end { return Err(ContractError::DepositsOver {  }) }
    //Validate lockup duration
    if new_lock_up_duration > lockdrop.lock_up_ceiling {  return Err(ContractError::CustomError { val: String::from("Can't lock that long")}) }
    
    //Find lockup duration in user's deposits
    if let Ok(mut locked_user) = LOCKED_USERS.load(deps.storage, info.clone().sender){

        //Check if user has already locked up for this duration && if so, add to it
        if let Some((i, _)) = locked_user.deposits.clone().into_iter().enumerate().find(|(_, lock)| lock.lock_up_duration == old_lock_up_duration) {
            //Validate uosmo amount
            let change_amount = if let Some(amount) = uosmo_amount {
                //Take minimum of amount or deposit
                min(amount, locked_user.deposits[i].deposit)
            } else {
                locked_user.deposits[i].deposit
            };

            //Set attributes
            attributes = vec![
                attr("method", "edit_lockup_duration"),
                attr("user", info.clone().sender),
                attr("old_lock_up_duration", old_lock_up_duration.to_string()),
                attr("new_lock_up_duration", new_lock_up_duration.to_string()),
                attr("amount_edited", change_amount.to_string()),
            ];

            //Subtract from existing
            locked_user.deposits[i].deposit -= change_amount;
            //if deposit is now zero, remove it
            if locked_user.deposits[i].deposit == Uint128::zero() {
                locked_user.deposits.remove(i);
            }

            //Check if user has already locked up for the new duration && if so, add to it
            if let Some((i, _)) = locked_user.deposits.clone().into_iter().enumerate().find(|(_, lock)| lock.lock_up_duration == new_lock_up_duration) {
                //Add to existing
                locked_user.deposits[i].deposit += change_amount;
                
            } else {
                //Add a new lock
                locked_user.deposits.push(
                    Lock { 
                        deposit: change_amount, 
                        lock_up_duration: new_lock_up_duration.clone(),
                    }
                );
            }
            
        } else {
            return Err(ContractError::CustomError { val: String::from("User has no deposit with that lockup duration")})
        }
        //Save user info
        LOCKED_USERS.save(deps.storage, info.clone().sender, &locked_user)?; 

    } else {
        return Err(ContractError::NotAUser {  })

    } 

    Ok(Response::new()
        .add_attributes(attributes))
}

/// Withdraw OSMO from the lockdrop during the withdrawal period
fn withdraw(    
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mut withdrawal_amount: Uint128,
    lock_up_duration: u64,
) -> Result<Response, ContractError>{
    let lockdrop = LOCKDROP.load(deps.storage)?;

    //Assert Lockdrop is in withdraw period
    if env.block.time.seconds() < lockdrop.deposit_end || env.block.time.seconds() > lockdrop.withdrawal_end { return Err(ContractError::WithdrawalsOver {  }) }

    let initial_withdraw_amount = withdrawal_amount;

    //Find & remove from LockedUser
    if let Ok(mut locked_user) = LOCKED_USERS.load(deps.storage, info.clone().sender){

        locked_user.deposits = locked_user.clone().deposits
            .into_iter()
            .map(|mut deposit| {
                if deposit.lock_up_duration == lock_up_duration {

                    if deposit.deposit >= withdrawal_amount {
                        deposit.deposit -= withdrawal_amount;
                        withdrawal_amount = Uint128::zero();

                        deposit
                    } else {
                        withdrawal_amount -= deposit.deposit;
                        deposit.deposit = Uint128::zero();

                        deposit
                    }

                } else { deposit }                 
                
                
            })
            .collect::<Vec<Lock>>()
            .into_iter()
            .filter(|deposit| deposit.deposit != Uint128::zero())
            .collect::<Vec<Lock>>();

        if !withdrawal_amount.is_zero() {
            return Err(ContractError::CustomError { val: format!("This user only owns {} of the locked asset in this lockup duration: {}, retry withdrawal at or below that amount", initial_withdraw_amount - withdrawal_amount, lock_up_duration) })
        }
        
        //Save LockedUser
        LOCKED_USERS.save(deps.storage, info.clone().sender, &locked_user)?;

    } else {
        return Err(ContractError::NotAUser {})
    }

    //Create Withdraw Msg
    let msg = withdrawal_msg(
        Asset {
            info: lockdrop.clone().locked_asset,
            amount: initial_withdraw_amount.clone(),            
    }, info.clone().sender)?;

    Ok(Response::new()
        .add_message(msg)
        .add_attributes(vec![
            attr("method", "withdraw"),
            attr("user", info.clone().sender),
            attr("lock_up_duration", lock_up_duration.to_string()),
            attr("withdraw", initial_withdraw_amount),
        ]))
}

/// Claim unlocked MBRN rewards
fn claim(    
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError>{
    let lockdrop = LOCKDROP.load(deps.storage)?;

    //Assert lockdrop has ended
    if env.block.time.seconds() <= lockdrop.withdrawal_end {
        return Err(ContractError::CustomError { val: String::from("Lockdrop hasn't ended yet") })
    }

    let addrs = ADDRESSES.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;

    //Only run the ticket calculation once 
    let mut user_ratios = INCENTIVE_RATIOS.load(deps.storage)?;
    
    if user_ratios.is_empty(){
        user_ratios = calc_ticket_distribution(deps.storage)?;
        
        //Save user incentive ratios
        INCENTIVE_RATIOS.save(deps.storage, &user_ratios)?;
    }
    
    //////Claim any unlocked incentives/////
    //Get total incentives the user is entitled to
    let incentives = get_user_incentives(
        user_ratios,
        info.sender.to_string(),
        lockdrop.num_of_incentives,
    )?;
    
    let mut withdrawable_tickets = Uint128::zero();
    let amount_to_mint: Uint128;
    //Find withdrawable tickets
    if let Ok(mut locked_user) = LOCKED_USERS.load(deps.storage, info.clone().sender){
        let time_since_lockdrop_end = env.block.time.seconds() - lockdrop.withdrawal_end;       

        for (_i, deposit) in locked_user.clone().deposits.into_iter().enumerate() {
            //Unlock deposit rewards that have passed their lock duration
            if time_since_lockdrop_end > deposit.lock_up_duration * SECONDS_PER_DAY {
                withdrawable_tickets += deposit.deposit * Uint128::from((deposit.lock_up_duration + 1) as u128);
            } else {
                //Unlock deposit rewards that have passed their lock duration LINEARLY
                let ratio_of_time_passed = decimal_division(
                    Decimal::from_ratio(time_since_lockdrop_end, Uint128::one()), 
                    Decimal::from_ratio(deposit.lock_up_duration * SECONDS_PER_DAY, Uint128::one()))?;
                    
                let total_tickets_for_deposit = deposit.deposit * Uint128::from((deposit.lock_up_duration + 1) as u128);

                withdrawable_tickets += total_tickets_for_deposit * ratio_of_time_passed;
            }
        }

        //Calc ratio of incentives to unlock
        let ratio_of_unlock = decimal_division(
            Decimal::from_ratio(withdrawable_tickets, Uint128::one()), 
            Decimal::from_ratio(locked_user.total_tickets, Uint128::one()))?;

        let unlocked_incentives = ratio_of_unlock * incentives;

        //Calc locked incentives
        let locked_incentives = incentives.checked_sub(unlocked_incentives).unwrap_or_else(|_| Uint128::zero());
        if locked_incentives < Uint128::new(1_000_000) && locked_incentives > Uint128::zero() {
            return Err(ContractError::CustomError { val: String::from("If you leave less than 1 MBRN still unlocking, it'll get stuck due to the minimum stake amount") })
        }

        //Calc amount available to mint
        amount_to_mint = match unlocked_incentives.checked_sub(locked_user.incentives_withdrawn){
            Ok(amount) => amount,
            Err(_) => Uint128::zero(),
        };
        //Update incentives withdraw
        locked_user.incentives_withdrawn += amount_to_mint;
        
        //Save updated incentive tally
        LOCKED_USERS.save(deps.storage, info.clone().sender, &locked_user)?;

    } else {
        return Err(ContractError::NotAUser {})
    }    

    let attrs = vec![
        attr("method", "claim"),
        attr("staked_ownership", amount_to_mint),
    ];

    //Create mint & stake msgs if there are incentives to withdraw
    if !amount_to_mint.is_zero(){

        let mint_msg = CosmosMsg::Wasm(WasmMsg::Execute { 
            contract_addr: addrs.osmosis_proxy.to_string(), 
            msg: to_binary(&OPExecuteMsg::MintTokens { 
                denom: config.clone().mbrn_denom, 
                amount: amount_to_mint.clone(), 
                mint_to_address: env.clone().contract.address.to_string(),
            })?, 
            funds: vec![] 
        });

        let stake_msg = CosmosMsg::Wasm(WasmMsg::Execute { 
            contract_addr: addrs.staking.to_string(), 
            msg: to_binary(&StakingExecuteMsg::Stake { user: Some(info.clone().sender.to_string()) })?, 
            funds: vec![coin(amount_to_mint.into(), config.clone().mbrn_denom)] 
        });

        Ok(Response::new()
            .add_attributes(attrs)
            .add_messages(vec![mint_msg, stake_msg])
        )
    } else {
        return Err(ContractError::CustomError { val: String::from("No incentives to claim") })
    }
    
}

/// Return the amount of incentives a user is entitled to
fn get_user_incentives(
    user_ratios: Vec<UserRatio>,
    user: String,
    total_incentives: Uint128,
) -> StdResult<Uint128>{

    let incentives: Uint128 = match user_ratios.clone().into_iter().enumerate().find(|(_i, user_ratio)| user_ratio.user.to_string() == user){
        Some((_i, user)) => {

            decimal_multiplication(
                user.ratio, 
                Decimal::from_ratio(total_incentives, Uint128::one())
            )? * Uint128::one()
        },
        None => {
            return Err(StdError::GenericErr { msg: String::from("User didn't participate in the lockdrop") })
        },
    };

    Ok(incentives)
}

/// Calculate the ratio of incentives each user is entitled to
fn calc_ticket_distribution(
    storage: &mut dyn Storage,
) -> StdResult<Vec<UserRatio>>{
    let mut error: Option<StdError> = None;

    let user_totals = LOCKED_USERS
        .range(storage, None, None, cosmwasm_std::Order::Ascending)
        .into_iter()
        .map(|item| {
            let (_, locked_user) = match item {
                Ok(locked_user) => locked_user,
                Err(err) => {
                    error = Some(err);
                    return (Addr::unchecked(""), Uint128::zero());
                }
            };

            let total_tickets: Uint128 = locked_user.deposits
                .into_iter()
                .map(|deposit| deposit.deposit * Uint128::from(deposit.lock_up_duration + 1) )
                .collect::<Vec<Uint128>>()
                .into_iter()
                .sum();

            (locked_user.user, total_tickets)
        })
        .collect::<Vec<(Addr, Uint128)>>();

    //Set each user's total_tickets
    for (addr, total) in user_totals.clone().into_iter(){
        LOCKED_USERS.update(storage, addr, |locked_user| -> StdResult<LockedUser>{
            let mut new_locked_user: LockedUser = locked_user.unwrap();
            
            new_locked_user.total_tickets = total;
            
            Ok(new_locked_user)
        })?;
    }

    let total_tickets: Uint128 = user_totals.clone()
        .into_iter()
        .map(|user| user.1)
        .collect::<Vec<Uint128>>()
        .into_iter()
        .sum();

    let user_ratios: Vec<UserRatio> = user_totals.clone()
        .into_iter()
        .map(|user| {
            let ratio = decimal_division(
                Decimal::from_ratio(user.1, Uint128::one()),
                Decimal::from_ratio(total_tickets, Uint128::one()),
            ).unwrap_or_else(|e| {
                error = Some(e);
                Decimal::zero()
            });

            UserRatio { user: user.0, ratio }
        })
        .collect::<Vec<UserRatio>>();

    if let Some(e) = error {
        return Err(e)
    }

    Ok(user_ratios)
}

/// Validate that the lockdrop asset is present in the message
fn validate_lockdrop_asset(info: MessageInfo, lockdrop_asset: AssetInfo) -> StdResult<Asset>{
    if info.clone().funds.len() > 1 {
        return Err(StdError::GenericErr { msg: format!("Invalid assets sent") })
    }

    if let Some(lockdrop_asset) = info.clone().funds
        .into_iter()
        .find(|coin| coin.denom == lockdrop_asset.to_string()){

            // Assert Minimum OSMO amount: 1 OSMO
            if lockdrop_asset.amount < Uint128::from(1_000_000u128) {
                return Err(StdError::GenericErr { msg: format!("Minimum deposit is 1_000_000 uosmo") })
            }

        Ok(Asset { 
            info: AssetInfo::NativeToken { denom: lockdrop_asset.denom }, 
            amount: lockdrop_asset.amount })
    } else {
        return Err(StdError::GenericErr { msg: format!("No valid lockdrop asset, looking for {}", lockdrop_asset) })
    }
}

/// Update contract configuration
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    update: UpdateConfig,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    //Assert authority
    if info.sender != config.clone().pre_launch_contributors {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(credit_denom) = update.credit_denom {
        config.credit_denom = credit_denom;
    }
    if let Some(mbrn_denom) = update.mbrn_denom {
        config.mbrn_denom = mbrn_denom;
    }
    if let Some(osmo_denom) = update.osmo_denom {
        config.osmo_denom = osmo_denom;
    }
    if let Some(usdc_denom) = update.usdc_denom {
        config.usdc_denom = usdc_denom;
    }

    //Save Config
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attribute("new_config", format!("{:?}", config)))
}

/// Update contract configuration
fn update_contract_configs(
    deps: DepsMut,
    info: MessageInfo,
    gov_contract: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.clone().pre_launch_contributors {
        return Err(ContractError::Unauthorized {});
    }

    let addresses = ADDRESSES.load(deps.storage)?;
    let mut msgs: Vec<CosmosMsg> = vec![];

    //Update Vesting
    // msgs.push(
    //     CosmosMsg::Wasm(WasmMsg::Execute {
    //         contract_addr: addresses.vesting.to_string(),
    //         msg: to_binary(&VestingExecuteMsg::UpdateConfig {
    //             owner: Some(gov_contract.clone()),
    //             osmosis_proxy: None,
    //             staking_contract: None,
    //             mbrn_denom: None,
    //             additional_allocation: None,                
    //         })?,
    //         funds: vec![],
    //     })
    // );
    //Update CDP
    // msgs.push(
    //     CosmosMsg::Wasm(WasmMsg::Execute {
    //         contract_addr: addresses.positions.to_string(),
    //         msg: to_binary(&CDPExecuteMsg::UpdateConfig(CDPUpdateConfig {
    //             owner: Some(gov_contract.clone()),
    //             stability_pool: None,
    //             debt_auction: None,
    //             staking_contract: None,
    //             rate_slope_multiplier: None,
    //             debt_minimum: None,
    //             dex_router: None,
    //             discounts_contract: None,
    //             base_debt_cap_multiplier: None,
    //             oracle_contract: None,
    //             oracle_time_limit: None,
    //             osmosis_proxy: None,
    //             liq_fee: None,
    //             liquidity_contract: None,
    //             collateral_twap_timeframe: None,
    //             cpc_multiplier: None,
    //             credit_twap_timeframe: None,
    //         }))?,
    //         funds: vec![],
    //     })
    // );
    //Update Auction
    // msgs.push(
    //     CosmosMsg::Wasm(WasmMsg::Execute {
    //         contract_addr: addresses.mbrn_auction.to_string(),
    //         msg: to_binary(&DAExecuteMsg::UpdateConfig(AuctionUpdateConfig {
    //             owner: Some(gov_contract.clone()),
    //             governance_contract: Some(gov_contract.clone()),
    //             staking_contract: None,
    //             oracle_contract: None,
    //             osmosis_proxy: None,
    //             positions_contract: None,
    //             mbrn_denom: None,
    //             cdt_denom: None,
    //             desired_asset: None,
    //             twap_timeframe: None,
    //             initial_discount: None,
    //             discount_increase_timeframe: None,
    //             discount_increase: None,
    //             send_to_stakers: None,

    //         }))?,
    //         funds: vec![],
    //     })
    // );
    //Update Discount Vault
    // msgs.push(
    //     CosmosMsg::Wasm(WasmMsg::Execute {
    //         contract_addr: addresses.discount_vault.to_string(),
    //         msg: to_binary(&DiscountVaultExecuteMsg::ChangeOwner { owner: gov_contract.clone() })?,
    //         funds: vec![],
    //     })
    // );
    //Update Liquidation Queue
    // msgs.push(
    //     CosmosMsg::Wasm(WasmMsg::Execute {
    //         contract_addr: addresses.liq_queue.to_string(),
    //         msg: to_binary(&LQExecuteMsg::UpdateConfig { 
    //             owner: Some(gov_contract.clone()),
    //             positions_contract:  None,
    //             osmosis_proxy_contract:  None,
    //             waiting_period:  None, 
    //             minimum_bid:  None, 
    //             maximum_waiting_bids: None
    //         })?,
    //         funds: vec![],
    //     })
    // );
    //Update Liquidity Check
    // msgs.push(
    //     CosmosMsg::Wasm(WasmMsg::Execute {
    //         contract_addr: addresses.liquidity_check.to_string(),
    //         msg: to_binary(&LCExecuteMsg::UpdateConfig { 
    //             owner: Some(gov_contract.clone()),
    //             positions_contract:  None,
    //             osmosis_proxy:  None,
    //             stableswap_multiplier:  None,
    //         })?,
    //         funds: vec![],
    //     })
    // );
    //Update Oracle
    // msgs.push(
    //     CosmosMsg::Wasm(WasmMsg::Execute {
    //         contract_addr: addresses.oracle.to_string(),
    //         msg: to_binary(&OracleExecuteMsg::UpdateConfig { 
    //             owner: Some(gov_contract.clone()),
    //             positions_contract:  None,
    //             osmosis_proxy_contract: None,
    //             osmo_usd_pyth_feed_id: None,
    //             pyth_osmosis_address: None,
    //             pools_for_usd_par_twap: None,
    //         })?,
    //         funds: vec![],
    //     })
    // );
    //Update Osmosis Proxy 
    msgs.push(
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: addresses.osmosis_proxy.to_string(),
            msg: to_binary(&OPExecuteMsg::UpdateConfig { 
                owners: Some(vec![
                    Owner {
                        owner: deps.api.addr_validate(&gov_contract.clone())?,
                        total_minted: Uint128::zero(), 
                        stability_pool_ratio: None,
                        non_token_contract_auth: true,
                        is_position_contract: false,
                    }
                ]),
                positions_contract:  None,
                add_owner: Some(true),
                liquidity_multiplier: None,
                debt_auction: None,
                liquidity_contract: None,
                oracle_contract: None,
                edit_routes: None,
            })?,
            funds: vec![],
        })
    );
    //Update Staking
    // msgs.push(
    //     CosmosMsg::Wasm(WasmMsg::Execute {
    //         contract_addr: addresses.staking.to_string(),
    //         msg: to_binary(&StakingExecuteMsg::UpdateConfig { 
    //             owner: Some(gov_contract.clone()),
    //             governance_contract: Some(gov_contract.clone()),
    //             positions_contract:  None,
    //             auction_contract: None,
    //             vesting_contract: None,
    //             osmosis_proxy: None,
    //             mbrn_denom: None,
    //             incentive_schedule: None,
    //             unstaking_period: None,
    //             max_commission_rate: None,
    //             keep_raw_cdt: None,
    //             vesting_rev_multiplier: None,
    //         })?,
    //         funds: vec![],
    //     })
    // );

    
    Ok(Response::new().add_messages(msgs))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Lockdrop {} => to_binary(&LOCKDROP.load(deps.storage)?),
        QueryMsg::ContractAddresses {} => to_binary(&ADDRESSES.load(deps.storage)?),
        QueryMsg::IncentiveDistribution {} => to_binary(&get_incentive_ratios(deps.storage)?),
        QueryMsg::UserIncentives { user } => to_binary(&calc_user_incentives(deps.storage, user)?),
        QueryMsg::UserInfo { user } => to_binary(&LOCKED_USERS.load(deps.storage, deps.api.addr_validate(&user)?)?),
    }
}

///Get incentive ratios
fn get_incentive_ratios(
    storage: &dyn Storage,
) -> StdResult<Vec<UserRatio>>{
    let mut user_ratios = INCENTIVE_RATIOS.load(storage)?;

    if user_ratios.is_empty(){
        user_ratios = calc_ticket_distribution_imut(storage)?;
    }

    Ok(user_ratios)
}

/// Calculate and return user incentives
fn calc_user_incentives(
    storage: &dyn Storage,
    user: String,
) -> StdResult<Uint128>{
    let mut user_ratios = INCENTIVE_RATIOS.load(storage)?;
    let lockdrop = LOCKDROP.load(storage)?;

    if user_ratios.is_empty(){
        user_ratios = calc_ticket_distribution_imut(storage)?;
    }
    
    //Calc any unlocked incentives
    let incentives = get_user_incentives(
        user_ratios,
        user,
        lockdrop.num_of_incentives,
    )?;

    Ok(incentives)
}


/// Calculate the ratio of incentives each user is entitled to
fn calc_ticket_distribution_imut(
    storage: &dyn Storage,
) -> StdResult<Vec<UserRatio>>{
    let mut error: Option<StdError> = None;

    let user_totals = LOCKED_USERS
        .range(storage, None, None, cosmwasm_std::Order::Ascending)
        .into_iter()
        .map(|item| {
            let (_, locked_user) = match item {
                Ok(locked_user) => locked_user,
                Err(err) => {
                    error = Some(err);
                    return (Addr::unchecked(""), Uint128::zero());
                }
            };

            let total_tickets: Uint128 = locked_user.deposits
                .into_iter()
                .map(|deposit| deposit.deposit * Uint128::from(deposit.lock_up_duration + 1) )
                .collect::<Vec<Uint128>>()
                .into_iter()
                .sum();

            (locked_user.user, total_tickets)
        })
        .collect::<Vec<(Addr, Uint128)>>();

    let total_tickets: Uint128 = user_totals.clone()
        .into_iter()
        .map(|user| user.1)
        .collect::<Vec<Uint128>>()
        .into_iter()
        .sum();

    let user_ratios: Vec<UserRatio> = user_totals.clone()
        .into_iter()
        .map(|user| {
            let ratio = decimal_division(
                Decimal::from_ratio(user.1, Uint128::one()),
                Decimal::from_ratio(total_tickets, Uint128::one()),
            ).unwrap_or_else(|e| {
                error = Some(e);
                Decimal::zero()
            });

            UserRatio { user: user.0, ratio }
        })
        .collect::<Vec<UserRatio>>();

    if let Some(e) = error {
        return Err(e)
    }

    Ok(user_ratios)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.id {
        OSMOSIS_PROXY_REPLY_ID => handle_op_reply(deps, env, msg),
        ORACLE_REPLY_ID => handle_oracle_reply(deps, env, msg),
        STAKING_REPLY_ID => handle_staking_reply(deps, env, msg),
        VESTING_REPLY_ID => handle_vesting_reply(deps, env, msg),
        GOVERNANCE_REPLY_ID => handle_gov_reply(deps, env, msg),
        POSITIONS_REPLY_ID => handle_cdp_reply(deps, env, msg),
        STABILITY_POOL_REPLY_ID => handle_sp_reply(deps, env, msg),
        LIQ_QUEUE_REPLY_ID => handle_lq_reply(deps, env, msg),
        LIQUIDITY_CHECK_REPLY_ID => handle_lc_reply(deps, env, msg),
        DEBT_AUCTION_REPLY_ID => handle_auction_reply(deps, env, msg),
        CREATE_DENOM_REPLY_ID => handle_create_denom_reply(deps, env, msg),
        SYSTEM_DISCOUNTS_REPLY_ID => handle_system_discounts_reply(deps, env, msg),
        DISCOUNT_VAULT_REPLY_ID => handle_discount_vault_reply(deps, env, msg),
        BALANCER_POOL_REPLY_ID => handle_balancer_reply(deps, env, msg),
        NO_ACTION_ID => Ok(Response::new()),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
}

/// This gets called at the end of the lockdrop.
/// Create MBRN & CDT pools and deposit into MBRN/OSMO pool.
pub fn end_of_launch(
    deps: DepsMut,
    env: Env,
) -> Result<Response, ContractError>{
    let mut lockdrop = LOCKDROP.load(deps.storage)?;

    //Assert launch hasn't happened yet, don't want this called twice
    if lockdrop.launched { return Err(ContractError::LaunchHappened {  }) }
    
    //Toggle launched and save
    lockdrop.launched = true;
    LOCKDROP.save(deps.storage, &lockdrop)?;

    //Assert Lockdrop withdraw period has ended
    if !(env.block.time.seconds() > lockdrop.withdrawal_end) { return Err(ContractError::LockdropOngoing {  }) }

    let config = CONFIG.load(deps.storage)?;
    let addrs = ADDRESSES.load(deps.storage)?;
    let mut sub_msgs: Vec<SubMsg> = vec![];

    //Get uosmo contract balance
    let uosmo_balance = get_contract_balances(deps.querier, env.clone(), vec![AssetInfo::NativeToken { denom: String::from("uosmo") }])?[0];
    //Make sure to deduct the amount of OSMO used to create Pools. Contract balance - 1000uosmo * 2 pools - 1 OSMO to init CDT LP - 50 OSMO to create a gauge
    let uosmo_pool_delegation_amount = (uosmo_balance - Uint128::new(2051_000_000)).to_string(); 
    
    //Mint MBRN for LP
    let msg = OPExecuteMsg::MintTokens { 
        denom: config.clone().mbrn_denom, 
        amount: Uint128::new(1_000_000_000_000), //1M mbrn
        mint_to_address: env.clone().contract.address.to_string(),
    };
    let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
        contract_addr: addrs.clone().osmosis_proxy.to_string(), 
        msg: to_binary(&msg)?, 
        funds: vec![],
    });
    sub_msgs.push(SubMsg::new(msg));
    
    //Create & deposit into MBRN-OSMO LP 
    let msg = MsgCreateBalancerPool {
        sender: env.contract.address.to_string(),
        pool_params: Some(PoolParams {
            swap_fee: String::from("002000000000000000"), //0.2% in sdk.Dec 18 places
            exit_fee: String::from("0"),
            smooth_weight_change_params: None,
        }),
        pool_assets: vec![
            PoolAsset { 
                token: Some(Coin { denom: config.clone().mbrn_denom, amount: "1_000_000_000_000".to_string() }), 
                weight: String::from("50") 
            },
            PoolAsset { 
                token: Some(Coin { denom: config.clone().osmo_denom, amount: uosmo_pool_delegation_amount }), 
                weight: String::from("50") 
            }
        ],
        future_pool_governor: addrs.clone().osmosis_proxy.to_string(),
    };
    let sub_msg = SubMsg::reply_on_success(msg, BALANCER_POOL_REPLY_ID);
    sub_msgs.push(sub_msg);

    //Mint 1 CDT for LP
    let msg = OPExecuteMsg::MintTokens { 
        denom: config.clone().credit_denom, 
        amount: Uint128::new(1_000_000), 
        mint_to_address: env.clone().contract.address.to_string(),
    };
    let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
        contract_addr: addrs.clone().osmosis_proxy.to_string(), 
        msg: to_binary(&msg)?, 
        funds: vec![], 
    });
    sub_msgs.push(SubMsg::new(msg));

    //Create OSMO CDT pool
    let msg: CosmosMsg = MsgCreateBalancerPool {
        sender: env.contract.address.to_string(),
        pool_params: Some(PoolParams {
            swap_fee: String::from("002000000000000000"), //0.2% in sdk.Dec 18 places
            exit_fee: String::from("0"),
            smooth_weight_change_params: None,
        }),
        pool_assets: vec![
            PoolAsset { 
                token: Some(Coin { denom: config.clone().credit_denom, amount: "1_000_000".to_string() }), 
                weight: String::from("50") 
            },
            PoolAsset { 
                token: Some(Coin { denom: config.clone().osmo_denom, amount: "1_000_000".to_string() }), 
                weight: String::from("50") 
            }
        ],
        future_pool_governor: addrs.clone().osmosis_proxy.to_string(),
    }.into();
    let sub_msg = SubMsg::reply_on_success(msg, BALANCER_POOL_REPLY_ID);
    sub_msgs.push(sub_msg);


    //Set liquidity_multiplier
    let msg = OPExecuteMsg::UpdateConfig { 
        owners: None, 
        add_owner: None,
        liquidity_multiplier: Some(Decimal::percent(5_00)), //5x or 20% liquidity to supply ratio
        debt_auction: None,
        positions_contract: None,
        liquidity_contract: None,
        oracle_contract: None,
        edit_routes: None,
    };
    let config_msg = CosmosMsg::Wasm(WasmMsg::Execute { 
        contract_addr: addrs.clone().osmosis_proxy.to_string(), 
        msg: to_binary(&msg)?, 
        funds: vec![], 
    });

    Ok(Response::new()
        .add_submessages(sub_msgs)
        .add_message(config_msg)
    )
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    
    let vesting = ADDRESSES.load(deps.storage)?.vesting;

    //Add recipients to vesting contract
    let pre_launch_community =  vec![
        "osmo1ssgz49n7a6uc0xvxwmx59t60mgktnk238m5yf0",
        "osmo1d9ryqp7yfmr92vkk2yal96824pewf2g5wx0h2r",
        "osmo10k0qxqh39fwk2khv49g0n673kfqwe778f09ffg",
        "osmo1ks3p0rtxkph9us3rg5z3as2hl4gfq4fua5d0vh",
        "osmo1xaaxw8xzyxh6cflqd4g2xqmypsa4x8hzl8t4nc",
        "osmo1dxrfdpxju5jmcqz3lffn5qj4kqvtpuvjdet49z",
        "osmo1w7tql40ad8m4hzw65ahlcgd72vm4zwfq6uw68j",
        "osmo1t7f0rmcjxvmdmftsfsy66k2yny9zq703hwa9qg",
        "osmo1vg5gzyfqfnqz7l6hqh307q9mczjrns266a2q06",
        "osmo1pgsya6vgfr0rzxuc24wn5t3x9azy6r83mumygz"
    ];
    let founder = "osmo1988s5h45qwkaqch8km4ceagw2e08vdw28mwk4n";
    
    let mut msgs: Vec<CosmosMsg> = vec![];
    //Loop through pre_launch_community to create add_recipient messages
    for recipient in pre_launch_community.clone() {
        let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
            contract_addr: vesting.to_string(), 
            msg: to_binary(&VestingExecuteMsg::AddRecipient { 
                recipient: recipient.to_string(),
            })?, 
            funds: vec![], 
        });
        msgs.push(msg);
    }
    //Add founder recipient
    let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
        contract_addr: vesting.to_string(), 
        msg: to_binary(&VestingExecuteMsg::AddRecipient { 
            recipient: founder.to_string(),
        })?, 
        funds: vec![], 
    });
    msgs.push(msg);

    //////Allocations
    //Subtract 99 days from first vesting date.
    //PLC: 0 cliff 266 linear 
    //Founder: 631 cliff 365 linear

    //Loop through pre_launch_community to create add_allocation messages
    for recipient in pre_launch_community {
        let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
            contract_addr: vesting.to_string(), 
            msg: to_binary(&VestingExecuteMsg::AddAllocation { 
                recipient: recipient.to_string(),
                allocation: Uint128::new(1_000_000_000u128),
                vesting_period: Some(VestingPeriod {
                    cliff: 0,
                    linear: 266,
                })
            })?, 
            funds: vec![], 
        });
        msgs.push(msg);
    }
    //Set founder allocation
    let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
        contract_addr: vesting.to_string(), 
        msg: to_binary(&VestingExecuteMsg::AddAllocation { 
            recipient: founder.to_string(),
            allocation: Uint128::new(9_000_000_000_000u128), //9M
            vesting_period: Some(VestingPeriod {
                cliff: 631,
                linear: 365,
            })
        })?, 
        funds: vec![], 
    });
    msgs.push(msg);

    Ok(Response::new().add_messages(msgs))
}