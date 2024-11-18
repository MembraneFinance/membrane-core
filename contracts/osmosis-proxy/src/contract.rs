//Token factory fork
//https://github.com/osmosis-labs/bindings/blob/main/contracts/tokenfactory

use std::convert::TryInto;

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, to_binary, to_json_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Order, QuerierWrapper, Reply, Response, StdError, StdResult, SubMsg, Uint128, WasmMsg
};
use cw2::set_contract_version;
use membrane::helpers::get_asset_liquidity;
use membrane::math::{decimal_multiplication, decimal_division};
use osmosis_std::types::osmosis::gamm::v1beta1::{GammQuerier, MsgExitPool};
use osmosis_std::types::osmosis::poolmanager::v1beta1::PoolmanagerQuerier;
use osmosis_std::types::osmosis::incentives::MsgCreateGauge;

use crate::error::TokenFactoryError;
use crate::state::{PendingTokenInfo, TokenInfo, SwapInfo, CONFIG, PENDING, SWAP_INFO, SWAP_ROUTES, TOKENS};
use membrane::osmosis_proxy::{
    Config, ExecuteMsg, GetDenomResponse, InstantiateMsg, QueryMsg, MigrateMsg, TokenInfoResponse, OwnerResponse, ContractDenomsResponse,
};
use membrane::cdp::{QueryMsg as CDPQueryMsg, Config as CDPConfig};
use membrane::oracle::{QueryMsg as OracleQueryMsg, PriceResponse};
use membrane::types::{PoolStateResponse, Basket, Owner, AssetInfo, SwapRoute};
use membrane::mars_vault_token::ExecuteMsg as MarsVaultExecuteMsg;
use osmosis_std::types::osmosis::tokenfactory::v1beta1::{self as TokenFactory, QueryDenomsFromCreatorResponse, MsgCreateDenomResponse};
use osmosis_std::types::osmosis::poolmanager::v1beta1::{MsgSwapExactAmountIn, SwapAmountInRoute};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:osmosis-proxy";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

//Constants
const MAX_LIMIT: u32 = 64;

const CREATE_DENOM_REPLY_ID: u64 = 1u64;
const SWAP_REPLY_ID: u64 = 2u64;
const USE_BALANCE_SWAP_REPLY_ID: u64 = 3u64;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    _msg: InstantiateMsg,
) -> Result<Response, TokenFactoryError> {
    let config = Config {
        owners: vec![
            Owner {
                owner: info.sender.clone(),
                total_minted: Uint128::zero(),
                stability_pool_ratio: Some(Decimal::zero()),
                non_token_contract_auth: true, 
                is_position_contract: false,
            }],
        liquidity_multiplier: None,
        debt_auction: None,
        positions_contract: None,
        liquidity_contract: None,
        oracle_contract: None,
    };
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("method", "instantiate")
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
) -> Result<Response, TokenFactoryError> {
    match msg {
        ExecuteMsg::ExecuteSwaps { token_out, max_slippage } => {
            execute_swaps(deps, env, info.sender.clone(), info.funds.clone(), token_out, max_slippage)
        }
        ExecuteMsg::CreateDenom {
            subdenom,
            max_supply,
        } => create_denom(
            deps,
            env,
            info,
            subdenom,
            max_supply,
        ),
        ExecuteMsg::ChangeAdmin {
            denom,
            new_admin_address,
        } => change_admin(deps, env, info, denom, new_admin_address),
        ExecuteMsg::MintTokens {
            denom,
            amount,
            mint_to_address,
        } => mint_tokens(deps, env, info, denom, amount, mint_to_address),
        ExecuteMsg::BurnTokens {
            denom,
            amount,
            burn_from_address,
        } => burn_tokens(deps, env, info, denom, amount, burn_from_address),
        ExecuteMsg::CreateOsmosisGauge { gauge_msg } => create_gauge(gauge_msg),
        ExecuteMsg::EditTokenMaxSupply { denom, max_supply } => {
            edit_token_max(deps, info, denom, max_supply)
        }
        ExecuteMsg::UpdateConfig {
            owners,
            add_owner,
            liquidity_multiplier,
            debt_auction,
            positions_contract,
            liquidity_contract,
            oracle_contract,
            edit_routes
        } => update_config(deps, info, owners, liquidity_multiplier, debt_auction, positions_contract, liquidity_contract, oracle_contract, add_owner, edit_routes),
        ExecuteMsg::EditOwner { owner, stability_pool_ratio, non_token_contract_auth } => {
            edit_owner(deps, info, owner, stability_pool_ratio, non_token_contract_auth)
        }
    }
}

