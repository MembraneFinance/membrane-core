

use std::env;
use std::str::FromStr;

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{to_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, StdError, Addr, Uint128, QueryRequest, WasmQuery, Decimal, CosmosMsg, WasmMsg, BankMsg, Coin, from_binary, Order, Storage, Api, QuerierWrapper, Querier, SubMsg, Reply, attr, coin};
use cw2::set_contract_version;
use cw20::{Cw20ReceiveMsg, Cw20ExecuteMsg};
use cw_storage_plus::Bound;
use cosmwasm_bignumber::{ Uint256, Decimal256 };
use osmo_bindings::{ SpotPriceResponse, OsmosisMsg, FullDenomResponse, OsmosisQuery };

use membrane::stability_pool::{ExecuteMsg as SP_ExecuteMsg};
use membrane::positions::{ExecuteMsg, InstantiateMsg, QueryMsg, Cw20HookMsg, PositionResponse, PositionsResponse, BasketResponse, ConfigResponse, PropResponse, CallbackMsg};
use membrane::types::{ AssetInfo, Asset, cAsset, Basket, Position, LiqAsset, SellWallDistribution, UserInfo, TWAPPoolInfo };
use membrane::osmosis_proxy::{ QueryMsg as OsmoQueryMsg, GetDenomResponse };
use membrane::debt_auction::{ ExecuteMsg as AuctionExecuteMsg };


//use crate::liq_queue::LiquidatibleResponse;
use crate::math::{decimal_multiplication, decimal_division, decimal_subtraction};
use crate::error::ContractError;
use crate::positions::{create_basket, assert_basket_assets, assert_sent_native_token_balance, deposit, withdraw, increase_debt, repay, liq_repay, edit_contract_owner, liquidate, edit_basket, sell_wall_using_ids, SELL_WALL_REPLY_ID, STABILITY_POOL_REPLY_ID, LIQ_QUEUE_REPLY_ID, withdrawal_msg, update_position_claims, CREATE_DENOM_REPLY_ID, BAD_DEBT_REPLY_ID, mint_revenue, WITHDRAW_REPLY_ID, get_contract_balances};
use crate::query::{query_stability_pool_liquidatible, query_config, query_position, query_user_positions, query_basket_positions, query_basket, query_baskets, query_prop, query_stability_pool_fee, query_basket_debt_caps, query_bad_debt, query_basket_insolvency, query_position_insolvency};
//use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, AssetInfo, Cw20HookMsg, Asset, PositionResponse, PositionsResponse, BasketResponse, LiqModuleMsg};
//use crate::stability_pool::{Cw20HookMsg as SP_Cw20HookMsg, QueryMsg as SP_QueryMsg, LiquidatibleResponse as SP_LiquidatibleResponse, PoolResponse, ExecuteMsg as SP_ExecuteMsg};
//use crate::liq_queue::{ExecuteMsg as LQ_ExecuteMsg, QueryMsg as LQ_QueryMsg, LiquidatibleResponse as LQ_LiquidatibleResponse, Cw20HookMsg as LQ_Cw20HookMsg};
use crate::state::{ Config, CONFIG, POSITIONS, BASKETS, RepayPropagation, REPAY, WITHDRAW };

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:cdp";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");



