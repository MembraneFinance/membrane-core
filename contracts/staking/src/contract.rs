#[cfg(not(feature = "library"))]
use std::env;
use std::cmp::min;

use cosmwasm_std::{entry_point, Coin};
use cosmwasm_std::{
    attr, coin, to_binary, Addr, Api, BankMsg, Binary, CosmosMsg, Decimal, Deps,
    DepsMut, Env, MessageInfo, Response, StdError, StdResult, Storage, Uint128, WasmMsg, QueryRequest, WasmQuery, QuerierWrapper,
};
use cw2::set_contract_version;

use membrane::governance::{QueryMsg as Gov_QueryMsg, ProposalListResponse, ProposalStatus};
use membrane::helpers::{assert_sent_native_token_balance, validate_position_owner, asset_to_coin, query_basket};
use membrane::osmosis_proxy::ExecuteMsg as OsmoExecuteMsg;
use membrane::auction::ExecuteMsg as AuctionExecuteMsg;
use membrane::staking::{ Config, ExecuteMsg, InstantiateMsg, QueryMsg, Totals, MigrateMsg};
use membrane::vesting::{QueryMsg as Vesting_QueryMsg, RecipientsResponse};
use membrane::types::{Asset, AssetInfo, Basket, Delegate, Delegation, DelegationInfo, FeeEvent, LiqAsset, StakeDeposit, StakeDistribution, StakeDistributionLog};
use membrane::math::{decimal_division, decimal_multiplication};

use crate::error::ContractError;
use crate::query::{query_declared_delegates, query_delegations, query_fee_events, query_staked, query_totals, query_user_rewards, query_user_stake};
use crate::state::{CONFIG, DELEGATE_CLAIMS, DELEGATE_INFO, DELEGATIONS, FEE_EVENTS, INCENTIVE_SCHEDULING, OWNERSHIP_TRANSFER, STAKED, STAKING_TOTALS, VESTING_REV_MULTIPLIER, VESTING_STAKE_TIME};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:staking";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

//Constants
const SECONDS_PER_YEAR: u64 = 31_536_000u64;
pub const SECONDS_PER_DAY: u64 = 86_400u64;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let mut config: Config;

    if msg.owner.is_some() {
        config = Config {
            owner: deps.api.addr_validate(&msg.owner.unwrap())?,
            positions_contract: None,
            auction_contract: None,
            vesting_contract: None,
            governance_contract: None,
            osmosis_proxy: None,
            incentive_schedule: msg.incentive_schedule.unwrap_or_else(|| StakeDistribution {
                rate: Decimal::percent(9),
                duration: 240,
            }),
            unstaking_period: msg.unstaking_period.unwrap_or(4u64),
            max_commission_rate: Decimal::percent(10),
            keep_raw_cdt: true,
            vesting_rev_multiplier: Decimal::percent(20),
            mbrn_denom: msg.mbrn_denom,
        };
    } else {
        config = Config {
            owner: info.sender,
            positions_contract: None,
            auction_contract: None,
            vesting_contract: None,
            governance_contract: None,
            osmosis_proxy: None,
            incentive_schedule: msg.incentive_schedule.unwrap_or_else(|| StakeDistribution {
                rate: Decimal::percent(9),
                duration: 240,
            }),
            unstaking_period: msg.unstaking_period.unwrap_or(4u64),
            max_commission_rate: Decimal::percent(10),
            keep_raw_cdt: true,
            vesting_rev_multiplier: Decimal::percent(20),
            mbrn_denom: msg.mbrn_denom,
        };
    }

    //Set optional config parameters
    if let Some(vesting_contract) = msg.vesting_contract {        
        config.vesting_contract = Some(deps.api.addr_validate(&vesting_contract)?);
    };
    if let Some(positions_contract) = msg.positions_contract {
        config.positions_contract = Some(deps.api.addr_validate(&positions_contract)?);
    };
    if let Some(auction_contract) = msg.auction_contract {
        config.auction_contract = Some(deps.api.addr_validate(&auction_contract)?);
    };
    if let Some(governance_contract) = msg.governance_contract {
        config.governance_contract = Some(deps.api.addr_validate(&governance_contract)?);
    };
    if let Some(osmosis_proxy) = msg.osmosis_proxy {
        config.osmosis_proxy = Some(deps.api.addr_validate(&osmosis_proxy)?);
    };

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(deps.storage, &config)?;

    //Initialize stake Totals
    STAKING_TOTALS.save(
        deps.storage,
        &Totals {
            stakers: Uint128::zero(),
            vesting_contract: Uint128::zero(),
        },
    )?;
    //Initialize fee events
    FEE_EVENTS.save(deps.storage, &vec![])?;

    //Initialize Vesting stake time
    VESTING_STAKE_TIME.save(deps.storage, &env.block.time.seconds())?;

    //Initialize INCENTIVE_SCHEDULING
    INCENTIVE_SCHEDULING.save(deps.storage, &StakeDistributionLog {
        ownership_distribution: config.clone().incentive_schedule,
        start_time: env.block.time.seconds(),
    })?;

    //Initialize Delegate state
    DELEGATE_INFO.save(deps.storage, &vec![])?;
    
    Ok(Response::new()
        .add_attribute("method", "instantiate")
        .add_attribute("config", format!("{:?}", config))
        .add_attribute("contract_address", env.contract.address))
}

/// Return total MBRN vesting
pub fn get_total_vesting(
    querier: QuerierWrapper,    
    vesting_contract: String,
) -> StdResult<Uint128>{
    let recipients = querier.query::<RecipientsResponse>(&QueryRequest::Wasm(WasmQuery::Smart { 
        contract_addr: vesting_contract, 
        msg: to_binary(&Vesting_QueryMsg::Recipients { })?
    }))?;    

    Ok(recipients.get_total_vesting())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateConfig {
            owner,
            mbrn_denom,
            vesting_contract,
            governance_contract,
            osmosis_proxy,
            positions_contract,
            auction_contract,
            incentive_schedule,
            unstaking_period,
            max_commission_rate,
            keep_raw_cdt,
            vesting_rev_multiplier,
        } => update_config(
            deps,
            info,
            env,
            owner,
            positions_contract,
            auction_contract,
            vesting_contract,
            governance_contract,
            osmosis_proxy,
            mbrn_denom,
            incentive_schedule,
            unstaking_period,
            max_commission_rate,
            keep_raw_cdt,            
            vesting_rev_multiplier,
        ),
        ExecuteMsg::Stake { user } => stake(deps, env, info, user),
        ExecuteMsg::Unstake { mbrn_amount } => unstake(deps, env, info, mbrn_amount),
        ExecuteMsg::UpdateDelegations { governator_addr, mbrn_amount, delegate, fluid, voting_power_delegation, commission } => update_delegations(
            deps,
            env,
            info,
            governator_addr,
            mbrn_amount,
            fluid,
            delegate,
            commission,
            voting_power_delegation,
        ),
        ExecuteMsg::DelegateFluidDelegations { governator_addr, mbrn_amount } => delegate_fluid_delegations(
            deps,
            env,
            info,
            governator_addr,
            mbrn_amount,
        ),
        ExecuteMsg::DeclareDelegate { delegate_info, remove } => delegate_declarations(
            deps,
            info,
            env,
            delegate_info,
            remove,
        ),
        ExecuteMsg::Restake { mbrn_amount } => restake(deps, env, info, mbrn_amount),
        ExecuteMsg::ClaimRewards {
            send_to,
            restake,
        } => claim_rewards(
            deps,
            env,
            info,
            send_to,
            restake,
        ),
        ExecuteMsg::DepositFee {} => {
            let config = CONFIG.load(deps.storage)?;

            if info.sender != config.positions_contract.unwrap() && info.sender != config.auction_contract.unwrap(){
                return Err(ContractError::Unauthorized {});
            }

            //Take fee_assets from sent_assets
            let fee_assets = {
                info.clone()
                    .funds
                    .into_iter()
                    .map(|coin| Asset {
                        info: AssetInfo::NativeToken { denom: coin.denom },
                        amount: coin.amount,
                    })
                    .collect::<Vec<Asset>>()
            };

            deposit_fee(deps, info, env, fee_assets)
        },
        ExecuteMsg::TrimFeeEvents {  } => trim_fee_events(deps.storage, info),
    }
}

/// Update delegate declarations
fn delegate_declarations(
    deps: DepsMut,
    info: MessageInfo,
    _env: Env,
    delegate_info: Delegate,
    remove: bool
) -> Result<Response, ContractError> {

    //Validate Delegate address
    deps.api.addr_validate(&delegate_info.delegate.to_string())?;
    
    //Sender must be the delegate
    if info.sender != delegate_info.delegate {
        return Err(ContractError::CustomError {
            val: String::from("Sender must be the delegate"),
        });
    }

    if remove {
        //Remove delegate from DELEGATE_INFO
        DELEGATE_INFO.update(deps.storage, |mut delegate_infos| -> StdResult<_> {
            delegate_infos.retain(|delegate| delegate.delegate != info.sender);
            Ok(delegate_infos)
        })?;
    } else {
        //Update delegate declarations
        DELEGATE_INFO.update(deps.storage, |mut delegate_infos| -> StdResult<_> {
            if let Some ((index, mut delegate)) = delegate_infos.clone().into_iter().enumerate().find(|(_, delegate)| delegate.delegate == info.sender) {
                //Update any fields that are not None
                if let Some (discord_username) = delegate_info.clone().discord_username {
                    delegate.discord_username = Some(discord_username);
                }
                if let Some (twitter_username) = delegate_info.clone().twitter_username {
                    delegate.twitter_username = Some(twitter_username);
                }
                if let Some (url) = delegate_info.clone().url {
                    delegate.url = Some(url);
                }
                if let Some (alias) = delegate_info.clone().alias {
                    delegate.alias = Some(alias);
                }
                //Save updated delegate
                delegate_infos[index] = delegate;
            } else {
                //If delegate doesn't exist in the DELEGATE_INFO, add it
                delegate_infos.push(delegate_info.clone());
            }
            Ok(delegate_infos)
        })?;
    }

    Ok(Response::new()
        .add_attribute("method", "delegate_declarations")
        .add_attribute("delegate", format!("{:?}", delegate_info))
    )

}