/// Execute a swap to token out 
fn execute_swaps(
    deps: DepsMut,
    env: Env,
    swapper: Addr,
    funds: Vec<Coin>,
    token_out: String,
    max_slippage: Decimal,
) -> Result<Response, TokenFactoryError> {
    let config = CONFIG.load(deps.storage)?;
    let swap_routes = SWAP_ROUTES.load(deps.storage)?;
    let mut msgs = vec![];
    let mut used_special = false;

    //If no funds sent, error
    if funds.is_empty() {
        return Err(TokenFactoryError::ZeroAmount {});
    }

    //create swap msgs for each asset sent
    for coin in funds.into_iter() {
        //Get routes
        let routes: Vec<SwapAmountInRoute> = get_swap_route(swap_routes.clone(), coin.denom.clone(), token_out.clone())?;
        
        //If coin's denom is a VT or a GAMM, do special exit
        if coin.denom.contains("gamm/"){
            //Toggle used_special
            used_special = true;
            //Withdraw from GAMM pool
            let withdraw_msg: CosmosMsg = MsgExitPool {
                sender: env.contract.address.to_string(),
                pool_id: routes[0].pool_id,
                share_in_amount: coin.amount.to_string(),
                token_out_mins: vec![],
            }.into();
            //Add as Submsg
            msgs.push(SubMsg::reply_on_success(withdraw_msg, USE_BALANCE_SWAP_REPLY_ID));
        } 
        //ID 0 means its a VT
        else if routes[0].pool_id == 0 {
            //Toggle used_special
            used_special = true;
            //Get the VT address from the token denom
            let vt_address = coin.denom.split('/').collect::<Vec<&str>>()[1];
            //Exit VT
            let exit_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: vt_address.to_string(),
                msg: to_json_binary(&MarsVaultExecuteMsg::ExitVault {  })?,
                funds: vec![coin.clone()],
            });
            //Add as Submsg
            msgs.push(SubMsg::reply_on_success(exit_msg, USE_BALANCE_SWAP_REPLY_ID));
        } 
        //Act Normal
        else {        

            //Get token_in & token_out prices
            let token_prices: Vec<PriceResponse> = deps.querier.query_wasm_smart(
                config.oracle_contract.clone().unwrap().to_string(), 
                &OracleQueryMsg::Prices { 
                asset_infos: vec![
                    AssetInfo::NativeToken { denom: coin.denom.clone() },
                    AssetInfo::NativeToken { denom: token_out.clone() },
                    ],
                twap_timeframe: 0u64,
                oracle_time_limit: 0u64,
            })?;
            let token_in_price = token_prices[0].clone();
            let token_out_price = token_prices[1].clone();

            //Calculate min amount out
            let token_in_value = token_in_price.get_value(coin.amount)?;
            let token_out_min_value = decimal_multiplication(token_in_value, Decimal::one() - max_slippage)?;
            let token_out_min_amount = token_out_price.get_amount(token_out_min_value)?;

            //Create Msg
            let msg: CosmosMsg = MsgSwapExactAmountIn {
                sender: env.contract.address.to_string(),
                routes,
                token_in: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
                    amount: coin.amount.to_string(),
                    denom: coin.denom,
                }),
                token_out_min_amount: token_out_min_amount.to_string(),
                
            }.into();
            //Add Msgs
            msgs.push(SubMsg::new(msg));
        }
    }

    //Set Swap Info
    SWAP_INFO.save(deps.storage, &SwapInfo {
        swapper,
        token_out: token_out.clone(),
        max_slippage,
    })?;
    
    //If we are using a special exit & its the only msg, we don't want to change the reply ID
    if !(msgs.len() == 1 && used_special) {
        //Remove last msg from msgs
        let last_msg = match msgs.pop(){
            Some(msg) => msg,
            None => return Err(TokenFactoryError::CustomError { val: String::from("No messages to swap") })
        };

        //Set the last msg of the list to be a submessage with a swap reply
        msgs.push(SubMsg::reply_on_success(last_msg.msg, SWAP_REPLY_ID));
    }


    Ok(Response::new()
    .add_attribute("token_out", token_out)
    .add_attribute("max_slippage", max_slippage.to_string())
    .add_submessages(msgs))
}

fn get_swap_route(swap_routes: Vec<SwapRoute>, mut token_in: String, token_out: String) -> Result<Vec<SwapAmountInRoute>, TokenFactoryError> {
    let mut iterations = 0u64;
    
    //Search swap route for token_in
    let swap_route = match swap_routes.clone().into_iter().find(|route| route.token_in == token_in){
        Some(route) => route,
        None => return Err(TokenFactoryError::CustomError { val: format!("{:?} not found in swap routes", token_in) })
    };
    let mut routes: Vec<SwapAmountInRoute> = vec![swap_route.route_out.clone()];
    
    //Update token_in
    token_in = swap_route.route_out.token_out_denom;

    while (token_out != routes[routes.len() - 1].token_out_denom && iterations < 5){
        //Search swap route for token_in
        let swap_route = match swap_routes.clone().into_iter().find(|route| route.token_in == token_in){
            Some(route) => route,
            None => return Err(TokenFactoryError::CustomError { val: format!("{:?} not found in embedded swap routes", token_in) })
        };

        //Add route to routes
        routes.push(swap_route.route_out.clone());

        //Update token_in
        token_in = swap_route.route_out.token_out_denom;

        //Increment iterations
        iterations += 1;
    }

    //Check if token_out is in the last route
    if token_out != routes[routes.len() - 1].token_out_denom {
        return Err(TokenFactoryError::CustomError { val: format!("{:?} not found in swap routes for {:?} within 6 hops", token_out, token_in) })
    }

    Ok(routes)
}

/// Update contract configuration
/// This function is only callable by an owner with non_token_contract_auth set to true
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owners: Option<Vec<Owner>>,
    liquidity_multiplier: Option<Decimal>,
    debt_auction: Option<String>,
    positions_contract: Option<String>,
    liquidity_contract: Option<String>,
    oracle_contract: Option<String>,
    add_owner: Option<bool>,
    edit_routes: Option<Vec<SwapRoute>>,
) -> Result<Response, TokenFactoryError> {
    let mut config = CONFIG.load(deps.storage)?;

    let (authorized, owner_index) = validate_authority(config.clone(), info.clone());
    if !authorized || !config.owners[owner_index].non_token_contract_auth {
        return Err(TokenFactoryError::Unauthorized {});
    }

    //Edit Owner
    if add_owner.is_some() {        
        if let Some(owners) = owners {
            if add_owner.unwrap() {
                //Add all new owners
                for owner in owners {
                    //Validate Owner address
                    deps.api.addr_validate(&owner.owner.to_string())?;

                    //Error if owner already exists
                    for stored_owner in config.clone().owners {
                        if stored_owner.owner == owner.owner {
                            return Err(TokenFactoryError::AlreadyOwner {});
                        }
                    }

                    //Add owner to config
                    config.owners.push( owner );
                }
            } else {
                //Filter out owners that are in the owners list
                for owner in owners {
                    config.owners = config
                        .clone()
                        .owners
                        .into_iter()
                        .filter(|stored_owner| stored_owner.owner.to_string() != owner.owner)
                        .collect::<Vec<Owner>>();
                }
            }
        }
    }
    //Update Liquidity Multiplier
    if let Some(liquidity_multiplier) = liquidity_multiplier {
        config.liquidity_multiplier = Some(liquidity_multiplier);
    }

    //Edit Contracts
    if let Some(debt_auction) = debt_auction {
        config.debt_auction = Some(deps.api.addr_validate(&debt_auction)?);
    }
    if let Some(positions_contract) = positions_contract {
        config.positions_contract = Some(deps.api.addr_validate(&positions_contract)?);
    }
    if let Some(liquidity_contract) = liquidity_contract {
        config.liquidity_contract = Some(deps.api.addr_validate(&liquidity_contract)?);
    }
    if let Some(oracle_contract) = oracle_contract {
        config.oracle_contract = Some(deps.api.addr_validate(&oracle_contract)?);
    }
    //Edit Swap Routes
    if let Some(new_routes) = edit_routes {
        //Load current swap routes
        let mut swap_routes = SWAP_ROUTES.load(deps.storage)?;
        //Update routes with the same token_in
        for route in new_routes {
            //If route exists, update it
            if let Some((index, _route)) = swap_routes.clone().into_iter().enumerate().find(|(_i, route)| route.token_in == route.token_in){
                swap_routes[index] = route;
            } else {
                //Add new route
                swap_routes.push(route);
            }
        }

    }

    //Save Config
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes(vec![
        attr("method", "update_config"),
        attr("updated_config", format!("{:?}", config)),
        ]))
}