#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
      
    let mut config = Config {
        liq_fee: msg.liq_fee,
        owner: info.sender.clone(),
        current_basket_id: Uint128::from(1u128),
        stability_pool: None, 
        dex_router: None,
        interest_revenue_collector: None,
        staking_contract: None,
        osmosis_proxy: None,
        debt_auction: None,    
        oracle_time_limit: msg.oracle_time_limit,
        debt_minimum: msg.debt_minimum,
        twap_timeframe: msg.twap_timeframe,
    };
    
    //Set optional config parameters
    match msg.owner {
        Some( address ) => {
            
            match deps.api.addr_validate( &address ){
                Ok( addr ) => config.owner = addr,
                Err(_) => {},
            }
        },
        None => { },
    };
    match msg.stability_pool {
        Some( address ) => {
            
            match deps.api.addr_validate( &address ){
                Ok( addr ) => config.stability_pool = Some( addr ),
                Err(_) => {},
            }
        },
        None => {},
    };
    match msg.dex_router {
        Some( address ) => {
            
            match deps.api.addr_validate( &address ){
                Ok( addr ) => config.dex_router = Some( addr ),
                Err(_) => {},
            }
        },
        None => {},
    };
    match msg.staking_contract {
        Some( address ) => {
            
            match deps.api.addr_validate( &address ){
                Ok( addr ) => config.staking_contract = Some( addr ),
                Err(_) => {},
            }
        },
        None => {},
    };
    match msg.interest_revenue_collector {
        Some( address ) => {
            
            match deps.api.addr_validate( &address ){
                Ok( addr ) => config.interest_revenue_collector = Some( addr ),
                Err(_) => {},
            }
        },
        None => {},
    };
    match msg.osmosis_proxy {
        Some( address ) => {
            
            match deps.api.addr_validate( &address ){
                Ok( addr ) => config.osmosis_proxy = Some( addr ),
                Err(_) => {},
            }
        },
        None => {},
    };
    match msg.debt_auction {
        Some( address ) => {
            
            match deps.api.addr_validate( &address ){
                Ok( addr ) => config.debt_auction = Some( addr ),
                Err(_) => {},
            }
        },
        None => {},
    };

    let current_basket_id = &config.current_basket_id.clone().to_string();

    CONFIG.save(deps.storage, &config)?;

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let mut create_res = Response::new();
    let mut attrs = vec![];
    let sender = &info.sender.clone().to_string();

    attrs.push(("method", "instantiate"));
    attrs.push(("owner", sender));
    
    //Create Basket
    if msg.collateral_types.is_some() && msg.credit_asset.is_some(){

        let mut check = true;
        let collateral_types = msg.collateral_types.unwrap();

        //cAsset checks
        for cAsset in collateral_types.clone(){
            if cAsset.max_borrow_LTV >= cAsset.max_LTV && cAsset.max_borrow_LTV >= Decimal::from_ratio( Uint128::new(100u128), Uint128::new(1u128)){
                check = false;
            }
        }
        if( check ) && msg.credit_asset.is_some() && msg.credit_asset_twap_price_source.is_some(){
            
            create_res = create_basket(
                deps,
                info,
                env,
                Some( config.owner.to_string() ),
                collateral_types.clone(),
                msg.credit_asset.unwrap(),
                msg.credit_price,
                msg.credit_interest,
                msg.collateral_supply_caps,
                msg.base_interest_rate,
                msg.desired_debt_cap_util,
                msg.credit_pool_ids.unwrap_or_default(),
                msg.credit_asset_twap_price_source.unwrap(),
                msg.liquidity_multiplier_for_debt_caps,
                true,
            )?;
            
            attrs.push(("basket_id", current_basket_id));
        }else{
            attrs.push(("basket_status", "Not created: cAsset.max_LTV can't be less than or equal to cAsset.max_borrow_LTV"));
        }
        
    }else{
        attrs.push(("basket_status", "Not created: Basket only created w/ collateral_types AND credit_asset filled"));
    }

    //response.add_attributes(attrs);
    Ok(create_res.add_attributes(attrs))
}


#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateConfig { owner, stability_pool, dex_router, osmosis_proxy, debt_auction, staking_contract, interest_revenue_collector, liq_fee, debt_minimum, oracle_time_limit , twap_timeframe} => {
            update_config( deps, info, owner, stability_pool, dex_router, osmosis_proxy, debt_auction, staking_contract, interest_revenue_collector, liq_fee, debt_minimum, oracle_time_limit, twap_timeframe )
        },
        ExecuteMsg::Receive(msg) => receive_cw20(deps, env, info, msg),
        ExecuteMsg::Deposit{ assets, position_owner, position_id, basket_id} => {
            let mut valid_assets = vec![];

            if assets.len() != info.funds.len() { return Err( ContractError::CustomError { val: String::from("Length discrepency between sent assets and AssetInfo List") } ) }
            
            for asset in assets.clone(){
                valid_assets.push( assert_sent_native_token_balance( asset, &info )? );
            }
            let cAssets: Vec<cAsset> = assert_basket_assets(deps.storage, deps.querier, env.clone(), basket_id, valid_assets, true)?;
            deposit(deps, env, info, position_owner, position_id, basket_id, cAssets)
        }
    ,
        ExecuteMsg::Withdraw{ position_id, basket_id, assets } => {
            let cAssets: Vec<cAsset> = assert_basket_assets(deps.storage, deps.querier, env.clone(), basket_id, assets, false)?;
            withdraw(deps, env, info, position_id, basket_id, cAssets)
        },
        
        ExecuteMsg::IncreaseDebt { basket_id, position_id, amount } => increase_debt(deps, env, info, basket_id, position_id, amount),
        ExecuteMsg::Repay { basket_id, position_id, position_owner} => {
            let basket: Basket = match BASKETS.load(deps.storage, basket_id.to_string()) {
                Err(_) => { return Err(ContractError::NonExistentBasket {  })},
                Ok( basket ) => { basket },
            };

            let credit_asset = assert_sent_native_token_balance(basket.credit_asset.info, &info)?;
            repay(deps.storage, deps.querier, deps.api, env, info, basket_id, position_id, position_owner, credit_asset)
        },
        ExecuteMsg::LiqRepay { credit_asset} => {
            let credit_asset = assert_sent_native_token_balance(credit_asset.info, &info)?;
            liq_repay(deps, env, info, credit_asset)
        }
        ExecuteMsg::EditAdmin { owner } => edit_contract_owner(deps, info, owner),
        ExecuteMsg::EditcAsset { basket_id, asset, max_borrow_LTV, max_LTV } => edit_cAsset(deps, info, basket_id, asset, max_borrow_LTV, max_LTV),
        ExecuteMsg::EditBasket { basket_id, added_cAsset, owner, credit_interest, liq_queue, pool_ids, liquidity_multiplier, collateral_supply_caps, base_interest_rate, desired_debt_cap_util, credit_asset_twap_price_source } => edit_basket(deps, info, basket_id, added_cAsset, owner, credit_interest, liq_queue, pool_ids, liquidity_multiplier, collateral_supply_caps, base_interest_rate, desired_debt_cap_util, credit_asset_twap_price_source ),
        ExecuteMsg::CreateBasket { owner, collateral_types, credit_asset, credit_price, credit_interest, collateral_supply_caps, base_interest_rate, desired_debt_cap_util, credit_asset_twap_price_source, credit_pool_ids, liquidity_multiplier_for_debt_caps } => create_basket( deps, info, env, owner, collateral_types, credit_asset, credit_price, credit_interest, collateral_supply_caps, base_interest_rate, desired_debt_cap_util, credit_pool_ids, credit_asset_twap_price_source, liquidity_multiplier_for_debt_caps, false ),
        ExecuteMsg::Liquidate { basket_id, position_id, position_owner } => liquidate(deps.storage, deps.api, deps.querier, env, info, basket_id, position_id, position_owner),
        ExecuteMsg::MintRevenue { basket_id, send_to, repay_for, amount } => mint_revenue(deps, info, env, basket_id, send_to, repay_for, amount),
        ExecuteMsg::Callback( msg ) => {
            if info.sender == env.contract.address{
                callback_handler( deps, env, msg )
            }else{
                return Err( ContractError::Unauthorized {  } )
            }
        },     
    }
}