/// Update contract configuration
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    owner: Option<String>,
    positions_contract: Option<String>,
    auction_contract: Option<String>,
    vesting_contract: Option<String>,
    governance_contract: Option<String>,
    osmosis_proxy: Option<String>,
    mbrn_denom: Option<String>,
    incentive_schedule: Option<StakeDistribution>,
    unstaking_period: Option<u64>,
    max_commission_rate: Option<Decimal>,
    keep_raw_cdt: Option<bool>,
    vesting_rev_multiplier: Option<Decimal>,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    //Assert Authority
    if info.sender != config.owner {
        //Check if ownership transfer is in progress & transfer if so
        if info.sender == OWNERSHIP_TRANSFER.load(deps.storage)? {
            config.owner = info.sender;
        } else {
            return Err(ContractError::Unauthorized {});
        }
    }

    let mut attrs = vec![attr("method", "update_config")];

    //Match Optionals
    if let Some(owner) = owner {
        let valid_addr = deps.api.addr_validate(&owner)?;

        //Set owner transfer state
        OWNERSHIP_TRANSFER.save(deps.storage, &valid_addr)?;
        attrs.push(attr("owner_transfer", valid_addr));     
    };
    if let Some(incentive_schedule) = incentive_schedule {
        //Update incentive schedule
        config.incentive_schedule = incentive_schedule.clone();

        //Set Scheduling
        INCENTIVE_SCHEDULING.save(deps.storage, 
            &StakeDistributionLog { 
                ownership_distribution: incentive_schedule, 
                start_time: env.block.time.seconds(),
        })?;
    };
    if let Some(unstaking_period) = unstaking_period {
        config.unstaking_period = unstaking_period;
    };
    if let Some(max_commission_rate) = max_commission_rate {
        config.max_commission_rate = max_commission_rate;
    };
    if let Some(keep_raw_cdt) = keep_raw_cdt {
        config.keep_raw_cdt = keep_raw_cdt;
    };
    if let Some(vesting_rev_multiplier) = vesting_rev_multiplier {
        //Set vesting_rev_multiplier's state object
        //Config is only updated once vesting contract has claimed rewards
        VESTING_REV_MULTIPLIER.save(deps.storage, &vesting_rev_multiplier)?;
    };
    if let Some(mbrn_denom) = mbrn_denom {
        config.mbrn_denom = mbrn_denom.clone();
    };
    if let Some(vesting_contract) = vesting_contract {
        config.vesting_contract = Some(deps.api.addr_validate(&vesting_contract)?);
    };
    if let Some(positions_contract) = positions_contract {
        config.positions_contract = Some(deps.api.addr_validate(&positions_contract)?);
    };
    if let Some(auction_contract) = auction_contract {
        config.auction_contract = Some(deps.api.addr_validate(&auction_contract)?);
    };
    if let Some(governance_contract) = governance_contract {
        config.governance_contract = Some(deps.api.addr_validate(&governance_contract)?);
    };
    if let Some(osmosis_proxy) = osmosis_proxy {
        config.osmosis_proxy = Some(deps.api.addr_validate(&osmosis_proxy)?);
    };

    //Save new Config
    CONFIG.save(deps.storage, &config)?;
    attrs.push(attr("updated_config", format!("{:?}", config)));
    
    Ok(Response::new().add_attributes(attrs))
}

/// Stake MBRN
pub fn stake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    user: Option<String>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    let valid_asset: Asset;
    //Assert only MBRN was sent && its at least 1 MBRN
    if info.funds.len() == 1 && info.funds[0].denom == config.mbrn_denom {
        //The contract can stake less than 1 MBRN, but the user must stake at least 1 MBRN
        if info.clone().sender != env.contract.address && info.funds[0].amount < Uint128::from(1_000_000u128) {
            return Err(ContractError::CustomError {
                val: "Must stake at least 1 MBRN".to_string(),
            });
        }

        valid_asset = assert_sent_native_token_balance(
            AssetInfo::NativeToken {
                denom: config.clone().mbrn_denom,
            },
            &info,
        )?;
    } else {
        return Err(ContractError::CustomError {
            val: "No valid assets".to_string(),
        });
    }

    //Set valid address
    let valid_owner_addr = validate_position_owner(deps.api, info.clone(), user)?;

    add_staking_deposit(
        deps.storage, 
        env,
        config,
        valid_owner_addr.clone(),
        valid_asset.amount
    )?;

    //Response build
    let response = Response::new();
    let attrs = vec![
        attr("method", "stake"),
        attr("staker", valid_owner_addr.to_string()),
        attr("amount", valid_asset.amount.to_string()),
    ];

    Ok(response.add_attributes(attrs))
}

/// Add new staking deposit to user
fn add_staking_deposit(
    storage: &mut dyn Storage,
    env: Env,
    config: Config,
    staker: Addr,
    amount: Uint128,
) -> StdResult<()>{
    //Add new deposit to staker's list of StakeDeposits
    STAKED.update(storage, staker.clone(), |current_deposits| -> StdResult<_> {
        match current_deposits {
            Some(mut deposits) => {
                deposits.push(StakeDeposit {
                    staker: staker.clone(),
                    amount,
                    stake_time: env.block.time.seconds(),
                    unstake_start_time: None,
                    last_accrued: None,
                });
                Ok(deposits)
            }
            None => {
                let mut deposits = vec![];
                deposits.push(StakeDeposit {
                    staker: staker.clone(),
                    amount,
                    stake_time: env.block.time.seconds(),
                    unstake_start_time: None,
                    last_accrued: None,
                });
                Ok(deposits)
            }
        }
    })?;

    //Add to Totals
    let mut totals = STAKING_TOTALS.load(storage)?;
    if let Some(vesting_contract) = config.clone().vesting_contract{
        if staker == vesting_contract {
            totals.vesting_contract += amount;
        } else {
            totals.stakers += amount;
        }
    } else {
        totals.stakers += amount;
    }
    STAKING_TOTALS.save(storage, &totals)?;    

    Ok(())
}

/// First call is an unstake
/// 2nd call after unstake period is a withdrawal
pub fn unstake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mbrn_withdraw_amount: Option<Uint128>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    //Restrict unstaking
    can_this_addr_unstake(deps.querier, info.clone().sender, config.clone())?;

    //Get total Stake
    let total_stake = {
        let staker_deposits: Vec<StakeDeposit> = match STAKED.load(deps.storage, info.sender.clone()){
            Ok(deposits) => deposits,
            Err(_) => return Err(ContractError::CustomError {
                val: String::from("User has no stake"),
            }),
        };

        let total_staker_deposits: Uint128 = staker_deposits
            .into_iter()
            .map(|deposit| deposit.amount)
            .collect::<Vec<Uint128>>()
            .into_iter()
            .sum();

        total_staker_deposits
    };

    //Enforce valid withdraw amount
    let mut withdraw_amount = mbrn_withdraw_amount.unwrap_or(total_stake).min(total_stake);

    //info.sender is user
    let (claimables, accrued_interest, withdrawable_amount) = withdraw_from_state(
        deps.storage,
        env.clone(),
        info.clone().sender,
        withdraw_amount,
    )?;

    /////withdraw_from_state() check
    //Load state and make sure withdrawable_amount + new_total_stake == total_stake
    let new_total_staked = {
        let staker_deposits: Vec<StakeDeposit> = match STAKED.load(deps.storage, info.sender.clone()){
            Ok(deposits) => deposits,
            Err(_) => return Err(ContractError::CustomError {
                val: String::from("User has no stake"),
            }),
        };

        let total_staker_deposits: Uint128 = staker_deposits
            .into_iter()
            .map(|deposit| deposit.amount)
            .collect::<Vec<Uint128>>()
            .into_iter()
            .sum();

        total_staker_deposits
    };
    //if withdrawable_amount is greater than total stake or there is a stake discrepancy, error
    if withdrawable_amount > total_stake || withdrawable_amount + new_total_staked != total_stake {
        return Err(ContractError::CustomError {
            val: format!("Invalid withdrawable amount: {}", withdrawable_amount),
        });
    }

    //Initialize variables
    let mut native_claims = vec![];

    //If user can withdraw, add to native_claims & update delegations
    if !withdrawable_amount.is_zero() {
        //Push to native claims list
        native_claims.push(asset_to_coin(Asset {
            info: AssetInfo::NativeToken {
                denom: config.clone().mbrn_denom,
            },
            amount: withdrawable_amount,
        })?);     

        //Get user's delegation info
        if let Ok(mut staker_delegation_info) = DELEGATIONS.load(deps.storage, info.sender.clone()){
            //Save indices to remove 
            let mut remove_indices = vec![];
            //Get user's delegated stake
            let total_delegations: Uint128 = staker_delegation_info.clone()
                .delegated_to
                .into_iter()
                .map(|delegation| delegation.amount)
                .collect::<Vec<Uint128>>()
                .into_iter()
                .sum();

            //If withdrawing more than is undelegated, undelegate the excess
            let undelegated_stake = match total_stake.checked_sub(total_delegations){
                Ok(undelegated) => undelegated,
                Err(_) => Uint128::zero(),
            };
            if withdrawable_amount > undelegated_stake {
                let mut undelegate_amount = match withdrawable_amount.checked_sub(undelegated_stake){
                    Ok(undelegated) => undelegated,
                    Err(_) => return Err(ContractError::CustomError {
                        val: format!("Undelegated stake ({}) is some how greater than withdrawable amount ({})", withdrawable_amount, undelegated_stake),
                    }),
                };
                for (i, delegation) in staker_delegation_info.clone().delegated_to.into_iter().enumerate() {
                    
                    //If undelegate amount is greater than the current delegation, undelegate the whole delegation & update undelegate amount
                    if undelegate_amount >= delegation.amount && !undelegate_amount.is_zero() {
                        undelegate_amount -= delegation.amount;
                        
                        //Remove staker delegation
                        remove_indices.push(i);

                        //Remove delegate delegation
                        let mut delegate_delegation_info = DELEGATIONS.load(deps.storage, delegation.delegate.clone())?;
                        for (i, delegate_delegation) in delegate_delegation_info.clone().delegated.into_iter().enumerate() {
                            if delegate_delegation.delegate == info.sender.clone() {
                                delegate_delegation_info.delegated.remove(i);
                                break;
                            }
                        }
                        DELEGATIONS.save(deps.storage, delegation.delegate.clone(), &delegate_delegation_info)?;
                    } else if staker_delegation_info.delegated_to[i].amount > undelegate_amount && !undelegate_amount.is_zero(){
                        //If undelegate amount is less than the current delegation, undelegate the undelegate amount & break
                        staker_delegation_info.delegated_to[i].amount -= undelegate_amount;

                        //Update delegate delegation
                        let mut delegate_delegation_info = DELEGATIONS.load(deps.storage, delegation.delegate.clone())?;
                        for (i, delegate_delegation) in delegate_delegation_info.clone().delegated.into_iter().enumerate() {
                            if delegate_delegation.delegate == info.sender.clone() {
                                delegate_delegation_info.delegated[i].amount = match delegate_delegation_info.delegated[i].amount.checked_sub(undelegate_amount){
                                    Ok(diff) => diff,
                                    Err(_) => {
                                        undelegate_amount -= delegate_delegation_info.delegated[i].amount;
                                        
                                        Uint128::zero()
                                    },
                                };
                                break;
                            }
                        }
                        DELEGATIONS.save(deps.storage, delegation.delegate.clone(), &delegate_delegation_info)?;
                        break;
                    }
                }
                //Remove indices now that we've iterated through the delegations
                for i in remove_indices.into_iter(){
                    staker_delegation_info.delegated_to.remove(i);
                }

                //Save updated delegation info
                DELEGATIONS.save(deps.storage, info.sender.clone(), &staker_delegation_info)?;
            }
        };
        
    }

    //Create claimable msgs
    let claims_msgs = create_rewards_msgs(
        deps.storage,
        deps.querier,
        deps.api,
        env,
        config.clone(), 
        claimables.clone(), 
        accrued_interest.clone(),
        info.clone().sender.to_string(),
        native_claims,
    )?;

    //Update Totals
    //We update with the difference between the withdraw_amount and the withdrawable amount bc whatever isn't withdrawable was newly unstaked
    let mut totals = STAKING_TOTALS.load(deps.storage)?;
    //Set withdraw_amount to newly unstaked amount
    if withdrawable_amount >= withdraw_amount {
        withdraw_amount = Uint128::zero();
    } else {
        withdraw_amount -= withdrawable_amount;
    }
    if let Some(vesting_contract) = config.clone().vesting_contract{
        if info.clone().sender == vesting_contract {
            totals.vesting_contract = match totals.vesting_contract.checked_sub(withdraw_amount){
                Ok(new_amount) => new_amount,
                Err(_) => return Err(ContractError::CustomError {
                    val: format!("Vesting total accounting error. Vesting: {}, Withdraw: {}", totals.vesting_contract, withdraw_amount),
                }),
            };
        } else {            
            totals.stakers = match totals.stakers.checked_sub(withdraw_amount){
                Ok(new_amount) => new_amount,
                Err(_) => return Err(ContractError::CustomError {
                    val: format!("Staker total accounting error. Stakers: {}, Withdraw: {}", totals.stakers, withdraw_amount),
                }),
            };
        }
    } else {
        totals.stakers = match totals.stakers.checked_sub(withdraw_amount){
            Ok(new_amount) => new_amount,
            Err(_) => return Err(ContractError::CustomError {
                val: format!("Staker total accounting error. Stakers: {}, Withdraw: {}", totals.stakers, withdraw_amount),
            }),
        };
    }
    STAKING_TOTALS.save(deps.storage, &totals)?;

    //Response builder
    let attrs = vec![
        attr("method", "unstake"),
        attr("staker", info.sender.to_string()),
        attr("unstake_amount", withdrawable_amount.to_string()),
    ];
    
    //Create Response
    let response = Response::new().add_attributes(attrs).add_messages(claims_msgs);

    Ok(response)
}