/// Edit Owner params
/// This function is only callable by an owner with non_token_contract_auth set to true
fn edit_owner(
    deps: DepsMut,
    info: MessageInfo,
    owner: String,
    stability_pool_ratio: Option<Decimal>,
    non_token_contract_auth: Option<bool>,
) -> Result<Response, TokenFactoryError>{
    let mut config = CONFIG.load(deps.storage)?;

    //Assert Authority
    let (authorized, owner_index) = validate_authority(config.clone(), info.clone());
    if !authorized || !config.owners[owner_index].non_token_contract_auth {
        return Err(TokenFactoryError::Unauthorized {});
    }
    let valid_owner_addr = deps.api.addr_validate(&owner)?;

    //Find Owner to edit
    if let Some((owner_index, mut owner)) = config.clone().owners
        .into_iter()
        .enumerate()
        .find(|(_i, owner)| owner.owner == valid_owner_addr){
        //Update Optionals
        if stability_pool_ratio.clone().is_some() {
            owner.stability_pool_ratio = stability_pool_ratio;
        }
        if let Some(toggle) = non_token_contract_auth.clone() {
            owner.non_token_contract_auth = toggle;
        }

        //Update Owner
        config.owners[owner_index] = owner;
    } else { return Err(TokenFactoryError::CustomError { val: String::from("Non-existent owner address") }) }

    //Save edited Owner
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attribute("edited_owner", format!("{:?}", config.owners[owner_index])))
}

/// Assert info.sender is an owner
fn validate_authority(config: Config, info: MessageInfo) -> (bool, usize) {
    //Owners && Debt Auction have contract authority
    match config
        .owners
        .into_iter()
        .enumerate()
        .find(|(_i, owner)| owner.owner == info.sender)
    {
        Some((index, _owner)) => (true, index),
        None => (false, 0),        
    }
}

/// Create a new denom using TokenFactory.
/// Saves the denom in the reply.
pub fn create_denom(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    subdenom: String,
    max_supply: Option<Uint128>,
) -> Result<Response, TokenFactoryError> {
    let config = CONFIG.load(deps.storage)?;

    //Assert Authority
    let (authorized, _owner_index) = validate_authority(config.clone(), info.clone());
    if !authorized {
        return Err(TokenFactoryError::Unauthorized {});
    }

    if subdenom.eq("") {
        return Err(TokenFactoryError::InvalidSubdenom { subdenom });
    }    

    //Create Msg
    let msg = TokenFactory::MsgCreateDenom { sender: env.contract.address.to_string(), subdenom: subdenom.clone() };
    let create_denom_msg = SubMsg::reply_on_success(msg, CREATE_DENOM_REPLY_ID );
    
    //Save PendingTokenInfo
    PENDING.save(deps.storage, &PendingTokenInfo { subdenom: subdenom.clone(), max_supply })?;

    let res = Response::new()
        .add_attribute("method", "create_denom")
        .add_attribute("sub_denom", subdenom)
        .add_attribute("max_supply", max_supply.unwrap_or_else(Uint128::zero))
        .add_submessage(create_denom_msg);

    Ok(res)
}

/// Change the admin of a denom created from this contract
pub fn change_admin(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    denom: String,
    new_admin_address: String,
) -> Result<Response, TokenFactoryError> {

    let config = CONFIG.load(deps.storage)?;
    //Assert Authority
    let (authorized, _owner_index) = validate_authority(config.clone(), info.clone());
    if !authorized {
        return Err(TokenFactoryError::Unauthorized {});
    }

    deps.api.addr_validate(&new_admin_address)?;

    validate_denom(denom.clone())?;

    let change_admin_msg = TokenFactory::MsgChangeAdmin {
        denom: denom.clone(),
        sender: env.contract.address.to_string(),
        new_admin: new_admin_address.clone(),
    };

    let res = Response::new()
        .add_attribute("method", "change_admin")
        .add_attribute("denom", denom)
        .add_attribute("new_admin_address", new_admin_address)
        .add_message(change_admin_msg);

    Ok(res)
}

/// Edit token max supply
fn edit_token_max(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
    max_supply: Uint128,
) -> Result<Response, TokenFactoryError> {

    let config = CONFIG.load(deps.storage)?;
    //Assert Authority
    let (authorized, _owner_index) = validate_authority(config.clone(), info.clone());
    if !authorized {
        return Err(TokenFactoryError::Unauthorized {});
    }

    //Update Token Max
    TOKENS.update(
        deps.storage,
        denom.clone(),
        |token_info| -> Result<TokenInfo, TokenFactoryError> {
            match token_info {
                Some(mut token_info) => {
                    token_info.max_supply = Some(max_supply);

                    Ok(token_info)
                }
                None => {
                    Err(TokenFactoryError::CustomError {
                        val: String::from("Denom was not created in this contract"),
                    })
                }
            }
        },
    )?;

    //If max supply is changed to under current_supply, it halts new mints.

    Ok(Response::new().add_attributes(vec![
        attr("method", "edit_token_max"),
        attr("denom", denom),
        attr("new_max", max_supply),
    ]))
}