fn edit_cAsset(
    deps: DepsMut,
    info: MessageInfo,
    basket_id: Uint128,
    asset: AssetInfo,
    max_borrow_LTV: Option<Decimal>,
    max_LTV: Option<Decimal>,
) -> Result<Response, ContractError>{
    let config = CONFIG.load( deps.storage )?;

    //Assert Authority
    if info.sender != config.owner { return Err( ContractError::Unauthorized {  } ) }

    let mut basket: Basket = match BASKETS.load(deps.storage, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };

    let mut attrs = vec![ 
        attr("method", "edit_cAsset"),
        attr("basket", basket_id.clone().to_string()) ];

    let mut new_asset: cAsset;

    match basket.clone().collateral_types.into_iter().find(|cAsset| cAsset.asset.info.equal(&asset)){

        Some( mut asset ) => {
            attrs.push( attr("asset", asset.clone().asset.info.to_string() ) );

            match max_LTV{
                Some( LTV ) => {
                    
                    asset.max_LTV = LTV.clone();
                    attrs.push( attr("max_LTV", LTV.to_string() ) );
                    
                },
                None => {},
            }
            match max_borrow_LTV{
                Some( LTV ) => {
                    if LTV < Decimal::percent(100) && LTV < asset.max_LTV {
                        asset.max_borrow_LTV = LTV.clone();
                        attrs.push( attr("max_borrow_LTV", LTV.to_string() ) );
                    }
                },
                None => {},
            }
            new_asset = asset;
        },
        None => { return Err( ContractError::CustomError { val: format!("Collateral type doesn't exist in basket {}", basket_id.clone().to_string()) } ) }
    };
    //Set and Save new basket
    basket.collateral_types = basket.clone().collateral_types
        .into_iter()
        .filter(|asset| !asset.asset.info.equal(&new_asset.asset.info))
        .collect::<Vec<cAsset>>();

    basket.collateral_types.push( new_asset );

    BASKETS.save( deps.storage, basket_id.to_string(), &basket )?;

    Ok( Response::new().add_attributes(attrs) )
}

fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    stability_pool: Option<String>,
    dex_router: Option<String>,
    osmosis_proxy: Option<String>,
    debt_auction: Option<String>,
    staking_contract: Option<String>,
    interest_revenue_collector: Option<String>,
    liq_fee: Option<Decimal>,
    debt_minimum: Option<Uint128>,
    oracle_time_limit: Option<u64>,
    twap_timeframe: Option<u64>,
) -> Result<Response, ContractError>{

    let mut config = CONFIG.load( deps.storage )?;

    //Assert Authority
    if info.sender != config.owner { return Err( ContractError::Unauthorized {  } ) }

    let mut attrs = vec![
        attr( "method", "update_config" ),  
    ];

    //Match Optionals
    match owner {
        Some( owner ) => { 
            let valid_addr = deps.api.addr_validate(&owner)?;
            config.owner = valid_addr.clone();
            attrs.push( attr("new_owner", valid_addr.to_string()) );
        },
        None => {},
    }
    match stability_pool {
        Some( stability_pool ) => { 
            let valid_addr = deps.api.addr_validate(&stability_pool)?;
            config.stability_pool = Some( valid_addr.clone() );
            attrs.push( attr("new_stability_pool", valid_addr.to_string()) );
        },
        None => {},
    }
    match dex_router {
        Some( dex_router ) => { 
            let valid_addr = deps.api.addr_validate(&dex_router)?;
            config.dex_router = Some( valid_addr.clone() );
            attrs.push( attr("new_dex_router", valid_addr.to_string()) );
        },
        None => {},
    }
    match osmosis_proxy {
        Some( osmosis_proxy ) => { 
            let valid_addr = deps.api.addr_validate(&osmosis_proxy)?;
            config.osmosis_proxy = Some( valid_addr.clone() );
            attrs.push( attr("new_osmosis_proxy", valid_addr.to_string()) );
        },
        None => {},
    }
    match debt_auction {
        Some( debt_auction ) => { 
            let valid_addr = deps.api.addr_validate(&debt_auction)?;
            config.debt_auction = Some( valid_addr.clone() );
            attrs.push( attr("new_debt_auction", valid_addr.to_string()) );
        },
        None => {},
    }
    match staking_contract {
        Some( staking_contract ) => { 
            let valid_addr = deps.api.addr_validate(&staking_contract)?;
            config.staking_contract = Some( valid_addr.clone() );
            attrs.push( attr("new_staking_contract", valid_addr.to_string()) );
        },
        None => {},
    }
    match interest_revenue_collector {
        Some( interest_revenue_collector ) => { 
            let valid_addr = deps.api.addr_validate(&interest_revenue_collector)?;
            config.interest_revenue_collector = Some( valid_addr.clone() );
            attrs.push( attr("new_interest_revenue_collector", valid_addr.to_string()) );
        },
        None => {},
    }
    match liq_fee {
        Some( liq_fee ) => { 
            config.liq_fee = liq_fee.clone();
            attrs.push( attr("new_liq_fee", liq_fee.to_string()) );
        },
        None => {},
    }
    match debt_minimum {
        Some( debt_minimum ) => { 
            config.debt_minimum = debt_minimum.clone();
            attrs.push( attr("new_debt_minimum", debt_minimum.to_string()) );
        },
        None => {},
    }
    match oracle_time_limit {
        Some( oracle_time_limit ) => { 
            config.oracle_time_limit = oracle_time_limit.clone();
            attrs.push( attr("new_oracle_time_limit", oracle_time_limit.to_string()) );
        },
        None => {},
    }
    match twap_timeframe {
        Some( twap_timeframe ) => { 
            config.twap_timeframe = twap_timeframe.clone();
            attrs.push( attr("new_oracle_time_limit", twap_timeframe.to_string()) );
        },
        None => {},
    }

    //Save new Config
    CONFIG.save(deps.storage, &config)?;

    Ok( Response::new().add_attributes( attrs ) )
    
}

pub fn callback_handler(
    deps: DepsMut,
    env: Env,
    msg: CallbackMsg,
) -> Result<Response, ContractError>{
    
    match msg {
        CallbackMsg::BadDebtCheck { basket_id, position_owner, position_id } => {
            check_for_bad_debt( deps, env, basket_id, position_id, position_owner )
        },
    }
}

fn check_for_bad_debt(
    deps: DepsMut,
    env: Env,
    basket_id: Uint128,
    position_id: Uint128,
    position_owner: Addr,
) -> Result<Response, ContractError>{

    let config: Config = CONFIG.load( deps.storage )?;

    let basket: Basket = match BASKETS.load(deps.storage, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };
    let positions: Vec<Position> = match POSITIONS.load(deps.storage, (basket_id.to_string(), position_owner.clone())){
        Err(_) => {  return Err(ContractError::NoUserPositions {  }) },
        Ok( positions ) => { positions },
    };

    //Filter position by id
    let target_position = match positions.into_iter().find(|x| x.position_id == position_id) {
        Some(position) => position,
        None => return Err(ContractError::NonExistentPosition {  }) 
    };

    //We do a lazy check for bad debt by checking if there is debt without any assets left in the position
    //This is allowed bc any calls here will be after a liquidation where the sell wall would've sold all it could to cover debts
    let total_assets: Uint128 = 
        target_position.collateral_assets
            .iter()
            .map(|asset| asset.asset.amount)
            .collect::<Vec<Uint128>>()
            .iter()
            .sum();

    if total_assets > Uint128::zero() || target_position.credit_amount.is_zero(){
        return Err( ContractError::PositionSolvent {  } )
    } else {

        let mut messages: Vec<CosmosMsg> = vec![];
        let mut bad_debt_amount = target_position.credit_amount;

        //If the basket has revenue, mint and repay the bad debt
        if !basket.pending_revenue.is_zero() {

            if bad_debt_amount >= basket.pending_revenue {
                
                //If bad_debt is greater or equal, mint all revenue to repay
                //and send the rest to the auction
                let mint_msg = ExecuteMsg::MintRevenue { 
                    basket_id, 
                    send_to: None, 
                    repay_for: Some( UserInfo {
                        basket_id,
                        position_id,
                        position_owner: position_owner.to_string(),
                    }), 
                    amount: None, 
                };
    
                messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: env.contract.address.to_string(), 
                    msg: to_binary(&mint_msg)?, 
                    funds: vec![ ],
                }));

                bad_debt_amount -= basket.pending_revenue;
            } else {
                
                //If less than revenue, repay the debt and no auction
                let mint_msg = ExecuteMsg::MintRevenue { 
                    basket_id, 
                    send_to: None, 
                    repay_for: Some( UserInfo {
                        basket_id,
                        position_id,
                        position_owner: position_owner.to_string(),
                    }), 
                    amount: Some( bad_debt_amount ), 
                };
    
                messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: env.contract.address.to_string(), 
                    msg: to_binary(&mint_msg)?, 
                    funds: vec![ ],
                }));

                bad_debt_amount = Uint128::zero();
            }
            
        }

        //Send bad debt amount to the auction contract if greater than 0
        if config.debt_auction.is_some() && !bad_debt_amount.is_zero(){
            let auction_msg = AuctionExecuteMsg::StartAuction {
                    basket_id, 
                    position_id, 
                    position_owner: position_owner.to_string(), 
                    debt_amount: bad_debt_amount,
                };

            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.debt_auction.unwrap().to_string(), 
                msg: to_binary(&auction_msg)?, 
                funds: vec![ ],
            }));
        }else{
            return Err( ContractError::CustomError { val: "Debt Auction contract not added to config".to_string() } )
        }

        return Ok( Response::new().add_messages(messages)
            .add_attributes(vec![
                attr("method", "check_for_bad_debt"),
                attr("bad_debt_amount", bad_debt_amount)
            ]) )
    }

}