/// (Un)Delegate MBRN to a Governator
/// If mbrn_amount is None, then act on the user's total stake
/// Only edits delegations for the user's stake, not their fluid delegated stake
fn update_delegations(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    governator_addr: Option<String>,
    mbrn_amount: Option<Uint128>,
    fluid: Option<bool>,
    delegate: Option<bool>,
    mut commission: Option<Decimal>,
    voting_power_delegation: Option<bool>,
) -> Result<Response, ContractError> {
    //Load config
    let config = CONFIG.load(deps.storage)?;

    //Enforce max commission
    if let Some(new_commission) = commission {
        commission = Some(min(new_commission, config.max_commission_rate))
    }

    //If a delegate is simply changing their commission, no need to check for half the logic
    if commission.is_some() && governator_addr.is_none() && mbrn_amount.is_none() && delegate.is_none() && fluid.is_none(){
        //Edit & save user's commission
        if let Ok(mut user_delegation_info) = DELEGATIONS.load(deps.storage, info.sender.clone()){
            user_delegation_info.commission = commission.unwrap();
            DELEGATIONS.save(deps.storage, info.sender.clone(), &user_delegation_info)?;
        }        
    } else if let Some(governator_addr) = governator_addr {
        //Validate Governator, doesn't need to be a staker but can't be the user
        let valid_gov_addr = deps.api.addr_validate(&governator_addr)?;
        if valid_gov_addr == info.clone().sender {
            return Err(ContractError::CustomError {
                val: String::from("Delegate cannot be the user"),
            });
        }
        let mut attrs = vec![
            attr("action", "update_delegations"),
            attr("delegator", info.sender.clone()),
            attr("delegate", valid_gov_addr.clone()),
        ];

        //Assert user is a staker
        let staker_deposits: Vec<StakeDeposit> = STAKED.load(deps.storage, info.sender.clone())?;
        
        //Calc total stake
        let total_staker_deposits: Uint128 = staker_deposits
            .into_iter()
            .map(|deposit| deposit.amount)
            .collect::<Vec<Uint128>>()
            .into_iter()
            .sum();

        //Get user's delegations
        let mut user_delegation_info = match DELEGATIONS.load(deps.storage, info.clone().sender.clone()){
            Ok(delegations) => delegations,
            Err(_) => DelegationInfo {
                delegated: vec![],
                delegated_to: vec![],
                commission: commission.unwrap_or(Decimal::zero()),
            },
        };

        //Set total_delegated_amount
        let total_delegated_amount = user_delegation_info.delegated_to
            .iter()
            .map(|delegation| delegation.amount)
            .collect::<Vec<Uint128>>()
            .into_iter()
            .sum::<Uint128>();

        //Set total_delegatible_amount
        let total_delegatible_amount = total_staker_deposits.clone() - total_delegated_amount;
            
        let mut claim_msgs: Vec<CosmosMsg> = vec![];

        /////Act on Optionals/////
        //Delegations
        if let Some(delegate) = delegate {
            //Claim user & delegate claims
            let (claims, interest) = get_user_claimables(deps.storage, env.clone(), info.sender.clone())?;
            //Create claimable msgs
            claim_msgs = create_rewards_msgs(
                deps.storage,
                deps.querier,
                deps.api,
                env.clone(),
                config,
                claims,
                interest,
                info.clone().sender.to_string(),
                vec![],
            )?;

            //If delegating, add to staker's delegated_to & delegates delegated
            if delegate {                
                //Validate MBRN amount
                let mbrn_amount = mbrn_amount.unwrap_or(total_delegatible_amount).min(total_delegatible_amount);
                attrs.push(attr("amount", mbrn_amount));
                //If mbrn_amount is greater than total delegatible amount, return error
                if mbrn_amount > total_delegatible_amount {
                    return Err(ContractError::CustomError {
                        val: String::from("MBRN amount exceeds delegatible amount"),
                    });
                } else if mbrn_amount < 1_000_000u128.into(){
                    return Err(ContractError::CustomError {
                        val: String::from("MBRN amount must be greater than 1"),
                    });
                }
                //If no delegatible amount, return error
                if total_delegatible_amount.is_zero() {
                    return Err(ContractError::CustomError {
                        val: String::from("No delegatible amount"),
                    });
                }

                //Load delegate's info
                let mut delegates_delegations = match DELEGATIONS.load(deps.storage, valid_gov_addr.clone()){
                    Ok(delegations) => delegations,
                    Err(_) => DelegationInfo {
                        delegated: vec![],
                        delegated_to: vec![],
                        commission: Decimal::zero(),
                    }
                };
                //Add to existing "delegated" from the Staker or add new Delegation object 
                match delegates_delegations.delegated.iter().enumerate().find(|(_i, delegation)| delegation.delegate == info.sender.clone()){
                    Some((index, _)) => delegates_delegations.delegated[index].amount += mbrn_amount,
                    None => {
                        delegates_delegations.delegated.push(Delegation {
                            delegate: info.sender.clone(),
                            amount: mbrn_amount,
                            fluidity: fluid.unwrap_or(false),
                            voting_power_delegation: voting_power_delegation.unwrap_or(true),
                            time_of_delegation: env.block.time.seconds(),
                            last_accrued: None,
                        });
                    }
                };
                //Save delegate's info           
                DELEGATIONS.save(deps.storage, valid_gov_addr.clone(), &delegates_delegations)?;

                //Add to staker's delegated_to
                //Add to existing "delegated_to" or add new Delegation object 
                match user_delegation_info.delegated_to.iter().enumerate().find(|(_i, delegation)| delegation.delegate == valid_gov_addr.clone()){
                    Some((index, _)) => user_delegation_info.delegated_to[index].amount += mbrn_amount,
                    None => {
                        user_delegation_info.delegated_to.push(Delegation {
                            delegate: valid_gov_addr.clone(),
                            amount: mbrn_amount,
                            fluidity: fluid.unwrap_or(false),
                            voting_power_delegation: voting_power_delegation.unwrap_or(true),
                            time_of_delegation: env.block.time.seconds(),
                            last_accrued: None,
                        });
                    }
                };
                //Save staker's info
                DELEGATIONS.save(deps.storage, info.sender.clone(), &user_delegation_info)?;
            } else {
                //Validate MBRN amount
                let mbrn_amount = mbrn_amount.unwrap_or(total_delegated_amount).min(total_delegated_amount);
                attrs.push(attr("amount", mbrn_amount));
                //If mbrn_amount is greater than total delegated amount, return error
                if mbrn_amount > total_delegated_amount {
                    return Err(ContractError::CustomError {
                        val: String::from("MBRN amount exceeds delegated amount"),
                    });
                } else if mbrn_amount < 1_000_000u128.into(){
                    return Err(ContractError::CustomError {
                        val: String::from("MBRN amount must be greater than 1"),
                    });
                }
                //If no delegatible amount, return error
                if total_delegated_amount.is_zero() {
                    return Err(ContractError::CustomError {
                        val: String::from("No delegated amount"),
                    });
                }
                /////If undelegating, remove from staker's "delegated_to" & delegates "delegated"///
                //Remove from delegate's
                let mut delegates_delegations = DELEGATIONS.load(deps.storage, valid_gov_addr.clone())?;
                match delegates_delegations.delegated.iter().enumerate().find(|(_i, delegation)| delegation.delegate == info.clone().sender){
                    Some((index, _)) => match delegates_delegations.delegated[index].amount.checked_sub(mbrn_amount){
                        Ok(new_amount) => {
                            //Can't leave less than 1 MBRN in delegation
                            if new_amount < 1_000_000u128.into(){
                                //Remove
                                delegates_delegations.delegated.remove(index);
                            } else {
                                //Update
                                delegates_delegations.delegated[index].amount = new_amount
                            }                            
                        },
                        Err(_) => {
                            //If more than delegated, remove from delegate's delegated
                            delegates_delegations.delegated.remove(index);
                        }
                    },
                    None => {
                        return Err(ContractError::CustomError {
                            val: String::from("Delegator not found in delegate's delegated"),
                        });
                    }
                };

                //Remove if empty, save otherwise
                if delegates_delegations.delegated.is_empty() && delegates_delegations.delegated_to.is_empty() {
                    DELEGATIONS.remove(deps.storage, valid_gov_addr.clone());
                } else {
                    DELEGATIONS.save(deps.storage, valid_gov_addr.clone(), &delegates_delegations)?;
                }

                //Subtract from staker's delegated_to
                match user_delegation_info.delegated_to.iter().enumerate().find(|(_i, delegation)| delegation.delegate == valid_gov_addr.clone()){
                    Some((index, _)) => match user_delegation_info.delegated_to[index].amount.checked_sub(mbrn_amount){
                        Ok(new_amount) => 
                            //Can't leave less than 1 MBRN in delegation
                            if new_amount < 1_000_000u128.into(){
                                //Remove
                                user_delegation_info.delegated_to.remove(index);
                            } else {
                                //Update
                                user_delegation_info.delegated_to[index].amount = new_amount;
                            }
                        Err(_) => {
                            //If more than delegated, remove from staker's delegated_to
                            user_delegation_info.delegated_to.remove(index);
                        }
                    },
                    None => {
                        return Err(ContractError::CustomError {
                            val: String::from("Delegate not found in staker's delegated_to"),
                        });
                    }
                };
                
                //Remove if empty, save otherwise
                if user_delegation_info.delegated.is_empty() && user_delegation_info.delegated_to.is_empty() {
                    DELEGATIONS.remove(deps.storage, info.clone().sender);
                } else {
                    DELEGATIONS.save(deps.storage, info.clone().sender, &user_delegation_info)?;
                }
            }
        }

        
        //Edit & save user's commission
        if let Some(commission) = commission {
            if let Ok(mut user_delegation_info) = DELEGATIONS.load(deps.storage, info.sender.clone()){
                user_delegation_info.commission = commission;
                DELEGATIONS.save(deps.storage, info.sender.clone(), &user_delegation_info)?;
            }
        }

        //Update fluidity for both staker & delegate info if fluidity or vp delegation is Some
        if fluid.is_some() || voting_power_delegation.is_some(){
            //Staker's delegations
            if let Ok(mut user_delegation_info) = DELEGATIONS.load(deps.storage, info.sender.clone()){
                user_delegation_info.delegated_to = user_delegation_info.delegated_to.clone()
                    .into_iter()
                    .map(|mut delegation| {
                        if delegation.delegate == valid_gov_addr.clone() {
                            if let Some(fluid) = fluid {
                                delegation.fluidity = fluid;
                            }
                            if let Some(vp_delegation) = voting_power_delegation {
                                delegation.voting_power_delegation = vp_delegation;
                            }                            

                            delegation
                        } else {
                            delegation
                        }
                    })
                    .collect::<Vec<Delegation>>();
                DELEGATIONS.save(deps.storage, info.sender.clone(), &user_delegation_info)?;
            };

            //Delegate's delegations
            if let Ok(mut delegates_delegations) = DELEGATIONS.load(deps.storage, valid_gov_addr.clone()){
                delegates_delegations.delegated = delegates_delegations.delegated.clone()
                    .into_iter()
                    .map(|mut delegation| {
                        if delegation.delegate == info.clone().sender {
                            if let Some(fluid) = fluid {
                                delegation.fluidity = fluid;
                            }
                            if let Some(vp_delegation) = voting_power_delegation {
                                delegation.voting_power_delegation = vp_delegation;
                            }

                            delegation
                        } else {
                            delegation
                        }
                    })
                    .collect::<Vec<Delegation>>();
                DELEGATIONS.save(deps.storage, valid_gov_addr.clone(), &delegates_delegations)?;
            };        
        }
        
        return Ok(Response::new().add_messages(claim_msgs).add_attributes(attrs))
    }

    Ok(Response::new().add_attributes(vec![
        attr("action", "update_delegations"),
        attr("delegate", info.sender),
        attr("commission", commission.unwrap().to_string()),
    ]))
}

