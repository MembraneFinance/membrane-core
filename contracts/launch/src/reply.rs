use std::convert::TryInto;
use std::str::FromStr;

use cosmwasm_std::{
    to_binary, Decimal, DepsMut, Env, WasmMsg, WasmQuery,
    Response, StdResult, Uint128, Reply, StdError, CosmosMsg, SubMsg, coins, QueryRequest, BankMsg,
};
use membrane::math::Uint256;
use crate::error::ContractError;
use crate::contracts::{SECONDS_PER_DAY, POSITIONS_REPLY_ID, DEBT_AUCTION_REPLY_ID, SYSTEM_DISCOUNTS_REPLY_ID, DISCOUNT_VAULT_REPLY_ID, CREATE_DENOM_REPLY_ID, ORACLE_REPLY_ID, STAKING_REPLY_ID, VESTING_REPLY_ID, LIQ_QUEUE_REPLY_ID, GOVERNANCE_REPLY_ID, STABILITY_POOL_REPLY_ID ,LIQUIDITY_CHECK_REPLY_ID, NO_ACTION_ID};
use crate::state::{ADDRESSES, CONFIG, MBRN_POOL, OSMO_POOL_ID};

use membrane::governance::{InstantiateMsg as Gov_InstantiateMsg, VOTING_PERIOD_INTERVAL, STAKE_INTERVAL};
use membrane::stability_pool::InstantiateMsg as SP_InstantiateMsg;
use membrane::staking::{InstantiateMsg as Staking_InstantiateMsg, ExecuteMsg as StakingExecuteMsg};
use membrane::vesting::{InstantiateMsg as Vesting_InstantiateMsg, ExecuteMsg as VestingExecuteMsg};
use membrane::cdp::{InstantiateMsg as CDP_InstantiateMsg, EditBasket, ExecuteMsg as CDPExecuteMsg, QueryMsg as CDPQueryMsg, UpdateConfig as CDPUpdateConfig, CreateBasket};
use membrane::oracle::{InstantiateMsg as Oracle_InstantiateMsg, ExecuteMsg as OracleExecuteMsg};
use membrane::liq_queue::{InstantiateMsg as LQInstantiateMsg, ExecuteMsg as LQExecuteMsg};
use membrane::liquidity_check::{InstantiateMsg as LCInstantiateMsg, ExecuteMsg as LCExecuteMsg};
use membrane::auction::{InstantiateMsg as DAInstantiateMsg, ExecuteMsg as DAExecuteMsg, UpdateConfig as AuctionUpdateConfig};
use membrane::osmosis_proxy::{ExecuteMsg as OPExecuteMsg, QueryMsg as OPQueryMsg, ContractDenomsResponse};
use membrane::system_discounts::InstantiateMsg as SystemDiscountInstantiateMsg;
use membrane::discount_vault::{InstantiateMsg as DiscountVaultInstantiateMsg, ExecuteMsg as DiscountVaultExecuteMsg};
use membrane::types::{AssetInfo, Basket, AssetPool, Asset, PoolInfo, LPAssetInfo, cAsset, TWAPPoolInfo, SupplyCap, AssetOracleInfo, PoolStateResponse, Owner, LiquidityInfo, PoolType};

use osmosis_std::shim::{Duration, Timestamp};
use osmosis_std::types::cosmos::base::v1beta1::Coin;
use osmosis_std::types::osmosis::gamm::poolmodels::balancer::v1beta1::MsgCreateBalancerPoolResponse;
use osmosis_std::types::osmosis::incentives::MsgCreateGauge;
use osmosis_std::types::osmosis::lockup::QueryCondition;

//Governance constants
const PROPOSAL_VOTING_PERIOD: u64 = *VOTING_PERIOD_INTERVAL.start();
const PROPOSAL_EFFECTIVE_DELAY: u64 = 1; //1 day
const PROPOSAL_EXPIRATION_PERIOD: u64 = *VOTING_PERIOD_INTERVAL.end(); //14 days
const PROPOSAL_REQUIRED_STAKE: u128 = *STAKE_INTERVAL.start();
const PROPOSAL_REQUIRED_QUORUM: &str = "0.33";
const PROPOSAL_REQUIRED_THRESHOLD: &str = "0.66";


/// Create Membrane denoms and instantiate oracle contract
pub fn handle_op_reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save Osmosis Proxy address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.osmosis_proxy = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;

            let mut sub_msgs = vec![];

            //Create CDT & MBRN denom
            let create_denom_msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().osmosis_proxy.to_string(), 
                msg: to_binary(&OPExecuteMsg::CreateDenom { 
                    subdenom: String::from("ucdt"), 
                    max_supply: None,
                })?, 
                funds: coins(10_000_000, "uosmo"),
            });            
            let create_denom_submsg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().osmosis_proxy.to_string(), 
                msg: to_binary(&OPExecuteMsg::CreateDenom { 
                    subdenom: String::from("umbrn"), 
                    max_supply: None,
                })?, 
                funds: coins(10_000_000, "uosmo"),
            });
            sub_msgs.push(SubMsg::reply_on_success(create_denom_submsg, CREATE_DENOM_REPLY_ID));

            //Instantiate Oracle
            let oracle_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(env.contract.address.to_string()), 
                code_id: config.clone().oracle_id, 
                msg: to_binary(&Oracle_InstantiateMsg {
                    owner: None,
                    positions_contract: None,
                    osmosis_proxy_contract: None,
                    oracle_contract: None,
                })?, 
                funds: vec![], 
                label: String::from("oracle"), 
            });
            sub_msgs.push(SubMsg::reply_on_success(oracle_instantiation, ORACLE_REPLY_ID));
            
            Ok(Response::new()
                .add_message(create_denom_msg)
                .add_submessages(sub_msgs)
            )
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}