//From a receive cw20 hook. Comes from the contract address so easy to validate sent funds. 
//Check if sent funds are equal to amount in msg so we don't have to recheck in the function
pub fn receive_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {

    let passed_asset: Asset = Asset {
        info: AssetInfo::Token {
            address: info.sender.clone(),
        },
        amount: cw20_msg.amount,
    };

    match from_binary(&cw20_msg.msg){
        //This only allows 1 cw20 token at a time when opening a position, whereas you can add multiple native assets
        Ok(Cw20HookMsg::Deposit { position_owner, basket_id, position_id}) => {      
            let valid_owner_addr: Addr = if let Some(position_owner) = position_owner {
                deps.api.addr_validate(&position_owner)?
            }else {
                deps.api.addr_validate(&cw20_msg.sender.clone())?
            };

            let cAssets: Vec<cAsset> = assert_basket_assets(deps.storage, deps.querier, env.clone(), basket_id, vec![ passed_asset ], true)?;

            deposit(deps, env, info, Some(valid_owner_addr.to_string()), position_id, basket_id, cAssets) 
        },
        Err(_) => Err(ContractError::Cw20MsgError {}),
    }

}



#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    //panic!("here".to_string());
    match msg.id {
        LIQ_QUEUE_REPLY_ID => handle_liq_queue_reply(deps, msg, env),
        STABILITY_POOL_REPLY_ID => handle_stability_pool_reply(deps, env, msg),
        SELL_WALL_REPLY_ID => handle_sell_wall_reply(deps, msg, env),
        CREATE_DENOM_REPLY_ID => handle_create_denom_reply(deps, msg),
        WITHDRAW_REPLY_ID => handle_withdraw_reply( deps, env, msg),
        BAD_DEBT_REPLY_ID => Ok( Response::new() ),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
}

fn handle_withdraw_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply
) -> StdResult<Response>{
    match msg.result.into_result(){
        Ok( _result ) => {
            let mut withdraw_prop = WITHDRAW.load( deps.storage )?;  
            
            let asset_info: AssetInfo = withdraw_prop.positions_prev_collateral[0].clone().info;
            let position_amount: Uint128 = withdraw_prop.positions_prev_collateral[0].amount;
            let withdraw_amount: Uint128 = withdraw_prop.withdraw_amounts[0];

            let current_asset_balance = match get_contract_balances( deps.querier, env, vec![ asset_info ] ){
                Ok( balances ) => { balances[0] },
                Err( err ) => return Err( StdError::GenericErr { msg: err.to_string() })
            };

            //If balance differnce is more than what they tried to withdraw or position amount, error
            if withdraw_prop.contracts_prev_collateral_amount[0] - current_asset_balance > position_amount || withdraw_prop.contracts_prev_collateral_amount[0] - current_asset_balance > withdraw_amount {
                return Err( StdError::GenericErr { msg: String::from("Invalid withdrawal, possible bug found") } )
            }
            
            //Remove the first entry from each field
            withdraw_prop.positions_prev_collateral.remove(0);
            withdraw_prop.withdraw_amounts.remove(0);
            withdraw_prop.contracts_prev_collateral_amount.remove(0);

            //Save new prop
            WITHDRAW.save( deps.storage, &withdraw_prop )?;

            //We can go by first entries for these fields bc the replies will come in FIFO in terms of assets sent
            //This only works bc we send native tokens one at a time

        },//We only reply on success 
        Err( err ) => {return Err( StdError::GenericErr { msg: err } )}

    }


    Ok( Response::new() ) 
}