/// Delegating Fluid delegatations
/// Delegates don't need to be stakers
/// Delegate loses control over the delegated amount, i.e. the initial staker retains control over all delegated amounts
fn delegate_fluid_delegations(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    governator_addr: String,
    mbrn_amount: Option<Uint128>,
) -> Result<Response, ContractError>{
    //Validate Governator, doesn't need to be a staker but can't be the user
    let valid_gov_addr = deps.api.addr_validate(&governator_addr)?;
    if valid_gov_addr == info.clone().sender {
        return Err(ContractError::CustomError {
            val: String::from("Delegate cannot be the user"),
        });
    }

    //Get delegate's delegations, assert they are a delegate
    let mut delegate_delegation_info = DELEGATIONS.load(deps.storage, info.clone().sender.clone())?;

    //Load config
    let config = CONFIG.load(deps.storage)?;
    //Get claims
    let (claims, interest) = get_user_claimables(deps.storage, env.clone(), info.sender.clone())?;
    //Make rewards msg
    let claims_msgs = create_rewards_msgs(
        deps.storage, 
        deps.querier,
        deps.api,        
        env.clone(),
        config,
        claims,
        interest,
        info.sender.clone().to_string(),
        vec![]
    )?;

    //Set delegation_info variants
    let mut fluid_delegations: Vec<Delegation> = delegate_delegation_info.delegated.clone()
        .into_iter()
        .filter(|delegation| delegation.fluidity)
        .collect();
    let non_fluid_delegations: Vec<Delegation> = delegate_delegation_info.delegated.clone()
        .into_iter()
        .filter(|delegation| !delegation.fluidity)
        .collect();

    //Set total_fluid_delegatible_amount
    let total_fluid_delegatible_amount = fluid_delegations.clone()
        .into_iter()
        .map(|delegation| delegation.amount)
        .collect::<Vec<Uint128>>()
        .into_iter()
        .sum::<Uint128>();
    //Validate MBRN amount
    let mut mbrn_amount = mbrn_amount.unwrap_or(total_fluid_delegatible_amount).min(total_fluid_delegatible_amount);
    
    if total_fluid_delegatible_amount < mbrn_amount {
        return Err(ContractError::CustomError {
            val: String::from("MBRN amount exceeds total fluid delegatible amount"),
        });
    } else if mbrn_amount < 1_000_000u128.into(){
        return Err(ContractError::CustomError {
            val: String::from("MBRN amount must be greater than 1"),
        });
    }
    if total_fluid_delegatible_amount.is_zero() {
        return Err(ContractError::CustomError {
            val: String::from("No fluid delegations to delegate"),
        });
    }
 
    //Parse through delegate's fluid delegations
    for (i, delegation) in fluid_delegations.clone().into_iter().enumerate() {
        /////////Calc delegation amount & update fluid_delegations
        //If delegation amount is less than mbrn_amount, remove delegation from delegate's delegated
        let delegation_amount = if delegation.amount <= mbrn_amount {
            fluid_delegations.remove(i);
            //Subtract delegation amount from mbrn_amount
            mbrn_amount -= delegation.amount;

            delegation.amount
        } else {
            //If delegation amount is greater than mbrn_amount, subtract mbrn_amount from delegation amount
            fluid_delegations[i].amount -= mbrn_amount;

            //Assert remaining delegation amount is greater than 1
            if fluid_delegations[i].amount < 1_000_000u128.into(){
                mbrn_amount += fluid_delegations[i].amount;
                fluid_delegations.remove(i);
            }

            let delegation_amount = mbrn_amount;         
            
            //Set mbrn_amount to 0
            mbrn_amount = Uint128::zero();

            delegation_amount
        };

        //Delegate delegation_amount to governator
        let mut delegate_delegation_info = match DELEGATIONS.load(deps.storage, valid_gov_addr.clone()){
            Ok(delegation_info) => delegation_info,
            Err(_) => DelegationInfo {
                delegated_to: vec![],
                delegated: vec![],
                commission: Decimal::zero(),
            }
        };
        ///We are searching for the initial Delegate's delegation to the Governator
        match delegate_delegation_info.delegated.iter().enumerate().find(|(_i, listed_delegation)| listed_delegation.delegate == delegation.delegate.clone()){
            Some((index, _)) => delegate_delegation_info.delegated[index].amount += delegation_amount,
            None => {
                delegate_delegation_info.delegated.push(Delegation {
                    delegate: delegation.delegate.clone(),
                    amount: delegation_amount,
                    fluidity: true,
                    voting_power_delegation: delegation.voting_power_delegation,
                    time_of_delegation: env.block.time.seconds(),
                    last_accrued: None,
                });
            }
        };
        //Save delegate's info           
        DELEGATIONS.save(deps.storage, valid_gov_addr.clone(), &delegate_delegation_info)?;

        //Add delegation_amount to initial delegate's delegated_to the Governator
        let mut initial_delegator_delegation_info = DELEGATIONS.load(deps.storage, delegation.delegate.clone())?;
        match initial_delegator_delegation_info.delegated_to.iter().enumerate().find(|(_i, listed_delegation)| listed_delegation.delegate == valid_gov_addr.clone()){
            Some((index, _)) => initial_delegator_delegation_info.delegated_to[index].amount += delegation_amount,
            None => {
                initial_delegator_delegation_info.delegated_to.push(Delegation {
                    delegate: valid_gov_addr.clone(),
                    amount: delegation_amount,
                    fluidity: true,
                    voting_power_delegation: delegation.voting_power_delegation,
                    time_of_delegation: env.block.time.seconds(),
                    last_accrued: None,
                });
            }
        };
        //Subtract delegation_amount from initial delegator's delegated_to from the executing delegator
        if let Some((index, _)) = initial_delegator_delegation_info.delegated_to.iter().enumerate().find(|(_i, listed_delegation)| listed_delegation.delegate == info.clone().sender){
            //Subtract delegation_amount
            initial_delegator_delegation_info.delegated_to[index].amount -= delegation_amount;
        } else {
            //This should be unreachable
            return Err(ContractError::CustomError {
                val: String::from("Delegate is not a delegate of the initial delegator"),
            });
        };
        //Save initial delegate's info
        DELEGATIONS.save(deps.storage, delegation.delegate.clone(), &initial_delegator_delegation_info)?;

        //If mbrn_amount is 0, break
        if mbrn_amount == Uint128::zero() {
            break;
        }
    }

    //Update delegate's delegations
    fluid_delegations.extend(non_fluid_delegations);
    delegate_delegation_info.delegated = fluid_delegations;
    DELEGATIONS.save(deps.storage, info.clone().sender.clone(), &delegate_delegation_info)?;


    Ok(Response::new().add_messages(claims_msgs).add_attributes(vec![
        attr("action", "delegate_fluid_delegations"),
        attr("delegator", info.sender),
        attr("delegate", valid_gov_addr),
        attr("amount", mbrn_amount),
    ]))
}

