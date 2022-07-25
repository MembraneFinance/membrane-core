

use std::str::FromStr;

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{to_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, StdError, Addr, Uint128, QueryRequest, WasmQuery, Decimal, CosmosMsg, WasmMsg, BankMsg, Coin, from_binary, Order, Storage, Api, QuerierWrapper, Querier, SubMsg, Reply};
use cw2::set_contract_version;
use cw_storage_plus::Bound;

use crate::cw20::{Cw20ReceiveMsg, Cw20ExecuteMsg};
use crate::math::{decimal_multiplication, decimal_division, decimal_subtraction};
use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, AssetInfo, Cw20HookMsg, Asset, PositionResponse, PositionsResponse, BasketResponse, LiqModuleMsg};
use crate::stability_pool::{Cw20HookMsg as SP_Cw20HookMsg, QueryMsg as SP_QueryMsg, LiquidatibleResponse, ExecuteMsg as SP_ExecuteMsg};
use crate::state::{Config, CONFIG, Position, POSITIONS, cAsset, Basket, BASKETS, LiqAsset, RepayPropagation, REPAY, RepayFee};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:cdp";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const PRICE_EXPIRE_TIME: u64 = 60;

const STABILITY_POOL_REPLY_ID: u64 = 1u64;
const LIQ_QUEUE_REPLY_ID: u64 = 2u64;

//TODO: //Add function to update existing cAssets and Baskets and Config

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    
  
    let config = Config {
        owner: info.sender.clone(),
        current_basket_id: Uint128::from(1u128),
        stability_pool: None,
    }; 

    let current_basket_id = &config.current_basket_id.clone().to_string();

    CONFIG.save(deps.storage, &config)?;

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let response = Response::new();
    let mut attrs = vec![];
    let sender = &info.sender.clone().to_string();

    attrs.push(("method", "instantiate"));
    attrs.push(("owner", sender));
    

    if msg.collateral_types.is_some() && msg.credit_asset.is_some(){

        let mut check = true;
        let collateral_types = msg.collateral_types.unwrap();

        //cAsset checks
        for cAsset in collateral_types.clone(){
            if cAsset.max_borrow_LTV >= cAsset.max_LTV && cAsset.max_borrow_LTV < Decimal::percent(100){
                check = false;
            }
        }
        if( check ){
            let _res = create_basket(
                deps,
                info,
                Some(config.owner.to_string()),
                collateral_types.clone(),
                msg.credit_asset.unwrap(),
                msg.credit_price,
                msg.credit_interest,
            )?;
            
            attrs.push(("basket_id", current_basket_id));
        }else{
            attrs.push(("basket_status", "Not created: cAsset.max_LTV can't be less than or equal to cAsset.max_borrow_LTV"));
        }
        
    }else{
        attrs.push(("basket_status", "Not created: Basket only created w/ collateral_types AND credit_asset filled"));
    }

    //response.add_attributes(attrs);
    Ok(response.add_attributes(attrs))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Receive(msg) => receive_cw20(deps, env, info, msg),
        ExecuteMsg::Deposit{ assets, position_owner, position_id, basket_id} => {
            for asset in assets.clone(){
                assert_sent_native_token_balance(&asset, &info)?;
            }
            let cAssets: Vec<cAsset> = assert_basket_assets(deps.storage, basket_id, assets.clone())?;
            deposit(deps, info, position_owner, position_id, basket_id, cAssets)
        }
    ,
        ExecuteMsg::Withdraw{ position_id, basket_id, assets } => {
            
            let cAssets: Vec<cAsset> = assert_basket_assets(deps.storage, basket_id, assets)?;
            withdraw(deps, info, position_id, basket_id, cAssets)
        },
        
        ExecuteMsg::IncreaseDebt { basket_id, position_id, amount } => increase_debt(deps, info, basket_id, position_id, amount),
        ExecuteMsg::Repay { basket_id, position_id, position_owner, credit_asset } => {
            assert_sent_native_token_balance(&credit_asset, &info)?;
            repay(deps, info, basket_id, position_id, position_owner, credit_asset)
        },
        ExecuteMsg::LiqRepay { credit_asset, collateral_asset, fee_ratios } => {
            assert_sent_native_token_balance(&credit_asset, &info)?;
            liq_repay(deps, info, credit_asset, collateral_asset, fee_ratios)
        }
        ExecuteMsg::EditAdmin { owner } => edit_contract_owner(deps, info, owner),
        ExecuteMsg::EditBasket { basket_id, added_cAsset, owner, credit_interest } => edit_basket(deps, info, basket_id, added_cAsset, owner, credit_interest),
        ExecuteMsg::CreateBasket { owner, collateral_types, credit_asset, credit_price, credit_interest } => create_basket(deps, info, owner, collateral_types, credit_asset, credit_price, credit_interest),
     

    }
}



//From a receive cw20 hook. Comes from the contract address so easy to validate sent funds. 
//Check if sent funds are equal to amount in msg so we don't have to recheck in the function
pub fn receive_cw20(
    deps: DepsMut,
    _env: Env,
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

            let mut assets: Vec<Asset> = Vec::new();
            assets.push(passed_asset);
            let cAssets: Vec<cAsset> = assert_basket_assets(deps.storage, basket_id, assets)?;

            deposit(deps, info, Some(valid_owner_addr.to_string()), position_id, basket_id, cAssets) 
        },

        Ok(Cw20HookMsg::Repay { basket_id, position_id, position_owner, credit_asset }) => {

            repay(deps, info, basket_id, position_id, position_owner, credit_asset)
        }
        Err(_) => Err(ContractError::Cw20MsgError {}),
    }



}

pub fn create_basket(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    collateral_types: Vec<cAsset>,
    credit_asset: Asset,
    credit_price: Option<Decimal>,
    credit_interest: Option<Decimal>
) -> Result<Response, ContractError>{
    let mut config: Config = CONFIG.load(deps.storage)?;

    let valid_owner: Addr = validate_position_owner(deps.api, info.clone(), owner)?;

    //Only contract owner can create new baskets. This can be governance.
    if info.sender != config.owner{
        return Err(ContractError::NotContractOwner {})
    }


    let new_basket: Basket = Basket {
        owner: valid_owner.clone(),
        basket_id: config.current_basket_id.clone(),
        current_position_id: Uint128::from(1u128),
        collateral_types: collateral_types.clone(),
        credit_asset: credit_asset.clone(),
        credit_price,
        credit_interest,
    };

    BASKETS.update(deps.storage, new_basket.basket_id.to_string(), |basket| -> Result<Basket, ContractError>{
        match basket{
            Some( _basket ) => {
                //This is a new basket so there shouldn't already be one made
                return Err(ContractError::ConfigIDError {  })
            },
            None =>{
                Ok(new_basket)
            }
        }
    })?;

    config.current_basket_id += Uint128::from(1u128);
    CONFIG.save(deps.storage, &config)?;

    //Response Building
    let response = Response::new();
    let mut attrs = vec![];

    let l = &config.current_basket_id.clone().to_string();
    let i = &valid_owner.to_string();
    let v = &credit_asset.to_string();

    attrs.push(("method", "create_basket"));
    attrs.push(("basket_id", l));
    attrs.push(("position_owner", i));
    attrs.push(("credit_asset", v));

    let e = match credit_price{
        Some(x) => { x.to_string()},
        None => { "None".to_string() },
    };
    attrs.push(("credit_price", &e));
    
    let s = match credit_interest{
        Some(x) => { x.to_string()},
        None => { "None".to_string() },
    };
    attrs.push(("credit_price", &s));

    Ok(response.add_attributes(attrs))
}

pub fn edit_basket(//Can't edit basket id, current_position_id or credit_asset. Can only add cAssets. Can edit owner. Credit price can only be chaged thru the accrue function, but credit_interest is mutable here.
    deps: DepsMut,
    info: MessageInfo,
    basket_id: Uint128,
    added_cAsset: Option<cAsset>,
    owner: Option<String>,
    credit_interest: Option<Decimal>,
)->Result<Response, ContractError>{

    let new_owner: Option<Addr>;

    if let Some(owner) = owner {
        new_owner = Some(deps.api.addr_validate(&owner)?);
    }else{ new_owner = None }      
    

    BASKETS.update(deps.storage, basket_id.to_string(), |basket| -> Result<Basket, ContractError>   {

        match basket{
            Some( mut basket ) => {

                if info.sender != basket.owner{
                    return Err(ContractError::NotBasketOwner {  })
                }else{
                    if added_cAsset.is_some(){
                        basket.collateral_types.push(added_cAsset.clone().unwrap());
                    }
                    if new_owner.is_some(){
                        basket.owner = new_owner.clone().unwrap();
                    }
                    if credit_interest.is_some(){
                        basket.credit_interest = credit_interest.clone();
                    }
                }

                Ok( basket )
            },
            None => return Err(ContractError::NonExistentBasket { })
        }
    })?;

let res = Response::new();
let mut attrs = vec![];

if added_cAsset.is_some(){
    attrs.push(("asset", added_cAsset.unwrap().asset.info.to_string()));
}
if new_owner.is_some(){
    attrs.push(("owner", new_owner.unwrap().to_string()));
}
if credit_interest.is_some(){
    attrs.push(("credit_interest rate", credit_interest.unwrap().to_string()));
}

Ok(res.add_attributes(attrs))

}

pub fn edit_contract_owner(
    deps: DepsMut,
    info: MessageInfo,
    owner: String,
)-> Result<Response, ContractError>{
    if info.sender.to_string() == owner{

        let valid_owner: Addr = deps.api.addr_validate(&owner)?;
        let mut config: Config = CONFIG.load(deps.storage)?;
        
        config.owner = valid_owner;

        CONFIG.save(deps.storage, &config)?;
    }else{
        return Err(ContractError::NotContractOwner {  })
    }

    let response = Response::new()
    .add_attribute("method","edit_contract_owner")
    .add_attribute("new_owner", owner);

    Ok(response)
}

//create_position = check collateral types, create position object
pub fn create_position(
    deps: &mut dyn Storage,
    assets: Vec<Asset>, //Assets being added into the position
    basket_id: Uint128,
) -> Result<Position, ContractError> {

    let basket: Basket = match BASKETS.load(deps, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };

    //Assert assets are in the basket
    let collateral_assets: Vec<cAsset> = assert_basket_assets(deps, basket_id, assets)?;

    //increment config id
    BASKETS.update(deps, basket_id.to_string(),|basket| -> Result<_, ContractError> {
        match basket{
            Some( mut basket ) => {
                basket.current_position_id += Uint128::from(1u128);
                Ok(basket)
            },
            None => return Err(ContractError::NonExistentBasket {  }), //Due to the first check this should never get hit
        }
        
    })?;

    //Create Position instance
    let new_position: Position;

    new_position = Position {
        position_id: basket.current_position_id,
        collateral_assets,
        avg_borrow_LTV: Decimal::percent(0),
        avg_max_LTV: Decimal::percent(0),
        credit_amount: Decimal::new(Uint128::from(0u128)*Uint128::new(1000000000000000000u128)),
        basket_id,
    };   


    return Ok( new_position )
}