/// Mint tokens to an address
pub fn mint_tokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    denom: String,
    amount: Uint128,
    mint_to_address: String,
) -> Result<Response, TokenFactoryError> {
    let config = CONFIG.load(deps.storage)?;

    //Assert Authority
    let (authorized, _) = validate_authority(config.clone(), info.clone());
    if !authorized {
        return Err(TokenFactoryError::Unauthorized {});
    }    

    //Validate mint_to_address
    deps.api.addr_validate(&mint_to_address)?;

    if amount.eq(&Uint128::new(0_u128)) {
        return Result::Err(TokenFactoryError::ZeroAmount {});
    }
    //Validate denom
    validate_denom(denom.clone())?;

    //Debt Auction can mint over max supply
    let mut mint_allowed = false;
    if let Some(debt_auction) = config.clone().debt_auction {
        if info.sender == debt_auction {
            mint_allowed = true;
        }
    }; 
    
    //Update Token Supply
    TOKENS.update(
        deps.storage,
        denom.clone(),
        |token_info| -> Result<TokenInfo, TokenFactoryError> {
            match token_info {
                Some(mut token_info) => {
                    if token_info.clone().max_supply.is_some() {
                        if token_info.current_supply <= token_info.max_supply.unwrap()
                            || mint_allowed
                        {
                            token_info.current_supply += amount;
                            mint_allowed = true;
                        }
                    } else {
                        token_info.current_supply += amount;
                        mint_allowed = true;
                    }

                    Ok(token_info)
                }
                None => {
                    Err(TokenFactoryError::CustomError {
                        val: String::from("Denom was not created in this contract"),
                    })
                }
            }
        },
    )?;

    //Create mint msg
    let mint_tokens_msg: CosmosMsg = TokenFactory::MsgMint{
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin{
            denom: denom.clone(),
            amount: amount.to_string(),
        }), 
        mint_to_address: mint_to_address.clone(),
    }.into(); 

    let mut res = Response::new()
        .add_attribute("method", "mint_tokens")
        .add_attribute("mint_status", mint_allowed.to_string())
        .add_attribute("denom", denom.clone())
        .add_attribute("amount", Uint128::zero());

    //If a mint was made/allowed
    if mint_allowed {
        res = Response::new()
            .add_attribute("method", "mint_tokens")
            .add_attribute("mint_status", mint_allowed.to_string())
            .add_attribute("denom", denom)
            .add_attribute("amount", amount)
            .add_attribute("mint_to_address", mint_to_address)
            .add_messages(vec![mint_tokens_msg]);
    }

    Ok(res)
}

/// Create Osmosis Incentive Gauge.
/// Uses osmosis-std to make it easier for contracts to execute osmosis messages.
fn create_gauge(
    gauge_msg: MsgCreateGauge,
) -> Result<Response, TokenFactoryError>{
    Ok(Response::new().add_message(gauge_msg))
}

/// Query's Position Basket collateral supplyCaps and finds the owner's ratio of the total supply
// fn get_owner_liquidity_multiplier(
//     querier: QuerierWrapper,
//     liquidity_multiplier: Decimal,
//     owners: Vec<Owner>,
//     owner: Addr,
//     oracle_contract: Addr,
//     positions_contract: Addr,
// ) -> Result<Decimal, TokenFactoryError> {

//     //Initialize variables
//     let mut owner_totals: Vec<(Addr, Decimal)> = vec![];

//     //Get twap_timeframe
//     let cdp_config: CDPConfig = querier.query_wasm_smart(positions_contract, &CDPQueryMsg::Config {  })?;
//     let twap_timeframe = cdp_config.collateral_twap_timeframe;

//     //Get per owner collateral total value
//     for owner in owners {
//         //Must have GetBasket query
//         if owner.is_position_contract {
//             let basket: Basket = querier.query_wasm_smart(owner.clone().owner, &CDPQueryMsg::GetBasket {  })?;
//             let mut total = Decimal::zero();

//             //Parse thru assets and value them
//             for asset in basket.collateral_supply_caps {

//                 //Get Price
//                 let asset_price: PriceResponse = querier.query_wasm_smart(oracle_contract.clone(), &OracleQueryMsg::Price { 
//                     asset_info: asset.asset_info.clone(),
//                     twap_timeframe: twap_timeframe.clone(),
//                     oracle_time_limit: cdp_config.oracle_time_limit,
//                     basket_id: None,
//                 })?;

//                 //Get Value
//                 let asset_value = asset_price.get_value(asset.current_supply)?;

//                 //Add to total
//                 total += asset_value;
//             }

//             owner_totals.push((owner.owner, total));
//         }
//     }

//     //Get total collateral value
//     let mut total_collateral_value = Decimal::zero();
//     for owner in owner_totals.clone() {
//         total_collateral_value += owner.1;
//     }

//     //Get owner's ratio of total collateral value
//     let mut owner_ratio = Decimal::zero();
//     for listed_owner in owner_totals {
//         if listed_owner.0 == owner && total_collateral_value > Decimal::zero(){
//             owner_ratio = decimal_division(listed_owner.1, total_collateral_value)?;
//         }
//     }
    
//     //Return owner's liquidity multiplier
//     Ok(decimal_multiplication(owner_ratio, liquidity_multiplier)?)
// }