/// Called after the Osmosis Proxy (OP) reply to save created denoms
pub fn handle_create_denom_reply(deps: DepsMut, _env: Env, msg: Reply) -> StdResult<Response>{ 
    match msg.result.into_result() {
        Ok(_result) => {
        let mut config = CONFIG.load(deps.storage)?;
        let addrs = ADDRESSES.load(deps.storage)?;
        
        //Get denoms
        let res: ContractDenomsResponse = deps.querier.query_wasm_smart(addrs.osmosis_proxy, &OPQueryMsg::GetContractDenoms { limit: None })?;
        //We know CDT is first
        config.credit_denom = res.denoms[0].clone();
        config.mbrn_denom = res.denoms[1].clone();

        //Save config
        CONFIG.save(deps.storage, &config)?;

        Ok(Response::new()
            .add_attribute("saved_denoms", format!("{:?}", res.denoms))
        )
    },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}


/// Instantiate Staking Contract
pub fn handle_oracle_reply(deps: DepsMut, env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save Oracle address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.oracle = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;

            //Instantiate Staking
            let staking_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(env.contract.address.to_string()), 
                code_id: config.clone().staking_id, 
                msg: to_binary(&Staking_InstantiateMsg {
                    owner: None,
                    positions_contract: None,
                    auction_contract: None,
                    vesting_contract: None,
                    governance_contract: None,
                    osmosis_proxy: Some(addrs.osmosis_proxy.to_string()),
                    incentive_schedule: None,
                    unstaking_period: None,
                    mbrn_denom: config.clone().mbrn_denom,
                })?, 
                funds: vec![], 
                label: String::from("staking"), 
            });
            let sub_msg = SubMsg::reply_on_success(staking_instantiation, STAKING_REPLY_ID);
            
            Ok(Response::new().add_submessage(sub_msg))
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Instantiate Vesting Contract
pub fn handle_staking_reply(deps: DepsMut, env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save Staking address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.staking = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;

            //Instantiate Vesting
            let vesting_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(env.contract.address.to_string()), 
                code_id: config.clone().vesting_id, 
                msg: to_binary(&Vesting_InstantiateMsg {
                    owner: None,
                    initial_allocation: Uint128::new(9_000_000_000_000),
                    pre_launch_contributors: config.clone().pre_launch_contributors.to_string(),
                    pre_launch_community: config.clone().pre_launch_community,
                    mbrn_denom: config.clone().mbrn_denom,
                    osmosis_proxy: addrs.clone().osmosis_proxy.to_string(),
                    staking_contract: addrs.clone().staking.to_string(),
                })?, 
                funds: vec![], 
                label: String::from("vesting"), 
            });
            let sub_msg = SubMsg::reply_on_success(vesting_instantiation, VESTING_REPLY_ID);            
            Ok(Response::new().add_submessage(sub_msg))
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Instantiate Governance Contract
pub fn handle_vesting_reply(deps: DepsMut, env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save Vesting address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.vesting = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;

            //Instantiate Gov
            let gov_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(env.contract.address.to_string()), 
                code_id: config.clone().governance_id, 
                msg: to_binary(&Gov_InstantiateMsg {
                    mbrn_staking_contract_addr: addrs.clone().staking.to_string(),
                    vesting_contract_addr: addrs.clone().vesting.to_string(),
                    vesting_voting_power_multiplier: Decimal::percent(50),
                    proposal_voting_period: PROPOSAL_VOTING_PERIOD * 7, //7 days
                    expedited_proposal_voting_period: PROPOSAL_VOTING_PERIOD * 3, //3 days
                    proposal_effective_delay: PROPOSAL_EFFECTIVE_DELAY,
                    proposal_expiration_period: PROPOSAL_EXPIRATION_PERIOD,
                    proposal_required_stake: Uint128::from(PROPOSAL_REQUIRED_STAKE),
                    proposal_required_quorum: String::from(PROPOSAL_REQUIRED_QUORUM),
                    proposal_required_threshold: String::from(PROPOSAL_REQUIRED_THRESHOLD),
                    whitelisted_links: vec![
                        String::from("https://discord.com/channels/1060217330258432010/"),
                        String::from("https://commonwealth.im/membrane/")
                        ],
                })?, 
                funds: vec![], 
                label: String::from("governance"), 
            });
            let sub_msg = SubMsg::reply_on_success(gov_instantiation, GOVERNANCE_REPLY_ID);            
            Ok(Response::new().add_submessage(sub_msg))
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Instantiate Positions Contract & update existing contract admins to Governance
pub fn handle_gov_reply(deps: DepsMut, _env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save Gov address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.governance = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;

            let mut msgs = vec![];
            //Update previous contract admins to Governance
            msgs.push(CosmosMsg::Wasm(WasmMsg::UpdateAdmin { 
                contract_addr: addrs.osmosis_proxy.to_string(), 
                admin: addrs.clone().governance.to_string(),
            }));
            msgs.push(CosmosMsg::Wasm(WasmMsg::UpdateAdmin { 
                contract_addr: addrs.oracle.to_string(), 
                admin: addrs.clone().governance.to_string(),
            }));
            msgs.push(CosmosMsg::Wasm(WasmMsg::UpdateAdmin { 
                contract_addr: addrs.staking.to_string(), 
                admin: addrs.clone().governance.to_string(),
            }));
            msgs.push(CosmosMsg::Wasm(WasmMsg::UpdateAdmin { 
                contract_addr: addrs.vesting.to_string(), 
                admin: addrs.clone().governance.to_string(),
            }));
            msgs.push(CosmosMsg::Wasm(WasmMsg::UpdateAdmin { 
                contract_addr: addrs.governance.to_string(), 
                admin: addrs.clone().governance.to_string(),
            }));

            let mut sub_msgs = vec![];             
            //Add Collateral Oracles
            /// ATOM
            let msg = 
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().oracle.to_string(), 
                    msg: to_binary(&OracleExecuteMsg::AddAsset { 
                        asset_info: AssetInfo::NativeToken { denom: config.clone().atom_denom }, 
                        oracle_info: AssetOracleInfo { 
                            basket_id: Uint128::one(), 
                            pools_for_osmo_twap: vec![
                                //ATOM/OSMO
                                TWAPPoolInfo { 
                                    pool_id: config.clone().atomosmo_pool_id, 
                                    base_asset_denom: config.clone().atom_denom.to_string(), 
                                    quote_asset_denom: config.clone().osmo_denom.to_string(),  
                                }
                            ],
                            is_usd_par: false,
                            lp_pool_info: None,
                            decimals: 6,
                            pyth_price_feed_id: Some(String::from("b00b60f88b03a6a625a8d1c048c3f66653edf217439983d037e7222c4e612819")),
                            vault_info: None,
                        },
                    })?, 
                    funds: vec![],
                });
            sub_msgs.push(SubMsg::new(msg));
            /// OSMO
            let msg =
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().oracle.to_string(), 
                    msg: to_binary(&OracleExecuteMsg::AddAsset { 
                        asset_info: AssetInfo::NativeToken { denom: config.clone().osmo_denom }, 
                        oracle_info: AssetOracleInfo { 
                            basket_id: Uint128::one(), 
                            pools_for_osmo_twap: vec![],
                            is_usd_par: false,
                            lp_pool_info: None,
                            decimals: 6,
                            pyth_price_feed_id: Some(String::from("5867f5683c757393a0670ef0f701490950fe93fdb006d181c8265a831ac0c5c6")),
                            vault_info: None,
                        },
                    })?, 
                    funds: vec![],
                });
            sub_msgs.push(SubMsg::new(msg));
            /// axlUSDC
            let msg = 
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().oracle.to_string(), 
                    msg: to_binary(&OracleExecuteMsg::AddAsset { 
                        asset_info: AssetInfo::NativeToken { denom: config.clone().usdc_denom }, 
                        oracle_info: AssetOracleInfo { 
                            basket_id: Uint128::one(), 
                            pools_for_osmo_twap: vec![
                                TWAPPoolInfo { 
                                    pool_id: config.clone().osmousdc_pool_id,
                                    base_asset_denom: config.clone().usdc_denom.to_string(), 
                                    quote_asset_denom: config.clone().osmo_denom.to_string(),  
                                }
                            ],
                            is_usd_par: true,
                            lp_pool_info: None,
                            decimals: 6,
                            pyth_price_feed_id: None, //We don't set a pyth price feed for axlUSDC bc its a non-IBC bridged asset
                            vault_info: None,
                        },
                    })?, 
                    funds: vec![],
                });
            sub_msgs.push(SubMsg::new(msg));

            //Set CreaetBasket struct
            let create_basket = CreateBasket {
                basket_id: Uint128::one(),
                collateral_types: vec![cAsset {
                    asset: Asset {
                        info: AssetInfo::NativeToken {
                            denom: config.clone().atom_denom,
                        },
                        amount: Uint128::from(0u128),
                    },
                    max_borrow_LTV: Decimal::percent(45),
                    max_LTV: Decimal::percent(60),
                    pool_info: None,
                    rate_index: Decimal::one(),
                    hike_rates: Some(false),
                },
                cAsset {
                    asset: Asset {
                        info: AssetInfo::NativeToken {
                            denom: config.clone().osmo_denom,
                        },
                        amount: Uint128::from(0u128),
                    },
                    max_borrow_LTV: Decimal::percent(45),
                    max_LTV: Decimal::percent(60),
                    pool_info: None,
                    rate_index: Decimal::one(),
                    hike_rates: Some(false),
                },                
                cAsset {
                    asset: Asset {
                        info: AssetInfo::NativeToken {
                            denom: config.clone().usdc_denom,
                        },
                        amount: Uint128::from(0u128),
                    },
                    max_borrow_LTV: Decimal::percent(90),
                    max_LTV: Decimal::percent(96),
                    pool_info: None,
                    rate_index: Decimal::one(),
                    hike_rates: Some(false),
                }],
                credit_asset: Asset {
                    info: AssetInfo::NativeToken {
                        denom: config.clone().credit_denom,
                    },
                    amount: Uint128::from(0u128),
                },
                credit_price: Decimal::one(),
                base_interest_rate: Some(Decimal::percent(1)),
                credit_pool_infos: vec![],
                liq_queue: None,
            };
            
            //Instantiate Positions
            let cdp_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(addrs.clone().governance.to_string()), 
                code_id: config.clone().positions_id, 
                msg: to_binary(&CDP_InstantiateMsg {
                    owner: None,
                    liq_fee: Decimal::percent(1),
                    oracle_time_limit: 600u64, //10 mins
                    debt_minimum: Uint128::new(100u128),
                    collateral_twap_timeframe: 60u64,
                    credit_twap_timeframe: 480u64,
                    rate_slope_multiplier: Decimal::from_str("0.618").unwrap(),
                    base_debt_cap_multiplier: Uint128::new(1000_000_000u128), //1000 positions of the debt minimum + 6 decimal points
                    stability_pool: None,
                    dex_router: Some(config.clone().apollo_router.to_string()),
                    staking_contract: Some(addrs.clone().staking.to_string()),
                    oracle_contract: Some(addrs.clone().oracle.to_string()),
                    osmosis_proxy: Some(addrs.clone().osmosis_proxy.to_string()),
                    debt_auction: None,
                    liquidity_contract: None,
                    discounts_contract: None,
                    create_basket,
                })?, 
                funds: vec![], 
                label: String::from("positions"), 
            });
            let sub_msg = SubMsg::reply_on_success(cdp_instantiation, POSITIONS_REPLY_ID);
            sub_msgs.push(sub_msg);
            
            
            Ok(Response::new()
                .add_submessages(sub_msgs)
                .add_messages(msgs)
            )
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Add initial collateral oracles & create Basket with initial collateral types.
/// Instantiate Stability Pool contract.
pub fn handle_cdp_reply(deps: DepsMut, _env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save CDP address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.positions = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;         

            //CreateBasket
            // let msg = CDPExecuteMsg::CreateBasket {
            //     basket_id: Uint128::one(),
            //     collateral_types: vec![cAsset {
            //         asset: Asset {
            //             info: AssetInfo::NativeToken {
            //                 denom: config.clone().atom_denom,
            //             },
            //             amount: Uint128::from(0u128),
            //         },
            //         max_borrow_LTV: Decimal::percent(45),
            //         max_LTV: Decimal::percent(60),
            //         pool_info: None,
            //         rate_index: Decimal::one(),
            //     },
            //     cAsset {
            //         asset: Asset {
            //             info: AssetInfo::NativeToken {
            //                 denom: config.clone().osmo_denom,
            //             },
            //             amount: Uint128::from(0u128),
            //         },
            //         max_borrow_LTV: Decimal::percent(45),
            //         max_LTV: Decimal::percent(60),
            //         pool_info: None,
            //         rate_index: Decimal::one(),
            //     },                
            //     cAsset {
            //         asset: Asset {
            //             info: AssetInfo::NativeToken {
            //                 denom: config.clone().usdc_denom,
            //             },
            //             amount: Uint128::from(0u128),
            //         },
            //         max_borrow_LTV: Decimal::percent(90),
            //         max_LTV: Decimal::percent(96),
            //         pool_info: None,
            //         rate_index: Decimal::one(),
            //     }],
            //     credit_asset: Asset {
            //         info: AssetInfo::NativeToken {
            //             denom: config.clone().credit_denom,
            //         },
            //         amount: Uint128::from(0u128),
            //     },
            //     credit_price: Decimal::one(),
            //     base_interest_rate: Some(Decimal::percent(1)),
            //     credit_pool_infos: vec![],
            //     liq_queue: None,
            // };
            // let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
            //     contract_addr: addrs.clone().positions.to_string(), 
            //     msg: to_binary(&msg)?, 
            //     funds: vec![], 
            // });
            // msgs.push(msg);

            //Instantiate SP
            let sp_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(addrs.clone().governance.to_string()), 
                code_id: config.clone().stability_pool_id, 
                msg: to_binary(&SP_InstantiateMsg {
                    owner: Some(addrs.clone().governance.to_string()),
                    asset_pool: AssetPool { 
                        credit_asset: Asset { info: AssetInfo::NativeToken { denom: config.clone().credit_denom }, amount: Uint128::zero()}, 
                        liq_premium: Decimal::percent(10), 
                        deposits: vec![] 
                    },
                    incentive_rate: None,
                    max_incentives: None,
                    minimum_deposit_amount: Uint128::new(5_000_000), //5
                    osmosis_proxy: addrs.clone().osmosis_proxy.to_string(),
                    positions_contract: addrs.clone().positions.to_string(),
                    oracle_contract: addrs.clone().oracle.to_string(),
                    mbrn_denom: config.clone().mbrn_denom,
                })?, 
                funds: vec![], 
                label: String::from("stability_pool"), 
            });
            let sub_msg = SubMsg::reply_on_success(sp_instantiation, STABILITY_POOL_REPLY_ID);    

            Ok(Response::new().add_submessage(sub_msg))
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Instantiate Liquidation Queue
pub fn handle_sp_reply(deps: DepsMut, _env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save Stability Pool address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.stability_pool = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;
                       
            //Instantiate Liquidation Queue
            let lq_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(addrs.clone().governance.to_string()), 
                code_id: config.clone().liq_queue_id, 
                msg: to_binary(&LQInstantiateMsg {
                    owner: None,
                    positions_contract: addrs.clone().positions.to_string(),
                    osmosis_proxy_contract: addrs.clone().osmosis_proxy.to_string(),
                    waiting_period: 60u64,
                    minimum_bid: Uint128::new(5_000_000), //5
                    maximum_waiting_bids: 5_000u64, //5,000
                })?, 
                funds: vec![], 
                label: String::from("liquidation_queue"), 
            });
            let sub_msg = SubMsg::reply_on_success(lq_instantiation, LIQ_QUEUE_REPLY_ID);
            
            Ok(Response::new()
                .add_submessage(sub_msg)
            )
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Add LQ to Basket alongside 1 LP & 3/4 SupplyCaps.
/// Instantiate Liquidity Check
pub fn handle_lq_reply(deps: DepsMut, _env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save LQ address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.liq_queue = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;

            let mut msgs = vec![];
            //Add positions contract to oracle contract to use EditBasket
            msgs.push(
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().oracle.to_string(), 
                    msg: to_binary(&OracleExecuteMsg::UpdateConfig { 
                        owner: None, 
                        positions_contract: Some(addrs.clone().positions.to_string()),
                        osmosis_proxy_contract: None, 
                        pyth_osmosis_address: None,
                        osmo_usd_pyth_feed_id: None,
                        pools_for_usd_par_twap: None,
                    })?, 
                    funds: vec![],
                }));
            
            //Add LQ to Basket alongside 1/2 LPs & 3/5 SupplyCaps
            let msg = CDPExecuteMsg::EditBasket(EditBasket {
                added_cAsset: Some(cAsset {
                    asset: Asset {
                        info: AssetInfo::NativeToken {
                            denom: config.clone().atomosmo_pool_id.to_string(), //This gets auto-filled
                        },
                        amount: Uint128::from(0u128),
                    },
                    max_borrow_LTV: Decimal::percent(45),
                    max_LTV: Decimal::percent(60),
                    pool_info: Some(PoolInfo { 
                        pool_id: config.clone().atomosmo_pool_id, 
                        asset_infos: vec![
                            LPAssetInfo { info: AssetInfo::NativeToken { denom: config.clone().atom_denom }, decimals: 6, ratio: Decimal::percent(50) },
                            LPAssetInfo { info: AssetInfo::NativeToken { denom: config.clone().osmo_denom }, decimals: 6, ratio: Decimal::percent(50) }], 
                    }),
                    rate_index: Decimal::one(),
                    hike_rates: Some(false),
                }),
                liq_queue: Some(addrs.clone().liq_queue.to_string()),
                collateral_supply_caps: Some(vec![
                SupplyCap {
                    asset_info: AssetInfo::NativeToken {
                        denom: config.clone().osmo_denom,
                    },
                    current_supply: Uint128::zero(),
                    debt_total: Uint128::zero(),
                    supply_cap_ratio: Decimal::percent(100),
                    lp: false,
                    stability_pool_ratio_for_debt_cap: None,
                },
                SupplyCap {
                    asset_info: AssetInfo::NativeToken {
                        denom: config.clone().atom_denom,
                    },
                    current_supply: Uint128::zero(),
                    debt_total: Uint128::zero(),
                    supply_cap_ratio: Decimal::percent(100),
                    lp: false,
                    stability_pool_ratio_for_debt_cap: None,
                },
                SupplyCap {
                    asset_info: AssetInfo::NativeToken {
                        denom: config.clone().usdc_denom,
                    },
                    current_supply: Uint128::zero(),
                    debt_total: Uint128::zero(),
                    supply_cap_ratio: Decimal::percent(100),
                    lp: false,
                    stability_pool_ratio_for_debt_cap: None,
                }]),
                base_interest_rate: None,
                credit_asset_twap_price_source: None,
                negative_rates: Some(false),
                cpc_margin_of_error: Some(Decimal::percent(1)),
                frozen: None,
                rev_to_stakers: None,
                multi_asset_supply_caps: None,
                credit_pool_infos: None,
                take_revenue: None,
                revenue_destinations: None,
            });
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().positions.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);
            //Add 2/2 LPs
            let msg = CDPExecuteMsg::EditBasket(EditBasket {
                added_cAsset: Some(cAsset {
                    asset: Asset {
                        info: AssetInfo::NativeToken {
                            denom: config.clone().osmousdc_pool_id.to_string(), //This gets auto-filled
                        },
                        amount: Uint128::from(0u128),
                    },
                    max_borrow_LTV: Decimal::percent(45),
                    max_LTV: Decimal::percent(60),
                    pool_info: Some(PoolInfo { 
                        pool_id: config.clone().osmousdc_pool_id, 
                        asset_infos: vec![
                            LPAssetInfo { info: AssetInfo::NativeToken { denom: config.clone().osmo_denom }, decimals: 6, ratio: Decimal::percent(50) },
                            LPAssetInfo { info: AssetInfo::NativeToken { denom: config.clone().usdc_denom }, decimals: 6, ratio: Decimal::percent(50) }], 
                    }),
                    rate_index: Decimal::one(),
                    hike_rates: Some(false),
                }),
                liq_queue: None,
                collateral_supply_caps: None,
                base_interest_rate: None,
                credit_asset_twap_price_source: None,
                negative_rates: None,
                cpc_margin_of_error: None,
                frozen: None,
                rev_to_stakers: None,
                multi_asset_supply_caps: None,
                credit_pool_infos: None,
                take_revenue: None,
                revenue_destinations: None,
            });
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().positions.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);

            //AddQueues for 3 initial collateral types
            //OSMO
            let msg = LQExecuteMsg::AddQueue { 
                bid_for: AssetInfo::NativeToken { denom: config.clone().osmo_denom }, 
                max_premium: Uint128::new(35), 
                bid_threshold: Uint256::from(1_000_000_000_000u128), 
            };
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().liq_queue.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);            
            //ATOM
            let msg = LQExecuteMsg::AddQueue { 
                bid_for: AssetInfo::NativeToken { denom: config.clone().atom_denom }, 
                max_premium: Uint128::new(35), 
                bid_threshold: Uint256::from(1_000_000_000_000u128), 
            };
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().liq_queue.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);
            //axlUSDC
            let msg = LQExecuteMsg::AddQueue { 
                bid_for: AssetInfo::NativeToken { denom: config.clone().usdc_denom }, 
                max_premium: Uint128::new(10), 
                bid_threshold: Uint256::from(1_000_000_000_000u128), 
            };
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().liq_queue.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);

            //Update LQ owner to governance
            let msg = LQExecuteMsg::UpdateConfig { 
                owner: Some(addrs.clone().governance.to_string()), 
                positions_contract: None,
                osmosis_proxy_contract: None,
                waiting_period: None, 
                minimum_bid: None, 
                maximum_waiting_bids: None 
            };
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().liq_queue.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);
            
            //Instantiate Liquidity Check
            let lc_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(addrs.clone().governance.to_string()), 
                code_id: config.clone().liquidity_check_id, 
                msg: to_binary(&LCInstantiateMsg {
                    osmosis_proxy: addrs.clone().osmosis_proxy.to_string(),   
                    owner: None,    
                    positions_contract: addrs.clone().positions.to_string(),             
                })?, 
                funds: vec![], 
                label: String::from("liquidity_check"), 
            });
            let sub_msg = SubMsg::reply_on_success(lc_instantiation, LIQUIDITY_CHECK_REPLY_ID);     
            
            
            Ok(Response::new()
                .add_messages(msgs)
                .add_submessage(sub_msg)
            )
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Instantiate Discount Vault
pub fn handle_lc_reply(deps: DepsMut, _env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save Liquidity Check address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.liquidity_check = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;
                       
            //Instantiate Discount Vault
            let discount_vault_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(addrs.clone().governance.to_string()), 
                code_id: config.clone().discount_vault_id, 
                msg: to_binary(&DiscountVaultInstantiateMsg {
                    owner: None,
                    positions_contract: addrs.clone().positions.to_string(),
                    osmosis_proxy: addrs.clone().osmosis_proxy.to_string(),
                    accepted_LPs: vec![],
                })?, 
                funds: vec![], 
                label: String::from("discount_vault"), 
            });
            let sub_msg = SubMsg::reply_on_success(discount_vault_instantiation, DISCOUNT_VAULT_REPLY_ID);     
            
            
            Ok(Response::new()
                .add_submessage(sub_msg)
            )
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Instantiate System Discounts
pub fn handle_discount_vault_reply(deps: DepsMut, _env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save Vault address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.discount_vault = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;
                       
            //Instantiate System Discounts
            let system_discounts_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(addrs.clone().governance.to_string()), 
                code_id: config.clone().system_discounts_id, 
                msg: to_binary(&SystemDiscountInstantiateMsg {
                    owner: Some(addrs.clone().governance.to_string()),
                    oracle_contract: addrs.clone().oracle.to_string(),
                    positions_contract: addrs.clone().positions.to_string(),
                    staking_contract: addrs.clone().staking.to_string(),
                    stability_pool_contract: addrs.clone().stability_pool.to_string(),
                    lockdrop_contract: None,
                    discount_vault_contract: Some(addrs.clone().discount_vault.to_string()),
                    minimum_time_in_network: 7, //in days
                })?, 
                funds: vec![], 
                label: String::from("system_discounts"), 
            });
            let sub_msg = SubMsg::reply_on_success(system_discounts_instantiation, SYSTEM_DISCOUNTS_REPLY_ID);     
            
            
            Ok(Response::new()
                .add_submessage(sub_msg)
            )
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Instantiate Debt Auction
pub fn handle_system_discounts_reply(deps: DepsMut, _env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => {
            let config = CONFIG.load(deps.storage)?;

            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;
            //Save System Discounts address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.system_discounts = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;
                       
            //Instantiate Debt Auction
            let da_instantiation = CosmosMsg::Wasm(WasmMsg::Instantiate { 
                admin: Some(addrs.clone().governance.to_string()), 
                code_id: config.clone().mbrn_auction_id, 
                msg: to_binary(&DAInstantiateMsg {
                    owner: None,
                    oracle_contract: addrs.clone().oracle.to_string(),
                    osmosis_proxy: addrs.clone().osmosis_proxy.to_string(),
                    positions_contract: addrs.clone().positions.to_string(),
                    governance_contract: addrs.clone().governance.to_string(),
                    staking_contract: addrs.clone().staking.to_string(),
                    twap_timeframe: 60u64,
                    mbrn_denom: config.clone().mbrn_denom,
                    initial_discount: Decimal::percent(1),
                    discount_increase_timeframe: 36, //Discount will hit 100% in 1 hour, 1.6667% per minute
                    discount_increase: Decimal::percent(1),
                })?, 
                funds: vec![], 
                label: String::from("auction"), 
            });
            let sub_msg = SubMsg::reply_on_success(da_instantiation, DEBT_AUCTION_REPLY_ID);     
            
            
            Ok(Response::new()
                .add_submessage(sub_msg)
            )
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Add Owners & contracts to the Osmosis Proxy.
/// Add contracts to contract configurations & change owners to Governance.
/// Query saved share tokens in Position's contract & add Supply Caps for them.
/// Instantiate Margin Proxy.
pub fn handle_auction_reply(deps: DepsMut, _env: Env, msg: Reply)-> StdResult<Response>{
    match msg.result.into_result() {
        Ok(result) => { 
            let config = CONFIG.load(deps.storage)?;            
            
            //Get contract address
            let instantiate_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "_contract_address")
                })
                .ok_or_else(|| {
                    StdError::generic_err(format!("unable to find instantiate event"))
                })?;

            let contract_address = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "_contract_address")
                .unwrap()
                .value;

            let valid_address = deps.api.addr_validate(&contract_address)?;

            //Save MBRN Auction address
            let mut addrs = ADDRESSES.load(deps.storage)?;
            addrs.mbrn_auction = valid_address.clone();
            ADDRESSES.save(deps.storage, &addrs)?;
            
            let mut msgs = vec![];

            //Add owners & new contracts to OP
            let msg = OPExecuteMsg::UpdateConfig { 
                owners: Some(vec![
                    Owner {
                        owner: addrs.clone().positions, 
                        total_minted: Uint128::zero(),
                        //Makes more sense to start low and scale up. The LM is our best capping mechanism. DAI's scaled with its Lindy and risk profile.
                        stability_pool_ratio: Some(Decimal::zero()), //CDP contracts gets 0% of the Stability Pool cap space initially, let the DAO decide later on. Reduction of scale at launch is fine.
                        non_token_contract_auth: false,
                        is_position_contract: true,
                    },
                    // No other owners mint CDT atm
                    Owner {
                        owner: addrs.clone().vesting, 
                        total_minted: Uint128::zero(),
                        stability_pool_ratio: None,
                        non_token_contract_auth: false,
                        is_position_contract: false,
                    },
                    Owner {
                        owner: addrs.clone().staking, 
                        total_minted: Uint128::zero(),
                        stability_pool_ratio: None,
                        non_token_contract_auth: false,
                        is_position_contract: false,
                    },
                    Owner {
                        owner: addrs.clone().stability_pool, 
                        total_minted: Uint128::zero(),
                        stability_pool_ratio: None,
                        non_token_contract_auth: false,
                        is_position_contract: false,
                    },
                    Owner {
                        owner: addrs.clone().liq_queue,  //For repayment burns
                        total_minted: Uint128::zero(),
                        stability_pool_ratio: None,
                        non_token_contract_auth: false,
                        is_position_contract: false,
                    },
                    Owner {
                        owner: addrs.clone().governance, 
                        total_minted: Uint128::zero(),
                        stability_pool_ratio: None,
                        non_token_contract_auth: true, //Governance has full control over the system & mints CDT for revenue distributions (no it doesn't)
                        is_position_contract: false,
                    },
                    Owner {
                        owner: addrs.clone().mbrn_auction, 
                        total_minted: Uint128::zero(),
                        stability_pool_ratio: None,
                        non_token_contract_auth: false,
                        is_position_contract: false,
                    }
                    ]), 
                liquidity_multiplier: None,
                add_owner: Some(true), 
                debt_auction: Some(addrs.clone().mbrn_auction.to_string()), 
                positions_contract: Some(addrs.clone().positions.to_string()), 
                liquidity_contract: Some(addrs.clone().liquidity_check.to_string()),
                oracle_contract: Some(addrs.clone().oracle.to_string()),
                edit_routes: None,
            };
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().osmosis_proxy.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);
            
            ////Add contracts to contract configurations & change owners to Governance
            //Staking
            msgs.push(
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().staking.to_string(), 
                    msg: to_binary(&StakingExecuteMsg::UpdateConfig { 
                        owner: Some(addrs.clone().governance.to_string()), 
                        positions_contract: Some(addrs.clone().positions.to_string()),
                        auction_contract: Some(addrs.clone().mbrn_auction.to_string()),
                        osmosis_proxy: None,
                        vesting_contract: Some(addrs.clone().vesting.to_string()),
                        governance_contract: Some(addrs.clone().governance.to_string()),
                        mbrn_denom: None,
                        incentive_schedule: None,
                        unstaking_period: None,
                        max_commission_rate: None,
                        keep_raw_cdt: None,
                        vesting_rev_multiplier: None,
                    })?, 
                    funds: vec![],
                }));
            //Vesting
            msgs.push(
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().vesting.to_string(), 
                    msg: to_binary(&VestingExecuteMsg::UpdateConfig { 
                        owner: Some(addrs.clone().governance.to_string()), 
                        osmosis_proxy: None,
                        mbrn_denom: None,
                        staking_contract: None,
                        additional_allocation: None, 
                    })?, 
                    funds: vec![],
                }));
            
            /////Query saved share tokens in Position's contract & add Supply Caps for them
            let basket: Basket = deps.querier.query_wasm_smart(
                addrs.clone().positions.to_string(), 
            &CDPQueryMsg::GetBasket {  }
            )?;
            let lp_supply_caps = basket.clone().collateral_types
                .into_iter()
                .filter(|cAsset| cAsset.pool_info.is_some())
                .collect::<Vec<cAsset>>()
                .into_iter()
                .map(|cAsset| SupplyCap {
                    asset_info: cAsset.asset.info,
                    current_supply: Uint128::zero(),
                    debt_total: Uint128::zero(),
                    supply_cap_ratio: Decimal::one(),
                    lp: true,
                    stability_pool_ratio_for_debt_cap: Some(Decimal::percent(33)),
                })
                .collect::<Vec<SupplyCap>>();
            
            let msg = CDPExecuteMsg::EditBasket(EditBasket {
                added_cAsset: None,
                liq_queue: None,
                collateral_supply_caps: Some(lp_supply_caps),
                base_interest_rate: None,
                credit_asset_twap_price_source: None,
                negative_rates: Some(false),
                cpc_margin_of_error: None,
                frozen: None,
                rev_to_stakers: None,
                multi_asset_supply_caps: None,
                credit_pool_infos: None,
                take_revenue: None,
                revenue_destinations: None,
            });
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().positions.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);

            
            // Add USD Par TWAP pool to oracle
            msgs.push(
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().oracle.to_string(), 
                    msg: to_binary(&OracleExecuteMsg::UpdateConfig { 
                        owner: None, 
                        positions_contract: None,
                        osmosis_proxy_contract: Some(addrs.clone().osmosis_proxy.to_string()),
                        pyth_osmosis_address: None,
                        osmo_usd_pyth_feed_id: None,
                        pools_for_usd_par_twap: Some(vec![
                            TWAPPoolInfo { 
                                pool_id: config.clone().osmousdc_pool_id, 
                                base_asset_denom: config.clone().osmo_denom.to_string(), 
                                quote_asset_denom: config.clone().usdc_denom.to_string(),  
                            }
                        ])
                    })?, 
                    funds: vec![],
                }));

            Ok(Response::new()
                .add_messages(msgs)
            )
        },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}