//Deposit collateral to existing position. New or same collateral.
//Anyone can deposit, to any position. There will be barriers for withdrawals.
pub fn deposit(
    deps: DepsMut,
    info: MessageInfo,
    position_owner: Option<String>,
    position_id: Option<Uint128>,
    basket_id: Uint128,
    cAssets: Vec<cAsset>,
) -> Result<Response, ContractError>{

    let mut new_position_id: Uint128 = Uint128::new(0u128);

    let valid_owner_addr = validate_position_owner(deps.api, info, position_owner)?;

    let basket: Basket = match BASKETS.load(deps.storage, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };

    //This has to error bc users can't withdraw without a price set. Don't want to trap users.
    if basket.credit_price.is_none(){ return Err(ContractError::NoRepaymentPrice {  })}

    let mut new_position: Position;

    //Finds the list of positions the position_owner has in the selected basket
    //POSITIONS.update(deps.storage, (basket_id.to_string(), valid_owner_addr), |positions: Option<Vec<Position>>| -> Result<Vec<Position>, ContractError>{
       
    match POSITIONS.load(deps.storage, (basket_id.to_string(), valid_owner_addr.clone())){
        
        //If Some, adds collateral to the position_id or a new position is created            
        Ok( positions) => {

            //If the user wants to create a new/separate position, no position id is passed         
            if position_id.is_some(){

                let pos_id = position_id.unwrap();
                let position = positions.clone().into_iter().find(|x| x.position_id == pos_id);

                if position.is_some() {

                    //Go thru each deposited asset to add quantity to position
                    for deposited_cAsset in cAssets.clone(){
                        let deposited_asset = deposited_cAsset.asset;

                        //HAve to reload positions each loop or else the state won't be edited for multiple deposits
                        //We can unwrap and ? safety bc of the layered conditionals
                        let position_s =  POSITIONS.load(deps.storage, (basket_id.to_string(), valid_owner_addr.clone()))?;
                        let existing_position = position_s.clone().into_iter().find(|x| x.position_id == pos_id).unwrap();

                        //Add amount if collateral asset exists in the position
                        let temp_cAsset: Option<cAsset> = existing_position.clone().collateral_assets.into_iter().find(|x| x.asset.info.equal(&deposited_asset.clone().info));

                        match temp_cAsset {
                            Some(cAsset) => {
                                let new_cAsset = cAsset{
                                    asset: Asset {
                                        amount: cAsset.clone().asset.amount + deposited_asset.clone().amount,
                                        info: cAsset.clone().asset.info,
                                    },
                                    oracle: cAsset.clone().oracle,
                                    max_borrow_LTV: cAsset.clone().max_borrow_LTV,
                                    max_LTV: cAsset.clone().max_LTV,
                                };

                                let mut temp_list: Vec<cAsset> = existing_position.clone().collateral_assets.into_iter().filter(|x| !x.asset.info.equal(&deposited_asset.clone().info)).collect::<Vec<cAsset>>();
                                temp_list.push(new_cAsset);

                                let temp_pos = Position {
                                    position_id: existing_position.clone().position_id,
                                    collateral_assets: temp_list,
                                    avg_borrow_LTV: existing_position.clone().avg_borrow_LTV, //We don't recalc bc it changes w/ price, leave it for solvency chcks
                                    avg_max_LTV: existing_position.clone().avg_max_LTV,
                                    credit_amount: existing_position.clone().credit_amount,
                                    basket_id: existing_position.clone().basket_id,
                                };


                                POSITIONS.update(deps.storage, (basket_id.to_string(), valid_owner_addr.clone()), |positions| -> Result<Vec<Position>, ContractError> 
                                {
                                    let unwrapped_pos = positions.unwrap();

                                    let mut update = unwrapped_pos.clone().into_iter().filter(|x| x.position_id != pos_id).collect::<Vec<Position>>();
                                    update.push(temp_pos);

                                    Ok( update )

                                })?;
                                

                            },
                            
                            // //if not, add cAsset to Position if in Basket options
                            None => {
                                let mut assets: Vec<Asset> = Vec::new();
                                assets.push(deposited_asset.clone());
                                let new_cAsset = assert_basket_assets(deps.storage, basket_id, assets)?;

                                POSITIONS.update(deps.storage, (basket_id.to_string(), valid_owner_addr.clone()), |positions| -> Result<Vec<Position>, ContractError> 
                                {
                                    let temp_pos = positions.unwrap();
                                                                      
                                    let position = temp_pos.clone().into_iter().find(|x| x.position_id == pos_id);
                                    let mut p = position.clone().unwrap();
                                    p.collateral_assets.push(
                                        cAsset{
                                            asset: deposited_asset, 
                                            oracle: new_cAsset[0].clone().oracle,
                                            max_borrow_LTV:  new_cAsset[0].clone().max_borrow_LTV,
                                            max_LTV:  new_cAsset[0].clone().max_LTV,                                            
                                        }
                                    );

                                    let mut update = temp_pos.clone().into_iter().filter(|x| x.position_id != pos_id).collect::<Vec<Position>>();
                                    update.push( p );
                                    
                                    Ok( update )
                                        
                                })?;

                                
                            }
                        }

                    }
                    
                
                }else{
                    //If position_ID is passed but no position is found. In case its a mistake, don't want to add a new position.
                    return Err(ContractError::NonExistentPosition {  }) 
                }

            }else{
                //If user doesn't pass an ID, we create a new position
                let mut assets: Vec<Asset> = Vec::new();
                for cAsset in cAssets.clone(){
                    assets.push(cAsset.asset);
                }

                new_position = create_position(deps.storage, assets, basket_id)?;
                
                //For response
                new_position_id = new_position.clone().position_id;
                
                //Need to add new position to the old set of positions if a new one was created.
                POSITIONS.update(deps.storage, (basket_id.to_string(), valid_owner_addr.clone()), |positions| -> Result<Vec<Position>, ContractError> 
                {
                    let unwrapped_pos = positions.unwrap();

                    let mut update = unwrapped_pos.clone().into_iter().filter(|x| x.position_id != new_position_id.clone()).collect::<Vec<Position>>();
                    update.push( new_position );

                    Ok( update )

                })?;

            }
    

        
    },
    // If Err() meaning no positions loaded, new position is created 
    Err(_) => {
        let mut assets: Vec<Asset> = Vec::new();
        for cAsset in cAssets.clone(){
                assets.push(cAsset.asset);
            }

            new_position = create_position(deps.storage, assets, basket_id)?;
                
            //For response
            new_position_id = new_position.clone().position_id;
            
            //Need to add new position to the old set of positions if a new one was created.
            POSITIONS.update(deps.storage, (basket_id.to_string(), valid_owner_addr.clone()), |_positions| -> Result<Vec<Position>, ContractError> 
            {
                let mut update: Vec<Position> = Vec::new();

                //let mut update = unwrapped_pos.clone().into_iter().filter(|x| x.position_id != new_position_id.clone()).collect::<Vec<Position>>();
                update.push( new_position );

                Ok( update )

            })?;
        }
    };

    //Response build
    let response = Response::new();
    let mut attrs = vec![];

    attrs.push(("method", "deposit"));

    let b = &basket_id.to_string();
    attrs.push(("basket_id", b));

    let v = &valid_owner_addr.to_string();
    attrs.push(("position_owner", v));

    let p = &position_id.unwrap_or_else(|| new_position_id).to_string();
    attrs.push(("position_id", p));

    let assets: Vec<String> = cAssets.iter().map(|x| x.asset.clone().to_string()).collect();
    
    for i in 0..assets.clone().len(){
        attrs.push(("assets", &assets[i]));    
    }

    Ok( response.add_attributes(attrs) )

}