/// Restake unstaking deposits for a user
fn restake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mut restake_amount: Uint128,
) -> Result<Response, ContractError> {
    //Load state
    let config = CONFIG.load(deps.storage)?;

    //Initialize variables
    let initial_restake = restake_amount;
    let error: Option<StdError> = None;

    //Load staker's deposits
    let deposits = STAKED.load(deps.storage, info.clone().sender)?;

    //Iterate through staker's deposits
    let restaked_deposits: Vec<StakeDeposit> = deposits.clone()
        .into_iter()
        .map(|mut deposit| {
            if !restake_amount.is_zero() && deposit.unstake_start_time.is_some(){
                if deposit.amount >= restake_amount {
                    //Zero restake_amount
                    restake_amount = Uint128::zero();
                    
                    //Restake
                    deposit.unstake_start_time = None;
                    deposit.last_accrued = Some(env.block.time.seconds());
                } else if deposit.amount < restake_amount {
                    //Sub from restake_amount
                    restake_amount -= deposit.amount;                  

                    //Restake
                    deposit.unstake_start_time = None;
                    deposit.last_accrued = Some(env.block.time.seconds());
                }
            }
            deposit
        })
        .collect::<Vec<StakeDeposit>>();

    //Return error if any
    if let Some(err) = error {
        return Err(ContractError::Std(err));
    }

    let (claimables, accrued_interest) = get_user_claimables(
        deps.storage,
        env.clone(),
        info.clone().sender,
    )?;

    //Save new Deposits
    STAKED.save(deps.storage, info.clone().sender,&restaked_deposits)?;

    //Add the restaked amount to total staked
    let mut totals = STAKING_TOTALS.load(deps.storage)?;
    totals.stakers += initial_restake - restake_amount;
    STAKING_TOTALS.save(deps.storage, &totals)?;

    //Create rewards msgs
    let rewards_msgs = create_rewards_msgs(
        deps.storage,
        deps.querier,
        deps.api,
        env.clone(),
        config.clone(),
        claimables,
        accrued_interest,
        info.clone().sender.to_string(),
        vec![],
    )?;

    Ok(Response::new().add_messages(rewards_msgs).add_attributes(vec![
        attr("method", "restake"),
        attr("restake_amount", initial_restake - restake_amount),
    ]))
}

/// Sends available claims to info.sender or as specified in send_to.
/// If claim_as is passed, the claims will be sent as said asset.
/// If restake is true, the accrued ownership will be restaked.
pub fn claim_rewards(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    send_to: Option<String>,
    restake: bool,
) -> Result<Response, ContractError> {
    let config: Config = CONFIG.load(deps.storage)?;

    let mut messages: Vec<CosmosMsg>;
    let accrued_interest: Uint128;
    let user_claimables: Vec<Asset>;

    //Get user claim msgs and accrued interest
    (messages, user_claimables, accrued_interest) = user_claims(
        deps.storage,
        deps.querier,
        deps.api,
        env.clone(),
        config.clone(),
        info.clone(),
        send_to.clone(),
    )?;

    //Create MBRN Mint Msg
    if config.osmosis_proxy.is_some() {
        //Vesting contract gets no MBRN inflation
        if info.sender != config.clone().vesting_contract.unwrap_or_else(|| Addr::unchecked("")) && !accrued_interest.is_zero() {
            //Who to send to?
            if send_to.is_some() {
                let valid_recipient = deps.api.addr_validate(&send_to.clone().unwrap())?;

                let message = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.clone().osmosis_proxy.unwrap().to_string(),
                    msg: to_binary(&OsmoExecuteMsg::MintTokens {
                        denom: config.mbrn_denom,
                        amount: accrued_interest,
                        mint_to_address: valid_recipient.to_string(),
                    })?,
                    funds: vec![],
                });
                messages.push(message);
            } else if restake {
                //Mint to contract
                let message = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.clone().osmosis_proxy.unwrap().to_string(),
                    msg: to_binary(&OsmoExecuteMsg::MintTokens {
                        denom: config.clone().mbrn_denom,
                        amount: accrued_interest,
                        mint_to_address: env.contract.address.to_string(),
                    })?,
                    funds: vec![],
                });
                messages.push(message);
                //Stake for user
                let message = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: env.contract.address.to_string(),
                    msg: to_binary(&ExecuteMsg::Stake {
                        user: Some(info.sender.to_string()),
                    })?,
                    funds: vec![coin(accrued_interest.u128(), config.mbrn_denom)],
                });
                messages.push(message);
            } else {
                //Send stake to sender
                let message = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.osmosis_proxy.unwrap().to_string(),
                    msg: to_binary(&OsmoExecuteMsg::MintTokens {
                        denom: config.mbrn_denom,
                        amount: accrued_interest,
                        mint_to_address: info.sender.to_string(),
                    })?,
                    funds: vec![],
                });
                messages.push(message);
            }
        }
    } else {
        return Err(ContractError::CustomError {
            val: String::from("No proxy contract setup"),
        });
    }

    //Error if there is nothing to claim
    if messages.is_empty() {
        return Err(ContractError::CustomError {
            val: String::from("Nothing to claim"),
        });
    }

    let user_claimables_string: Vec<String> = user_claimables
        .into_iter()
        .map(|claims| claims.to_string())
        .collect::<Vec<String>>();

    let res = Response::new()
        .add_attribute("method", "claim")
        .add_attribute("user", info.sender)
        .add_attribute("send_to", send_to.unwrap_or_else(|| String::from("None")))
        .add_attribute("restake", restake.to_string())
        .add_attribute("mbrn_rewards", accrued_interest.to_string())
        .add_attribute("fee_rewards", format!("{:?}", user_claimables_string));

    Ok(res.add_messages(messages))
}

/// Deposit assets for staking rewards
fn deposit_fee(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    fee_assets: Vec<Asset>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut messages: Vec<CosmosMsg> = vec![];

    //Create response attribute
    let string_fee_assets = fee_assets.clone()
        .into_iter()
        .map(|asset| asset.to_string())
        .collect::<Vec<String>>();

    //Get CDT denom
    let basket: Basket = query_basket(deps.querier, config.clone().positions_contract.unwrap_or_else(|| Addr::unchecked("")).to_string())?;
    let cdt_denom = basket.credit_asset.info;

    //Filter assets if stakers are keeping raw CDT
    let (non_CDT_assets, CDT_assets) = if config.keep_raw_cdt {
        //Filter
        let non_cdt = fee_assets.clone()
            .into_iter()
            .filter(|fee_asset| fee_asset.info != cdt_denom)
            .collect::<Vec<Asset>>();
        
        let cdt = fee_assets.clone()
            .into_iter()
            .filter(|fee_asset| fee_asset.info == cdt_denom)
            .collect::<Vec<Asset>>();

        ( non_cdt, cdt )
    } else {
        //Don't filter
        (fee_assets.clone(), vec![])
    };    
    
    //Act if there are non-CDT assets that didn't come from the auction contract
    if non_CDT_assets.len() != 0 {
        if let Some(auction_contract) = config.clone().auction_contract {
            if info.sender != auction_contract {
                //Create auction msgs
                for asset in non_CDT_assets.clone() {
                    let message: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: auction_contract.to_string(),
                        msg: to_binary(&AuctionExecuteMsg::StartAuction { 
                            repayment_position_info: None, 
                            send_to: None, 
                            auction_asset: asset.clone(),
                        })?,
                        funds: vec![asset_to_coin(asset)?],
                    });

                    messages.push(message);
                }
            }
        }
    }

    //Distribute fees to stakers if:
    // - there are CDT assets
    // - this is the auction contract sending assets 
    // - there is no auction contract
    if !CDT_assets.is_empty() || (config.clone().auction_contract.is_some() && config.clone().auction_contract.unwrap() == info.sender) || config.auction_contract.is_none(){
        //Set fee assets
        let fee_assets = if (config.clone().auction_contract.is_some() && config.clone().auction_contract.unwrap() == info.sender) || config.auction_contract.is_none(){
            //If auction contract, set fee_assets to all assets
            //bc it has just sent back the system's desired_Asset
            //If no auction contract then nothing was sent so deposit all to stakers
            fee_assets.clone()
        } else {
            //If not auction contract, set fee_assets to CDT_assets
            //bc the other assets were sent to the auction
            CDT_assets.clone()
        };
    
        //Load Fee Events
        let mut fee_events = FEE_EVENTS.load(deps.storage)?;

        //Load Total staked
        let mut totals = STAKING_TOTALS.load(deps.storage)?;

        //Update vesting total
        if let Some(vesting_contract) = config.clone().vesting_contract {        
            let vesting_total = get_total_vesting(deps.querier, vesting_contract.to_string())?;

            totals.vesting_contract = vesting_total;
            STAKING_TOTALS.save(deps.storage, &totals)?;
            
            //Transform total with vesting rev multiplier
            totals.vesting_contract = decimal_multiplication(
                Decimal::from_ratio(vesting_total, Uint128::one()),
                config.vesting_rev_multiplier)?
            .to_uint_floor();
        }

        //Set total
        let mut total: Uint128 = totals.vesting_contract + totals.stakers;
        if total.is_zero() {
            total = Uint128::new(1u128)
        }
        let decimal_total = Decimal::from_ratio(total, Uint128::new(1u128));
        
        //Add new Fee Event
        for asset in fee_assets.clone() {        
            let amount = Decimal::from_ratio(asset.amount, Uint128::new(1u128));

            fee_events.push(FeeEvent {
                time_of_event: env.block.time.seconds(),
                fee: LiqAsset {
                    //Amount = Amount per Staked MBRN
                    info: asset.info,
                    amount: decimal_division(amount, decimal_total)?,
                },
            });
        }

        FEE_EVENTS.save(deps.storage, &fee_events)?;
    }
    
    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("method", "deposit_fee"),
        attr("fee_assets", format!("{:?}", string_fee_assets)),
    ]))
}