/// Burns tokens 
pub fn burn_tokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    denom: String,
    amount: Uint128,
    burn_from_address: String,
) -> Result<Response, TokenFactoryError> {    
    let mut config = CONFIG.load(deps.storage)?;

    //Assert Authority
    let (authorized, owner_index) = validate_authority(config.clone(), info.clone());
    if !authorized {
        return Err(TokenFactoryError::Unauthorized {});
    }

    if amount.eq(&Uint128::new(0_u128)) {
        return Result::Err(TokenFactoryError::ZeroAmount {});
    }

    validate_denom(denom.clone())?;

    //Update Owner total_mints
    // if config.owners[owner_index].total_minted.is_zero() {
    //     //////Subtracted from the Position Contract's minted amount
    //     //Find the is_position_contract = true in config owners
    //     let mut position_contract_index = 0;
    //     for (i, owner) in config.clone().owners.into_iter().enumerate() {
    //         if owner.is_position_contract {
    //             position_contract_index = i;
    //         }
    //     }
    //     //Subtract from the position contract's total minted
    //     config.owners[position_contract_index].total_minted = match config.owners[position_contract_index].total_minted.checked_sub(amount){
    //         Ok(diff) => diff,
    //         Err(err) => return Err(TokenFactoryError::CustomError { val: err.to_string() })
    //     };
    // } else {
    //     config.owners[owner_index].total_minted = match config.owners[owner_index].total_minted.checked_sub(amount){
    //         Ok(diff) => diff,
    //         Err(err) => return Err(TokenFactoryError::CustomError { val: err.to_string() })
    //     }
    // };
    CONFIG.save(deps.storage, &config)?;


    //Update Token Supply
    TOKENS.update(
        deps.storage,
        denom.clone(),
        |token_info| -> Result<TokenInfo, TokenFactoryError> {
            match token_info {
                Some(mut token_info) => {
                    //Update token_info
                    token_info.current_supply -= amount;
                    token_info.burned_supply += amount;
                    
                    Ok(token_info)
                }
                None => {
                    Err(TokenFactoryError::CustomError {
                        val: String::from("Denom was not created in this contract"),
                    })
                }
            }
        },
    )?;

    let burn_token_msg: CosmosMsg = TokenFactory::MsgBurn {
        sender: env.contract.address.to_string(),
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin{
            denom,
            amount: amount.to_string(),
        }),
        burn_from_address: burn_from_address.clone(),
    }.into();

    let res = Response::new()
        .add_attribute("method", "burn_tokens")
        .add_attribute("amount", amount)
        .add_attribute("burn_from_address", burn_from_address)
        .add_message(burn_token_msg);

    Ok(res)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config { } => to_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::GetOwner { owner } => to_binary(&get_contract_owner(deps, owner)?),
        QueryMsg::GetDenom {
            creator_address,
            subdenom,
        } => to_binary(&get_denom(deps, creator_address, subdenom)?),
        QueryMsg::GetContractDenoms { limit } => to_binary(&get_contract_denoms(deps, limit)?),
        QueryMsg::PoolState { id } => to_binary(&get_pool_state(deps, id)?),
        QueryMsg::GetTokenInfo { denom } => to_binary(&get_token_info(deps, denom)?),
        QueryMsg::GetSwapRoutes { } => to_binary(&SWAP_ROUTES.load(deps.storage)?),
    }
}

/// Returns state data regarding a specified contract owner
fn get_contract_owner(deps: Deps, owner: String) -> StdResult<OwnerResponse> {
    let config = CONFIG.load(deps.storage)?;
    if let Some(owner) = config.clone().owners.into_iter().find(|stored_owner| stored_owner .owner == owner) {

        // If we end up with multiple positions contracts, we'll need to query OP's total minted in the Positions contracts instead of only using the Basket's total minted

        Ok(OwnerResponse {
            owner, 
            liquidity_multiplier: config.liquidity_multiplier.unwrap_or_else(|| Decimal::one()),
        })
    } else {
        Err(StdError::generic_err("Owner not found"))
    }
}

/// Returns token info for a specified denom
fn get_token_info(deps: Deps, denom: String) -> StdResult<TokenInfoResponse> {
    let token_info = TOKENS.load(deps.storage, denom.clone())?;
    
    Ok(TokenInfoResponse {
        denom,
        current_supply: token_info.current_supply,
        max_supply: token_info.max_supply.unwrap_or_else(Uint128::zero),
        burned_supply: token_info.burned_supply,
    })
    
}

/// Returns a list of all denoms created by this contract
fn get_contract_denoms(deps: Deps, limit: Option<u32>) -> StdResult<ContractDenomsResponse> {
    let limit = limit.unwrap_or_else(|| MAX_LIMIT);

    let denoms = 
        TOKENS
            .range(deps.storage, None, None, Order::Ascending)
            .take(limit as usize)
            .map(|info|{
                if let Ok(info) = info {
                    info.0
                } else { String::from("error") }
            })
            .collect::<Vec<String>>();

    Ok(
        ContractDenomsResponse {
            denoms,
        }
    )
}

/// Returns PoolStateResponse for a specified pool id
fn get_pool_state(
    deps: Deps,
    pool_id: u64,
) -> StdResult<PoolStateResponse> {
    let liquidity_res: osmosis_std::types::osmosis::poolmanager::v1beta1::TotalPoolLiquidityResponse = PoolmanagerQuerier::new(&deps.querier).total_pool_liquidity(pool_id)?;
    let shares_res: osmosis_std::types::osmosis::gamm::v1beta1::QueryTotalSharesResponse = match GammQuerier::new(&deps.querier).total_shares(pool_id){
        Ok(res) => res,
        //We return None as it'll error for CL pools but I'm pretty sure we need this query for GAMM pricing in the oracle
        Err(_) => osmosis_std::types::osmosis::gamm::v1beta1::QueryTotalSharesResponse { total_shares: None }
    };
        
    Ok(PoolStateResponse { 
        assets: liquidity_res.liquidity, 
        shares: shares_res.total_shares.unwrap_or_default(),
    })
    
}

/// Returns denom for a specified creator address and subdenom
fn get_denom(deps: Deps, creator_addr: String, subdenom: String) -> StdResult<GetDenomResponse> {
    let response: QueryDenomsFromCreatorResponse = TokenFactory::TokenfactoryQuerier::new(&deps.querier).denoms_from_creator(creator_addr)?;

    let denom = if let Some(denom) = response.denoms.into_iter().find(|denoms| denoms.contains(&subdenom)){
        denom
    } else {
        return Err(StdError::GenericErr { msg: String::from("Can't find subdenom in list of contract denoms") })
    };

    Ok(GetDenomResponse {
        denom,
    })
}