pub fn withdraw(
    deps: DepsMut,
    info: MessageInfo,
    position_id: Uint128,
    basket_id: Uint128,
    cAssets: Vec<cAsset>,
) ->Result<Response, ContractError>{
   
    let basket: Basket = match BASKETS.load(deps.storage, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };
    
    let mut message: CosmosMsg;
    let mut msgs = vec![];
    let response = Response::new();
    

    

    //Each cAsset
    //We reload at every loop to account for edited state data. Elsewise users could siphon funds they don't own.
    for cAsset in cAssets.clone(){
        let withdraw_asset = cAsset.asset;

         //This forces withdrawals to be done by the info.sender 
        //so no need to check if the withdrawal is done by the position owner
        let positions: Vec<Position> = match POSITIONS.load(deps.storage, (basket_id.to_string(), info.sender.clone())){
            Err(_) => {  return Err(ContractError::NoUserPositions {  }) },
            Ok( positions ) => { positions },
        };

        //Search position by user and then filter by id
        let target_position = match positions.into_iter().find(|x| x.position_id == position_id) {
            Some(position) => position,
            None => return Err(ContractError::NonExistentPosition {  })
        };

        //If the cAsset is found in the position, attempt withdrawal 
        match target_position.clone().collateral_assets.into_iter().find(|x| x.asset.info.equal(&withdraw_asset.info)){
            //Some cAsset
            Some( position_collateral ) => {
                
                //Cant withdraw more than the positions amount
                if withdraw_asset.amount > position_collateral.asset.amount{
                    return Err(ContractError::InvalidWithdrawal {  })
                }else{
                    //Update cAsset data to account for the withdrawal
                    let leftover_amount = position_collateral.asset.amount - withdraw_asset.amount;
                    

                    let mut updated_cAsset_list: Vec<cAsset> = target_position.clone().collateral_assets
                            .into_iter()
                            .filter(|x| ! x.asset.info.equal(&withdraw_asset.info))
                            .collect::<Vec<cAsset>>();


                    //Delete asset from the position if the amount is being fully withdrawn. In this case just don't readd it
                    if leftover_amount != Uint128::new(0u128){
                        
                        let new_asset = Asset {
                            info: position_collateral.asset.info,
                            amount: leftover_amount,
                        };
    
                        let new_cAsset: cAsset = cAsset{
                            asset: new_asset,
                            ..position_collateral
                        };

                        updated_cAsset_list.push(new_cAsset);
                    }
                    
                                    
                    
                    //If resulting LTV makes the position insolvent, error. If not construct withdrawal_msg
                    if basket.credit_price.is_some(){
                        
                        if insolvency_check(deps.querier, updated_cAsset_list.clone(), target_position.clone().credit_amount, basket.credit_price.unwrap(), true)?.0{ //This is taking max_borrow_LTV so users can't max borrow and then withdraw to get a higher initial LTV
                            return Err(ContractError::PositionInsolvent {  })
                        }else{
                            
                            POSITIONS.update(deps.storage, (basket_id.to_string(), info.sender.clone()), |positions: Option<Vec<Position>>| -> Result<Vec<Position>, ContractError>{

                                match positions {
                                    
                                    //Find the position we are withdrawing from to update
                                    Some(position_list) =>  
                                        match position_list.clone().into_iter().find(|x| x.position_id == position_id) {
                                        Some(position) => {

                                            let mut updated_positions: Vec<Position> = position_list
                                            .into_iter()
                                            .filter(|x| x.position_id != position_id)
                                            .collect::<Vec<Position>>();

                                            //Leave finding LTVs for solvency checks bc it uses deps. Can't be used inside of an update function
                                            // let new_avg_LTV = get_avg_LTV(deps.querier, updated_cAsset_list)?;

                                            updated_positions.push(
                                                Position{
                                                    avg_borrow_LTV: Decimal::percent(0),
                                                    avg_max_LTV: Decimal::percent(0),
                                                    collateral_assets: updated_cAsset_list.clone(),
                                                    ..position
                                            });
                                            Ok( updated_positions )
                                        },
                                        None => return Err(ContractError::NonExistentPosition {  })
                                    },
                                
                                    None => return Err(ContractError::NoUserPositions {  }),
                                }
                            })?;
                        }
                    }else{
                        return Err(ContractError::NoRepaymentPrice {  })
                    }
                    
                    //This is here in case there are multiple withdrawal messages created.
                    message = withdrawal_msg(withdraw_asset, info.sender.clone())?;
                    msgs.push(message);
                }
                
            },
            None => return Err(ContractError::InvalidCollateral {  })
        };
        
    }

    let mut attrs = vec![];
    attrs.push(("method", "withdraw"));
    
    //These placeholders are for lifetime warnings
    let b = &basket_id.to_string();
    attrs.push(("basket_id", b));

    let p = &position_id.to_string();
    attrs.push(("position_id", p));

    let temp: Vec<String> = cAssets.into_iter().map( |cAsset|
        cAsset.asset.to_string()
    ).collect::<Vec<String>>();

    for i in 0..temp.clone().len(){
        attrs.push(("assets", &temp[i]));    
    }

    
    Ok( response.add_attributes(attrs).add_messages(msgs) )
}

pub fn repay(
    deps: DepsMut,
    info: MessageInfo,
    basket_id: Uint128,
    position_id: Uint128,
    position_owner: Option<String>,
    credit_asset: Asset,
) ->Result<Response, ContractError>{

    let basket: Basket = match BASKETS.load(deps.storage, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };

    if basket.credit_price.is_none(){
        return Err(ContractError::NoRepaymentPrice {  })
    }

    let valid_owner_addr = validate_position_owner(deps.api, info.clone(), position_owner)?;
    let response = Response::new();
    let mut attrs = vec![];
    let mut total_loan: Decimal = Decimal::percent(0);
    let mut updated_list: Vec<Position> = vec![];


    //Assert that the correct credit_asset was sent
    //Only one of these match arms will be used once the credit_contract type is decided on
    match credit_asset.clone().info {
        AssetInfo::Token { address: submitted_address } => {
            if let AssetInfo::Token { address } = basket.credit_asset.info{

                if submitted_address != address || info.sender.clone() != address {
                    return Err(ContractError::InvalidCollateral {  })
                }
            };

            
        },
        AssetInfo::NativeToken { denom: submitted_denom } => { 
           
            if let AssetInfo::NativeToken { denom } = basket.credit_asset.info{

                if submitted_denom != denom {
                    return Err(ContractError::InvalidCollateral {  })
                }

                //Assert sent tokens are the same as the Asset parameter
                assert_sent_native_token_balance( &credit_asset, &info )?;
            };
            
            
        }
    }
    
    
    POSITIONS.update(deps.storage, (basket_id.to_string(), valid_owner_addr.clone()), |positions: Option<Vec<Position>>| -> Result<Vec<Position>, ContractError>{

        match positions {

            Some(position_list) => {

               updated_list = match position_list.clone().into_iter().find(|x| x.position_id == position_id.clone()) {

                    Some( mut position) => {
                        //Can the amount be repaid?
                        if position.credit_amount >= Decimal::new(credit_asset.amount*Uint128::new(1000000000000000000u128)) {
                            //Repay amount
                            position.credit_amount -= Decimal::new(credit_asset.amount*Uint128::new(1000000000000000000u128));

                            total_loan = position.clone().credit_amount;
                        }else{
                            return Err(ContractError::ExcessRepayment {  })
                        }

                        //Create replacement Vec<Position> to update w/
                        let mut update: Vec<Position> = position_list.clone().into_iter().filter(|x| x.position_id != position_id.clone()).collect::<Vec<Position>>();
                        update.push( 
                            Position {
                                credit_amount: total_loan.clone(),
                                ..position
                            }
                         );

                        update


                    },
                    None => return Err(ContractError::NonExistentPosition {  })

                };
                
                //Now update w/ the updated_list
                //The compiler is saying this value is never read so check in tests
                Ok( updated_list )
            },
                        
            None => return Err(ContractError::NoUserPositions {  }),

            }
    
    })?;
    
    attrs.push(("method", "repay"));
    
    //These placeholders are for lifetime warnings
    let b = &basket_id.to_string();
    attrs.push(("basket_id", b));

    let p = &position_id.to_string();
    attrs.push(("position_id", p));

    let t =  &total_loan.to_string();
    attrs.push(("loan_amount",t ));
    
    
    Ok( response.add_attributes(attrs) )
}


//This is what the liquidation contracts will call to repay for a liquidation and get their collateral distribution
//1) Repay
//2) Send position collateral + fee
pub fn liq_repay(
    deps: DepsMut,
    info: MessageInfo,
    credit_asset: Asset,
    collateral_asset: Option<Asset>, //Only used by the liquidation queue since it repays by specific assets
    fee_ratios: Option<Vec<RepayFee>>, //Used by liq_queue to provide list of ratios of repay amount in said fee 
) ->Result<Response, ContractError>
{
    let config = CONFIG.load(deps.storage)?;
    let repay_propagation = REPAY.load(deps.storage)?;

    //Can only be called by the liquidation contracts 
    if info.sender != config.liq_queue.unwrap() || info.sender != config.stability_pool.unwrap(){
        return Err( ContractError::Unauthorized {  })
    } 

    let basket: Basket = match BASKETS.load(deps.storage, repay_propagation.basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };

    let positions: Vec<Position> = match POSITIONS.load(deps.storage, (repay_propagation.basket_id.to_string(), repay_propagation.position_owner.clone())){
        Err(_) => {  return Err(ContractError::NoUserPositions {  }) },
        Ok( positions ) => { positions },
    };

    let target_position = match positions.into_iter().find(|x| x.position_id == repay_propagation.position_id) {
        Some(position) => position,
        None => return Err(ContractError::NonExistentPosition {  }) 
    };


    let repay_propagation = REPAY.load(deps.storage)?;

   
    let res = match repay(deps, info, repay_propagation.basket_id, repay_propagation.position_id, Some(repay_propagation.position_owner.to_string()), credit_asset){
        Ok( res ) => { res },
        Err( e ) => { return Err( e )  }
    };


    let cAsset_ratios = get_cAsset_ratios(deps.querier, target_position.clone().collateral_assets)?;
    let (avg_borrow_LTV, avg_max_LTV, total_value, cAsset_prices) = get_avg_LTV(deps.querier, target_position.clone().collateral_assets)?;

    let repay_value = decimal_multiplication(Decimal::new(credit_asset.amount* Uint128::new(1000000000000000000u128)), basket.credit_price.unwrap());

    let mut messages = vec![];
    let mut coins: Vec<Coin> = vec![];
    
    //Some: Liq Queue (1 asset)
    //None: Stability Pool (pro rata assets)

    //TODO: Add distribute messages to the message builder, so the contract knows what to do with the received funds 
    //Only liq_queue left to do
    let distribution_assets: Vec<cAsset> = vec![];

    match collateral_asset{
        None => {
            for (num, cAsset) in target_position.clone().collateral_assets.into_iter().enumerate(){
                //Builds msgs to the sender (liq contract)
        
                let collateral_value = decimal_multiplication(repay_value, cAsset_ratios[num]);
                let collateral_amount = decimal_division(collateral_value, cAsset_prices[num]) *Uint128::new(1u128); //Multiplied by 1 to get a Uint128

                let collateral_w_fee: Decimal;

                if liq_fee.is_some(){//TODO: Query the liq fee instead
                    collateral_w_fee = decimal_multiplication(Decimal::new(collateral_amount* Uint128::new(1000000000000000000u128)), liq_fee.unwrap());
                }
                
                //SP Distribution needs list of cAsset's and is pulling the amount from the Asset object                
                match cAsset.clone().asset.info {
        
                    AssetInfo::Token { address } => {

                        //DistributionMsg builder
                        //Only adding the 1 cAsset for the CW20Msg
                        let distribution_msg = SP_Cw20HookMsg::Distribute { 
                                distribution_assets: vec![ cAsset{
                                    asset: Asset {
                                        amount: collateral_w_fee,
                                        ..cAsset.asset
                                    },
                                    ..cAsset
                                }],
                                credit_asset: credit_asset.info, 
                            };
                        
                        //CW20 Send                         
                        let msg = CosmosMsg::Wasm(WasmMsg::Execute {
                            contract_addr: address.to_string(),
                            msg: to_binary(&Cw20ExecuteMsg::Send {
                                amount: collateral_w_fee,
                                contract: info.sender.to_string(),
                                msg: to_binary(&distribution_msg)?,
                            })?,
                            funds: vec![],
                        });
                        messages.push(msg);
                    
                    }
                    AssetInfo::NativeToken { denom: _ } => {

                        //Adding each native token to the list of distribution assets
                        distribution_assets.push( cAsset{
                            asset: Asset {
                                amount: collateral_w_fee,
                                ..cAsset.asset
                            },
                            ..cAsset
                        });
        
                        let asset = Asset{ 
                            amount: collateral_w_fee,
                            ..cAsset.clone().asset
                        };
        
                        coins.push(asset_to_coin(asset)?);
                        
                    },
                }
            }
            
            //Adds Native token distribution msg to messages
            let distribution_msg = SP_ExecuteMsg::Distribute { 
                distribution_assets, 
                credit_asset: credit_asset.info,
                credit_price: basket.clone().credit_price, 
            };

            let msg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.stability_pool.unwrap().to_string(),
                msg: to_binary(&distribution_msg)?,
                funds: coins,
            });


            messages.push(msg);

        },

        Some( asset ) => {
            //Same as above but for only 1 asset
            let num = target_position.collateral_assets.iter().position(|cAsset| cAsset.asset.info.equal(&collateral_asset.unwrap().info)).unwrap();

            

            let collateral_value = decimal_multiplication(repay_value, cAsset_ratios[num]);
            let collateral_amount = decimal_division(collateral_value, cAsset_prices[num]) *Uint128::new(1u128); //Multiplied by 1 to get a Uint128

            let collateral_w_fee: Decimal;

            //Add the fee to the ratio of collateral that got liquidated at said fee
            if fee_ratios.is_some(){
                let total = vec![];

                for fees in fee_ratios.unwrap().iter(){
                    let fee_distri = decimal_multiplication(decimal_multiplication(Decimal::new(collateral_amount* Uint128::new(1000000000000000000u128)), fees.ratio), fees.fee);
                    total.push(fee_distri);
                    
                }

                collateral_w_fee = total.iter().sum();
            }
    
            match asset.clone().info {
    
                AssetInfo::Token { address } => {
                    
                    //CW20 Transfer
                    let msg = CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: address.to_string(),
                        msg: to_binary(&Cw20ExecuteMsg::Transfer {
                            amount: collateral_w_fee *Uint128::new(1u128),
                            recipient: info.sender.to_string(),
                        })?,
                        funds: vec![],
                    });
                    messages.push(msg);
                
                }
                AssetInfo::NativeToken { denom: _ } => {
    
                    let asset = Asset{ 
                        amount: collateral_w_fee *Uint128::new(1u128),
                        ..asset.clone()
                    };
    
                    coins.push(asset_to_coin(asset)?);
                    
                },
            }
        }
    }
    

    //Build the native token send msg w/ the full list of native tokens
    let msg = CosmosMsg::Bank(BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: coins,
    });

   

    messages.push(msg);
    res.add_messages(messages);

    Ok( res )
}