/// Create rewards msgs from claimables and accrued interest
fn create_rewards_msgs(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    api: &dyn Api,
    env: Env,
    config: Config,
    claimables: Vec<Asset>,
    mut accrued_interest: Uint128,
    user: String,
    mut native_claims: Vec<Coin>,
) -> StdResult<Vec<CosmosMsg>>{

    let mut msgs: Vec<CosmosMsg> = vec![];

    //Create msg for claimable fees
    if claimables != vec![] {
        //Aggregate native tokens
        for asset in claimables {
            match asset.clone().info {
                AssetInfo::Token { address: _ } => {
                    return Err(StdError::GenericErr { msg: String::from("Non-native token unclaimable") })
                }
                AssetInfo::NativeToken { denom: _ } => {
                    native_claims.push(asset_to_coin(asset)?);
                }
            }
        }
    }

    //Validate staker
    let staker = api.addr_validate(&user.clone())?;

    //Add claims as a delegate to native claims && add new interest as a staking deposit
    if let Ok((claims, interest)) = DELEGATE_CLAIMS.load(storage, staker.clone()) {
        //Add interest to accrued interest
        accrued_interest += interest;
        
        //Add claims 
        native_claims.extend(claims.clone());

        //Remove claims
        DELEGATE_CLAIMS.remove(storage, staker.clone());
    }

    //Add accrued interest as a staking deposit && mint the amount to the contract
    if !accrued_interest.is_zero(){
        //Add accrued interest as a staking deposit
        add_staking_deposit(storage, env.clone(), config.clone(), staker, accrued_interest)?;

        //mint to contract for accounting purposes
        let msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.osmosis_proxy.unwrap().to_string(),
            msg: to_binary(&OsmoExecuteMsg::MintTokens {
                denom: config.mbrn_denom,
                amount: accrued_interest,
                mint_to_address: env.contract.address.to_string(),
            })?,
            funds: vec![],
        });
        msgs.push(msg);
    }

    if native_claims != vec![] {
        ////Token send sanity check////
        //Remove 0'd coins
        native_claims = native_claims
            .into_iter()
            .filter(|coin| !coin.amount.is_zero() && coin.amount > Uint128::zero())
            .collect::<Vec<Coin>>();
        //Sort alphabetically
        native_claims.sort_by(|a, b| a.denom.cmp(&b.denom));
        //Remove assets not owned by the contract
        let contract_balances = querier.query_all_balances(env.contract.address.clone())?;
        native_claims = native_claims
            .into_iter()
            .filter(|coin| {
                contract_balances
                    .clone()
                    .into_iter()
                    .find(|balance| balance.denom == coin.denom)
                    .is_some()
            })
            .collect::<Vec<Coin>>();        

        //Send native claims
        if native_claims != vec![] {
            let msg = CosmosMsg::Bank(BankMsg::Send {
                to_address: user.clone(),
                amount: native_claims,
            });
            msgs.push(msg);
        }
    }

    Ok(msgs)
}

/// Get deposit claims and add to list of claims/total interest
pub fn add_deposit_claimables(
    storage: &mut dyn Storage,
    config: Config,
    incentive_schedule: StakeDistributionLog,
    env: Env,
    fee_events: Vec<FeeEvent>,
    deposit: StakeDeposit,
    delegated_to: Vec<Delegation>,
    delegated: Vec<Delegation>,
    claimables: &mut Vec<Asset>,
    accrued_interest: &mut Uint128,
    total_rewarding_stake: Uint128, //stake thats being rewarded
    user_commission_rate: Decimal,
) -> StdResult<()>{
    //Calc claimables from this deposit
    let (deposit_claimables, deposit_interest) = get_deposit_claimables(
        storage,
        config.clone(),
        incentive_schedule.clone(),
        env.clone(),
        fee_events.clone(),
        deposit.clone(),
        delegated.clone(),
        delegated_to.clone(),
        total_rewarding_stake,
        user_commission_rate,
    )?;
    *accrued_interest += deposit_interest;

    //Condense like Assets
    for claim_asset in deposit_claimables {
        //Check if asset is already in the list of claimables and add accordingly
        match claimables
            .clone()
            .into_iter()
            .enumerate()
            .find(|(_i, asset)| asset.info == claim_asset.info)
        {
            Some((index, _asset)) => claimables[index].amount += claim_asset.amount,
            None => claimables.push(claim_asset),
        }
    }

    Ok(())
}
/// Can't Unstake if...
/// 1. There is an active proposal by the address
/// 2. The address has voted for a proposal that has passed but not yet executed
pub fn can_this_addr_unstake(
    querier: QuerierWrapper,
    user: Addr,
    config: Config,
) -> Result<(), ContractError> {
    
    //Can't unstake if there is an active proposal by user
    let proposal_list: ProposalListResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart { 
        contract_addr: config.clone().governance_contract.unwrap().to_string(), 
        msg: to_binary(&Gov_QueryMsg::ActiveProposals { start: None, limit: Some(32) })?
    }))?;

    for proposal in proposal_list.clone().proposal_list {
        if proposal.submitter == user && proposal.status == ProposalStatus::Active {
            return Err(ContractError::CustomError { val: String::from("Can't unstake while your proposal is active") })
        }
    }

    //Can't unstake if the user has voted for a proposal that is Active or has passed but not yet executed
    //Get list of proposals that have passed & have executables or are active
    for proposal in proposal_list.proposal_list {
        if (proposal.status == ProposalStatus::Passed && proposal.messages.is_some()) || proposal.status == ProposalStatus::Active{
            //Get list of voters for this proposal
            let _voters: Vec<Addr> = match querier.query_wasm_smart::<Vec<Addr>>(
                config.clone().governance_contract.unwrap().to_string(), 
                &Gov_QueryMsg::ProposalVoters { 
                    proposal_id: proposal.proposal_id.into(), 
                    vote_option: membrane::governance::ProposalVoteOption::For, 
                    start: None, 
                    limit: None,
                    specific_user: Some(user.to_string())
                }
            ){
                // if the query doesn't error then the user has voted For this proposal
                Ok(_) => return Err(ContractError::CustomError { val: format!("Can't unstake if the proposal you helped pass hasn't executed its messages yet: {}", proposal.proposal_id) }),
                Err(_) => vec![]
            };
        }
    }

    Ok(())
}

/// Update deposits being withdrawn from.
/// Returns claimable assets, accrued interest, withdrawable amount.
fn withdraw_from_state(
    storage: &mut dyn Storage,
    env: Env,
    staker: Addr,
    mut withdrawal_amount: Uint128,
) -> StdResult<(Vec<Asset>, Uint128, Uint128)> {
    let config = CONFIG.load(storage)?;
    let deposits = STAKED.load(storage, staker.clone())?;

    let error: Option<StdError> = None;
    let mut returning_deposits: Vec<StakeDeposit> = vec![];


    //Find withdrawable deposits
    let withdrawable_deposits: Vec<StakeDeposit> = deposits
        .clone()
        .into_iter()
        .filter(|deposit| deposit.unstake_start_time.is_some() && env.block.time.seconds() - deposit.unstake_start_time.unwrap() >= config.unstaking_period * SECONDS_PER_DAY)
        .collect::<Vec<StakeDeposit>>();
    let total_withdrawable = withdrawable_deposits.clone()
        .into_iter()
        .map(|deposit| deposit.amount)
        .collect::<Vec<Uint128>>()
        .into_iter()
        .sum::<Uint128>();

    //If there is still leftover withdrawal_amount, begin to unstake staked deposits
    if total_withdrawable < withdrawal_amount {
        withdrawal_amount -= total_withdrawable;
    } else {
        withdrawal_amount = Uint128::zero();
    }
    //Only look at deposits that are not unstaking
    let staked_deposits: Vec<StakeDeposit> = deposits
        .clone()
        .into_iter()
        .filter(|deposit| deposit.unstake_start_time.is_none())
        .collect::<Vec<StakeDeposit>>();

    //Iterate through deposits
    let mut new_deposits: Vec<StakeDeposit> = staked_deposits.clone()
        .into_iter()
        .map(|mut deposit| {
            
            //Subtract from each deposit until there is none left to withdraw or begin to unstake
            if withdrawal_amount != Uint128::zero() && deposit.amount > withdrawal_amount {
               {
                    //Since we claimed rewards
                    deposit.last_accrued = Some(env.block.time.seconds());                    
                    
                    //Create a StakeDeposit object for the amount not getting unstaked
                    returning_deposits.push(StakeDeposit {
                        amount: deposit.amount - withdrawal_amount,
                        unstake_start_time: None,
                        ..deposit.clone()
                    });
                    
                    //Set new deposit amount
                    deposit.amount = withdrawal_amount;                       

                    //Set the unstaking_start_time 
                    if deposit.unstake_start_time.is_none() {
                        deposit.unstake_start_time = Some(env.block.time.seconds());
                        //Zero withdrawal_amount since the deposit total fulfills the withdrawal
                        //Only true if this is a new unstake
                        withdrawal_amount = Uint128::zero();
                    }
                }                

            } else if withdrawal_amount != Uint128::zero() && deposit.amount <= withdrawal_amount {
                
                {
                    //if stake time is some but can't be withdrawn (i.e. made it within this conditional but skips the next)
                    // we don't count that towards the withdrawal_amount tally.

                    //Else, Set the unstaking_start_time 
                    if deposit.unstake_start_time.is_none() {
                        deposit.unstake_start_time = Some(env.block.time.seconds());

                        //Since the Deposit amount is less or equal, substract it from the withdrawal amount
                        withdrawal_amount -= deposit.amount;
                    }
                    //Since we claimed rewards
                    deposit.last_accrued = Some(env.block.time.seconds());
                }
        
            }
            deposit
        })
        .collect::<Vec<StakeDeposit>>()
        .into_iter()
        .filter(|deposit| deposit.amount != Uint128::zero())
        .collect::<Vec<StakeDeposit>>();

    if withdrawal_amount != Uint128::zero() {
        return Err(StdError::GenericErr {
            msg: format!(
                "Attempting to withdraw {} MBRN over ( {} )'s total currently staked amount",
                withdrawal_amount, staker
            ),
        });
    }

    if error.is_some() {
        return Err(error.unwrap());
    }
    //Add any returning_deposits
    new_deposits.extend(returning_deposits);
    //Filter for deposits that are unstaking but not yet withdrawable
    let mut unstaking_deposits: Vec<StakeDeposit> = deposits
        .clone()
        .into_iter()
        .filter(|deposit| deposit.unstake_start_time.is_some() && env.block.time.seconds() - deposit.unstake_start_time.unwrap() < config.unstaking_period * SECONDS_PER_DAY)
        .collect::<Vec<StakeDeposit>>();
    //Aggregate deposits
    unstaking_deposits.extend(new_deposits.clone());

    //Before we save, claim rewards for the staker
    let (claimables, accrued_interest) = get_user_claimables(
        storage, 
        env, 
        staker.clone(),
    )?;

    //Save new deposit stack
    STAKED.save(storage, staker.clone(), &unstaking_deposits)?;

    Ok((claimables, accrued_interest, total_withdrawable))
}