fn handle_create_denom_reply(deps: DepsMut, msg: Reply) -> StdResult<Response>{
    match msg.result.into_result(){
        Ok( result ) => {

            let instantiate_event = result
                .events
                .into_iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "basket_id")
                })
                .ok_or_else(|| StdError::generic_err(format!("unable to find create_denom event")))?;

            let subdenom = &instantiate_event.attributes
                .iter()
                .find(|attr| attr.key == "subdenom")
                .unwrap()
                .value;

            let basket_id = &instantiate_event.attributes
                .iter()
                .find(|attr| attr.key == "basket_id")
                .unwrap()
                .value;

            let config: Config = CONFIG.load( deps.storage )?;

            //Query fulldenom to save to basket 
            let res: GetDenomResponse = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: config.clone().osmosis_proxy.unwrap().to_string(),
                msg: to_binary(&OsmoQueryMsg::GetDenom {   
                    creator_address: config.osmosis_proxy.unwrap().to_string(),
                    subdenom: subdenom.to_string(),
                })?,
            }))?;

            BASKETS.update( deps.storage, basket_id.to_string(), |basket| -> StdResult<Basket>{
                match basket{
                    Some( mut basket ) => {
                        
                        basket.credit_asset = Asset {
                            info: AssetInfo::NativeToken { denom: res.denom },
                            ..basket.credit_asset
                        };

                        Ok( basket )
                    },
                    None => {return Err( StdError::GenericErr { msg: "Non-existent basket".to_string() } )},
                }
            })?;
                
        },//We only reply on success 
        Err( err ) => {return Err( StdError::GenericErr { msg: err } )}

    }


    Ok( Response::new() ) 
}