pub fn increase_debt(
    deps: DepsMut,
    info: MessageInfo,
    basket_id: Uint128,
    position_id: Uint128,
    amount: Uint128,
) ->Result<Response, ContractError>{

    let basket: Basket= match BASKETS.load(deps.storage, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };
    let positions: Vec<Position> = match POSITIONS.load(deps.storage, (basket_id.to_string(), info.sender.clone())){
        Err(_) => {  return Err(ContractError::NoUserPositions {  }) },
        Ok( positions ) => { positions },
    };

    //Filter position by id
    let target_position = match positions.into_iter().find(|x| x.position_id == position_id) {
        Some(position) => position,
        None => return Err(ContractError::NonExistentPosition {  }) 
    };
    let decimal_amount: Decimal = Decimal::new(amount*Uint128::new(1000000000000000000u128));
    let total_credit = target_position.credit_amount + decimal_amount;
    
    let message: CosmosMsg;

    //Can't take credit before there is a preset repayment price
    if basket.credit_price.is_some(){
        
        //If resulting LTV makes the position insolvent, error. If not construct mint msg
        //credit_value / asset_value > avg_LTV
        if insolvency_check(deps.querier, target_position.collateral_assets, total_credit, basket.credit_price.unwrap(), true)?.0 { 
            return Err(ContractError::PositionInsolvent {  })
        }else{
            
            message = credit_mint_msg(basket.credit_asset, info.sender.clone())?;
            
            //Add credit amount to the position
            POSITIONS.update(deps.storage, (basket_id.to_string(), info.sender.clone()), |positions: Option<Vec<Position>>| -> Result<Vec<Position>, ContractError>{

                match positions {
                    
                    //Find the open positions from the info.sender() in this basket
                    Some(position_list) => 

                        //Find the position we are updating
                        match position_list.clone().into_iter().find(|x| x.position_id == position_id.clone()) {

                            Some(position) => {

                                let mut updated_positions: Vec<Position> = position_list
                                .into_iter()
                                .filter(|x| x.position_id != position_id)
                                .collect::<Vec<Position>>();
                                
                                updated_positions.push(
                                    Position{
                                        credit_amount: total_credit,
                                        ..position
                                });
                                Ok( updated_positions )
                            },
                            None => return Err(ContractError::NonExistentPosition {  }) 
                    },

                    None => return Err(ContractError::NoUserPositions {  })
            }})?;
            }
            
        }else{
            return Err(ContractError::NoRepaymentPrice {  })
        }
        

    let response = Response::new()
    .add_message(message)
    .add_attribute("method", "increase_debt")
    .add_attribute("basket_id", basket_id.to_string())
    .add_attribute("position_id", position_id.to_string())
    .add_attribute("total_loan", total_credit.to_string());     

    Ok(response)
            
}

pub fn credit_mint_msg(
    credit_asset: Asset,
    recipient: Addr,
)-> Result<CosmosMsg, ContractError>{

    match credit_asset.info{
        AssetInfo::Token { address } => {
            let message = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: address.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Mint {
                    recipient: recipient.to_string(),
                    amount: credit_asset.amount,
                })?,
                funds: vec![],
            });
            Ok(message)
        },
        AssetInfo::NativeToken { denom } => {

            //TODO: How to mint native tokens
            //THIS IS WRONG CLEARLY PASTED FROM ABOVE. FOR TESTING PURPOSES.
            let message = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: denom.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Mint {
                    recipient: recipient.to_string(),
                    amount: credit_asset.amount,
                })?,
                funds: vec![],
            });
            Ok(message)
        },
    }
}

pub fn withdrawal_msg(
    asset: Asset,
    recipient: Addr,
)-> Result<CosmosMsg, ContractError>{
    //let credit_contract: Addr = basket.credit_contract;

    match asset.clone().info{
        AssetInfo::Token { address } => {
            let message = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: address.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: recipient.to_string(),
                    amount: asset.amount,
                })?,
                funds: vec![],
            });
            Ok(message)
        },
        AssetInfo::NativeToken { denom: _ } => {

            let coin: Coin = asset_to_coin(asset)?;
            let message = CosmosMsg::Bank(BankMsg::Send {
                to_address: recipient.to_string(),
                amount: vec![coin],
            });
            Ok(message)
        },
    }
    
}

//Confirms insolvency and calculates repayment amount
//Then sends liquidation messages to the modules if they have funds
//If not, sell wall
pub fn liquidate(
    deps: DepsMut,
    info: MessageInfo,
    basket_id: Uint128,
    position_id: Uint128,
    position_owner: String,
) -> Result<Response, ContractError>{

    //TODO: if repay value is ever greater than or equal to collateral value, automatic sell wall
    
    //TODO: Add batch liquidations so we don't need to do minimum fee bonuses for small accounts

    let config: Config = CONFIG.load(deps.storage)?;

    let basket: Basket= match BASKETS.load(deps.storage, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };
    let valid_position_owner = validate_position_owner(deps.api, info, Some(position_owner))?;

    let positions: Vec<Position> = match POSITIONS.load(deps.storage, (basket_id.to_string(), valid_position_owner.clone())){
        Err(_) => {  return Err(ContractError::NoUserPositions {  }) },
        Ok( positions ) => { positions },
    };

    let target_position = match positions.into_iter().find(|x| x.position_id == position_id) {
        Some(position) => position,
        None => return Err(ContractError::NonExistentPosition {  }) 
    };

    //Check position health comparative to max_LTV
    let (healthy, current_LTV) = match insolvency_check(deps.querier, target_position.clone().collateral_assets, target_position.clone().credit_amount, basket.credit_price.unwrap(), false) {
        Ok( healthy ) => { healthy },
        Err(_) => {
            return Err(ContractError::PositionSolvent { })
        }
    };

    //TODO: Once insolvency is confirmed, disable withdrawals from this position
    //Actually don't think this is necessary due to messaging semantics
    
    //Send liquidation amounts and info to the module
    //1) We need to calculate how much needs to be liquidated (down to max_borrow_LTV): 
    
    let (avg_borrow_LTV, avg_max_LTV, total_value, cAsset_prices) = get_avg_LTV(deps.querier, target_position.clone().collateral_assets)?;
    
    
    // max_borrow_LTV/ current_LTV, * current_loan_value, current_loan_value - __ = value of loan amount  
    let loan_value = decimal_multiplication(basket.credit_price.unwrap(), target_position.clone().credit_amount);
    
    //repay value = the % of the loan insolvent. Insolvent is anything between current and max borrow LTV.
    let repay_value = loan_value - decimal_multiplication(decimal_division(avg_borrow_LTV, current_LTV), loan_value);
    let repay_amount = match decimal_division(repay_value, basket.clone().credit_price.unwrap()){
        
        //Repay amount has to be above 0, or there is nothing to liquidate and there was a mistake prior
        x if x <= Decimal::new(Uint128::new(0u128)) => {
            return Err(ContractError::PositionSolvent {  })
        },
        //Allowing to get repaid more would drain the liquidity pool/queue and grant the contract extra funds
        x if x > target_position.clone().credit_amount => {
            return Err(ContractError::FaultyCalc { })
        }
        x => { x }
    };

     
    // Don't send any funds here, only send user_ids and repayment amounts.
    // We want to act on the reply status but since that means our state won't revert, assets we send won't come back.
     
    let mut res = Response::new();
    let mut submessages = vec![];
    let mut messages = vec![];
    let mut coins: Vec<Coin> = vec![];
    
    let cAsset_ratios = get_cAsset_ratios(deps.querier, target_position.clone().collateral_assets)?;
        
    let mut repayment_distribution: Vec<LiqAsset> = Vec::new();
    
    //1) Calcs repay amount per asset
    //2) Calcs collateral amount to be liquidated per asset (Fees not included yet)
    //2 will happen again in the reply. This instance is to pay the function caller
    for (num, cAsset) in  target_position.clone().collateral_assets.iter().enumerate(){

        let repay_amount_per_asset = decimal_multiplication(repay_amount, cAsset_ratios[num]);

        repayment_distribution.push(LiqAsset {
            info: cAsset.clone().asset.info,
            amount: repay_amount_per_asset,
        });

        //TODO: Add dynamic liq fee for caller
        //Which means collateral_amount has to be the fee % and not the full amount
         let collateral_value = decimal_multiplication(repay_value, cAsset_ratios[num]);
         let collateral_amount = decimal_division(collateral_value, cAsset_prices[num]) *Uint128::new(1u128); //Multiplied by 1 to get a Uint128
       
        match cAsset.clone().asset.info {
            

            AssetInfo::Token { address } => {
                
                //CW20 Transfer
                //Then add it to list of messages
                let msg = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: address.to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        amount: collateral_amount,
                        recipient: info.sender.to_string(),
                    })?,
                    funds: vec![],
                });
                messages.push(msg);
               
            }
            AssetInfo::NativeToken { denom: _ } => {
    
                let asset = Asset{ 
                    amount: collateral_amount,
                    ..cAsset.clone().asset
                };
    
                coins.push(asset_to_coin(asset)?);
                
            },
        }
    }
    

  
    // This is where any new modules would get added (ie liquidation queue)
    //The modules are scrolled thru until either leftover repayment amount is 0 or it hits the sell wall

    //If this is some that means the module is in use.
    //1) Build SubMsgs to send to the SP
    if config.stability_pool.is_some(){

      
        //Check for stability pool funds before any of this happens.
        //If no funds, go directly to the sell wall
        let query_res: LiquidatibleResponse = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                    contract_addr: config.stability_pool.unwrap().to_string(),
                    msg: to_binary(&SP_QueryMsg::CheckLiquidatible {
                        asset: LiqAsset{
                            amount: repay_amount,
                            info: basket.clone().credit_asset.info,
                        },
                    })?,
                }))?;

        let leftover_repayment = query_res.leftover;

        if leftover_repayment > Decimal::new(Uint128::new(0u128)){

            todo!()
            //TODO:
            //Call Sell wall function on remaining funds
        }

        //TODO: repay_amount variable name will change bc it has to go thru the liq_queue first
        let sp_repay_amount = repay_amount - leftover_repayment;
        
        //TODO: Set repay values for reply msg
        let repay_propagation = RepayPropagation {
            liq_queue:,
            stability_pool: sp_repay_amount,
            sell_wall: leftover_repayment,
            basket_id,
            position_id,
            position_owner: valid_position_owner,
        };

        ///////////////////

        //Stability Pool message builder
        let liq_msg = SP_ExecuteMsg::Liquidate {
            credit_asset: LiqAsset{
                amount: sp_repay_amount,
                info: basket.clone().credit_asset.info,
            },
            position_id,
            basket_id,
            position_owner,
        };

        
        let msg: CosmosMsg =  CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.stability_pool.unwrap().to_string(),
            msg: to_binary(&liq_msg)?,
            funds: vec![],
        });

        let sub_msg: SubMsg = SubMsg::reply_always(msg, STABILITY_POOL_REPLY_ID);

        submessages.push( sub_msg );
       
        //Because these are reply always, we can NOT make state changes that we wouldn't allow no matter the tx result, as our altereed state will NOT revert.
        //Errors also won't revert the whole transaction
        //( https://github.com/CosmWasm/cosmwasm/blob/main/SEMANTICS.md#submessages )
        
        res = res.add_submessages(submessages);

        //Collateral distributions get handled in the reply

    }

   
   Ok( res.add_messages(messages) )

}