/// Calculates the accrued interest for a given stake
fn accumulate_interest(stake: Uint128, rate: Decimal, time_elapsed: u64) -> StdResult<Uint128> {
    let applied_rate = rate.checked_mul(Decimal::from_ratio(
        Uint128::from(time_elapsed),
        Uint128::from(SECONDS_PER_YEAR),
    ))?;

    let accrued_interest = stake * applied_rate;

    Ok(accrued_interest)
}

/// Return claim messages for a given user 
fn user_claims(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    api: &dyn Api,
    env: Env,
    config: Config,
    info: MessageInfo,
    send_to: Option<String>,
) -> StdResult<(Vec<CosmosMsg>, Vec<Asset>, Uint128)> {
    //Can only claim for oneself (info.sender)
    let (user_claimables, accrued_interest) =
        get_user_claimables(storage, env.clone(), info.clone().sender)?;
        
    ///Claim the available assets///
    //If we are sending to the sender
    if send_to.clone().is_none() {                
        //Send to sender
        let rewards_msgs = create_rewards_msgs(
            storage,
            querier,
            api,
            env.clone(),
            config.clone(), 
            user_claimables.clone(), 
            Uint128::zero(), //Dont send interest here
            info.clone().sender.to_string(), 
            vec![],
        )?;
        
        return Ok((rewards_msgs, user_claimables, accrued_interest))
    } else {
        //Validate recipient
        let valid_recipient = api.addr_validate(&send_to.clone().unwrap())?;

        //Send to recipient
        let rewards_msgs = create_rewards_msgs(
            storage,
            querier,
            api,
            env,
            config.clone(), 
            user_claimables.clone(), 
            Uint128::zero(), //Dont send interest here
            valid_recipient.to_string(), 
            vec![],
        )?;

        return Ok((rewards_msgs, user_claimables, accrued_interest))
    }  
}

/// Return user claimables for a given user
fn get_user_claimables(
    storage: &mut dyn Storage,
    env: Env,
    user: Addr,
) -> StdResult<(Vec<Asset>, Uint128)> {
    //Load contract state
    let mut config = CONFIG.load(storage)?;
    let incentive_schedule = INCENTIVE_SCHEDULING.load(storage)?;
    let fee_events = FEE_EVENTS.load(storage)?;
    //Load User State
    let deposits: Vec<StakeDeposit> = match STAKED.load(storage, user.clone()){
        Ok(deposits) => { deposits },
        Err(_) => { vec![] },
    };
    let DelegationInfo { delegated, delegated_to, commission: user_commission_rate } = match DELEGATIONS.load(storage, user.clone()){
        Ok(res) => res,
        Err(_) => DelegationInfo {
            delegated: vec![],
            delegated_to: vec![],
            commission: Decimal::zero(),
        },
    };

    //Find rewards from deposits
    if deposits != vec![] {
        // println!("{:?}", deposits);
        let mut claimables: Vec<Asset> = vec![];
        let mut accrued_interest = Uint128::zero();

        //Return the unstaking deposits as they don't get any claims
        let mut returning_deposits: Vec<StakeDeposit> = deposits.clone()
            .into_iter()
            .filter(|deposit| deposit.unstake_start_time.is_some())
            .collect::<Vec<StakeDeposit>>();

        //Filter out the unstaking deposits
        let deposits = deposits
            .into_iter()
            .filter(|deposit| deposit.unstake_start_time.is_none())
            .collect::<Vec<StakeDeposit>>();

        //Get earliest stake time from staked deposits
        let earliest_stake_time = deposits.clone()
            .into_iter()
            .map(|deposit| deposit.stake_time)
            .min()
            .unwrap_or_else(|| env.block.time.seconds());
        
        //Calc total deposits used to reward
        let total_rewarding_stake: Uint128 = deposits.clone()
            .into_iter()
            .map(|deposit| deposit.amount)
            .sum();

        //Get claimables per deposit
        for deposit in deposits {
            add_deposit_claimables(
                storage,
                config.clone(), 
                incentive_schedule.clone(), 
                env.clone(), 
                fee_events.clone(), 
                deposit.clone(), 
                delegated_to.clone(),
                delegated.clone(),
                &mut claimables, 
                &mut accrued_interest,
                total_rewarding_stake,
                user_commission_rate,
            )?;
        }

        //Add condensed deposit to returning_deposits
        returning_deposits.push(
            StakeDeposit {
                staker: user.clone(),
                amount: total_rewarding_stake,
                stake_time: earliest_stake_time,
                unstake_start_time: None,
                last_accrued: Some(env.block.time.seconds()),
            }
        );

        //Save new condensed deposit for user
        STAKED.save(storage, user.clone(), &returning_deposits)?;

        //Find and save claimables for the user's delegates
        if !delegated_to.is_empty(){
            for delegate in delegated_to {
                //Load delegate's commission
                let commission = DELEGATIONS.load(storage, delegate.delegate.clone())?.commission;
                
                //Update last_accrued for delegate
                DELEGATIONS.update(storage, delegate.delegate.clone(), |delegation_info| -> StdResult<DelegationInfo>{
                    match delegation_info {
                        Some(mut delegation_info) => {
                            //Find the delegation
                            if let Some((i, _)) = delegation_info.delegated.clone()
                                .into_iter()
                                .enumerate()
                                .find(|(_, delegation)| delegation.delegate == user.clone())
                                {
                                    //Update last_accrued
                                    delegation_info.delegated[i].last_accrued = Some(env.block.time.seconds());
                                };

                            Ok(delegation_info)
                        },
                        //Unreachable
                        None => Err(StdError::generic_err("Delegate not found")),
                    }
                })?;
                //Update last_accrued on user's (delegator's) side
                DELEGATIONS.update(storage, user.clone(), |delegation_info| -> StdResult<DelegationInfo>{
                    match delegation_info {
                        Some(mut delegation_info) => {
                            //Find the delegation
                            if let Some((i, _)) = delegation_info.delegated_to.clone()
                                .into_iter()
                                .enumerate()
                                .find(|(_, delegation)| delegation.delegate == delegate.delegate.clone())
                                {
                                    //Update last_accrued
                                    delegation_info.delegated_to[i].last_accrued = Some(env.block.time.seconds());
                                };

                            Ok(delegation_info)
                        },
                        //Unreachable
                        None => Err(StdError::generic_err("Delegate not found")),
                    }
                })?;

                //Calc the delegate_commission commission amount (delegated * commission)
                let delegate_commission: Uint128 =
                    match decimal_multiplication(Decimal::from_ratio(delegate.amount, Uint128::one()), commission){
                        Ok(res) => res,
                        Err(_) => Decimal::zero(),
                    }.to_uint_floor();

                let delegate_temp_deposit =
                    StakeDeposit {
                        staker: user.clone(),
                        amount: delegate_commission,
                        stake_time: delegate.time_of_delegation,
                        unstake_start_time: None,
                        last_accrued: delegate.last_accrued,
                    };

                //Get claimables 
                let (delegate_claimables, delegate_accrued_interest) = get_deposit_claimables(
                    storage,
                    config.clone(), 
                    incentive_schedule.clone(), 
                    env.clone(), 
                    fee_events.clone(), 
                    delegate_temp_deposit.clone(), 
                    vec![],
                    vec![],
                    delegate_commission,
                    Decimal::zero(), //Since it won't be used anyway
                )?;

                //Transform claimables to sendable Coins
                let delegate_claimables = delegate_claimables
                    .into_iter()
                    .map(|asset| asset_to_coin(asset))
                    .filter(|res| res.is_ok())
                    .map(|res| res.unwrap())
                    .collect::<Vec<Coin>>();
                
                //Update delegate claimables
                DELEGATE_CLAIMS.update(storage, delegate.delegate.clone(), |claims| -> StdResult<(Vec<Coin>, Uint128)>{
                    match claims {
                        Some((mut claims, mut interest)) => {
                            //Combine claimables with similar denoms
                            for new_claim in delegate_claimables.clone() {
                                if let Some((i, _)) = claims.clone().into_iter().enumerate().find(|(_, claim)| claim.denom == new_claim.denom) {
                                    claims[i].amount += new_claim.amount;
                                } else {
                                    claims.push(new_claim);
                                }
                            }

                            //Add MBRN interest
                            interest += delegate_accrued_interest;        

                            Ok((claims, interest))
                        },
                        None => Ok((delegate_claimables, delegate_accrued_interest))
                    }
                })?;                                        
            }

            //Filter out empty claimables
            claimables = claimables
                .into_iter()
                .filter(|claimable| claimable.amount != Uint128::zero())
                .collect::<Vec<Asset>>();

        }   return Ok((claimables, accrued_interest))

    } else if config.vesting_contract.is_some() && user == config.clone().vesting_contract.unwrap().to_string() {
        //Load total vesting, altered by the vesting rev multiplier
        let total = STAKING_TOTALS.load(storage)?
            .vesting_contract;
        //Transform total with vesting rev multiplier
        let total = decimal_multiplication(
            Decimal::from_ratio(total, Uint128::one()),
            config.vesting_rev_multiplier)?
        .to_uint_floor();
                    
        let mut claimables = vec![];

        let temp_deposit = StakeDeposit {
            staker: Addr::unchecked(config.clone().vesting_contract.unwrap().to_string()),
            amount: total,
            stake_time: VESTING_STAKE_TIME.load(storage)?,
            unstake_start_time: None,
            last_accrued: None,
        };

        //Save new vesting multiplier to config if necessary
        if let Ok(multiplier) = VESTING_REV_MULTIPLIER.load(storage){
            config.vesting_rev_multiplier = multiplier;
            CONFIG.save(storage, &config)?;
        };

        //Set new vesting stake time to move up claims & to put it past claimed events
        VESTING_STAKE_TIME.save(storage, &env.block.time.seconds())?;

        let (claims, _) = get_deposit_claimables(
            storage, 
            config.clone(), 
            incentive_schedule.clone(), 
            env.clone(), 
            fee_events.clone(), 
            temp_deposit,
            vec![],
            vec![],
            total,
            Decimal::zero(),
        )?;
        claimables.extend(claims);

        //Filter out empty claimables
        claimables = claimables
            .into_iter()
            .filter(|claimable| claimable.amount != Uint128::zero())
            .collect::<Vec<Asset>>();

        return Ok((claimables, Uint128::zero()))
    }

    Ok((vec![], Uint128::zero()))
}