/// Save Balancer Pool ID
pub fn handle_balancer_reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response>{
    match msg.clone().result.into_result() {
        Ok(result) => {
        let mut osmo_pool_id = OSMO_POOL_ID.load(deps.storage)?;
        let addrs = ADDRESSES.load(deps.storage)?;
        let config = CONFIG.load(deps.storage)?;

        let mut sub_msgs: Vec<SubMsg> = vec![];
        let mut msgs: Vec<CosmosMsg> = vec![];
        
        //Get Balancer Pool denom from Response
        if let Some(b) = result.data {
            let res: MsgCreateBalancerPoolResponse = match b.try_into().map_err(ContractError::Std){
                Ok(res) => res,
                Err(err) => return Err(StdError::GenericErr { msg: String::from(err.to_string()) })
            };

            //Save Pool ID if unsaved
            //this is the first replying message so skip the rest of the logic
            if let Err(_) = MBRN_POOL.load(deps.storage){
                MBRN_POOL.save(deps.storage, &res.pool_id)?;

                return Ok(Response::new()
                    .add_attribute("pool_saved", res.pool_id.to_string())
                )
            }
            
            //Save Pool ID
            //OSMO pool replies 2nd
            if osmo_pool_id == 0 {
                osmo_pool_id = res.pool_id;

                //Mint MBRN for Incentives
                let op_msg = OPExecuteMsg::MintTokens { 
                    denom: config.clone().mbrn_denom, 
                    amount: Uint128::new(2_000_000_000_000), 
                    mint_to_address: env.clone().contract.address.to_string(),
                };
                let op_msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().osmosis_proxy.to_string(), 
                    msg: to_binary(&op_msg)?, 
                    funds: vec![], 
                });
                sub_msgs.push(SubMsg::reply_on_error(op_msg, NO_ACTION_ID));
                
                //Get Balancer denom from Response
                let pool_denom = deps.querier.query::<PoolStateResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
                    contract_addr: addrs.clone().osmosis_proxy.to_string(), 
                    msg: to_binary(&OPQueryMsg::PoolState {
                        id: res.pool_id,
                    })?,
                }))?.shares.denom;
                
                //Set the CDT/OSMO LP denom oracle
                msgs.push(
                    CosmosMsg::Wasm(WasmMsg::Execute { 
                        contract_addr: addrs.clone().oracle.to_string(), 
                        msg: to_binary(&OracleExecuteMsg::AddAsset { 
                            asset_info: AssetInfo::NativeToken { denom: pool_denom.clone() }, 
                            oracle_info: AssetOracleInfo { 
                                basket_id: Uint128::one(), 
                                pools_for_osmo_twap: vec![],
                                is_usd_par: false,
                                lp_pool_info: Some(
                                    PoolInfo { 
                                        pool_id: osmo_pool_id,
                                        asset_infos: vec![
                                            LPAssetInfo { 
                                                info: AssetInfo::NativeToken { denom: config.clone().osmo_denom }, 
                                                decimals: 6, 
                                                ratio: Decimal::percent(50),
                                            },
                                            LPAssetInfo { 
                                                info: AssetInfo::NativeToken { denom: config.clone().credit_denom }, 
                                                decimals: 6, 
                                                ratio: Decimal::percent(50),
                                            },
                                        ],
                                    }
                                ),
                                decimals: 18,
                                pyth_price_feed_id: None,
                                vault_info: None,
                            },
                        })?, 
                        funds: vec![],
                    }));
                //Set CDT oracle
                msgs.push(
                    CosmosMsg::Wasm(WasmMsg::Execute { 
                        contract_addr: addrs.clone().oracle.to_string(), 
                        msg: to_binary(&OracleExecuteMsg::AddAsset { 
                            asset_info: AssetInfo::NativeToken { denom: config.clone().credit_denom }, 
                            oracle_info: AssetOracleInfo { 
                                basket_id: Uint128::one(), 
                                pools_for_osmo_twap: vec![
                                    TWAPPoolInfo { 
                                        pool_id: osmo_pool_id,
                                        base_asset_denom: config.clone().credit_denom.to_string(),  
                                        quote_asset_denom: config.clone().osmo_denom.to_string(), 
                                    }
                                ],
                                is_usd_par: false,
                                lp_pool_info: None,
                                decimals: 6,
                                pyth_price_feed_id: None,
                                vault_info: None,
                            },
                        })?, 
                        funds: vec![],
                    }));
                //Set Auction's desired asset to the CDT/OSMO LP denom
                //and switch owner to governance
                msgs.push(
                    CosmosMsg::Wasm(WasmMsg::Execute { 
                        contract_addr: addrs.clone().mbrn_auction.to_string(), 
                        msg: to_binary(&DAExecuteMsg::UpdateConfig(AuctionUpdateConfig {
                            owner: Some(addrs.clone().governance.to_string()),
                            oracle_contract: None,
                            osmosis_proxy: None,
                            mbrn_denom: None,
                            cdt_denom: None,
                            desired_asset: Some(pool_denom.clone()),
                            positions_contract: None,
                            governance_contract: None,
                            staking_contract: None,
                            twap_timeframe: None,
                            initial_discount: None,
                            discount_increase_timeframe: None,
                            discount_increase: None,
                            send_to_stakers: None,
                        }))?, 
                        funds: vec![],
                    })
                );

                //Incentivize the OSMO/CDT pool
                //14 day guage
                let msg: CosmosMsg = MsgCreateGauge { 
                    pool_id:  0,
                    is_perpetual: false, 
                    owner: env.clone().contract.address.to_string(),
                    distribute_to: Some(QueryCondition { 
                        lock_query_type: 0, //ByDuration
                        denom: pool_denom,
                        duration: Some(Duration { seconds: 14 * SECONDS_PER_DAY as i64, nanos: 0 }), 
                        timestamp: None,
                    }), 
                    coins: vec![Coin {
                        denom: config.clone().mbrn_denom, 
                        amount: String::from("2_000_000_000_000"),
                    }], 
                    start_time: Some(
                        Timestamp { 
                            seconds: env.clone().block.time.seconds()as i64,
                            nanos: 0 
                        }
                    ), 
                    num_epochs_paid_over: 365, //days, 1 year
                }.into();
                sub_msgs.push(SubMsg::reply_on_error(msg, NO_ACTION_ID));                
            } 
            OSMO_POOL_ID.save(deps.storage, &osmo_pool_id)?;

            //Set credit_pool_infos
            //Add Credit LPs to Basket
            let msg = CDPExecuteMsg::EditBasket(EditBasket {
                added_cAsset: None,
                liq_queue: None,
                credit_pool_infos: Some(vec![
                    membrane::types::PoolType::Balancer { pool_id: osmo_pool_id }
                    ]),
                collateral_supply_caps: None,
                multi_asset_supply_caps: None,
                base_interest_rate: None,
                credit_asset_twap_price_source: Some(TWAPPoolInfo {
                    pool_id: osmo_pool_id,
                    base_asset_denom: config.clone().credit_denom,
                    quote_asset_denom: config.clone().osmo_denom,
                }),
                negative_rates: None,
                cpc_margin_of_error: None,
                frozen: None,
                rev_to_stakers: None,
                take_revenue: None,
                revenue_destinations: None,
            });
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().positions.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);
            // Add OSMO Pool as accepted LP for the Discount Vault
            let msg = DiscountVaultExecuteMsg::EditAcceptedLPs { 
                pool_ids: vec![osmo_pool_id], 
                remove: false 
            };
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().discount_vault.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);
            // Set DV Owner to governance
            let msg = DiscountVaultExecuteMsg::ChangeOwner { owner: addrs.clone().governance.to_string() };
            let msg = CosmosMsg::Wasm(WasmMsg::Execute { 
                contract_addr: addrs.clone().discount_vault.to_string(), 
                msg: to_binary(&msg)?, 
                funds: vec![], 
            });
            msgs.push(msg);

            // Add MBRN-OSMO LP to oracle for MBRN pricing
            // msgs.push(
            //     CosmosMsg::Wasm(WasmMsg::Execute { 
            //         contract_addr: addrs.clone().oracle.to_string(), 
            //         msg: to_binary(&OracleExecuteMsg::AddAsset { 
            //             asset_info: AssetInfo::NativeToken { denom: config.clone().mbrn_denom }, 
            //             oracle_info: AssetOracleInfo { 
            //                 basket_id: Uint128::one(), 
            //                 pools_for_osmo_twap: vec![
            //                     TWAPPoolInfo { 
            //                         pool_id: MBRN_POOL.load(deps.storage)?,
            //                         base_asset_denom: config.clone().mbrn_denom.to_string(),  
            //                         quote_asset_denom: config.clone().osmo_denom.to_string(), 
            //                     }
            //                 ],
            //                 is_usd_par: false,
            //                 lp_pool_info: None,
            //                 decimals: 6,       
            //                 pyth_price_feed_id: None,                     
            //             },
            //         })?, 
            //         funds: vec![],
            //     }));
            // Set oracle ownership to governance & add USD Par TWAP pool
            msgs.push(
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().oracle.to_string(), 
                    msg: to_binary(&OracleExecuteMsg::UpdateConfig { 
                        owner: Some(addrs.clone().governance.to_string()), 
                        positions_contract: None,
                        osmosis_proxy_contract: Some(addrs.clone().osmosis_proxy.to_string()),
                        pyth_osmosis_address: None,
                        osmo_usd_pyth_feed_id: None,
                        pools_for_usd_par_twap: Some(vec![
                            TWAPPoolInfo { 
                                pool_id: config.clone().osmousdc_pool_id, 
                                base_asset_denom: config.clone().osmo_denom.to_string(), 
                                quote_asset_denom: config.clone().usdc_denom.to_string(),  
                            }
                        ])
                    })?, 
                    funds: vec![],
                }));

            //Add CDT/OSMO LP to Liquidity Check
            msgs.push(
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().liquidity_check.to_string(), 
                    msg: to_binary(&LCExecuteMsg::AddAsset { asset: LiquidityInfo {
                        asset: AssetInfo::NativeToken {
                            denom: config.clone().credit_denom,
                        },
                        pool_infos: vec![PoolType::Balancer { pool_id: osmo_pool_id }]
                    } })?, 
                    funds: vec![],
                }));
            //Change LC ownership to governance 
            msgs.push(
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().liquidity_check.to_string(), 
                    msg: to_binary(&LCExecuteMsg::UpdateConfig { 
                        owner: Some(addrs.clone().governance.to_string()),
                        osmosis_proxy: None, 
                        positions_contract: None, 
                        stableswap_multiplier: None
                    })?, 
                    funds: vec![],
                }));
            // Change Positions contract ownership to governance & add misc. contracts
            msgs.push(
                CosmosMsg::Wasm(WasmMsg::Execute { 
                    contract_addr: addrs.clone().positions.to_string(), 
                    msg: to_binary(&CDPExecuteMsg::UpdateConfig(CDPUpdateConfig {
                        owner: Some(addrs.clone().governance.to_string()), 
                        stability_pool: Some(addrs.clone().stability_pool.to_string()), 
                        dex_router: None,
                        osmosis_proxy: None,
                        debt_auction: Some(addrs.clone().mbrn_auction.to_string()), 
                        staking_contract: None,
                        oracle_contract: None,
                        liquidity_contract: Some(addrs.clone().liquidity_check.to_string()), 
                        discounts_contract: Some(addrs.clone().system_discounts.to_string()), 
                        liq_fee: None,
                        debt_minimum: None,
                        base_debt_cap_multiplier: None,
                        oracle_time_limit: None,
                        credit_twap_timeframe: None,
                        collateral_twap_timeframe: None,
                        cpc_multiplier: None,
                        rate_slope_multiplier: None,
                        rate_hike_rate: None,
                        redemption_fee: None,
                    }))?, 
                    funds: vec![],
                }));

            //Query contract balance of any GAMM shares 
            //but we only care about MBRN/CDT-OSMO LP
            let coins: Vec<cosmwasm_std::Coin> = deps.querier.query_all_balances(env.contract.address.to_string())?;
            let gamm_coins = coins
                .into_iter()
                .filter( |coin| coin.denom.contains("gamm"))
                .collect::<Vec<cosmwasm_std::Coin>>();
                
                
            //Send gamm_coins to Governance
            let msg = BankMsg::Send {
                to_address: addrs.clone().governance.to_string(),
                amount: gamm_coins,
            };
            msgs.push(msg.into());
        }

        Ok(Response::new()
            //If incentive msgs error I don't want it to halt the launch since we are using a duration that is untestable on testnet
            .add_submessages(sub_msgs)
            .add_messages(msgs)
            .add_attribute("pool_saved", format!("{:?}", osmo_pool_id))
            .add_attribute("mbrn_pool_saved", format!("{:?}", MBRN_POOL.load(deps.storage).unwrap_or(0)))
        )
    },
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }    
}