#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, msg: Reply) -> StdResult<Response> {
    match msg.id {
        STABILITY_POOL_REPLY_ID => handle_stability_pool_reply(deps, msg),
    }
}


fn handle_stability_pool_reply(deps: DepsMut, msg: Reply) -> StdResult<Response>{

    match msg.result.into_result(){
         Ok(result)  => {
            //1) Parse potential leftover amount and send to sell_wall if there is any
            //Don't need to change state bc the SP will be repaying thru the contract
            //There should only be leftover here if the SP loses funds between the query and the repayment
            //2) Send collateral to the SP in the repay function and call distribute

            let liq_event = result
                .events
                .iter()
                .find(|e| {
                    e.attributes
                        .iter()
                        .any(|attr| attr.key == "leftover_repayment")
                })
                .ok_or_else(|| StdError::generic_err(format!("unable to find stability pool event")))?;

            let leftover = liq_event.attributes
                .iter()
                .find(|attr| attr.key == "leftover_repayment")
                .unwrap()
                .value;

            let leftover_amount = Uint128::from_str(&leftover)?;

            if leftover_amount != Uint128::zero(){

                //TODO: Sell Wall
            }

            //TODO: Add detail
            Ok(Response::new())

             
            
        },
        Err( string ) => {
            //If error, sell wall the SP repay amount
            let repay_propagation = REPAY.load(deps.storage)?;

        }
        
    }
    
    
}



#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetPosition { position_id, basket_id, user} => {
            let valid_addr: Addr = deps.api.addr_validate(&user)?;
            to_binary(&query_position(deps, position_id, basket_id, valid_addr)?)
        },
        QueryMsg::GetUserPositions { basket_id, user } => {
            let valid_addr: Addr = deps.api.addr_validate(&user)?;
            to_binary(&query_user_positions(deps, basket_id, valid_addr)?)
        },
        QueryMsg::GetBasketPositions { basket_id, start_after, limit } => {
            to_binary(&query_basket_positions(deps, basket_id, start_after, limit)?)
        },
        QueryMsg::GetBasket { basket_id } => {
            to_binary(&query_basket(deps, basket_id)?)
        },
        QueryMsg::GetAllBaskets { start_after, limit } => {
            to_binary(&query_baskets(deps, start_after, limit)?)
        }
    }
}

fn query_position(
    deps: Deps,
    position_id: Uint128,
    basket_id: Uint128,
    user: Addr
) -> StdResult<PositionResponse>{
    let positions: Vec<Position> = match POSITIONS.load(deps.storage, (basket_id.to_string(), user.clone())){
        Err(_) => {  return Err(StdError::generic_err("No User Positions")) },
        Ok( positions ) => { positions },
    };

    let position = positions
    .into_iter()
    .find(|x| x.position_id == position_id);

    match position{
        Some (position) => {
            Ok(PositionResponse {
                position_id: position.position_id.to_string(),
                collateral_assets: position.collateral_assets,
                avg_borrow_LTV: position.avg_borrow_LTV.to_string(),
                avg_max_LTV: position.avg_max_LTV.to_string(),
                credit_amount: position.credit_amount.to_string(),
                basket_id: position.basket_id.to_string(),
            })
        },

        None => return  Err(StdError::generic_err("NonExistent Position"))
    }
}

pub fn query_user_positions(
    deps: Deps,
    basket_id: Option<Uint128>,
    user: Addr,
) -> StdResult<Vec<PositionResponse>>{
    
    //Basket_id means only position from said basket
    if basket_id.is_some(){

        let positions: Vec<Position> = match POSITIONS.load(deps.storage, (basket_id.unwrap().clone().to_string(), user.clone())){
            Err(_) => {  return Err(StdError::generic_err("No User Positions")) },
            Ok( positions ) => { positions },
        };

        let mut response: Vec<PositionResponse> = Vec::new();
        for position in positions{
            response.push(
                PositionResponse {
                    position_id: position.position_id.to_string(),
                    collateral_assets: position.collateral_assets,
                    avg_borrow_LTV: position.avg_borrow_LTV.to_string(),
                    avg_max_LTV: position.avg_max_LTV.to_string(),
                    credit_amount: position.credit_amount.to_string(),
                    basket_id: position.basket_id.to_string(),
                }
            );
        }

        Ok( response )

    }else{ //If no basket_id, return all basket positions
        //Can use config.current basket_id-1 as the limiter to check all baskets

        let config = CONFIG.load(deps.storage)?;
        let mut response: Vec<PositionResponse> = Vec::new();

        //Uint128 to int
        let range: i32 = config.current_basket_id.to_string().parse().unwrap();

        for basket_id in 1..range{

                        
            match POSITIONS.load(deps.storage, (basket_id.to_string(), user.clone())) {
                Ok(positions) => {

                    for position in positions{
                        response.push(
                            PositionResponse {
                                position_id: position.position_id.to_string(),
                                collateral_assets: position.collateral_assets,
                                avg_borrow_LTV: position.avg_borrow_LTV.to_string(),
                                avg_max_LTV: position.avg_max_LTV.to_string(),
                                credit_amount: position.credit_amount.to_string(),
                                basket_id: position.basket_id.to_string(),
                            }
                        );
                    
                    }
                },
                Err(_) => {} //This is so errors don't stop the response builder, but we don't actually care about them here
            }
            
        }
        Ok( response )

    }

}

pub fn query_basket(
    deps: Deps,
    basket_id: Uint128,
) -> StdResult<BasketResponse>{

    let basket_res = match BASKETS.load(deps.storage, basket_id.to_string()){
        Ok( basket ) => {

            let credit_price = match basket.credit_price{
                Some(x) => { x.to_string()},
                None => { "None".to_string() },
            };
                        
            let credit_interest = match basket.credit_interest{
                Some(x) => { x.to_string()},
                None => { "None".to_string() },
            };

            BasketResponse {
                owner: basket.owner.to_string(),
                basket_id: basket.basket_id.to_string(),
                current_position_id: basket.current_position_id.to_string(),
                collateral_types: basket.collateral_types,
                credit_asset: basket.credit_asset,
                credit_price,
                credit_interest,
            }
        },
        Err(_) => { return Err(StdError::generic_err("Invalid basket_id")) },
    };

    Ok( basket_res )


}

pub fn query_baskets(
    deps: Deps,
    start_after: Option<Uint128>,
    limit: Option<u32>,
) -> StdResult<Vec<BasketResponse>>{

    let limit = limit.unwrap_or(32) as usize;

    let start: Option<Bound<String>> = match BASKETS.load(deps.storage, start_after.unwrap().to_string()){
        Ok(_x) => {
            Some(Bound::exclusive(start_after.unwrap().to_string()))
        },
        Err(_) => {
            None
        },
    };

    BASKETS
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (k, basket) = item?;

            let credit_price = match basket.credit_price{
                Some(x) => { x.to_string()},
                None => { "None".to_string() },
            };
                        
            let credit_interest = match basket.credit_interest{
                Some(x) => { x.to_string()},
                None => { "None".to_string() },
            };

            Ok(BasketResponse {
                owner: basket.owner.to_string(),
                basket_id: k,
                current_position_id: basket.current_position_id.to_string(),
                collateral_types: basket.collateral_types,
                credit_asset: basket.credit_asset,
                credit_price,
                credit_interest,
            })
            
        })
        .collect()
}