/// Trim fee events to only include events after the earliest deposit
fn trim_fee_events(
    storage: &mut dyn Storage,
    info: MessageInfo,
) -> Result<Response, ContractError>{

    let config = CONFIG.load(storage)?;

    if info.sender != config.owner { return Err( ContractError::Unauthorized {  } )}

    let mut fee_events = FEE_EVENTS.load(storage)?;

    //Initialize earliest deposit
    let mut earliest_accrue = None;

    let _iter = STAKED
        .range(storage, None, None, cosmwasm_std::Order::Ascending)
        .map(|stakers| {
            let (_, deposits) = stakers.unwrap();

            //Set earliest accrue to first deposit
            let mut earliest_accrue_loop = deposits[0].clone().last_accrued.unwrap_or_else(|| deposits[0].stake_time);

            //Find the earliest deposit
            for deposit in deposits {
                if deposit.last_accrued.unwrap_or_else(|| deposit.stake_time) < earliest_accrue_loop {
                    earliest_accrue_loop = deposit.last_accrued.unwrap_or_else(|| deposit.stake_time);
                }
            }

            earliest_accrue = Some(earliest_accrue_loop);
        })
        .collect::<Vec<()>>();

    //Filter for fee events that are after the earliest last_accrued to trim state
    if let Some(earliest_accrue) = earliest_accrue{
        fee_events = fee_events.clone()
            .into_iter()
            .filter(|event| event.time_of_event > earliest_accrue)
            .collect::<Vec<FeeEvent>>();
    }
    //In a situation where no one is staked the contract will need to be upgraded to handle its assets
    //This won't happen as long as the builder's allocation is vesting so the functionality isn't necessary rn
    
    //Save Fee events
    FEE_EVENTS.save(storage, &fee_events)?;

    Ok(Response::new().add_attribute("trimmed", "true"))
}

pub fn get_delegation_commission(
    storage: &dyn Storage,
    delegated: Vec<Delegation>,
    delegated_to: Vec<Delegation>,
    total_rewarding_stake: Uint128,
    user_commission_rate: Decimal,
) -> StdResult<Decimal>{
    if total_rewarding_stake == Uint128::zero() || (delegated.is_empty() && delegated_to.is_empty()){
        return Ok(Decimal::zero())
    }

    //Initialize the total the amount of MBRN delegated_to a delegate
    let mut total_delegated_to = Uint128::zero();

    //Get the average commission rate of the delegations
    let commission_rate = {
        let mut commission_rates: Vec<(Decimal, Uint128)> = vec![];

        //Create tuples for (Commission rate + total delegated_to) for each delegate
        for delegation in delegated_to.clone() {
            let delegator_commission = DELEGATIONS.load(storage, delegation.delegate.clone())?.commission;

            let like_delegations = delegated_to.clone()
                .into_iter()
                .filter(|listed_delegation| listed_delegation.delegate == delegation.delegate)
                .collect::<Vec<Delegation>>();

            let delegated_to_sum: Uint128 = like_delegations
                .into_iter()
                .map(|delegation| delegation.amount)
                .collect::<Vec<Uint128>>()
                .iter()
                .sum();
            total_delegated_to += delegated_to_sum;

            commission_rates.push((delegator_commission, delegated_to_sum));
        }

        //Get the average commission rate, weighted by the amount delegated_to
        let mut weighted_commission_rate = Decimal::zero();
        for (commission_rate, delegated_to) in commission_rates {
            weighted_commission_rate += commission_rate * Decimal::from_ratio(delegated_to, total_delegated_to);
        }

        weighted_commission_rate
    };
    
    //Calc the ratio of the total delegated_to to the total stake
    //Largest it can be is 1
    // println!("total_delegated_to: {}, {}", total_delegated_to, total_rewarding_stake);
    let total_delegated_ratio = min(Decimal::from_ratio(total_delegated_to, total_rewarding_stake), Decimal::one());
    // println!("total_delegated_ratio: {}", total_delegated_ratio);

    //Calculate the per deposit commission rate
    let per_deposit_commission_subtraction = decimal_multiplication(total_delegated_ratio, commission_rate)?;

    Ok(per_deposit_commission_subtraction)
}

/// Get deposit's claimable fee assets based on which FeeEvents it experienced
pub fn get_deposit_claimables(
    storage: &dyn Storage,
    mut config: Config,
    incentive_schedule: StakeDistributionLog,
    env: Env,
    fee_events: Vec<FeeEvent>,
    mut deposit: StakeDeposit,
    delegated: Vec<Delegation>,
    delegated_to: Vec<Delegation>,
    total_rewarding_stake: Uint128, //stake thats being rewarded
    user_commission_rate: Decimal,
) -> StdResult<(Vec<Asset>, Uint128)> {
    let mut claimables: Vec<Asset> = vec![];

    //Filter for events that the deposit was staked for
    //ie event times after the deposit 
    let events_experienced = fee_events
        .into_iter()
        .filter(|event| event.time_of_event > deposit.last_accrued.unwrap_or_else(|| deposit.stake_time) && event.time_of_event <= env.block.time.seconds())
        .collect::<Vec<FeeEvent>>();
        
    //Filter for delegations who were accrued before the current_time
    let delegated = delegated
        .into_iter()
        .filter(|delegation| delegation.last_accrued.unwrap_or_else(|| delegation.time_of_delegation) < env.block.time.seconds())
        .collect::<Vec<Delegation>>();
    
    let delegated_to = delegated_to
        .into_iter()
        .filter(|delegation| delegation.last_accrued.unwrap_or_else(|| delegation.time_of_delegation) < env.block.time.seconds())
        .collect::<Vec<Delegation>>();
    // println!("total_rewarding stake {:?}, {:?}, {:?}", total_rewarding_stake, delegated.clone(), delegated_to.clone());
    //Get commission rates per deposit
    let (per_deposit_commission_subtraction) = get_delegation_commission(
        storage, 
        delegated.clone(), 
        delegated_to.clone(), 
        total_rewarding_stake,
        user_commission_rate,
    )?;
    // println!("total_rewarding stake fin");

    
    //Subtract commission from deposit
    if per_deposit_commission_subtraction > Decimal::zero() {
        deposit.amount = decimal_multiplication(
            (Decimal::one() - per_deposit_commission_subtraction), 
            Decimal::from_ratio(deposit.amount, Uint128::one())
        )?.to_uint_floor();
    }

    //Calc & condense claimables
    //due to the above, claims incorporate the delegation commissions (additions are done separately)
    for event in events_experienced {
        //Check if asset is already in the list of claimables and add accordingly
        match claimables
            .clone()
            .into_iter()
            .enumerate()
            .find(|(_i, asset)| asset.info == event.fee.info)
        {
            Some((index, _asset)) => claimables[index].amount += event.fee.amount * deposit.amount,
            None => claimables.push(Asset {
                info: event.fee.info,
                amount: event.fee.amount * deposit.amount,
            }),
        }
    }

    //Assert staking rate is still active, if not set to 0
    let rate_duration = incentive_schedule.ownership_distribution.duration * SECONDS_PER_DAY;
    if env.block.time.seconds() - incentive_schedule.start_time > rate_duration {
        config.incentive_schedule.rate = Decimal::zero();
    }

    //Calc MBRN denominated rewards
    let deposit_interest = if !config.incentive_schedule.rate.is_zero() {
        let time_elapsed = env.block.time.seconds() - deposit.last_accrued.unwrap_or_else(|| deposit.stake_time);        
        accumulate_interest(deposit.amount, config.incentive_schedule.rate, time_elapsed)?
    } else {
        Uint128::zero()
    };

    Ok((claimables, deposit_interest))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::UserStake { staker } => to_binary(&query_user_stake(deps, staker)?),
        QueryMsg::UserRewards { user } => to_binary(&query_user_rewards(deps, env, user)?),
        QueryMsg::Staked {
            limit,
            start_after,
            end_before,
            unstaking,
        } => to_binary(&query_staked(
            deps,
            env,
            limit,
            start_after,
            end_before,
            unstaking,
        )?),
        QueryMsg::Delegations { limit, start_after, end_before, user } => {
            to_binary(&query_delegations(deps, env, limit, start_after, end_before, user)?)
        }
        QueryMsg::DeclaredDelegates { limit, start_after, end_before, user } => {
            to_binary(&query_declared_delegates(deps, env, limit, start_after, end_before, user)?)
        }
        QueryMsg::FeeEvents { limit, start_after } => {
            to_binary(&query_fee_events(deps, limit, start_after)?)
        }
        QueryMsg::TotalStaked {} => to_binary(&query_totals(deps)?),
        QueryMsg::IncentiveSchedule {  } => to_binary(&INCENTIVE_SCHEDULING.load(deps.storage)?),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    //Remove fee events
    FEE_EVENTS.remove(deps.storage);

    Ok(Response::default())
}