/// Validate token factory denom
pub fn validate_denom(denom: String) -> Result<(), TokenFactoryError> {
    let denom_to_split = denom.clone();
    let tokenfactory_denom_parts: Vec<&str> = denom_to_split.split('/').collect();

    if tokenfactory_denom_parts.len() != 3 {
        return Result::Err(TokenFactoryError::InvalidDenom {
            denom,
            message: std::format!(
                "denom must have 3 parts separated by /, had {}",
                tokenfactory_denom_parts.len()
            ),
        });
    }

    let prefix = tokenfactory_denom_parts[0];

    if !prefix.eq_ignore_ascii_case("factory") {
        return Result::Err(TokenFactoryError::InvalidDenom {
            denom,
            message: std::format!("prefix must be 'factory', was {}", prefix),
        });
    }

    Result::Ok(())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.id {
        CREATE_DENOM_REPLY_ID => handle_create_denom_reply(deps, env, msg),
        SWAP_REPLY_ID => handle_swap_reply(deps, env, msg),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
}

fn handle_swap_balances_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            //Get swapper
            let swap_info = SWAP_INFO.load(deps.storage)?;

            //Swap all assets in the contract
            let balances = deps.querier.query_all_balances(&env.contract.address)?;

            //Execute swap with new balances
            let res = match execute_swaps(
                deps, 
                env, 
                swap_info.swapper.clone(),
                balances.clone(),
                swap_info.token_out.clone(),
                swap_info.max_slippage.clone(),
            ){
                Ok(res) => res,
                Err(err) => return Err(StdError::GenericErr { msg: err.to_string() }),
            };  

            return Ok(res
            .add_attribute("swap_info", format!("{:?}", swap_info))
            .add_attribute("tokens_received", format!("{:?}", balances)))
        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

fn handle_swap_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_) => {
            //Get swapper
            let swapper = SWAP_INFO.load(deps.storage)?.swapper;

            //Send all assets in the contract to the swapper
            let balances = deps.querier.query_all_balances(&env.contract.address)?;

            let msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
                to_address: swapper.clone().to_string(),
                amount: balances.clone(),
            });

            //Remove swapper
            // SWAP_INFO.remove(deps.storage);
            //Don't remove incase we have 2 swap replies due to a special exit

            return Ok(Response::new()
            .add_attribute("swapper", swapper)
            .add_attribute("tokens_received", format!("{:?}", balances))
            .add_message(msg))
        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