pub fn query_basket_positions(
    deps: Deps,
    basket_id: Uint128,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<Vec<PositionsResponse>>{
     
    let limit = limit.unwrap_or(32) as usize;

    let start_after_addr = deps.api.addr_validate(&start_after.unwrap())?;
    let start = Some(Bound::exclusive(start_after_addr));

    POSITIONS
        .prefix(basket_id.to_string())
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (k, v) = item?;
            Ok(PositionsResponse {
                user: k.to_string(),
                positions: v,
            })
        })
        .collect()
    
}


pub fn asset_to_coin(
    asset: Asset
)-> Result<Coin, ContractError>{

    match asset.info{
        //
        AssetInfo::Token { address: _ } => 
            return Err(ContractError::InvalidParameters {  })
        ,
        AssetInfo::NativeToken { denom } => {
            Ok(
                Coin {
                    denom: denom,
                    amount: asset.amount,
                }
            )
        },
    }
    
}

pub fn assert_credit(credit: Option<Uint128>) -> StdResult<Uint128>{
    //Check if user wants to take credit out now
    let checked_amount = if credit.is_some() &&  !credit.unwrap().is_zero(){
        Uint128::from(credit.unwrap())
     }else{
        Uint128::from(0u128)
    };
    Ok(checked_amount)
}

pub fn get_avg_LTV(
    deps: QuerierWrapper, 
    collateral_assets: Vec<cAsset>,
)-> StdResult<(Decimal, Decimal, Decimal, Vec<Decimal>)>{

    let (cAsset_values, cAsset_prices) = get_asset_values(deps, collateral_assets.clone())?;

    let total_value: Decimal = cAsset_values.iter().sum();

    //getting each cAsset's % of total value
    let mut cAsset_ratios: Vec<Decimal> = vec![];
    for cAsset in cAsset_values{
        cAsset_ratios.push(cAsset/total_value) ;
    }

    //converting % of value to avg_LTV by multiplying collateral LTV by % of total value
    let mut avg_max_LTV: Decimal = Decimal::new(Uint128::from(0u128));
    let mut avg_borrow_LTV: Decimal = Decimal::new(Uint128::from(0u128));

    if cAsset_ratios.len() == 0{
        //TODO: Change back to no values. This is for testing without oracles
       //return Ok((Decimal::percent(0), Decimal::percent(0), Decimal::percent(0)))
       return Ok((Decimal::percent(50), Decimal::percent(50), Decimal::percent(100000000), vec![]))
    }

    //Skip unecessary calculations if length is 1
    if cAsset_ratios.len() == 1{ return Ok(( collateral_assets[0].max_borrow_LTV, collateral_assets[0].max_LTV, total_value, cAsset_prices ))}
    
    for (i, _cAsset) in collateral_assets.clone().iter().enumerate(){
        avg_borrow_LTV += decimal_multiplication(cAsset_ratios[i], collateral_assets[i].max_borrow_LTV);
    }

    for (i, _cAsset) in collateral_assets.clone().iter().enumerate(){
        avg_max_LTV += decimal_multiplication(cAsset_ratios[i], collateral_assets[i].max_LTV);
    }
    

    Ok((avg_borrow_LTV, avg_max_LTV, total_value, cAsset_prices))
}

pub fn get_cAsset_ratios(
    deps: QuerierWrapper,
    collateral_assets: Vec<cAsset>
) -> StdResult<Vec<Decimal>>{
    let (cAsset_values, cAsset_prices) = get_asset_values(deps, collateral_assets.clone())?;

    let total_value: Decimal = cAsset_values.iter().sum();

    //getting each cAsset's % of total value
    let mut cAsset_ratios: Vec<Decimal> = vec![];
    for cAsset in cAsset_values{
        cAsset_ratios.push(cAsset/total_value) ;
    }

    //Error correction for ratios so we end up w/ least amount undistributed funds
    let ratio_total: Option<Decimal> = Some(cAsset_ratios.iter().sum());

    if ratio_total.unwrap() != Decimal::percent(100){
        let mut new_ratios: Vec<Decimal> = vec![];
        
        match ratio_total{
            Some( total ) if total > Decimal::percent(100) => {

                    let margin_of_error = total - Decimal::percent(100);

                    let num_users = Decimal::new(Uint128::from( cAsset_ratios.len() as u128 ));

                    let error_correction = decimal_division( margin_of_error, num_users );

                    new_ratios = cAsset_ratios.into_iter()
                    .map(|ratio| 
                        decimal_subtraction( ratio, error_correction )
                    ).collect::<Vec<Decimal>>();
                    
            },
            Some( total ) if total < Decimal::percent(100) => {

                let margin_of_error = Decimal::percent(100) - total;

                let num_users = Decimal::new(Uint128::from( cAsset_ratios.len() as u128 ));

                let error_correction = decimal_division( margin_of_error, num_users );

                new_ratios = cAsset_ratios.into_iter()
                        .map(|ratio| 
                            ratio + error_correction
                        ).collect::<Vec<Decimal>>();
            },
            None => { return Err(StdError::GenericErr { msg: "Input amounts were null".to_string() }) },
            Some(_) => { /*Unreachable due to if statement*/ },
        }
        return Ok( new_ratios )
    }

    Ok( cAsset_ratios )
}


pub fn insolvency_check( //Returns true if insolvent
    deps: QuerierWrapper,
    collateral_assets: Vec<cAsset>,
    credit_amount: Decimal,
    credit_price: Decimal,
    max_borrow: bool, //Toggle for either over max_borrow or over max_LTV (liquidatable), ie taking the minimum collateral ratio into account.
) -> StdResult<(bool, Decimal)>{

    let avg_LTVs: (Decimal, Decimal, Decimal, Vec<Decimal>) = get_avg_LTV(deps, collateral_assets)?;
    
    //TODO: Change, this is solely for testing. This would liquidate anyone anytime oracles failed.
    //Returns insolvent if oracle's failed
    if avg_LTVs == (Decimal::percent(0), Decimal::percent(0), Decimal::percent(0), vec![]){
         return Ok((true, Decimal::percent(0))) 
        }

    let asset_values: Decimal = avg_LTVs.2; //pulls total_asset_value

    let check: bool;
    let current_LTV = decimal_division( decimal_multiplication(credit_amount, credit_price) , asset_values);

    match max_borrow{
        true => { //Checks max_borrow
            check = current_LTV > avg_LTVs.0;
        },
        false => { //Checks max_LTV
            check = current_LTV > avg_LTVs.1;
        },
    }

    
    Ok( (check, current_LTV) )
}

pub fn assert_basket_assets(
    deps: &mut dyn Storage,
    basket_id: Uint128,
    assets: Vec<Asset>,

) -> Result<Vec<cAsset>, ContractError> {
    //let config: Config = CONFIG.load(deps)?;

    let basket: Basket= match BASKETS.load(deps, basket_id.to_string()) {
        Err(_) => { return Err(ContractError::NonExistentBasket {  })},
        Ok( basket ) => { basket },
    };


    //Checking if Assets for the position are available collateral assets in the basket
    let mut valid = false;
    let mut collateral_assets: Vec<cAsset> = Vec::new();
    
    //TODO: if multi-asset sends arent't possible, will need to change this
    for asset in assets {
       for cAsset in basket.clone().collateral_types{
        match (asset.clone().info, cAsset.asset.info){

            (AssetInfo::Token { address }, AssetInfo::Token { address: cAsset_address }) => {
                if address == cAsset_address {
                    valid = true;
                    collateral_assets.push(cAsset{
                        asset: asset.clone(),
                        ..cAsset
                    });
                 }
            },
            (AssetInfo::NativeToken { denom }, AssetInfo::NativeToken { denom: cAsset_denom }) => {
                if denom == cAsset_denom {
                    valid = true;
                    collateral_assets.push(cAsset{
                        asset: asset.clone(),
                        ..cAsset
                    });
                 }
            },
            (_,_) => continue,
        }}
           
       //Error if invalid collateral, meaning it wasn't found in the list of cAssets
       if !valid {
           return Err(ContractError::InvalidCollateral {  })
        }
        valid = false;
    }
    Ok(collateral_assets)
}


//Validate Recipient
pub fn validate_position_owner(
    deps: &dyn Api, 
    info: MessageInfo, 
    recipient: Option<String>) -> StdResult<Addr>{
    
    let valid_recipient: Addr = if let Some(recipient) = recipient {
        deps.addr_validate(&recipient)?
    }else {
        info.sender.clone()
    };
    Ok(valid_recipient)
}

//Refactored Terraswap function
pub fn assert_sent_native_token_balance(
    asset: &Asset,
    message_info: &MessageInfo)-> StdResult<()> {

    if let AssetInfo::NativeToken { denom} = &asset.info {
        match message_info.funds.iter().find(|x| x.denom == *denom) {
            Some(coin) => {
                if asset.amount == coin.amount {
                    Ok(())
                } else {
                    Err(StdError::generic_err("Sent coin.amount is different from asset.amount"))
                }
            },
            None => {
                {
                    Err(StdError::generic_err("Incorrect denomination, sent asset denom and asset.info.denom differ"))
                }
            }
        }
    } else {
        Err(StdError::generic_err("Asset type not native, check Msg schema and use AssetInfo::Token{ address: Addr }"))
    }
}

//Get Asset values / query oracle
pub fn get_asset_values(deps: QuerierWrapper, assets: Vec<cAsset>) -> StdResult<(Vec<Decimal>, Vec<Decimal>)>
{

   /* let timeframe: Option<u64> = if check_expire {
        Some(PRICE_EXPIRE_TIME)
    } else {
        None
    };*/

    //Getting proportions for position collateral to calculate avg LTV
    //Using the index in the for loop to parse through the assets Vec and collateral_assets Vec
    //, as they are now aligned due to the collateral check w/ the Config's data
    let cAsset_values: Vec<Decimal> = Vec::new();
    let cAsset_prices: Vec<Decimal> = Vec::new();

    for (i,n) in assets.iter().enumerate() {

    //    //TODO: Query collateral prices from the oracle
    //    let collateral_price = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
    //         contract_addr: assets[i].oracle.to_string(),
    //         msg: to_binary(&OracleQueryMsg::Price {
    //             asset_token: assets[i].asset.info.to_string(),
    //             None,
    //         })?,
    //     }))?;

        //cAsset_prices.push(collateral_price.rate);
        // let collateral_value = decimal_multiplication(Decimal::new(assets[i].asset.amount), collateral_price.rate);
        // cAsset_values.push(collateral_value); 

    }
    Ok((cAsset_values, cAsset_prices))
}