fn handle_stability_pool_reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response>{

    match msg.result.into_result(){
         Ok(result)  => {
            //1) Parse potential leftover amount and send to sell_wall if there is any
            //Don't need to change state bc the SP will be repaying thru the contract
            //There should only be leftover here if the SP loses funds between the query and the repayment
            //2) Send collateral to the SP in the repay function and call distribute

            let mut res = Response::new();

            let liq_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "leftover_repayment")
                })
                .ok_or_else(|| StdError::generic_err(format!("unable to find stability pool event")))?;

            let leftover = &liq_event.attributes
                .iter()
                .find(|attr| attr.key == "leftover_repayment")
                .unwrap()
                .value;

            let leftover_amount = Uint128::from_str(&leftover)?;


            let mut repay_propagation = REPAY.load(deps.storage)?;
            let mut submessages = vec![];

            //Success w/ leftovers: Sell Wall combined leftovers
            //Success w/o leftovers: Send LQ leftovers to the SP
            //Error: Sell Wall combined leftovers
            if leftover_amount != Uint128::zero(){


                //Sell Wall SP leftovers and LQ leftovers
                let ( sell_wall_msgs, collateral_distributions ) = sell_wall_using_ids( 
                    deps.storage,
                    env,
                    deps.querier,
                    repay_propagation.clone().basket_id,
                    repay_propagation.clone().position_id,
                    repay_propagation.clone().position_owner,
                    repay_propagation.clone().liq_queue_leftovers + Decimal::from_ratio(leftover_amount, Uint128::new(1u128)),
                    )?;
        
                submessages.extend( sell_wall_msgs.
                    into_iter()
                    .map(|msg| {
                        
                        SubMsg::reply_on_success(msg, SELL_WALL_REPLY_ID)
                    }).collect::<Vec<SubMsg>>() );
                    
                
                repay_propagation.sell_wall_distributions = add_distributions( repay_propagation.clone().sell_wall_distributions, SellWallDistribution {distributions: collateral_distributions} , );
                
                //Save to propagate
                REPAY.save(deps.storage, &repay_propagation)?;
                
            }else{
                //Send LQ leftovers to SP
                //This is an SP reply so we don't have to check if the SP is okay to call 
                let config: Config = CONFIG.load(deps.storage)?;

                let basket: Basket = BASKETS.load(deps.storage, repay_propagation.clone().basket_id.to_string() )?;
                
                //let sp_liq_fee = query_stability_pool_fee( deps.querier, config.clone(), basket.clone() )?;

                //Check for stability pool funds before any liquidation attempts
                //Sell wall any leftovers
                let leftover_repayment = 
                        query_stability_pool_liquidatible(
                            deps.querier, 
                            config.clone(), 
                            repay_propagation.liq_queue_leftovers,
                             basket.clone().credit_asset.info
                        )?;

                        
                if leftover_repayment > Decimal::zero(){

                    //Sell wall remaining
                    let ( sell_wall_msgs, collateral_distributions ) = sell_wall_using_ids( 
                        deps.storage,
                        env,
                        deps.querier, 
                        repay_propagation.clone().basket_id,
                        repay_propagation.clone().position_id,
                        repay_propagation.clone().position_owner,
                        leftover_repayment,
                        )?;
                    
                    //Save new distributions from this liquidations
                    repay_propagation.sell_wall_distributions = add_distributions(repay_propagation.sell_wall_distributions, SellWallDistribution {distributions: collateral_distributions} );
                    REPAY.save(deps.storage, &repay_propagation)?;

                    submessages.extend( sell_wall_msgs.
                        into_iter()
                        .map(|msg| {
                            //If this succeeds, we update the positions collateral claims
                            //If this fails, do nothing. Try again isn't a useful alternative.
                            SubMsg::reply_on_success(msg, SELL_WALL_REPLY_ID)
                        }).collect::<Vec<SubMsg>>() );

                }
                //Send whatever u can to the Stability Pool
                let sp_repay_amount = repay_propagation.liq_queue_leftovers - leftover_repayment;
                
                
                if !sp_repay_amount.is_zero(){
                    //Stability Pool message builder
                    let liq_msg = SP_ExecuteMsg::Liquidate {
                        credit_asset: LiqAsset{
                            amount: sp_repay_amount,
                            info: basket.clone().credit_asset.info,
                        },
                    };
                    
                    let msg: CosmosMsg =  CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: config.stability_pool.unwrap().to_string(),
                        msg: to_binary(&liq_msg)?,
                        funds: vec![],
                    });

                    let sub_msg: SubMsg = SubMsg::reply_always(msg, STABILITY_POOL_REPLY_ID);

                    submessages.push( sub_msg );

                    //Remove repayment from leftovers
                    repay_propagation.liq_queue_leftovers -= sp_repay_amount;
                    REPAY.save(deps.storage, &repay_propagation)?;
                    

                }
                
            }
            
            //TODO: Add detail
            Ok( res.add_submessages(submessages) )

             
            
        },
        Err( _ ) => {
            //If error, sell wall the SP repay amount and LQ leftovers
            let mut repay_propagation = REPAY.load(deps.storage)?;

            //Sell wall remaining
            let ( sell_wall_msgs, collateral_distributions ) = sell_wall_using_ids( 
                deps.storage,
                env,
                deps.querier,
                repay_propagation.clone().basket_id,
                repay_propagation.clone().position_id,
                repay_propagation.clone().position_owner,
                repay_propagation.liq_queue_leftovers + repay_propagation.stability_pool,
                )?;

            
            
            //Save new distributions from this liquidations
            repay_propagation.sell_wall_distributions = add_distributions(repay_propagation.sell_wall_distributions, SellWallDistribution {distributions: collateral_distributions} );
            REPAY.save(deps.storage, &repay_propagation)?;
            
            let res = Response::new().add_submessages( sell_wall_msgs.
                into_iter()
                .map(|msg| {
                    //If this succeeds, we update the positions collateral claims
                    //If this fails, do nothing. Try again isn't a useful alternative.
                    SubMsg::reply_on_success(msg, SELL_WALL_REPLY_ID)
                }).collect::<Vec<SubMsg>>() );

            //TODO: Add detail
            Ok( res )

        }        
    }        
}

//Add to the front of the "queue" bc message semantics are depth first
//LIFO
fn add_distributions(
    mut old_distributions: Vec<SellWallDistribution>,
    new_distrbiutions: SellWallDistribution,
)-> Vec<SellWallDistribution>{
    
    old_distributions.push( new_distrbiutions );

    old_distributions
}