/// Find & save created full denom
fn handle_create_denom_reply(
    deps: DepsMut,
    _env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(result) => {
            //Load Pending TokenInfo
            let PendingTokenInfo { subdenom:_, max_supply} = PENDING.load(deps.storage)?;

            if let Some(b) = result.data {
                let res: MsgCreateDenomResponse = match b.try_into().map_err(TokenFactoryError::Std){
                    Ok(res) => res,
                    Err(err) => return Err(StdError::GenericErr { msg: String::from(err.to_string()) })
                };
                //Save Denom Info
                TOKENS.save(
                    deps.storage,
                    res.new_token_denom.clone(),
                    &TokenInfo {
                        current_supply: Uint128::zero(),
                        max_supply,
                        burned_supply: Uint128::zero(),
                    },
                )?;
            } else {
                return Err(StdError::GenericErr { msg: String::from("No data in reply") })
            }
        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, env: Env, _msg: MigrateMsg) -> Result<Response, TokenFactoryError> {
    let mut current_routes = SWAP_ROUTES.load(deps.storage)?;
    let mut new_routes = vec![];
    //Set WBTC to allBTC
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1z0qrq605sjgcqpylfl4aa6s90x738j7m58wyatt0tdzflg2ha26q67k743/wbtc"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC"),
            pool_id: 1868u64,
        },
    };
    new_routes.push(swap_route);
    //Set allBTC to WBTC
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo1z0qrq605sjgcqpylfl4aa6s90x738j7m58wyatt0tdzflg2ha26q67k743/wbtc"),
            pool_id: 1868u64,
        },
    };
    new_routes.push(swap_route);
    //Set allBTC to USDC
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1942u64,
        },
    };
    new_routes.push(swap_route);
    //Set USDC to allBTC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC"),
            pool_id: 1942u64,
        },
    };
    new_routes.push(swap_route);

    //Set WBTC.axl to allBTC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/D1542AA8762DB13087D8364F3EA6509FD6F009A34F00426AF9E4F9FA85CBBF1F"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC"),
            pool_id: 1868u64,
        },
    };
    new_routes.push(swap_route);
    //Set allBTC to WBTC.axl
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/D1542AA8762DB13087D8364F3EA6509FD6F009A34F00426AF9E4F9FA85CBBF1F"),
            pool_id: 1868u64,
        },
    };
    new_routes.push(swap_route);

    //Set OSMO to USDC
    let swap_route = SwapRoute {
        token_in: String::from("uosmo"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1464u64,
        },
    };
    new_routes.push(swap_route);
    //Set USDC to OSMO
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("uosmo"),
            pool_id: 1464u64,
        },
    };
    new_routes.push(swap_route);

    //Set stOSMO to OSMO
    let swap_route = SwapRoute {
        token_in: String::from("ibc/D176154B0C63D1F9C6DCFB4F70349EBF2E2B5A87A05902F57A6AE92B863E9AEC"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("uosmo"),
            pool_id: 1252u64,
        },
    };
    new_routes.push(swap_route);    
    //Set OSMO to stOSMO
    let swap_route = SwapRoute {
        token_in: String::from("uosmo"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/D176154B0C63D1F9C6DCFB4F70349EBF2E2B5A87A05902F57A6AE92B863E9AEC"),
            pool_id: 1252u64,
        },
    };
    new_routes.push(swap_route);

    //Set ATOM to USDC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1282u64,
        },
    };
    new_routes.push(swap_route);
    //Set USDC to ATOM
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2"),
            pool_id: 1282u64,
        },
    };
    new_routes.push(swap_route);

    
    //Set stATOM to ATOM
    let swap_route = SwapRoute {
        token_in: String::from("ibc/C140AFD542AE77BD7DCC83F13FDD8C5E5BB8C4929785E6EC2F4C636F98F17901"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2"),
            pool_id: 1283u64,
        },
    };
    new_routes.push(swap_route);
    //Set ATOM to stATOM
    let swap_route = SwapRoute {
        token_in: String::from("ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/C140AFD542AE77BD7DCC83F13FDD8C5E5BB8C4929785E6EC2F4C636F98F17901"),
            pool_id: 1283u64,
        },
    };
    new_routes.push(swap_route);

    
    //Set USDC.axl to USDC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/D189335C6E4A68B513C10AB227BF1C1D38C746766278BA3EEB4FB14124F1D858"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1212u64,
        },
    };
    new_routes.push(swap_route);
    //Set USDC to USDC.axl
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/D189335C6E4A68B513C10AB227BF1C1D38C746766278BA3EEB4FB14124F1D858"),
            pool_id: 1212u64,
        },
    };
    new_routes.push(swap_route);

    //Set WETH.axl to OSMO
    let swap_route = SwapRoute {
        token_in: String::from("ibc/EA1D43981D5C9A1C4AAEA9C23BB1D4FA126BA9BC7020A25E0AE4AA841EA25DC5"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("uosmo"),
            pool_id: 1134u64,
        },
    };
    new_routes.push(swap_route);    
    //Set OSMO to WETH.axl
    let swap_route = SwapRoute {
        token_in: String::from("uosmo"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/EA1D43981D5C9A1C4AAEA9C23BB1D4FA126BA9BC7020A25E0AE4AA841EA25DC5"),
            pool_id: 1134u64,
        },
    };
    new_routes.push(swap_route);

    //Set TIA to USDC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/D79E7D83AB399BFFF93433E54FAA480C191248FC556924A2A8351AE2638B3877"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1247u64,
        },
    };
    new_routes.push(swap_route);
    //Set USDC to TIA
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/D79E7D83AB399BFFF93433E54FAA480C191248FC556924A2A8351AE2638B3877"),
            pool_id: 1247u64,
        },
    };
    new_routes.push(swap_route);

    //Set TIA to MILKTIA
    let swap_route = SwapRoute {
        token_in: String::from("ibc/D79E7D83AB399BFFF93433E54FAA480C191248FC556924A2A8351AE2638B3877"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo1f5vfcph2dvfeqcqkhetwv75fda69z7e5c2dldm3kvgj23crkv6wqcn47a0/umilkTIA"),
            pool_id: 1475u64,
        },
    };
    new_routes.push(swap_route);
    //Set MILKTIA to TIA
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1f5vfcph2dvfeqcqkhetwv75fda69z7e5c2dldm3kvgj23crkv6wqcn47a0/umilkTIA"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/D79E7D83AB399BFFF93433E54FAA480C191248FC556924A2A8351AE2638B3877"),
            pool_id: 1475u64,
        },
    };
    new_routes.push(swap_route);

    //Set TIA to stTIA
    let swap_route = SwapRoute {
        token_in: String::from("ibc/D79E7D83AB399BFFF93433E54FAA480C191248FC556924A2A8351AE2638B3877"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/698350B8A61D575025F3ED13E9AC9C0F45C89DEFE92F76D5838F1D3C1A7FF7C9"),
            pool_id: 1428u64,
        },
    };
    new_routes.push(swap_route);
    //Set stTIA to TIA
    let swap_route = SwapRoute {
        token_in: String::from("ibc/698350B8A61D575025F3ED13E9AC9C0F45C89DEFE92F76D5838F1D3C1A7FF7C9"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/D79E7D83AB399BFFF93433E54FAA480C191248FC556924A2A8351AE2638B3877"),
            pool_id: 1428u64,
        },
    };
    new_routes.push(swap_route);

    //Set USDT.kava to USDC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/4ABBEF4C8926DDDB320AE5188CFD63267ABBCEFC0583E4AE05D6E5AA2401DDAB"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1220u64,
        },
    };
    new_routes.push(swap_route);
    //Set USDC to USDT.kava
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/4ABBEF4C8926DDDB320AE5188CFD63267ABBCEFC0583E4AE05D6E5AA2401DDAB"),
            pool_id: 1220u64,
        },
    };
    new_routes.push(swap_route);
    
    //Set stDYDX to DYDX
    let swap_route = SwapRoute {
        token_in: String::from("ibc/980E82A9F8E7CA8CD480F4577E73682A6D3855A267D1831485D7EBEF0E7A6C2C"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/831F0B1BBB1D08A2B75311892876D71565478C532967545476DF4C2D7492E48C"),
            pool_id: 1423u64,
        },
    };
    new_routes.push(swap_route);
    //Set DYDX to stDYDX
    let swap_route = SwapRoute {
        token_in: String::from("ibc/831F0B1BBB1D08A2B75311892876D71565478C532967545476DF4C2D7492E48C"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/980E82A9F8E7CA8CD480F4577E73682A6D3855A267D1831485D7EBEF0E7A6C2C"),
            pool_id: 1423u64,
        },
    };
    new_routes.push(swap_route);
     //Set OSMO to DYDX
     let swap_route = SwapRoute {
        token_in: String::from("uosmo"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/831F0B1BBB1D08A2B75311892876D71565478C532967545476DF4C2D7492E48C"),
            pool_id: 1423u64,
        },
    };
    new_routes.push(swap_route);
    //Set DYDX to OSMO
    let swap_route = SwapRoute {
        token_in: String::from("ibc/831F0B1BBB1D08A2B75311892876D71565478C532967545476DF4C2D7492E48C"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("uosmo"),
            pool_id: 1423u64,
        },
    };
    new_routes.push(swap_route);

    
    //Set OSMO to DVPN
     let swap_route = SwapRoute {
        token_in: String::from("uosmo"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/9712DBB13B9631EDFA9BF61B55F1B2D290B2ADB67E3A4EB3A875F3B6081B3B84"),
            pool_id: 5u64,
        },
    };
    new_routes.push(swap_route);
    //Set DVPN to OSMO
    let swap_route = SwapRoute {
        token_in: String::from("ibc/9712DBB13B9631EDFA9BF61B55F1B2D290B2ADB67E3A4EB3A875F3B6081B3B84"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("uosmo"),
            pool_id: 5u64,
        },
    };
    new_routes.push(swap_route);

    //Set USDC to AKT
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4"),
            pool_id: 1301u64,
        },
    };
    new_routes.push(swap_route);
    //Set AKT to USDC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1301u64,
        },
    };
    new_routes.push(swap_route);
    //Set OSMO to LVN
     let swap_route = SwapRoute {
        token_in: String::from("uosmo"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo1mlng7pz4pnyxtpq0akfwall37czyk9lukaucsrn30ameplhhshtqdvfm5c/ulvn"),
            pool_id: 1325u64,
        },
    };
    new_routes.push(swap_route);
    //Set LVN to OSMO
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1mlng7pz4pnyxtpq0akfwall37czyk9lukaucsrn30ameplhhshtqdvfm5c/ulvn"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("uosmo"),
            pool_id: 1325u64,
        },
    };
    new_routes.push(swap_route);
    //Set USDC to LAB
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo17fel472lgzs87ekt9dvk0zqyh5gl80sqp4sk4n/LAB"),
            pool_id: 1656u64,
        },
    };
    new_routes.push(swap_route);
    //Set LAB to USDC
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo17fel472lgzs87ekt9dvk0zqyh5gl80sqp4sk4n/LAB"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1656u64,
        },
    };
    new_routes.push(swap_route);
    //Set OSMO to JKL
     let swap_route = SwapRoute {
        token_in: String::from("uosmo"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/8E697BDABE97ACE8773C6DF7402B2D1D5104DD1EEABE12608E3469B7F64C15BA"),
            pool_id: 832u64,
        },
    };
    new_routes.push(swap_route);
    //Set JKL to OSMO
    let swap_route = SwapRoute {
        token_in: String::from("ibc/8E697BDABE97ACE8773C6DF7402B2D1D5104DD1EEABE12608E3469B7F64C15BA"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("uosmo"),
            pool_id: 832u64,
        },
    };
    new_routes.push(swap_route);
    
    //Set USDC to INJ
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/64BA6E31FE887D66C6F8F31C7B1A80C7CA179239677B4088BB55F5EA07DBE273"),
            pool_id: 1319u64,
        },
    };
    new_routes.push(swap_route);
    //Set INJ to USDC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/64BA6E31FE887D66C6F8F31C7B1A80C7CA179239677B4088BB55F5EA07DBE273"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1319u64,
        },
    };
    new_routes.push(swap_route);
    //Set USDC to ISLM
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/69110FF673D70B39904FF056CFDFD58A90BEC3194303F45C32CB91B8B0A738EA"),
            pool_id: 1690u64,
        },
    };
    new_routes.push(swap_route);
    //Set ISLM to USDC
    let swap_route = SwapRoute {
        token_in: String::from("ibc/69110FF673D70B39904FF056CFDFD58A90BEC3194303F45C32CB91B8B0A738EA"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1690u64,
        },
    };
    new_routes.push(swap_route); 
    //Set allSOL to SOL.wh
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1n3n75av8awcnw4jl62n3l48e6e4sxqmaf97w5ua6ddu4s475q5qq9udvx4/alloyed/allSOL"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/1E43D59E565D41FB4E54CA639B838FFD5BCFC20003D330A56CB1396231AA1CBA"),
            pool_id: 1925u64,
        },
    };
    new_routes.push(swap_route);
    //Set SOL.wh to allSOL
    let swap_route = SwapRoute {
        token_in: String::from("ibc/1E43D59E565D41FB4E54CA639B838FFD5BCFC20003D330A56CB1396231AA1CBA"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo1n3n75av8awcnw4jl62n3l48e6e4sxqmaf97w5ua6ddu4s475q5qq9udvx4/alloyed/allSOL"),
            pool_id: 1925u64,
        },
    };
    new_routes.push(swap_route);
    
    //Set USDC to allSOL
    let swap_route = SwapRoute {
        token_in: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("factory/osmo1n3n75av8awcnw4jl62n3l48e6e4sxqmaf97w5ua6ddu4s475q5qq9udvx4/alloyed/allSOL"),
            pool_id: 1960u64,
        },
    };
    new_routes.push(swap_route);
    //Set allSOL to USDC
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1n3n75av8awcnw4jl62n3l48e6e4sxqmaf97w5ua6ddu4s475q5qq9udvx4/alloyed/allSOL"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 1960u64,
        },
    };
    new_routes.push(swap_route);

    /////////SPECIALS (VTs & GAMM)///////////
    //Set marsUSDC to USDC
    let swap_route = SwapRoute {
        token_in: String::from("factory/osmo1fqcwupyh6s703rn0lkxfx0ch2lyrw6lz4dedecx0y3ced2jq04tq0mva2l/mars-usdc-tokenized"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from("ibc/498A0751C798A0D9A389AA3691123DADA57DAA4FE165D5C75894505B876BA6E4"),
            pool_id: 0u64, //if pool ID is 0, its a vault token so we must exit to get the token_out_denom
        },
    };
    new_routes.push(swap_route);
    //Set GAMM 1 
    let swap_route = SwapRoute {
        token_in: String::from("gamm/pool/1"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from(""),
            pool_id: 1u64,
        },
    };
    new_routes.push(swap_route);
    //Set GAMM 678
    let swap_route = SwapRoute {
        token_in: String::from("gamm/pool/678"),
        route_out: SwapAmountInRoute {
            token_out_denom: String::from(""),
            pool_id: 678u64,
        },
    };
    new_routes.push(swap_route);

    //Set new routes
    let new_routes = [current_routes, new_routes].concat();
    //Update current routes
    SWAP_ROUTES.save(deps.storage, &new_routes)?;

    Ok(Response::default().add_attribute("routes", format!("{:?}", new_routes)))
}