#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies_with_balance, mock_env, mock_info};
    use cosmwasm_std::{coins, from_binary, attr};

    #[test]
    fn open_position_deposit(){
        let mut deps = mock_dependencies_with_balance(&coins(2, "token"));

        let msg = InstantiateMsg {
                owner: Some("owner".to_string()),
                credit_asset: Some(Asset {
                    info: AssetInfo::NativeToken { denom: "credit".to_string() },
                    amount: Uint128::from(0u128),
                }),
                credit_price: Some(Decimal::new(Uint128::from(1u128)*Uint128::new(1000000000000000000u128))),
                collateral_types: Some(vec![
                cAsset {
                    asset:
                        Asset {
                            info: AssetInfo::NativeToken { denom: "debit".to_string() },
                            amount: Uint128::from(0u128),
                        },
                oracle: "funnybone".to_string(),
                max_borrow_LTV: Decimal::percent(50),
                max_LTV: Decimal::percent(90),
                } 
            ]),
                credit_interest: Some(Decimal::percent(1)),
        };

        //Instantiating contract
        let v_info = mock_info("sender88", &coins(11, "debit"));
        let _res = instantiate(deps.as_mut(), mock_env(), v_info.clone(), msg.clone()).unwrap();

        //Testing Position creation

        //Invalid id test
        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(11u128),
            }
        ];

        let error_exec_msg = ExecuteMsg::Deposit { 
            assets,
            position_owner: msg.clone().owner,
            basket_id: Uint128::from(1u128),
            position_id: Some(Uint128::from(3u128)),
        };

        //Fail due to a non-existent position
        //First msg deposits since no positions were initially found, meaning the _id never got tested
        let _res = execute(deps.as_mut(), mock_env(), v_info.clone(), error_exec_msg.clone());
        let res = execute(deps.as_mut(), mock_env(), v_info.clone(), error_exec_msg);

        match res {
            Err(ContractError::NonExistentPosition {}) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("Position deposit should've failed for passing in an invalid position ID"),
        } 


        //Fail for invalid collateral
        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "fake_debit".to_string() },
                amount: Uint128::from(666u128),
            }
        ];

        let info = mock_info("sender88", &coins(666, "fake_debit"));

        let exec_msg = ExecuteMsg::Deposit { 
            assets,
            position_owner: msg.clone().owner,
            basket_id: Uint128::from(1u128),
            position_id: None,
        };

        //fail due to invalid collateral
        let res = execute(deps.as_mut(), mock_env(), info.clone(), exec_msg);        

        match res {
            Err(ContractError::InvalidCollateral {}) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("Position creation should've failed due to invalid cAsset type"),
        }

        //Successful attempt
        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(11u128),
            }
        ];

        let exec_msg = ExecuteMsg::Deposit { 
            assets,
            position_owner: msg.clone().owner,
            basket_id: Uint128::from(1u128),
            position_id: None,
        };

        let res = execute(deps.as_mut(), mock_env(), v_info.clone(), exec_msg).unwrap();

        assert_eq!(
            res.attributes,
            vec![
            attr("method", "deposit"),
            attr("basket_id", "1"),
            attr("position_owner","owner"),
            attr("position_id", "2"),
            attr("assets", "11debit"),
            ]
        );

        //Query position data to make sure it was saved to state correctly
        let res = query(deps.as_ref(),
            mock_env(),
            QueryMsg::GetPosition {
                position_id: Uint128::from(1u128),
                basket_id: Uint128::from(1u128),
                user: "owner".to_string()
            })
            .unwrap();
        
        let resp: PositionResponse = from_binary(&res).unwrap();

        assert_eq!(resp.position_id, "1".to_string());
        assert_eq!(resp.basket_id, "1".to_string());
        assert_eq!(resp.avg_borrow_LTV, "0".to_string()); //This is 0 bc avg_LTV is calc'd and saved during solvency checks
        assert_eq!(resp.credit_amount, "0".to_string());

    }

    #[test]
    fn withdrawal(){

        let mut deps     = mock_dependencies_with_balance(&coins(2, "token"));
        
        let msg = InstantiateMsg {
                owner: Some("owner".to_string()),
                credit_asset: Some(Asset {
                    info: AssetInfo::NativeToken { denom: "credit".to_string() },
                    amount: Uint128::from(0u128),
                }),
                credit_price: Some(Decimal::new(Uint128::from(1u128)*Uint128::new(1000000000000000000u128))),
                collateral_types: Some(vec![
                cAsset {
                    asset:
                        Asset {
                            info: AssetInfo::NativeToken { denom: "debit".to_string() },
                            amount: Uint128::from(0u128),
                        },
                    oracle: "funnybone".to_string(),
                    max_borrow_LTV: Decimal::percent(50),
                    max_LTV: Decimal::percent(90),
                    } 
                ]),
                credit_interest: Some(Decimal::percent(1)),
        };

        //Instantiating contract
        let info = mock_info("sender88", &[]);
        let _res = instantiate(deps.as_mut(), mock_env(), info.clone(), msg.clone()).unwrap();

        let valid_assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(5u128),
            }
        ];

        //User has no positions in the basket error
        let withdrawal_msg = ExecuteMsg::Withdraw {
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            assets: valid_assets.clone(), 
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), withdrawal_msg);

        match res {
            Err(ContractError::NoUserPositions {}) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("Position withdrawal should've failed due to having no positions in the passed basket"),
        }

         //Initial deposit
        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(11u128),
            }
        ];

        let info = mock_info("sender88", &coins(11, "debit"));

        let exec_msg = ExecuteMsg::Deposit { //Deposit
            assets: assets.clone(),
            position_owner: Some(info.clone().sender.to_string()),
            basket_id: Uint128::from(1u128),
            position_id: None,
        };

        let _res = execute(deps.as_mut(), mock_env(), info.clone(), exec_msg).unwrap();


        //Non-existent position error but user still has positions in the basket
        let withdrawal_msg = ExecuteMsg::Withdraw {
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(3u128),
            assets: assets.clone(), 
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), withdrawal_msg);

        match res {
            Err(ContractError::NonExistentPosition {}) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("Position withdrawal should've failed due to invalid position id"),
        }

        //Invalid collateral fail
        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "notdebit".to_string() },
                amount: Uint128::from(10u128),
            }
        ];

        let withdrawal_msg = ExecuteMsg::Withdraw {
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            assets: assets.clone(), 
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), withdrawal_msg);

        match res {
            Err(ContractError::InvalidCollateral {}) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("Position withdrawal should've failed due to invalid cAsset type"),
        }
        
        //Withdrawing too much error
        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(333333333u128),
            }
        ];

        let withdrawal_msg = ExecuteMsg::Withdraw {
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            assets: assets.clone(), 
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), withdrawal_msg);

        match res {
            Err(ContractError::InvalidWithdrawal {}) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("Position withdrawal should've failed due to invalid withdrawal amount"),
        }
        
        //Insolvent withdrawal error
        //Need to add mock oracle abilities 
        let take_credit_msg = ExecuteMsg::IncreaseDebt {
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            amount: Uint128::from(3u128),
        };
        let res = execute(deps.as_mut(), mock_env(), info.clone(), take_credit_msg);

        match res{
            Err(ContractError::PositionInsolvent {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc the position is insolvent"),
        }

        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(11u128),
            }
        ];

        let withdrawal_msg = ExecuteMsg::Withdraw {
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            assets: assets.clone(), 
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), withdrawal_msg);

        match res {
            Err(ContractError::PositionInsolvent {}) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("Position withdrawal should've failed due to invalid withdrawal amount"),
        }

        //No repayment price error {}
        let create_basket_msg = ExecuteMsg::CreateBasket {
            owner: Some("owner".to_string()),
            collateral_types: vec![
                cAsset {
                    asset:
                        Asset {
                            info: AssetInfo::NativeToken { denom: "debit".to_string() },
                            amount: Uint128::from(0u128),
                        },
                    oracle: "funnybone".to_string(),
                    max_borrow_LTV: Decimal::percent(50),
                    max_LTV: Decimal::percent(90),
                       } 
            ],
            credit_asset: Asset {
                info: AssetInfo::NativeToken { denom: "credit".to_string() },
                amount: Uint128::from(0u128),
            },
            credit_price: None,
            credit_interest: Some(Decimal::percent(1))
        };

        let _res = execute(deps.as_mut(), mock_env(), info.clone(), create_basket_msg).unwrap();

        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(11u128),
            }
        ];

        //Depositing into the basket that lacks a credit_price
        let deposit_msg = ExecuteMsg::Deposit { 
            assets,
            position_owner: Some(info.clone().sender.to_string()),
            basket_id: Uint128::from(2u128),
            position_id: None,
        };

        let _res = execute(deps.as_mut(), mock_env(), info.clone(), deposit_msg).unwrap();
        
        let withdrawal_msg = ExecuteMsg::Withdraw {
            basket_id: Uint128::from(2u128),
            position_id: Uint128::from(1u128),
            assets: valid_assets.clone(), 
        };
        //Should fail due to no credit price
        let res = execute(deps.as_mut(), mock_env(), info.clone(), withdrawal_msg);

        match res{
            Err(ContractError::NoRepaymentPrice {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've error due to the basket not specifying a credit repayment price"),
        }

        //Successful attempt
        let withdrawal_msg = ExecuteMsg::Withdraw {
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            assets: valid_assets.clone(), 
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), withdrawal_msg).unwrap();

        //Assert the response
        assert_eq!(
            res.attributes,
            vec![
            attr("method", "withdraw"),
            attr("basket_id", "1"),
            attr("position_id", "1"),
           // attr("asset", "10debit"),
            ]
        );

         //Query position data to make sure it was saved to state correctly
         let res = query(deps.as_ref(),
         mock_env(),
         QueryMsg::GetPosition {
             position_id: Uint128::from(1u128),
             basket_id: Uint128::from(1u128),
             user: info.clone().sender.to_string(),
         })
         .unwrap();
     
     let resp: PositionResponse = from_binary(&res).unwrap();

     assert_eq!(resp.position_id, "1".to_string());
     assert_eq!(resp.basket_id, "1".to_string());
     assert_eq!(resp.collateral_assets[0].asset.to_string(), "6debit".to_string());
     

    }

    #[test]
    fn increase_debt() {
        
        let mut deps     = mock_dependencies_with_balance(&coins(2, "token"));
        
        let msg = InstantiateMsg {
                owner: Some("owner".to_string()),
                credit_asset: Some(Asset {
                    info: AssetInfo::NativeToken { denom: "credit".to_string() },
                    amount: Uint128::from(0u128),
                }),
                credit_price: Some(Decimal::new(Uint128::from(1u128)*Uint128::new(1000000000000000000u128))),
                collateral_types: Some(vec![
                cAsset {
                    asset:
                        Asset {
                            info: AssetInfo::NativeToken { denom: "debit".to_string() },
                            amount: Uint128::from(0u128),
                        },
                    oracle: "funnybone".to_string(),
                    max_borrow_LTV: Decimal::percent(50),
                    max_LTV: Decimal::percent(90),
                       } 
                ]),
                credit_interest: Some(Decimal::percent(1)),
        };

        //Instantiating contract
        let info = mock_info("sender88", &coins(11, "debit"));
        let _res = instantiate(deps.as_mut(), mock_env(), info.clone(), msg.clone()).unwrap();

        //NoUserPositions Error
        let increase_debt_msg = ExecuteMsg::IncreaseDebt{
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            amount: Uint128::from(1u128),
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), increase_debt_msg);

        match res{
            Err(ContractError::NoUserPositions {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc no positions have been created yet"),
        }

        //No repayment price error {}
        let create_basket_msg = ExecuteMsg::CreateBasket {
            owner: Some("owner".to_string()),
            collateral_types: vec![
                cAsset {
                    asset:
                        Asset {
                            info: AssetInfo::NativeToken { denom: "debit".to_string() },
                            amount: Uint128::from(0u128),
                        },
                    oracle: "funnybone".to_string(),
                    max_borrow_LTV: Decimal::percent(50),
                    max_LTV: Decimal::percent(90),
                       } 
            ],
            credit_asset: Asset {
                info: AssetInfo::NativeToken { denom: "credit".to_string() },
                amount: Uint128::from(0u128),
            },
            credit_price: None,
            credit_interest: Some(Decimal::percent(1))
        };

        let _res = execute(deps.as_mut(), mock_env(), info.clone(), create_basket_msg).unwrap();

        let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(11u128),
            }
        ];

        //Depositing into the basket that lacks a credit_price
        let deposit_msg = ExecuteMsg::Deposit { 
            assets,
            position_owner: Some(info.clone().sender.to_string()),
            basket_id: Uint128::from(2u128),
            position_id: None,
        };

        let _res = execute(deps.as_mut(), mock_env(), info.clone(), deposit_msg).unwrap();

         //NoRepaymentPrice Error
         let increase_debt_msg = ExecuteMsg::IncreaseDebt{
            basket_id: Uint128::from(2u128),
            position_id: Uint128::from(1u128),
            amount: Uint128::from(1u128),
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), increase_debt_msg);

        match res{
            Err(ContractError::NoRepaymentPrice {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc the basket has no repayment price"),
        }

         //Initial deposit
         let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(11u128),
            }
        ];

        let exec_msg = ExecuteMsg::Deposit { 
            assets,
            position_owner: Some(info.clone().sender.to_string()),
            basket_id: Uint128::from(1u128),
            position_id: None,
        };

        let _res = execute(deps.as_mut(), mock_env(), info.clone(), exec_msg).unwrap();

        //Insolvent position error 
        let take_credit_msg = ExecuteMsg::IncreaseDebt {
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            amount: Uint128::from(30000000u128),
        };
        let res = execute(deps.as_mut(), mock_env(), info.clone(), take_credit_msg);

        match res{
            Err(ContractError::PositionInsolvent {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc the position is insolvent"),
        }

        //NonExistentPosition Error
        let increase_debt_msg = ExecuteMsg::IncreaseDebt{
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(3u128),
            amount: Uint128::from(1u128),
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), increase_debt_msg);

        match res{
            Err(ContractError::NonExistentPosition {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc no position under the _id has been created"),
        }

        //NonExistentBasket Error
        let increase_debt_msg = ExecuteMsg::IncreaseDebt{
            basket_id: Uint128::from(3u128),
            position_id: Uint128::from(1u128),
            amount: Uint128::from(1u128),
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), increase_debt_msg);

        match res{
            Err(ContractError::NonExistentBasket {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc there is no basket under said _id"),
        }


        //Successful increase of user debt
        let increase_debt_msg = ExecuteMsg::IncreaseDebt{
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            amount: Uint128::from(1u128),
        };

        let res = execute(deps.as_mut(), mock_env(), info.clone(), increase_debt_msg).unwrap();

        //Assert the response
        assert_eq!(
            res.attributes,
            vec![
            attr("method", "increase_debt"),
            attr("basket_id", "1"),
            attr("position_id", "1"),
            attr("total_loan", "1")
            ]
        );

    } 

    #[test]
    fn repay(){

        let mut deps     = mock_dependencies_with_balance(&coins(2, "token"));
        
        let msg = InstantiateMsg {
                owner: Some("owner".to_string()),
                credit_asset: Some(Asset {
                    info: AssetInfo::NativeToken { denom: "credit".to_string() },
                    amount: Uint128::from(0u128),
                }),
                credit_price: Some(Decimal::new(Uint128::from(1u128)*Uint128::new(1000000000000000000u128))),
                collateral_types: Some(vec![
                cAsset {
                    asset:
                        Asset {
                            info: AssetInfo::NativeToken { denom: "debit".to_string() },
                            amount: Uint128::from(0u128),
                        },
                    oracle: "funnybone".to_string(),
                    max_borrow_LTV: Decimal::percent(50),
                    max_LTV: Decimal::percent(90),
                       } 
                ]),
                credit_interest: Some(Decimal::percent(1)),
        };

        //Instantiating contract
        let v_info = mock_info("sender88", &coins(1, "credit"));
        let _res = instantiate(deps.as_mut(), mock_env(), v_info.clone(), msg.clone()).unwrap();


        //NoUserPositions Error
        let repay_msg = ExecuteMsg::Repay { 
            basket_id: Uint128::from(1u128), 
            position_id: Uint128::from(1u128), 
            position_owner:  Some(v_info.clone().sender.to_string()), 
            credit_asset: Asset {
                info: AssetInfo::NativeToken { denom: "credit".to_string() },
                amount: Uint128::from(1u128),
            },
        };

        let res = execute(deps.as_mut(), mock_env(), v_info.clone(), repay_msg);

        match res{
            Err(ContractError::NoUserPositions {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc there are no open positions in this basket under the user's ownership"),
        }
        
         //Initial deposit
         let assets: Vec<Asset> = vec![
            Asset {
                info: AssetInfo::NativeToken { denom: "debit".to_string() },
                amount: Uint128::from(11u128),
            }
        ];

        let info = mock_info("sender88", &coins(11, "debit"));

        let exec_msg = ExecuteMsg::Deposit { 
            assets,
            position_owner: Some(info.clone().sender.to_string()),
            basket_id: Uint128::from(1u128),
            position_id: None,
        };

        let _res = execute(deps.as_mut(), mock_env(), info.clone(), exec_msg).unwrap();

        //Successful increase of user debt
        let increase_debt_msg = ExecuteMsg::IncreaseDebt{
            basket_id: Uint128::from(1u128),
            position_id: Uint128::from(1u128),
            amount: Uint128::from(1u128),
        };

        let _res = execute(deps.as_mut(), mock_env(), info.clone(), increase_debt_msg.clone()).unwrap();

         //Invalid Collateral Error
         let repay_msg = ExecuteMsg::Repay { 
            basket_id: Uint128::from(1u128), 
            position_id: Uint128::from(1u128), 
            position_owner:  Some(info.clone().sender.to_string()), 
            credit_asset: Asset {
                info: AssetInfo::NativeToken { denom: "not_credit".to_string() },
                amount: Uint128::from(1u128),
            },
        };

        let info = mock_info("sender88", &coins(1, "not_credit"));

        let res = execute(deps.as_mut(), mock_env(), info.clone(), repay_msg);

        match res{
            Err(ContractError::InvalidCollateral {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc the credit asset isn't correct for this basket"),
        }

        //NonExistent Basket Error
        let repay_msg = ExecuteMsg::Repay { 
            basket_id: Uint128::from(3u128), 
            position_id: Uint128::from(1u128), 
            position_owner:  Some(info.clone().sender.to_string()), 
            credit_asset: Asset {
                info: AssetInfo::NativeToken { denom: "credit".to_string() },
                amount: Uint128::from(1u128),
            },
        };

        let res = execute(deps.as_mut(), mock_env(), v_info.clone(), repay_msg);

        match res{
            Err(ContractError::NonExistentBasket {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc there is no basket under said _id"),
        }

        //ExcessRepayment Error
        let repay_msg = ExecuteMsg::Repay { 
            basket_id: Uint128::from(1u128), 
            position_id: Uint128::from(1u128), 
            position_owner:  Some(info.clone().sender.to_string()), 
            credit_asset: Asset {
                info: AssetInfo::NativeToken { denom: "credit".to_string() },
                amount: Uint128::from(333333u128),
            },
        };

        let info = mock_info("sender88", &coins(333333, "credit"));

        let res = execute(deps.as_mut(), mock_env(), info.clone(), repay_msg);

        match res{
            Err(ContractError::ExcessRepayment {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc the credit amount is more than the open loan amount"),
        }

        //NonExistent Position Error
        let repay_msg = ExecuteMsg::Repay { 
            basket_id: Uint128::from(1u128), 
            position_id: Uint128::from(3u128), 
            position_owner:  Some(info.clone().sender.to_string()), 
            credit_asset: Asset {
                info: AssetInfo::NativeToken { denom: "credit".to_string() },
                amount: Uint128::from(1u128),
            },
        };

        let res = execute(deps.as_mut(), mock_env(), v_info.clone(), repay_msg);

        match res{
            Err(ContractError::NonExistentPosition {  }) => {},
            Err(_) => {panic!("{}", res.err().unwrap().to_string())},
            _ => panic!("This should've errored bc the position_id passed is non existent under this basket"),
        }

        //Successful repayment
        let repay_msg = ExecuteMsg::Repay { 
            basket_id: Uint128::from(1u128), 
            position_id: Uint128::from(1u128), 
            position_owner:  Some(info.clone().sender.to_string()), 
            credit_asset: Asset {
                info: AssetInfo::NativeToken { denom: "credit".to_string() },
                amount: Uint128::from(1u128),
            },
        };

        let res = execute(deps.as_mut(), mock_env(), v_info.clone(), repay_msg).unwrap();

        //Assert the response
        assert_eq!(
            res.attributes,
            vec![
            attr("method", "repay"),
            attr("basket_id", "1"),
            attr("position_id", "1"),
            attr("loan_amount", "0")
            ]
        );

    }

}