fn handle_liq_queue_reply(deps: DepsMut, msg: Reply, env: Env) -> StdResult<Response>{

    match msg.result.into_result(){
         Ok(result)  => {
            //1) Parse potential repaid_amount and substract from running total
            //2) Send collateral to the Queue
            

            let liq_event = result
                .events
                .into_iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "repay_amount")
                })
                .ok_or_else(|| StdError::generic_err(format!("unable to find liq-queue event")))?;

            let repay = &liq_event.attributes
                .iter()
                .find(|attr| attr.key == "repay_amount")
                .unwrap()
                .value;

            
            let repay_amount = Uint128::from_str(&repay)?;

            let mut prop: RepayPropagation = REPAY.load(deps.storage)?;

            let basket = BASKETS.load(deps.storage, prop.basket_id.to_string())?;
            
            let config = CONFIG.load(deps.storage)?;

            //Send successfully liquidated amount
            let amount = &liq_event.attributes
                .iter()
                .find(|attr| attr.key == "collateral_amount")
                .unwrap()
                .value;

            let send_amount = Uint128::from_str(&amount)?;

            let token = &liq_event.attributes
                .iter()
                .find(|attr| attr.key == "collateral_token")
                .unwrap()
                .value;

            let asset_info = &liq_event.attributes
                .iter()
                .find(|attr| attr.key == "collateral_info")
                .unwrap()
                .value;
            
            let token_info: AssetInfo = if asset_info.eq(&"token".to_string()){
                    AssetInfo::Token { address: deps.api.addr_validate(&token)? }
                } else {
                    AssetInfo::NativeToken { denom: token.to_string() }
                };
            

            let msg = withdrawal_msg( 
                Asset {
                    info: token_info.clone(),
                    amount: send_amount,
                },
                basket.liq_queue.unwrap()
             )?;

                          
             //Subtract repaid amount from LQs repay responsibility. If it hits 0 then there were no LQ errors.
             if repay_amount != Uint128::zero(){

                prop.liq_queue_leftovers = decimal_subtraction( prop.liq_queue_leftovers, Decimal::from_ratio(repay_amount, Uint128::new(1u128)));              

                REPAY.save(deps.storage, &prop)?;
                //SP reply handles LQ_leftovers 

                update_position_claims(deps.storage, deps.querier, env, prop.basket_id, prop.position_id, prop.position_owner, token_info, send_amount)?;
            }

            
            //TODO: Add detail
            Ok(Response::new().add_message(msg))

             
            
        },
        Err( string ) => {
            //If error, do nothing
            //The SP reply will handle the sell wall
            Ok( Response::new().add_attribute( "error", string) )
        }        
    }        
}

fn handle_sell_wall_reply(deps: DepsMut, msg: Reply, env: Env) -> StdResult<Response>{
    
    match msg.result.into_result(){ 

        Ok( _result ) => {
            //On success we update the position owner's claims bc it means the protocol sent assets on their behalf
            let mut repay_propagation = REPAY.load( deps.storage )?;
            
            let mut res = Response::new();
            let mut attrs = vec![];

            //We use the distribution at the end of the list bc new ones were appended, and msgs are fulfilled depth first.
            match repay_propagation.sell_wall_distributions.pop(){
                Some( distribution ) => {

                    //Update position claims for each distributed asset
                    for (asset, amount) in distribution.distributions{
                        update_position_claims(
                            deps.storage, 
                            deps.querier, 
                            env.clone(),
                            repay_propagation.clone().basket_id, 
                            repay_propagation.clone().position_id, 
                            repay_propagation.clone().position_owner, 
                            asset.clone(), 
                            (amount * Uint128::new(1u128)),
                        )?;

                        let res_asset = LiqAsset {
                            info: asset,
                            amount,
                        };
                        attrs.push( ("distribution", res_asset.to_string()) );
                    }
                },
                None => { 
                    //If None it means the distribution wasn't added when the sell wall msg was added which should be impossible 
                    //Either way, Error
                    return Err( StdError::GenericErr { msg: "Distributions were added to the state propagation incorrectly".to_string() } )
                    }       
            }

            //Save propagation w/ removed tail
            REPAY.save(deps.storage, &repay_propagation)?;

            Ok( res.add_attributes(attrs) )
            
        }
        Err( string ) => {
            //This is only reply_on_success so this shouldn't be reached
            Ok( Response::new().add_attribute( "error", string) )
        }        
    }
}



#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => { to_binary(&query_config(deps)?) }
        QueryMsg::GetPosition { position_id, basket_id, position_owner} => {
            let valid_addr: Addr = deps.api.addr_validate(&position_owner)?;
            to_binary(&query_position(deps, env, position_id, basket_id, valid_addr)?)
        },
        QueryMsg::GetUserPositions { basket_id, user, limit } => {
            let valid_addr: Addr = deps.api.addr_validate(&user)?;
            to_binary(&query_user_positions(deps, env, basket_id, valid_addr, limit)?)
        },
        QueryMsg::GetBasketPositions { basket_id, start_after, limit } => {
            to_binary(&query_basket_positions(deps, basket_id, start_after, limit)?)
        },
        QueryMsg::GetBasket { basket_id } => {
            to_binary(&query_basket(deps, basket_id)?)
        },
        QueryMsg::GetAllBaskets { start_after, limit } => {
            to_binary(&query_baskets(deps, start_after, limit)?)
        },
        QueryMsg::Propagation {  } => {
            to_binary(&query_prop( deps )?)
        },
        QueryMsg::GetBasketDebtCaps { basket_id } => {
            to_binary( &query_basket_debt_caps(deps, env, basket_id)?)
        },
        QueryMsg::GetBasketBadDebt { basket_id } => {
            to_binary( &query_bad_debt( deps, basket_id )? ) 
        },
        QueryMsg::GetBasketInsolvency { basket_id, start_after, limit } => {
            to_binary( &query_basket_insolvency(deps, env, basket_id, start_after, limit)? )
        },
        QueryMsg::GetPositionInsolvency { basket_id, position_id, position_owner } => {
            to_binary( &query_position_insolvency(deps, env, basket_id, position_id, position_owner)? )
        }        
    }
}