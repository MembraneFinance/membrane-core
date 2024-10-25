#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, to_json_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, QuerierWrapper, Reply, Response, StdError, StdResult, Storage, SubMsg, Uint128, WasmMsg
};
use membrane::oracle::{self, PriceResponse};
use membrane::types::{Asset, AssetInfo, Basket, UserInfo, VTClaimCheckpoint, ClaimTracker, APR};
use osmosis_std::types::osmosis;
use serde::de;
use std::cmp::{max, min};
use std::str::FromStr;
use std::vec;
use cw2::set_contract_version;
use membrane::math::{decimal_division, decimal_multiplication, decimal_subtraction};

use crate::error::TokenFactoryError;
use crate::state::{CLAIM_TRACKER, TokenRateAssurance, UnloopProps, CONFIG, OWNERSHIP_TRANSFER, TOKEN_RATE_ASSURANCE, UNLOOP_PROPS, VAULT_TOKEN};
use membrane::stable_earn_vault::{Config, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use membrane::mars_vault_token::{ExecuteMsg as Vault_ExecuteMsg, QueryMsg as Vault_QueryMsg};
use membrane::cdp::{BasketPositionsResponse, CollateralInterestResponse, ExecuteMsg as CDP_ExecuteMsg, InterestResponse, PositionResponse, QueryMsg as CDP_QueryMsg};
use membrane::osmosis_proxy::{ExecuteMsg as OP_ExecuteMsg};
use membrane::oracle::QueryMsg as Oracle_QueryMsg;
use membrane::stability_pool_vault::{
    calculate_base_tokens, calculate_vault_tokens
};
use osmosis_std::types::osmosis::tokenfactory::v1beta1::{self as TokenFactory};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:stable-earn-vault";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

//Reply IDs
const ENTER_VAULT_REPLY_ID: u64 = 1u64;
const CDP_REPLY_ID: u64 = 2u64;
const LOOP_REPLY_ID: u64 = 3u64;
const UNLOOP_REPLY_ID: u64 = 4u64;
const EXIT_VAULT_STRAT_REPLY_ID: u64 = 5u64;

//Constants
const SECONDS_PER_DAY: u64 = 86_400u64;
const LOOP_MAX: u64 = 5u64;
const MIN_DEPOSIT_VALUE: Decimal = Decimal::percent(101_11);

////PROCEDURAL FLOW/NOTES////
// - There is a deposit and entry fee. 
// --The entry fee is added in manually thru the contract in get_total_deposits().
// -- The deposit fee is baked into the "liquid" valuation calc of the CDP position so deposits that don't get looped won't confer this fee to the vault.
// - We need to keep a buffer of vault tokens outside of the vault to allow for easy withdrawals. Buffer will likely break exit debt clearance logic. (todo)
// -

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, TokenFactoryError> {
    let mut config = Config {
        owner: info.sender.clone(),
        cdt_denom: msg.clone().cdt_denom,
        vault_token: String::from("factory/".to_owned() + env.contract.address.as_str() + "/" + msg.clone().vault_subdenom.as_str()),
        deposit_token: msg.clone().deposit_token,
        cdp_contract_addr: deps.api.addr_validate(&msg.clone().cdp_contract_addr)?,
        osmosis_proxy_contract_addr: deps.api.addr_validate(&msg.clone().osmosis_proxy_contract_addr)?,
        oracle_contract_addr: deps.api.addr_validate(&msg.clone().oracle_contract_addr)?,
        withdrawal_buffer: Decimal::percent(10),
        total_nonleveraged_vault_tokens: Uint128::new(1_000_000_000_000), //from initial deposit
        cdp_position_id: Uint128::zero(),
        deposit_cap: Uint128::new(10_000_000_000),
        swap_slippage: Decimal::from_str("0.005").unwrap(), //0.5%
        vault_cost_index: 0,
    };
    //Validate the deposit token vault addr
    deps.api.addr_validate(&config.deposit_token.vault_addr.to_string())?;
    //Query the basket to find the index of the vault_token
    let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasket { },
    ){
        Ok(basket) => basket,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP Basket") }),
    };
    //Find the index
    let mut saved_index: Option<u64> = None;
    for (index, asset) in basket.clone().collateral_types.into_iter().enumerate(){
        if asset.asset.info.to_string() == config.deposit_token.clone().vault_token {
            saved_index = Some(index as u64);
            break;
        }
    }
    if let Some(index) = saved_index {
        config.vault_cost_index = index as usize;
    } else {
        return Err(TokenFactoryError::CustomError { val: String::from("Failed to find the vault token in the CDP Basket") });
    }

    //Save initial state
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    //Query the underlying of the initial vault token deposit
    let underlying_deposit_token: Uint128 = match deps.querier.query_wasm_smart::<Uint128>(
        config.deposit_token.vault_addr.to_string(),
        &Vault_QueryMsg::VaultTokenUnderlying { vault_token_amount: Uint128::new(1_000_000_000_000) },
    ){
        Ok(underlying_deposit_token) => underlying_deposit_token,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the Mars Vault Token for the underlying deposit amount in instantiate") }),
    };

    //Set the initial vault token amount from the initial deposit
    let vault_tokens_to_distribute = calculate_vault_tokens(
        underlying_deposit_token,
        Uint128::zero(), 
        Uint128::zero()
    )?;
    CONFIG.save(deps.storage, &config)?;
    VAULT_TOKEN.save(deps.storage, &vault_tokens_to_distribute)?;
    UNLOOP_PROPS.save(deps.storage, &UnloopProps {
        sender: String::from(""),
        debt_to_clear: Uint128::zero(),
        owned_collateral: Uint128::zero(),
        loop_count: 0,
        running_collateral_amount: Uint128::zero(),
        running_credit_amount: Uint128::zero(),
        vt_token_price: PriceResponse {
            price: Decimal::zero(),
            prices: vec![],
            decimals: 6
        },
        cdt_peg_price: PriceResponse {
            price: Decimal::zero(),
            prices: vec![],
            decimals: 6
        },
        cdt_market_price: PriceResponse {
            price: Decimal::zero(),
            prices: vec![],
            decimals: 6
        }
    })?;
    CLAIM_TRACKER.save(deps.storage, &ClaimTracker {
        vt_claim_checkpoints: vec![],
        last_updated: env.block.time.seconds(),
    })?;
    //Create Denom Msg
    let denom_msg = TokenFactory::MsgCreateDenom { sender: env.contract.address.to_string(), subdenom: msg.vault_subdenom.clone() };
    //Create CDP deposit msg to get the position ID
    //Instantiatoor must send a vault token.
    //This initial deposit means the position should never be empty due to user withdrawals.
    let cdp_deposit_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.cdp_contract_addr.to_string(),
        msg: to_json_binary(&CDP_ExecuteMsg::Deposit { position_id: None, position_owner: None })?,
        funds: vec![Coin {
            denom: config.deposit_token.vault_token.clone(),
            amount: Uint128::new(1_000_000_000_000),
        }],
    });
    let cdp_submsg = SubMsg::reply_on_success(cdp_deposit_msg, CDP_REPLY_ID);
    
    //Create Response
    let res = Response::new()
        .add_attribute("method", "instantiate")
        .add_attribute("config", format!("{:?}", config))
        .add_attribute("contract_address", env.contract.address)
        .add_attribute("sub_denom", msg.clone().vault_subdenom)
    //UNCOMMENT
        .add_message(denom_msg)
        .add_submessage(cdp_submsg);
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
        ExecuteMsg::UpdateConfig { 
            owner, 
            cdp_contract_addr, 
            mars_vault_addr,
            osmosis_proxy_contract_addr, 
            oracle_contract_addr, 
            withdrawal_buffer,
            deposit_cap, 
            swap_slippage,
            vault_cost_index
        } => update_config(deps, info, owner, cdp_contract_addr, mars_vault_addr, osmosis_proxy_contract_addr, oracle_contract_addr, withdrawal_buffer, deposit_cap, swap_slippage, vault_cost_index),
        ExecuteMsg::EnterVault { } => enter_vault(deps, env, info),
        ExecuteMsg::ExitVault {  } => exit_vault(deps, env, info),
        ExecuteMsg::UnloopCDP { desired_collateral_withdrawal } => unloop_cdp(deps, env, info, desired_collateral_withdrawal),
        ExecuteMsg::LoopCDP { max_mint_amount } => loop_cdp(deps, env, info, max_mint_amount),
        ExecuteMsg::CrankRealizedAPR { } => crank_realized_apr(deps, env, info),
        ///CALLBACKS///
        ExecuteMsg::RateAssurance { exit } => rate_assurance(deps, env, info, exit),
        ExecuteMsg::UpdateNonleveragedVaultTokens {  } => update_nonleveraged_vault_tokens(deps, env, info),
    }
}



//LOOP NOTES: 
// - Loop to leave a 101 CDT LTV gap to allow easier unlooping under the minimum
// - Don't loop if CDT price is below 99% + slippage of peg
// - We don't loop the buffer of vault tokens in the contract
//POST LOOP NOTES:
// - Bc we only loop once, as long as we start above 99% + slippage, we'll never make a trade that is unprofitable (i.e. under 99% of peg)
fn loop_cdp(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    max_mint_amount: Option<Uint128>,
) -> Result<Response, TokenFactoryError> {
    //Load config
    let config = CONFIG.load(deps.storage)?;
    let mut msgs = vec![];
    
    //Ensure price is above 99% of peg
    //We want to ensure loops keep redemptions at 99% of peg profitable or even
    let (cdt_market_price, cdt_peg_price) = test_looping_peg_price(deps.querier, config.clone(), Decimal::percent(99))?;

    // Calc swap slippage based on the difference between the market and peg price.
    // Maximum slippage is 0.5% (0.005).
    let mut max_slippage = config.swap_slippage;
    let peg_ratio = decimal_division(cdt_market_price.price, cdt_peg_price.price)?;
    if peg_ratio > Decimal::percent(99) && peg_ratio < Decimal::percent(99) + config.swap_slippage {
        max_slippage = match peg_ratio.checked_sub(Decimal::percent(99)){
            Ok(v) => v,
            Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to calculate the max slippage in loop {:?}", peg_ratio) }),
        };
    }
    // panic!("max_slippage: {}, {}, {}", max_slippage, peg_ratio, config.swap_slippage);
    //So if market price is .991, slippage should be .001
    //If market price is .995, slippage should be .005
    //If market price is .999, slippage should be .005
    //This allows the vault to loop at prices lower than the max (0.995 - 0.99) 
    //..but not at rates that are unprofitable when redeeming

    let (
        running_credit_amount, 
        running_collateral_amount, 
        vt_price, 
        _cdt_price
    ) = get_cdp_position_info(deps.as_ref(), env.clone(), config.clone(), &mut msgs)?;

    //Get deposit token price
    let prices: Vec<PriceResponse> = match deps.querier.query_wasm_smart::<Vec<PriceResponse>>(
        config.oracle_contract_addr.to_string(),
        &Oracle_QueryMsg::Price {
            asset_info: AssetInfo::NativeToken { denom: config.clone().deposit_token.deposit_token },
            twap_timeframe: 0, //We want current swap price
            oracle_time_limit: 0,
            basket_id: None
        },
    ){
        Ok(prices) => prices,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the deposit token price in loop") }),
    };
    let deposit_token_price: PriceResponse = prices[0].clone();

    let (_, min_deposit_value, amount_to_mint) = calc_mintable(
        config.clone().swap_slippage, 
        vt_price.clone(),
        deposit_token_price.clone(), 
        cdt_peg_price.clone(), 
        running_collateral_amount, 
        running_credit_amount
    )?;
        
    //Leave a 101 CDT LTV gap to allow easier unlooping under the minimum debt (100)
    //$101 min deposit is $91 of LTV space which is ~101 withdrawal space so we can always fulfill the minimum debt of 100
    if min_deposit_value < MIN_DEPOSIT_VALUE {
        return Err(TokenFactoryError::CustomError { val: format!("Minimum deposit value for this loop: {}, is less than our minimum used to ensure unloopability: {}", min_deposit_value, MIN_DEPOSIT_VALUE) })
    }

    //If amount to mint is greater than max_mint_amount, set it to max_mint_amount while retaining the minimum 101 CDT LTV gap
    let amount_to_mint = match max_mint_amount {
        Some(max_mint_amount) => min(amount_to_mint, max(max_mint_amount, Uint128::new(102_000_000))), //Retain the minimum 101 CDT LTV gap + buffer
        None => amount_to_mint,
    };

    //Create mint msg
    let mint_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.cdp_contract_addr.to_string(),
        msg: to_json_binary(&CDP_ExecuteMsg::IncreaseDebt { 
            position_id: config.cdp_position_id,
            amount: Some(amount_to_mint),
            LTV: None,
            mint_to_addr: None,
        })?,
        funds: vec![],
    });
    msgs.push(mint_msg);
    //Create swap msg
    let swap_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.osmosis_proxy_contract_addr.to_string(),
        msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps { 
            token_out: config.deposit_token.deposit_token.clone(),
            max_slippage,
        })?,
        funds: vec![
            Coin {
                denom: config.cdt_denom.clone(),
                amount: amount_to_mint,
            }
        ],
    });
    let submsg = SubMsg::reply_on_success(swap_msg, LOOP_REPLY_ID);

    //Create Response
    let res = Response::new()
        .add_attribute("method", "loop_cdp")
        .add_attribute("current_collateral", running_collateral_amount)
        .add_attribute("current_debt", running_credit_amount)
        .add_messages(msgs)
        .add_submessage(submsg);

    Ok(res)
    
}

fn test_looping_peg_price(
    querier: QuerierWrapper,
    config: Config,
    desired_peg_price: Decimal, // Desired ratio between market and peg price 
) -> Result<(PriceResponse, PriceResponse), TokenFactoryError>{
    //Query basket for CDT peg price
    let basket: Basket = match  querier.query_wasm_smart::<Basket>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasket {  },
    ){
        Ok(basket) => basket,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP basket in test_looping_peg_price") }),
    };
    let cdt_peg_price: PriceResponse = basket.credit_price;

    //Check that CDT market price is equal or above 99% of peg
    let prices: Vec<PriceResponse> = match querier.query_wasm_smart::<Vec<PriceResponse>>(
        config.oracle_contract_addr.to_string(),
        &Oracle_QueryMsg::Price {
            asset_info: AssetInfo::NativeToken { denom: config.clone().cdt_denom },
            twap_timeframe: 0, //We want current swap price
            oracle_time_limit: 0,
            basket_id: None
        },
    ){
        Ok(prices) => prices,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the cdt price in post unloop") }),
    };
    let cdt_market_price: PriceResponse = prices[0].clone();

    if decimal_division(cdt_market_price.price, cdt_peg_price.price)? < desired_peg_price {
        return Err(TokenFactoryError::CustomError { val: format!("CDT price is below {} of peg, can't loop.", desired_peg_price) });
    }

    Ok((cdt_market_price, cdt_peg_price))
}
 
/// Calc mintable value & return new tokens to deposit, value & amount to mint
fn calc_mintable(
    swap_slippage: Decimal,
    vt_price: PriceResponse,
    deposit_token_price: PriceResponse,
    cdt_price: PriceResponse,
    vault_tokens: Uint128,
    debt: Uint128,
) -> StdResult<(Uint128, Decimal, Uint128)>{ 
    //Calc the value of the vault tokens
    let vault_tokens_value = vt_price.get_value(vault_tokens)?;
    //Calc the value of the CDT debt
    let debt_value = cdt_price.get_value(debt)?;
    //Calc LTV
    let ltv = decimal_division(debt_value, max(vault_tokens_value, Decimal::one()))?;
    //Calc the distance of the LTV to 90%
    let ltv_space_to_mint = match Decimal::percent(90).checked_sub(ltv){
        Ok(v) => v,
        Err(_) => return Err(StdError::GenericErr { msg: format!("LTV over 90%: {} > 0.9", ltv) }),
    }.checked_mul(Decimal::percent(100_00))?.to_uint_floor(); //this is done to get rid of the 3rd decimal place. Mints were erroring right above 90% LTV
    //Calc the value of the debt to mint
    let mintable_value = decimal_multiplication(vault_tokens_value, Decimal::percent(ltv_space_to_mint.u128() as u64))?;
    //Calc the amount of vault tokens to mint
    let amount_to_mint = cdt_price.get_amount(mintable_value)?;
    //Calc the value of the mintable value with slippage
    let min_deposit_value = decimal_multiplication(mintable_value, decimal_subtraction(Decimal::one(), swap_slippage)?)?;
    //Calc the minimum amount of deposit token we will send to the vault 
    let min_new_deposit_token = deposit_token_price.get_amount(min_deposit_value)?;

    // println!("Mintable Value: {}, {}, {}, {}", mintable_value, min_deposit_value, min_new_deposit_token, swap_slippage);

    Ok((min_new_deposit_token, min_deposit_value, amount_to_mint))
}


//Unloop the vaults CDP position
//..to withdraw for a user
//NOTE: 
//- Accrue beforehand if trying to fully unloop
//- All exits unloop at least once to ensure the vault has enough LTV space to cover the minimum debt in a single loop at all times. 
//..this is inefficient and can be changed to allow regular withdrawals until the minimum liquid value is reached ($91) (todo!): TO ADD
//POST LOOP NOTES:
// - Bc we only loop once, as long as our market price ceiling to start is at most the peg, we'll never make a trade more than, 100% + slippage, which is covered for by the entry fee.
fn unloop_cdp(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    desired_collateral_withdrawal: Uint128,
) -> Result<Response, TokenFactoryError> {
    //Load config
    let config = CONFIG.load(deps.storage)?;
    let mut unloop_props = UNLOOP_PROPS.load(deps.storage)?;
    let mut msgs: Vec<SubMsg> = vec![];

    //Error if not the contract calling
    if info.sender != env.contract.address {
        return Err(TokenFactoryError::Unauthorized {});
    }

    if unloop_props.loop_count == 0 {
        //Get running totals for CDP position & prices
        let (
            running_credit_amount, 
            running_collateral_amount, 
            vt_token_price, 
            cdt_market_price
        ) = get_cdp_position_info(deps.as_ref(), env.clone(), config.clone(), &mut vec![])?;

        //Get CDT peg price
        let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
            config.cdp_contract_addr.to_string(),
            &CDP_QueryMsg::GetBasket {  },
        ){
            Ok(basket) => basket,
            Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP basket in unloop") }),
        };
        let cdt_peg_price: PriceResponse = basket.credit_price.clone();
        
        //Ensure price is at or below peg
        //This will ensure unloops aren't unprofitable for remaining users
        //(todo!) CAN REMOVE: Cost conferred to the user "handles" this assurance.
        if decimal_division(cdt_market_price.price, cdt_peg_price.price)? > Decimal::one() {
            return Err(TokenFactoryError::CustomError { val: String::from("CDT price is above peg, can't unloop.") });
        }

        
        ///// 3) Get debt clearance amount/////    
        //Calc the value of the vault tokens
        let vault_tokens_value = vt_token_price.clone().get_value(running_collateral_amount)?;
        //Calc the value of the CDT debt
        let debt_value = cdt_peg_price.clone().get_value(running_credit_amount)?;
        //Calc debt_to_clear
        let liquid_value = decimal_subtraction(vault_tokens_value, debt_value)?;
        let leverage = decimal_division(vault_tokens_value, liquid_value)?;
        let owned_collateral = decimal_multiplication(Decimal::from_ratio(desired_collateral_withdrawal, Uint128::one()), leverage)?;
        let withdrawal_value = vt_token_price.clone().get_value(desired_collateral_withdrawal)?;
        let ltv = decimal_division(debt_value, vault_tokens_value)?;
        let debt_value_to_clear = decimal_multiplication(withdrawal_value, ltv)?;
        let debt_to_clear = cdt_peg_price.clone().get_amount(debt_value_to_clear)?;


        //Set unloop props
        unloop_props = UnloopProps {
            sender: unloop_props.sender.clone(),
            owned_collateral: owned_collateral.clone().to_uint_floor(),
            debt_to_clear: debt_to_clear.clone(),
            loop_count: 0,
            running_collateral_amount: running_collateral_amount.clone(),
            running_credit_amount: running_credit_amount.clone(),
            vt_token_price: vt_token_price.clone(),
            cdt_peg_price: cdt_peg_price.clone(),
            cdt_market_price: cdt_market_price.clone(),
        };        
    }

     
    //To exit without leaving remaining depositors with liabilities, the exitooor must unloop the debt they owe as well.
    // Ex: $100 collateral --- $50 debt = $50 liquid value (deposit value). If all $50 wanted to exit, the vault would fully unloop.
    //...If only half wanted to exit, the vault would unloop $25 of debt & $50 of collateral bc its in 2x leverage.
    
    //3) Get debt clearance amount
    //4) Clear that debt

    //4) Clear as much of the debt as possible in this loop 
    if !unloop_props.running_credit_amount.is_zero() && unloop_props.loop_count < LOOP_MAX && !unloop_props.debt_to_clear.is_zero() {
        //1) Withdraw vault token to swap and clear CDT debt
        let (_withdrawable_collateral, withdrawable_value_minus_slippage) = calc_withdrawable_collateral(
            config.clone().swap_slippage, 
            unloop_props.vt_token_price.clone(),
            unloop_props.cdt_peg_price.clone(),
            unloop_props.running_collateral_amount,
            unloop_props.running_credit_amount,
            false
        )?;

        //Set value needed to clear the debt
        let debt_to_clear_value = unloop_props.cdt_market_price.clone().get_value(unloop_props.debt_to_clear)?;
        let debt_to_clear_value_plus_slippage = decimal_multiplication(debt_to_clear_value, Decimal::one() + config.swap_slippage)?;        
        //Excess due to overcalculated slippage is vault profit

        //Can we withdraw more than the value needed to clear the debt?
        let withdrawable_value_minus_slippage = min(withdrawable_value_minus_slippage, debt_to_clear_value_plus_slippage.clone());
        //We're always unlooping at least once to withdraw 
        //& the unloop amount can be dynamic based on the desired withdrawal

        //Get withdrawable collateral
        let withdrawable_collateral = unloop_props.vt_token_price.clone().get_amount(withdrawable_value_minus_slippage)?;
        //Create withdraw msg
        let withdraw_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.cdp_contract_addr.to_string(),
            msg: to_json_binary(&CDP_ExecuteMsg::Withdraw { 
                position_id: config.cdp_position_id,
                assets: vec![
                    Asset {
                        info: AssetInfo::NativeToken {
                            denom: config.deposit_token.clone().vault_token,
                        },
                        amount: withdrawable_collateral,
                    }
                ],
                send_to: None,
            })?,
            funds: vec![],
        });
        msgs.push(SubMsg::new(withdraw_msg));
        
        //2) Exit vault
        let exit_vault_strat =  CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.deposit_token.vault_addr.to_string(),
            msg: to_json_binary(&Vault_ExecuteMsg::ExitVault { })?,
            funds: vec![
                Coin {
                    denom: config.deposit_token.vault_token.clone(),
                    amount: withdrawable_collateral,
                }
            ],
        });
        msgs.push(SubMsg::reply_on_success(exit_vault_strat, EXIT_VAULT_STRAT_REPLY_ID));

        //Update owned_collateral
        unloop_props.owned_collateral = match unloop_props.owned_collateral.checked_sub(withdrawable_collateral){
            Ok(v) => v,
            //This should never happen.
            //The only way this errors is if the debt is greater than the collateral amount || the conversion rate to swap for CDT is so bad the value of collateral can't reoup the debt
            Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract owned_collateral: {} - {}", unloop_props.owned_collateral, withdrawable_collateral) }),
        };
        //Owned collateral is used to transfer the cost of the withdrawal to the user

        //Update running collateral amount
        unloop_props.running_collateral_amount = match unloop_props.running_collateral_amount.checked_sub(withdrawable_collateral){
            Ok(v) => v,
            Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract running_collateral_amount: {} - {}", unloop_props.running_collateral_amount, withdrawable_collateral) }),
        };

        //Save UNLOOP propogations
        UNLOOP_PROPS.save(deps.storage, &unloop_props)?;

        ///// Split off into a submsg here /////
        //1) repay debt
        //2) increment loop
        //3) Check if allotted debt is cleared 
        //4) If not, reloop
        
    } else if unloop_props.running_credit_amount.is_zero() {
        //Attempt a normal withdrawal if the debt is 0 
        let withdraw_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.cdp_contract_addr.to_string(),
            msg: to_json_binary(&CDP_ExecuteMsg::Withdraw { 
                position_id: config.cdp_position_id,
                assets: vec![
                    Asset {
                        info: AssetInfo::NativeToken {
                            denom: config.deposit_token.clone().vault_token,
                        },
                        amount: unloop_props.owned_collateral.clone(),
                    }
                ],
                send_to: None,
            })?,
            funds: vec![],
        });
        msgs.push(SubMsg::new(withdraw_msg));
        
        //Exit the vault token vault 
        let withdraw_deposit_tokens_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.deposit_token.vault_addr.to_string(),
            msg: to_json_binary(&Vault_ExecuteMsg::ExitVault { })?,
            funds: vec![
                Coin {
                    denom: config.deposit_token.vault_token.clone(),
                    amount: unloop_props.owned_collateral.clone(),
                }
            ],
        });
        msgs.push(SubMsg::new(withdraw_deposit_tokens_msg));

        //Query the backing for the strategy's vault tokens
        let vt_backing: Uint128 = match deps.querier.query_wasm_smart::<Uint128>(
            config.deposit_token.vault_addr.to_string(),
            &Vault_QueryMsg::VaultTokenUnderlying { 
                vault_token_amount: unloop_props.owned_collateral.clone()
            },
        ){
            Ok(backing) => backing,
            Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the Mars Vault Token for the backing amount in unloop reply") }),
        };

        //Send the deposit tokens to the user
        let send_deposit_to_user_msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
            to_address: unloop_props.clone().sender,
            amount: vec![Coin {
                denom: config.deposit_token.deposit_token.clone(),
                amount: vt_backing - Uint128::one(), //Vault rounding error
            }],
        });
        msgs.push(SubMsg::new(send_deposit_to_user_msg));
    }

    //Create Response
    let res = Response::new()
        .add_attribute("method", "unloop_cdp")
        .add_attribute("remaining_collateral", unloop_props.running_collateral_amount)
        .add_attribute("remaining_debt", unloop_props.running_credit_amount)
        .add_submessages(msgs);

    Ok(res)
}

// fn post_unloop(
//     deps: DepsMut,
//     env: Env,
//     info: MessageInfo,
// ) -> Result<Response, TokenFactoryError>{
//     //Load config
//     let config = CONFIG.load(deps.storage)?;

//     //Error if not the contract calling
//     if info.sender != env.contract.address {
//         return Err(TokenFactoryError::Unauthorized {});
//     }

//     //Query basket for CDT peg price
//     let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
//         config.cdp_contract_addr.to_string(),
//         &CDP_QueryMsg::GetBasket {  },
//     ){
//         Ok(basket) => basket,
//         Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP basket in unloop") }),
//     };
//     let cdt_peg_price: Decimal = basket.credit_price.price;

//     //Check that CDT market price is equal or below 101% of peg
//     let prices: Vec<PriceResponse> = match deps.querier.query_wasm_smart::<Vec<PriceResponse>>(
//         config.oracle_contract_addr.to_string(),
//         &Oracle_QueryMsg::Price {
//             asset_info: AssetInfo::NativeToken { denom: config.clone().cdt_denom },
//             twap_timeframe: 0, //We want current swap price
//             oracle_time_limit: 0,
//             basket_id: None
//         },
//     ){
//         Ok(prices) => prices,
//         Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the cdt price in post unloop") }),
//     };
//     let cdt_market_price: Decimal = prices[0].clone().price;

//     if decimal_division(cdt_market_price, cdt_peg_price)? > Decimal::percent(100) + config.swap_slippage {
//         return Err(TokenFactoryError::CustomError { val: String::from("CDT price is above peg more than the config's slippage, can't unloop.") });
//     }

//     //Create Response
//     let res = Response::new()
//         .add_attribute("method", "post_unloop")
//         .add_attribute("cdt_market_price", cdt_market_price.to_string())
//         .add_attribute("cdt_peg_price", cdt_peg_price.to_string());


//     Ok(res)
    
// }

//Return CP position info
fn get_cdp_position_info(
    deps: Deps,
    env: Env,
    config: Config,
    msgs: &mut Vec<CosmosMsg>,
) -> StdResult<(Uint128,Uint128, PriceResponse, PriceResponse)> {
    //Query VT & CDT token price
    let prices: Vec<PriceResponse> = match deps.querier.query_wasm_smart::<Vec<PriceResponse>>(
        config.oracle_contract_addr.to_string(),
        &Oracle_QueryMsg::Prices {
            asset_infos: vec![AssetInfo::NativeToken { denom: config.clone().deposit_token.vault_token },
            AssetInfo::NativeToken { denom: config.cdt_denom.clone() }],
            twap_timeframe: 0, //We want current swap price
            oracle_time_limit: 0,
        },
    ){
        Ok(prices) => prices,
        Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the VT & CDT token price in get_cdp_position_info") }),
    };   
    let vt_token_price: PriceResponse = prices[0].clone();
    let cdt_price: PriceResponse = prices[1].clone();

    //Query the CDP position for the amount of vault tokens we have as collateral
    let vault_position: Vec<BasketPositionsResponse> = match deps.querier.query_wasm_smart::<Vec<BasketPositionsResponse>>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasketPositions { 
            start_after: None, 
            user: None,
            user_info: Some(UserInfo {
                position_owner: env.contract.address.to_string(),
                position_id: config.cdp_position_id,
            }), 
            limit: None, 
        },
    ){
        Ok(vault_position) => vault_position,
        Err(err) => return Err(StdError::GenericErr { msg: String::from("Failed to query the CDP Position for the vault token amount in get_cdp_position_info:") + &err.to_string() }),
    };
    let vault_position: PositionResponse = vault_position[0].positions[0].clone();

    //Set running credit amount 
    let running_credit_amount = vault_position.credit_amount;
    //Set running collateral amount
    let running_collateral_amount = vault_position.collateral_assets[0].asset.amount;

    //If vault position has more than 1 collateral asset, withdraw all except the 1st (the accepted vault token) and send them to the contract owner (Governance)
    if vault_position.collateral_assets.len() > 1 {
        let mut assets_to_withdraw: Vec<Asset> = vec![];
        for (index, asset) in vault_position.collateral_assets.iter().enumerate(){
            if index > 0 {
                assets_to_withdraw.push(asset.clone().asset);
            }
        }

        let withdraw_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.cdp_contract_addr.to_string(),
            msg: to_json_binary(&CDP_ExecuteMsg::Withdraw { 
                position_id: config.cdp_position_id,
                assets: assets_to_withdraw,
                send_to: Some(config.owner.clone().to_string()),
            })?,
            funds: vec![],
        });
        msgs.push(withdraw_msg);        
    }
    //We do this to ensure we don't loop (i.e. take risk) with value in volatile assets that may have been planted in our position maliciously
    

    Ok((running_credit_amount, running_collateral_amount, vt_token_price, cdt_price))   
}


//We want to loop if looped APR is greater than base APR 
// BUT unloop if looped APR is unprofitable
//Otherwise, returns Ok(()) if unprofitable
//So in this scenario, the unleveraged_apr is the buffer between loops and unloops.
// fn test_7day_apr_profitability(
//     deps: Deps,
//     env: Env,
//     config: Config,
//     is_loop: bool,
// ) -> Result<(), TokenFactoryError> {
//     //Query the vault's APR
//     let apr: APRResponse = query_apr(deps, env)?;

//     //Check that the 7 day APR is unprofitable.
//     //NOTE: The swap fees to loop & unloop is the only reason why we aren't also checking the APR-Cost difference is above the base APR.
//     //Users will withdraw if the base APR is better than the APR-Cost difference

//     let weekly_apr = apr.week_apr.unwrap_or_else(|| Decimal::zero());
//     let monthly_apr = apr.month_apr.unwrap_or_else(|| Decimal::zero());
//     let three_month_apr = apr.three_month_apr.unwrap_or_else(|| Decimal::zero());
//     let unleveraged_apr = decimal_division(weekly_apr, apr.leverage)?;
//     //Rate to lose max slippage in a month & 3 months
//     let month_rate_to_lose_swap_fee = decimal_multiplication(Decimal::percent(12_00), config.swap_slippage)?;
//     let three_month_rate_to_lose_swap_fee = decimal_multiplication(Decimal::percent(4_00), config.swap_slippage)?;



//     //We want to loop if looped APR is greater than base APR
//     // meaning we must *Error* to allow a loop
//     if is_loop && three_month_apr > apr.cost && decimal_subtraction(three_month_apr, apr.cost)? > unleveraged_apr 
//     ||
//     is_loop && apr.three_month_apr.is_none() && monthly_apr > apr.cost && decimal_subtraction(monthly_apr, apr.cost)? > unleveraged_apr 
//     || 
//     is_loop && (apr.three_month_apr.is_none() && apr.month_apr.is_none()) && weekly_apr > apr.cost && decimal_subtraction(weekly_apr, apr.cost)? > unleveraged_apr {
//         return Err(TokenFactoryError::CustomError { val: String::from("Looped APR is more profitable than unleveraged APR") });
//     }
//     //This stops UNLOOPS if the APR is profitable at all
//     //bc unlooping has a cost we don't want to unloop to optimize the APR.
//     else if !is_loop && weekly_apr > apr.cost {
//         return Err(TokenFactoryError::CustomError { val: String::from("Looped 7 day APR is profitable") });
//     }
//     ////////Both the 1 & 3 month need to be unprofitable to unloop/////////
//     //The idea is that yield is a long term game so we'll eat short term unprofitability for long term gains.
//     //Users who can't stomach short term losses will withdraw their funds which: 1) pays users staying & 2) reduces the vaults cost.
//     else if !is_loop && !monthly_apr.is_zero() && apr.cost > monthly_apr && decimal_subtraction(apr.cost, monthly_apr)? < month_rate_to_lose_swap_fee {
//         return Err(TokenFactoryError::CustomError { val: format!("Monthly cost didn't lose the vault {:?} this period so we can't unloop", config.swap_slippage) });
//     }
//     else if !is_loop && !three_month_apr.is_zero() && apr.cost > three_month_apr && decimal_subtraction(apr.cost, three_month_apr)? > three_month_rate_to_lose_swap_fee {
//         return Err(TokenFactoryError::CustomError { val: format!("3 month cost didn't lose the vault {:?} this period so we can't unloop", config.swap_slippage) });
//     }


//     //This return means we are allowing an unloop
//     Ok(())
// }

//Withdrawable collateral for unloops
fn calc_withdrawable_collateral(
    swap_slippage: Decimal,
    vt_price: PriceResponse,
    cdt_price: PriceResponse,
    vault_tokens: Uint128,
    debt: Uint128,
    in_reply: bool, //If this is in a reply, we don't want to return an error for being under debt minimum
) -> StdResult<(Uint128, Decimal)>{ //withdrawal_amount, withdraw_value
    //If debt is 0, quick return 
    if debt.is_zero() {
        return Ok((vault_tokens, Decimal::zero())); //we don't use withdraw value 
    } 
    //Calc the value of the vault tokens
    let vault_tokens_value = vt_price.get_value(vault_tokens)?;
    //Calc the value of the CDT debt
    let debt_value = cdt_price.get_value(debt)?;
    //Calc LTV
    let ltv = decimal_division(debt_value, max(vault_tokens_value, Decimal::one()))?;
    //Calc the distance of the LTV to 89% (the max borrowable LTV is 90 so we want to leave a 1% buffer to bypass and tiny precision errors)
    let ltv_space_to_withdraw = match Decimal::percent(89).checked_sub(ltv){
        Ok(v) => v,
        Err(_) => return Err(StdError::GenericErr { msg: format!("LTV over 89%: {} > 0.89", ltv) }),
    };
    //Calc the value of the vault tokens we withdraw
    //It's either clearing the debt (accounting for the swap slippage) or using the LTV space
    let mut withdrawable_value = min(
        decimal_division(decimal_multiplication(vault_tokens_value, ltv_space_to_withdraw)?, Decimal::percent(90))?,
        decimal_multiplication(debt_value, Decimal::one() + swap_slippage)?,
    );


    //WE HAVE TO UNLOOP MORE THAN DESIRED TO ENSURE THE MINIMUM DEBT BUFFER IS ALWAYS AVAILABLE
    //so we unloop extra but only withdraw the desired amount at the end.
    // ACTION: We add the minimum LTV space into the desired withdrawal amount to ensure we can always clear the debt.
    // The reason is, the LTV will always go up (read: the buffer will shrink) unless we withdraw double since they are pulling the desired out.
    // So instead we are left with whatever they don't pull out which will be the minimum or the min + the leftover.
    // Visual Representation (Art): In this manner it would be a "withdrawal queue" where the buffer decreases as people withdraw until it rests 
    //...at the minimum debt value (MDV) where the withdrawal max is the MDV starting point + MAX_LOOPS.
    // The buffer only regenerates after redemptions or new deposits that don't get looped.


    /////If withdrawable_value puts the debt value below $100, make sure to leave the minimum debt so the CDP doesn't error
    let minimum_debt_value = Decimal::percent(101_00);
    //We've failed if debt value is ever more than withdrawable value, we don't want to reach this
    if debt_value > withdrawable_value && decimal_subtraction(debt_value, withdrawable_value)? < minimum_debt_value {
        //Calc the difference
        let difference = match decimal_subtraction(debt_value, minimum_debt_value){
            Ok(v) => v,
            Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to subtract debt_value from minimum_debt_value: {} - {}", debt_value, minimum_debt_value) }),
        };

        //Set withdrawable_value to the difference
        withdrawable_value = difference;

    } 
    //We should never get here, 
    //If this errors the CDP repay function would've errored later.
    else if !in_reply && debt_value < minimum_debt_value {
        return Err(StdError::GenericErr { msg: format!("Debt value: ({}), is less than minimum debt value: ({}), which will error in the CDP repay function anyway. Someone needs to add more capital to the contract's CDP, whose position ID is in the config, to create more withdrawal space to totally unloop.", debt_value, minimum_debt_value) })
    }

    //Set minimum withdrawn & swapped value
    let withdrawal_w_slippage = decimal_multiplication(withdrawable_value, decimal_subtraction(Decimal::one(), swap_slippage)?)?;
    
    //Return the amount of vault tokens we can withdraw
    let withdrawable_collateral = vt_price.get_amount(withdrawable_value)?;
    
    Ok((withdrawable_collateral, withdrawal_w_slippage))
}


///Rate assurance
/// Ensures that the conversion rate is static for deposits & withdrawals
/// We are trusting that Mars deposits will only go up.
fn rate_assurance(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    exit: bool,
) -> Result<Response, TokenFactoryError> {
    //Load config    
    let config = CONFIG.load(deps.storage)?;

    //Error if not the contract calling
    if info.sender != env.contract.address {
        return Err(TokenFactoryError::Unauthorized {});
    }

    //Load State
    let token_rate_assurance = TOKEN_RATE_ASSURANCE.load(deps.storage)?;
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;

    //Get total deposit tokens
    let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config)?;

    //Calc the rate of vault tokens to deposit tokens
    let btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    //Check that the rates are static for everything other than exits.
    //Exits will show an increase bc of the entry fee & calculation logic.
    if !(btokens_per_one >= token_rate_assurance.pre_btokens_per_one) {
        return Err(TokenFactoryError::CustomError { val: format!("Conversation rate assurance failed, should be equal or greater than. If its 1 off just try again. Deposit tokens per 1 pre-tx: {:?} --- post-tx: {:?}", token_rate_assurance.pre_btokens_per_one, btokens_per_one) });
    }

    Ok(Response::new())
}


/// Accepts USDC, deposits these to the respective Mars Supply Vault & sends user vault tokens
/// - SubMsg deposits all vault tokens into CDP contract
/// NOTE: Deposits will error if the vt token used as collateral is over cap in the CDP contract
fn enter_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    //Load State
    let mut config = CONFIG.load(deps.storage)?;

    //Load position's info
    let (
        vault_credit_amount, 
        _, 
        _, 
        _
    ) = get_cdp_position_info(deps.as_ref(), env.clone(), config.clone(), &mut vec![])?;

    //No deposits if the vault debt is over 200 CDT
    if vault_credit_amount > Uint128::new(200_000_000){
        return Err(TokenFactoryError::CustomError { val: String::from("Vault debt is over 200 CDT, no deposits allowed") });
    }
 
    //Assert the only token sent is the deposit token
    if info.funds.len() != 1 {
        return Err(TokenFactoryError::CustomError { val: format!("More than 1 asset was sent, this function only accepts the deposit token: {:?}", config.clone().deposit_token) });
    }
    if info.funds[0].denom != config.deposit_token.deposit_token {
        return Err(TokenFactoryError::CustomError { val: format!("The wrong asset was sent ({:?}), this function only accepts the deposit token: {:?}", info.funds[0].denom, config.clone().deposit_token) });
    }
    
    //Get the amount of deposit token sent
    let deposit_amount = info.funds[0].amount;

    //////Calculate the amount of vault tokens to mint////
    //Get total deposit tokens
    let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;

    //Get the total amount of vault tokens circulating
    let total_vault_tokens: Uint128 = VAULT_TOKEN.load(deps.storage)?;
    //Calc & save base token rates
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    TOKEN_RATE_ASSURANCE.save(deps.storage, &TokenRateAssurance {
        pre_btokens_per_one,
    })?;

    let decimal_deposit_amount = Decimal::from_ratio(deposit_amount, Uint128::one());
    //Calculate the amount of vault tokens to mint
    let vault_tokens_to_distribute = calculate_vault_tokens(
        //Reduce the deposit amount by the slippage to account for the user's actual ownership amount.
        //This is the entry fee.
        decimal_multiplication(decimal_deposit_amount, decimal_subtraction(Decimal::one(), config.swap_slippage)?)?.to_uint_floor(),
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    // println!("vault_tokens_to_distribute: {:?}, {}, {}, {}", vault_tokens_to_distribute, total_deposit_tokens, decimal_multiplication(decimal_deposit_amount, decimal_subtraction(Decimal::one(), config.swap_slippage)?)?.to_uint_floor(), total_vault_tokens);
    ////////////////////////////////////////////////////

    let mut msgs: Vec<SubMsg> = vec![];
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
    msgs.push(SubMsg::new(mint_vault_tokens_msg));

    //Update the total token amounts
    VAULT_TOKEN.save(deps.storage, &(total_vault_tokens + vault_tokens_to_distribute))?;
    
    //Send the deposit tokens to the yield strategy
    let send_deposit_to_yield_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.deposit_token.vault_addr.to_string(),
        msg: to_json_binary(&Vault_ExecuteMsg::EnterVault {  })?,
        funds: vec![Coin {
            denom: config.deposit_token.deposit_token.clone(),
            amount: deposit_amount,
        }],
    });
    msgs.push(
        SubMsg::reply_on_success(send_deposit_to_yield_msg, ENTER_VAULT_REPLY_ID)
    );

    //Query the amount of vault tokens we'll get from this deposit
    let vt_received: Uint128 = match deps.querier.query_wasm_smart::<Uint128>(
        config.deposit_token.vault_addr.to_string(),
        &Vault_QueryMsg::DepositTokenConversion { deposit_token_amount: deposit_amount },
    ){
        Ok(vt_received) => vt_received,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the Mars Vault Token for the deposit token conversion amount") }),
    };
    
    //Update the total deposit tokens
    config.total_nonleveraged_vault_tokens += vt_received - Uint128::one(); //Vault rounding error

    //Save Updated Config
    CONFIG.save(deps.storage, &config)?;
    

    //Add rate assurance callback msg
    if !total_deposit_tokens.is_zero() && !total_vault_tokens.is_zero() {
        //UNCOMMENT
        msgs.push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::RateAssurance { exit: false })?,
            funds: vec![],
        })));
    }

    //Create Response
    let res = Response::new()
        .add_attribute("method", "enter_vault")
        .add_attribute("deposit_amount", deposit_amount)
        .add_attribute("vault_tokens_distributed", vault_tokens_to_distribute)
        .add_attribute("vt_sent_to_cdp", deposit_amount)
        .add_submessages(msgs);

    Ok(res)
}

/// User sends vault_tokens to withdraw the deposit_token from the vault
/// We burn vault tokens & unstake whatever was withdrawn
fn exit_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    let mut config = CONFIG.load(deps.storage)?;
    let mut msgs: Vec<CosmosMsg> = vec![];
    
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
        return Err(TokenFactoryError::CustomError { val: String::from("Need to send more than 0 vault tokens") });
    }
   
    //Get total deposit tokens
    let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config.clone())?;
    //Get the total amount of vault tokens circulating
    let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;
    //Calc & save token rate
    let pre_btokens_per_one = calculate_base_tokens(
        Uint128::new(1_000_000_000_000), 
        total_deposit_tokens, 
        total_vault_tokens
    )?;
    TOKEN_RATE_ASSURANCE.save(deps.storage, &TokenRateAssurance {
        pre_btokens_per_one,
    })?;
    //Get amount of tokens to exit
    let mut deposit_tokens_to_withdraw = calculate_base_tokens(
        vault_tokens, 
        total_deposit_tokens, 
        total_vault_tokens
    )?;

    ///////NO EXIT FEE///////////// 
    // The unlooper eats the conversion rate && the slippage.
    
    //Query how many vault tokens we need to withdraw from the CDP Position for the deposit token withdrawal
    let mars_vault_tokens_to_withdraw: Uint128 = match deps.querier.query_wasm_smart::<Uint128>(
        config.deposit_token.vault_addr.to_string(),
        &Vault_QueryMsg::DepositTokenConversion { deposit_token_amount: deposit_tokens_to_withdraw },
    ){
        Ok(vault_tokens) => vault_tokens,
        Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the Mars Vault Token for the deposit token conversion amount") }),
    };
        
    //Get the amount of vault tokens in the contract
    let contract_balance_of_deposit_vault_tokens = deps.querier.query_balance(env.clone().contract.address.to_string(), config.deposit_token.clone().vault_token)?.amount;

    //Calc the amount of vt tokens to withdraw from the CDP position
    let vtokens_to_unloop_from_cdp = match mars_vault_tokens_to_withdraw.checked_sub(contract_balance_of_deposit_vault_tokens){
        Ok(v) => v,
        Err(_) => Uint128::zero(), //This means contract balance is larger and we don't need to unloop any extra
    };
        
    ////Withdraw tokens from CDP Position///
    //..which requires us to unloop
    if !vtokens_to_unloop_from_cdp.is_zero() {
        let unloop_to_withdraw = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::UnloopCDP { 
                desired_collateral_withdrawal: vtokens_to_unloop_from_cdp,
                })?,
            funds: vec![],
        });
        // println!("deposit_tokens_to_withdraw: {:?}", deposit_tokens_to_withdraw);
        msgs.push(unloop_to_withdraw);
    } 
    //if vtokens_to_unloop_from_cdp is zero, we can just withdraw the necessary vault tokens
    else {
        //Exit the vault token vault 
        let withdraw_vault_tokens_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.deposit_token.vault_addr.to_string(),
            msg: to_json_binary(&Vault_ExecuteMsg::ExitVault { })?,
            funds: vec![
                Coin {
                    denom: config.deposit_token.vault_token.clone(),
                    amount: mars_vault_tokens_to_withdraw.clone(),
                }
            ],
        });
        msgs.push(withdraw_vault_tokens_msg);

        //Query the backing for the strategy's vault tokens
        let vt_backing: Uint128 = match deps.querier.query_wasm_smart::<Uint128>(
            config.deposit_token.vault_addr.to_string(),
            &Vault_QueryMsg::VaultTokenUnderlying { 
                vault_token_amount: mars_vault_tokens_to_withdraw.clone()
            },
        ){
            Ok(backing) => backing,
            Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the Mars Vault Token for the backing amount in unloop reply") }),
        };
        //Send the deposit tokens to the user
        let send_deposit_to_user_msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
            to_address: unloop_props.clone().sender,
            amount: vec![Coin {
                denom: config.deposit_token.deposit_token.clone(),
                amount: vt_backing - Uint128::one(), //Vault rounding error
            }],
        });
        msgs.push(send_deposit_to_user_msg);
    }

    //Reset & Set Sender for Unloop Props
    UNLOOP_PROPS.save(deps.storage, &UnloopProps {
        sender: info.sender.clone().to_string(),
        owned_collateral: Uint128::zero(),
        debt_to_clear: Uint128::zero(),
        loop_count: 0,
        running_collateral_amount: Uint128::zero(),
        running_credit_amount: Uint128::zero(),
        vt_token_price: PriceResponse {
            price: Decimal::zero(),
            prices: vec![],
            decimals: 6
        },
        cdt_peg_price: PriceResponse {
            price: Decimal::zero(),
            prices: vec![],
            decimals: 6
        },
        cdt_market_price: PriceResponse {
            price: Decimal::zero(),
            prices: vec![],
            decimals: 6
        },
    })?;

    
    //Burn vault tokens
    let burn_vault_tokens_msg: CosmosMsg = TokenFactory::MsgBurn {
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin {
            denom: config.vault_token.clone(),
            amount: vault_tokens.to_string(),
        }), 
        burn_from_address: env.contract.address.to_string(),
    }.into();
    //UNCOMMENT
    msgs.push(burn_vault_tokens_msg);

    //Update the total vault tokens
    let new_vault_token_supply = match total_vault_tokens.checked_sub(vault_tokens){
        Ok(v) => v,
        Err(_) => return Err(TokenFactoryError::CustomError { val: format!("Failed to subtract vault token total supply: {} - {}", total_vault_tokens, vault_tokens) }),
    };
    VAULT_TOKEN.save(deps.storage, &new_vault_token_supply)?;
       
    //Add rate assurance callback msg if this withdrawal leaves other depositors with tokens to withdraw
    if !new_vault_token_supply.is_zero() && total_deposit_tokens > deposit_tokens_to_withdraw {
        //UNCOMMENT
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_json_binary(&ExecuteMsg::RateAssurance { exit: false })?,
            funds: vec![],
        }));
    } 

    //Create Response 
    let res = Response::new()
        .add_attribute("method", "exit_vault")
        .add_attribute("vault_tokens", vault_tokens)
        .add_attribute("deposit_tokens_withdrawn_to_exit_with", deposit_tokens_to_withdraw)
        .add_messages(msgs);

    Ok(res)
}

fn update_nonleveraged_vault_tokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, TokenFactoryError> {
    //Load State
    let mut config = CONFIG.load(deps.storage)?;
    //Unloop props will be up to date bc this is called after the unloop process
    let unloop_props = UNLOOP_PROPS.load(deps.storage)?;

    //Error if not the contract calling
    if info.sender != env.contract.address {
        return Err(TokenFactoryError::Unauthorized {});
    }

    //Save old total
    let old_total_nonleveraged_vault_tokens = config.total_nonleveraged_vault_tokens;

    //Get balance of vault tokens in the contract
    let contract_balance_of_deposit_vault_tokens = deps.querier.query_balance(env.clone().contract.address.to_string(), config.deposit_token.clone().vault_token)?.amount;
    
    //Update config's total non-leveraged vault tokens if we reset to zero.
    //This allows us to take into account swap fees, slippage & redemptions.
    if unloop_props.running_credit_amount.is_zero() {
        config.total_nonleveraged_vault_tokens = unloop_props.running_collateral_amount + contract_balance_of_deposit_vault_tokens;
    
        CONFIG.save(deps.storage, &config)?;
    }

    Ok(Response::new().add_attributes(vec![
        attr("method", "update_unleveraged_total"),
        attr("old_total_nonleveraged_vault_tokens", old_total_nonleveraged_vault_tokens),
        attr("new_total_nonleveraged_vault_tokens", config.total_nonleveraged_vault_tokens),
    ]))
}
 
/// Update contract configuration
/// This function is only callable by an owner with non_token_contract_auth set to true
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    cdp_contract_addr: Option<String>,
    mars_vault_addr: Option<String>,
    osmosis_proxy_contract_addr: Option<String>,
    oracle_contract_addr: Option<String>,
    withdrawal_buffer: Option<Decimal>,
    deposit_cap: Option<Uint128>,
    swap_slippage: Option<Decimal>,
    vault_cost_index: Option<()>,
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
    if let Some(addr) = cdp_contract_addr {
        config.cdp_contract_addr = deps.api.addr_validate(&addr)?;
        attrs.push(attr("updated_cdp_contract_addr", addr));
    }
    if let Some(addr) = mars_vault_addr {
        config.deposit_token.vault_addr = deps.api.addr_validate(&addr)?;
        attrs.push(attr("updated_mars_vault_addr", addr));
    }
    if let Some(addr) = osmosis_proxy_contract_addr {
        config.osmosis_proxy_contract_addr = deps.api.addr_validate(&addr)?;
        attrs.push(attr("updated_osmosis_proxy_contract_addr", addr));
    }
    if let Some(addr) = oracle_contract_addr {
        config.oracle_contract_addr = deps.api.addr_validate(&addr)?;
        attrs.push(attr("updated_oracle_contract_addr", addr));
    }
    if let Some(buffer) = withdrawal_buffer {
        config.withdrawal_buffer = buffer;
        attrs.push(attr("updated_withdrawal_buffer", buffer.to_string()));
    }
    if let Some(debt_cap) = deposit_cap {
        config.deposit_cap = debt_cap;
        attrs.push(attr("updated_deposit_cap", debt_cap));
    }
    if let Some(slippage) = swap_slippage {
        config.swap_slippage = slippage;
        attrs.push(attr("updated_swap_slippage", slippage.to_string()));
    }
    if let Some(_) = vault_cost_index {
        //Query the basket to find the index of the vault_token
        let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
            config.cdp_contract_addr.to_string(),
            &CDP_QueryMsg::GetBasket { },
        ){
            Ok(basket) => basket,
            Err(_) => return Err(TokenFactoryError::CustomError { val: String::from("Failed to query the CDP Basket") }),
        };
        //Find the index
        let mut saved_index: Option<u64> = None;
        for (index, asset) in basket.clone().collateral_types.into_iter().enumerate(){
            if asset.asset.info.to_string() == config.deposit_token.clone().vault_token {
                saved_index = Some(index as u64);
                break;
            }
        }
        if let Some(index) = saved_index {
            config.vault_cost_index = index as usize;
        } else {
            return Err(TokenFactoryError::CustomError { val: String::from("Failed to find the vault token in the CDP Basket") });
        }    
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
    //Get total deposit tokens
    let total_deposit_tokens = get_total_deposit_tokens(deps.as_ref(), env.clone(), config)?;

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
            attr("added_time_to__checkpoint", time_since_last_checkpoint.to_string())
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
        QueryMsg::ClaimTracker {} => to_json_binary(&CLAIM_TRACKER.load(deps.storage)?),
    }
}

/// Return APR for the valid durations 7, 30, 90, 365 days using the Mars Vault's APR
/// & cost of the CDP
// fn query_apr(
//     deps: Deps,
//     env: Env,
// ) -> StdResult<APRResponse> {
//     let config = CONFIG.load(deps.storage)?;
//     //Load VT total
//     let total_vault_tokens = VAULT_TOKEN.load(deps.storage)?;    
//     //Get total deposit tokens
//     let total_deposit_tokens = get_total_deposit_tokens(deps, env.clone(), config.clone())?;
//     //Calc the rate of vault tokens to deposit tokens
//     let btokens_per_one = calculate_base_tokens(
//         Uint128::new(1_000_000_000_000), 
//         total_deposit_tokens,
//         total_vault_tokens
//     )?;
//     //Initiate APRResponse
//     let mut aprs = APRResponse {
//         week_apr: None,
//         month_apr: None,
//         three_month_apr: None,
//         year_apr: None,
//         leverage: Decimal::zero(),
//         cost: Decimal::zero()
//     };
//     //Load the claim tracker    
//     let claim_tracker = CLAIM_TRACKER.load(deps.storage)?;
//     //Query the CDP Basket
//     let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
//         config.cdp_contract_addr.to_string(),
//         &CDP_QueryMsg::GetBasket { },
//     ){
//         Ok(basket) => basket,
//         Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the CDP Basket") }),
//     };

//     ////Find the leverage of the contract////
//     //Query the collateral of the contract's CDP
//     //Query the CDP position for the amount of vault tokens we have
//     let vault_position: Vec<BasketPositionsResponse> = match deps.querier.query_wasm_smart::<Vec<BasketPositionsResponse>>(
//         config.cdp_contract_addr.to_string(),
//         &CDP_QueryMsg::GetBasketPositions { 
//             start_after: None, 
//             user: None,
//             user_info: Some(UserInfo {
//                 position_owner: env.contract.address.to_string(),
//                 position_id: config.cdp_position_id,
//             }), 
//             limit: None, 
//         },
//     ){
//         Ok(vault_position) => vault_position,
//         Err(err) => return Err(StdError::GenericErr { msg: String::from("Failed to query the CDP Position for the vault token amount in get_total_deposit_tokens:") + &err.to_string() }),
//     };
//     let vault_position: PositionResponse = vault_position[0].positions[0].clone();
    
//     //Query the collateral token price
//     let collateral_price: PriceResponse = match deps.querier.query_wasm_smart::<Vec<PriceResponse>>(
//         config.oracle_contract_addr.to_string(),
//         &Oracle_QueryMsg::Prices { 
//             asset_infos: vec![vault_position.collateral_assets[0].asset.info.clone()],            
//             twap_timeframe: 0,
//             oracle_time_limit: 0,
//         },
//     ){
//         Ok(price) => price[0].clone(),
//         Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the collateral token price") }),
//     };

//     //Set running collateral amount
//     let vt_collateral_amount = vault_position.collateral_assets[0].asset.amount;
//     //Calc collateral value
//     let vt_collateral_value: Decimal = collateral_price.get_value(vt_collateral_amount)?;
//     //Set running credit amount
//     let vt_credit_amount = vault_position.credit_amount;
//     //Calc credit value
//     let vt_credit_value: Decimal = basket.credit_price.get_value(vt_credit_amount)?;
//     //Set liquid value of the CDP
//     let vt_liquid_value = match decimal_subtraction(vt_collateral_value, vt_credit_value){
//         Ok(v) => v,
//         Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to subtract the credit value from the collateral value in query_apr") }),
//     };
    
//     //Find the leverage by dividing the value of the collateral by the liquid value of the CDP
//     let leverage = match decimal_division(vt_collateral_value, vt_liquid_value){
//         Ok(v) => v,
//         Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to divide the collateral value by the liquid value of the CDP in query_apr") }),
//     };
//     aprs.leverage = leverage;
    
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
//             let change_ratio = match decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
//              Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one())){
//                 Ok(v) => v,
//                 Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to divide the base tokens per one, {}, by the vt claim, {}, of the checkpoint in query_apr (weekly)", btokens_per_one, claim_checkpoint.vt_claim_of_checkpoint) }),
//              };

//             let percent_change = match change_ratio.checked_sub(Decimal::one()){
//                 Ok(diff) => diff,
//                 //For this to happen, the slippage from the swap is greater than the Mars APR & redemption profits in this timeframe
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
//             let change_ratio = match decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
//              Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one())){
//                 Ok(v) => v,
//                 Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to divide the base tokens per one, {}, by the vt claim, {}, of the checkpoint in query_apr (monthly)", btokens_per_one, claim_checkpoint.vt_claim_of_checkpoint) }),
//              };

//             let percent_change = match change_ratio.checked_sub(Decimal::one()){
//                 Ok(diff) => diff,
//                 //For this to happen, the slippage from the swap is greater than the Mars APR & redemption profits in this timeframe
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
//             let change_ratio = match decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
//              Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one())){
//                 Ok(v) => v,
//                 Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to divide the base tokens per one, {}, by the vt claim, {}, of the checkpoint in query_apr (3M)", btokens_per_one, claim_checkpoint.vt_claim_of_checkpoint) }),
//              };

//             let percent_change = match change_ratio.checked_sub(Decimal::one()){
//                 Ok(diff) => diff,
//                 //For this to happen, the slippage from the swap is greater than the Mars APR & redemption profits in this timeframe
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
//             let change_ratio = match decimal_division(Decimal::from_ratio(btokens_per_one, Uint128::one()),
//              Decimal::from_ratio(claim_checkpoint.vt_claim_of_checkpoint, Uint128::one())){
//                 Ok(v) => v,
//                 Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to divide the base tokens per one, {}, by the vt claim, {}, of the checkpoint in query_apr (annual)", btokens_per_one, claim_checkpoint.vt_claim_of_checkpoint) }),
//              };

//             let percent_change = match change_ratio.checked_sub(Decimal::one()){
//                 Ok(diff) => diff,
//                 //For this to happen, the slippage from the swap is greater than the Mars APR & redemption profits in this timeframe
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

//     //Query the cost of the deposit vault's vault token
//     let basket_interest: CollateralInterestResponse = match deps.querier.query_wasm_smart::<CollateralInterestResponse>(
//         config.cdp_contract_addr.to_string(),
//         &CDP_QueryMsg::GetCollateralInterest {  },
//     ){
//         Ok(basket_interest) => basket_interest,
//         Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the CDP collateral interest rates in query_apr") }),
//     };
//     let vt_cost: Decimal = basket_interest.rates[config.vault_cost_index].clone();
//     //Set cost
//     aprs.cost = vt_cost;

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
    
    //Get total deposit tokens
    let total_deposit_tokens = get_total_deposit_tokens(deps, env.clone(), config.clone())?;
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

//This calcs the value of tokens backing the CDP minus the debt minus swap fees if the debt is non-zero
// The swap fees won't be detracted if the withdrawal is within the buffer
//..but calculate it anyway as a reward for the people not withdrawing (if the CDP is open with debt)
fn get_total_deposit_tokens(
    deps: Deps,
    env: Env,
    config: Config,
) -> StdResult<Uint128> { //total deposit tokens
    //Get CDT price
    let basket: Basket = match deps.querier.query_wasm_smart::<Basket>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasket {  },
    ){
        Ok(basket) => basket,
        Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the CDP basket in get_total_deposit_tokens") }),
    };
    let cdt_peg_price: PriceResponse = basket.credit_price;
    //Get vault token price
    let prices: Vec<PriceResponse> = match deps.querier.query_wasm_smart::<Vec<PriceResponse>>(
        config.oracle_contract_addr.to_string(),
        &Oracle_QueryMsg::Price {
            asset_info: AssetInfo::NativeToken{ denom: config.clone().deposit_token.vault_token },
            twap_timeframe: 60, //We want the price the CDP will use
            oracle_time_limit: 600,
            basket_id: None
        },
    ){
        Ok(prices) => prices,
        Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the VT token price in get_total_deposit_tokens") }),
    };
    let vt_token_price: PriceResponse = prices[0].clone();
    //Query the CDP position for the amount of vault tokens we have as collateral
    let vault_position: Vec<BasketPositionsResponse> = match deps.querier.query_wasm_smart::<Vec<BasketPositionsResponse>>(
        config.cdp_contract_addr.to_string(),
        &CDP_QueryMsg::GetBasketPositions { 
            start_after: None, 
            user: None,
            user_info: Some(UserInfo {
                position_owner: env.contract.address.to_string(),
                position_id: config.cdp_position_id,
            }), 
            limit: None, 
        },
    ){
        Ok(vault_position) => vault_position,
        Err(err) => return Err(StdError::GenericErr { msg: String::from("Failed to query the CDP Position for the vault token amount in get_total_deposit_tokens:") + &err.to_string() }),
    };
    let vault_position: PositionResponse = vault_position[0].positions[0].clone();
    //Calc value of the debt
    let debt_value = cdt_peg_price.get_value(vault_position.credit_amount)?;
    //Calc value of the collateral
    let collateral_value = vt_token_price.get_value(vault_position.collateral_assets[0].asset.amount)?;
    //Calc the value of the collateral minus the debt
    let mut liquid_value = match collateral_value.checked_sub(debt_value){
        Ok(v) => v,
        Err(_) => return Err(StdError::GenericErr { msg: format!("Failed to subtract the debt from the collateral in get_total_deposit_tokens, collateral value: {}, debt value: {}", collateral_value, debt_value) }),
    };
    
    //Calc the amount of vaulted deposit tokens
    let mut total_vaulted_deposit_tokens = vt_token_price.get_amount(liquid_value)?;

    //Query the contract balance for the amount of buffered vault tokens
    let vt_buffer = match deps.querier.query_balance(env.contract.address.to_string(), config.deposit_token.vault_token){
        Ok(balance) => balance.amount,
        Err(_) => Uint128::zero(),
    };
    //Add buffered assets to the total deposit tokens
    total_vaulted_deposit_tokens += vt_buffer;
    
    //Deduct slippage costs.
    // let decimal_total_vdt = Decimal::from_ratio(total_vaulted_deposit_tokens, Uint128::one());
    //For buffered vt or zero'd debt, this acts as the entry fee.
    // total_vaulted_deposit_tokens = decimal_multiplication(decimal_total_vdt, decimal_subtraction(Decimal::one(), config.swap_slippage)?)?.to_uint_floor();\
    //^Commented out bc the entry fee is taken on deposit. This line messes up value calcs.

    //Query the underlying of the initial vault token deposit
    let underlying_deposit_token: Uint128 = match deps.querier.query_wasm_smart::<Uint128>(
        config.deposit_token.vault_addr.to_string(),
        &Vault_QueryMsg::VaultTokenUnderlying { vault_token_amount: total_vaulted_deposit_tokens },
    ){
        Ok(underlying_deposit_token) => underlying_deposit_token,
        Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the Mars Vault Token for the underlying deposit amount in instantiate") }),
    };

    Ok(underlying_deposit_token)
    
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.id {
        ENTER_VAULT_REPLY_ID => handle_enter_reply(deps, env, msg),
        CDP_REPLY_ID => handle_cdp_reply(deps, env, msg),
        LOOP_REPLY_ID => handle_loop_reply(deps, env, msg),
        UNLOOP_REPLY_ID => handle_unloop_reply(deps, env, msg),
        EXIT_VAULT_STRAT_REPLY_ID => handle_exit_deposit_token_vault_reply(deps, env, msg),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
} 

//This is used when the VT collateral is withdrawn from the CDP, exited from the vault & needs to be swapped to CDT
fn handle_exit_deposit_token_vault_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_result) => {
            //Load config
            let config = CONFIG.load(deps.storage)?;  
               
            //Query balance for the deposit token received from the exit vault
            let deposit_token_balance = deps.querier.query_balance(env.contract.address.to_string(), config.clone().deposit_token.deposit_token)?.amount;   

            //Sell tokens for CDT
            let sell_deposit_token_for_CDT = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.osmosis_proxy_contract_addr.to_string(),
                msg: to_json_binary(&OP_ExecuteMsg::ExecuteSwaps { 
                    token_out: config.cdt_denom.clone(),
                    max_slippage: config.swap_slippage,
                })?,
                funds: vec![
                    Coin {
                        denom: config.deposit_token.deposit_token.clone(),
                        amount: deposit_token_balance,
                    }
                ],
            });
            let submsg = SubMsg::reply_on_success(sell_deposit_token_for_CDT, UNLOOP_REPLY_ID);

            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_exit_deposit_token_vault_reply")
                .add_attribute("deposit_token_swapped_for", deposit_token_balance)
                .add_submessage(submsg);

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}


//1) repay debt 
//2) increment loop (state variable)
//3) Check if debt is cleared by updating debt_to_clear
//4) If not, reloop 
fn handle_unloop_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_result) => {
            //Load config
            let config = CONFIG.load(deps.storage)?;  
            let mut unloop_props = UNLOOP_PROPS.load(deps.storage)?;
            let mut msgs = vec![];
               
            //Query balance for the CDT received from the swap
            let cdt_balance = deps.querier.query_balance(env.contract.address.to_string(), config.clone().cdt_denom)?.amount;
            
            //1) Repay the CDP loan
            //Create repay_msg
            let repay_CDP_loan = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.cdp_contract_addr.to_string(),
                msg: to_json_binary(&CDP_ExecuteMsg::Repay { 
                    position_id: config.cdp_position_id,
                    position_owner: None,
                    send_excess_to: None,
                })?,
                funds: vec![
                    Coin {
                        denom: config.cdt_denom.clone(),
                        amount: cdt_balance,
                    }
                ],
            });
            msgs.push(repay_CDP_loan);

            //2) Increment loop count & update running credit amount
            unloop_props.loop_count += 1;
            unloop_props.running_credit_amount = match unloop_props.running_credit_amount.checked_sub(cdt_balance){
                Ok(v) => v,
                Err(_) => Uint128::zero(), //excess will sit in the contract for later bc the CDP will send it back when we repay
            };

            //3) Check if debt is cleared by updating debt_to_clear
            unloop_props.debt_to_clear = match unloop_props.debt_to_clear.checked_sub(cdt_balance){
                Ok(v) => v,
                Err(_) => Uint128::zero(),
            };

            //If debt is clear, we're done.
            //- Withdraw the remaining owned collateral.
            //- Exit the vault token vault.
            //- Send the deposit tokens to the user.
            if unloop_props.debt_to_clear.is_zero(){
                //Withdraw the remaining owned collateral
                let withdraw_msg = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.cdp_contract_addr.to_string(),
                    msg: to_json_binary(&CDP_ExecuteMsg::Withdraw { 
                        position_id: config.cdp_position_id,
                        assets: vec![
                            Asset {
                                info: AssetInfo::NativeToken {
                                    denom: config.deposit_token.clone().vault_token,
                                },
                                amount: unloop_props.owned_collateral.clone(),
                            }
                        ],
                        send_to: None,
                    })?,
                    funds: vec![],
                });
                msgs.push(withdraw_msg);
                //Exit the vault token vault 
                let withdraw_deposit_tokens_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.deposit_token.vault_addr.to_string(),
                    msg: to_json_binary(&Vault_ExecuteMsg::ExitVault { })?,
                    funds: vec![
                        Coin {
                            denom: config.deposit_token.vault_token.clone(),
                            amount: unloop_props.owned_collateral.clone(),
                        }
                    ],
                });
                msgs.push(withdraw_deposit_tokens_msg);

                //Query the backing for the strategy's vault tokens
                let vt_backing: Uint128 = match deps.querier.query_wasm_smart::<Uint128>(
                    config.deposit_token.vault_addr.to_string(),
                    &Vault_QueryMsg::VaultTokenUnderlying { 
                        vault_token_amount: unloop_props.owned_collateral.clone()
                    },
                ){
                    Ok(backing) => backing,
                    Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the Mars Vault Token for the backing amount in unloop reply") }),
                };
                // panic!("sender: {:?}", unloop_props.sender);
                //Send the deposit tokens to the user
                let send_deposit_to_user_msg: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
                    to_address: unloop_props.clone().sender,
                    amount: vec![Coin {
                        denom: config.deposit_token.deposit_token.clone(),
                        amount: vt_backing - Uint128::one(), //Vault rounding error
                    }],
                });
                msgs.push(send_deposit_to_user_msg);
            }
            //if this is the last loop & the debt is not cleared, error
            else if unloop_props.loop_count >= LOOP_MAX {
                //Error if debt is 0 and the debt isnt cleared
                //This should be unreachable.
                if unloop_props.running_credit_amount.is_zero(){
                    return Err(StdError::GenericErr { msg: format!("Failed to hit the desired collateral withdrawal in unloop reply yet debt is 0. Either exit_vault is asking for too much and the remaining balance is actually in the contract OR the vault owns less than it says your tokens are worth.") });
                } 
                return Err(StdError::GenericErr { msg: format!("Failed to clear the debt, {} leftover CDT, for this exit in {} loops, attempt to withdraw less at once", unloop_props.debt_to_clear, LOOP_MAX) });
            } 
            //4) If not & debt != 0, reloop by calling with the owned collateral
            // After the first loop the desired collateral withdrawal doesn't matter but owned collateral would be the wrong value if it did.     
            else if !unloop_props.running_credit_amount.is_zero(){
                //Create unloop msg
                let unloop_msg = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: env.contract.address.to_string(),
                    msg: to_json_binary(&ExecuteMsg::UnloopCDP { 
                        desired_collateral_withdrawal: unloop_props.owned_collateral.clone(),
                    })?,
                    funds: vec![],
                });
                msgs.push(unloop_msg);
            }            
           
            //Save updated unloop props
            UNLOOP_PROPS.save(deps.storage, &unloop_props.clone())?;

            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_unloop_reply")
                .add_attribute("loop_count", unloop_props.loop_count.to_string())                
                .add_attribute("cdt_swapped_for", cdt_balance)
                .add_attribute("unloop_props", format!("{:?}", unloop_props))
                .add_messages(msgs);

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

fn handle_loop_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_result) => {
            //Load config
            let config = CONFIG.load(deps.storage)?;  
            let mut msgs = vec![];
               
            //Query balances for the deposit token received from the swap
            let deposit_token_balance = deps.querier.query_balance(env.contract.address.to_string(), config.clone().deposit_token.deposit_token)?.amount;
            
            //Query how many vault tokens we'll get for this deposit
            let vault_tokens: Uint128 = match deps.querier.query_wasm_smart::<Uint128>(
                config.deposit_token.vault_addr.to_string(),
                &Vault_QueryMsg::DepositTokenConversion { deposit_token_amount: deposit_token_balance.clone() },
            ){
                Ok(vault_tokens) => vault_tokens,
                Err(_) => return Err(StdError::GenericErr { msg: String::from("Failed to query the Mars Vault Token for the new deposit amount in loop") }),
            };           
            
            //Create enter into vault msg
            let deposit_msg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.deposit_token.vault_addr.to_string(),
                msg: to_json_binary(&Vault_ExecuteMsg::EnterVault { })?,
                funds: vec![
                    Coin {
                        denom: config.deposit_token.deposit_token.clone(),
                        amount: deposit_token_balance,
                    }
                ],
            });
            msgs.push(deposit_msg);
                     
            //Deposit any excess vtokens into the CDP
            let ( _, _, _, vt_sent_to_cdp ) = get_buffer_amounts(
                deps.querier, 
                config.clone(),
                env.contract.address.to_string(),
            )?;

            //Create deposit msg
            let cdp_deposit_msg = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.cdp_contract_addr.to_string(),
                msg: to_json_binary(&CDP_ExecuteMsg::Deposit { 
                    position_id: Some(config.cdp_position_id),
                    position_owner: None,
                })?,
                funds: vec![
                    Coin {
                        denom: config.deposit_token.clone().vault_token,
                        amount: vault_tokens + vt_sent_to_cdp,
                    }
                ],
            });
            msgs.push(cdp_deposit_msg);
            
            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_loop_reply")
                .add_attribute("deposit_tokens_swapped_for", deposit_token_balance)
                .add_attribute("vault_tokens_sent_to_cdp", vault_tokens + vt_sent_to_cdp)
                .add_messages(msgs);

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

fn handle_cdp_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(result) => {
            let cdp_event = result
                .events
                .into_iter()
                .find(|e| e.attributes.iter().any(|attr| attr.key == "position_id"))
                .ok_or_else(|| StdError::GenericErr {  msg: String::from("unable to find cdp deposit event")})?;

                let position_id = &cdp_event
                .attributes
                .iter()
                .find(|attr| attr.key == "position_id")
                .unwrap()
                .value;
                let position_id = Uint128::from_str(position_id)?;
            //Load config
            let mut config = CONFIG.load(deps.storage)?;  
            //Save the position ID
            config.cdp_position_id = position_id;
            //Save Updated Config
            CONFIG.save(deps.storage, &config)?;

            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_initial_cdp_deposit_reply")
                .add_attribute("vault_position_id", position_id);  

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

/// - Add the vault tokens received from the vault deposit into config state
/// - Deposit all vault tokens into CDP contract
fn handle_enter_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(_result) => {
            //Load config
            let mut config = CONFIG.load(deps.storage)?;  
            let mut msgs = vec![];
            
            let ( _, _, vt_kept, vt_sent_to_cdp ) = get_buffer_amounts(
                deps.querier, 
                config.clone(),
                env.contract.address.to_string(),
            )?;

            // println!("vt_kept: {}, vt_sent_to_cdp: {}", vt_kept, vt_sent_to_cdp);

            
            //Deposit the calc'd amount to the CDP Position
            if !vt_sent_to_cdp.is_zero() {
                let send_deposit_to_yield_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.cdp_contract_addr.to_string(),
                    msg: to_json_binary(&CDP_ExecuteMsg::Deposit { 
                        position_id: Some(config.cdp_position_id),
                        position_owner: None,
                    })?,
                    funds: vec![Coin {
                        denom: config.deposit_token.clone().vault_token,
                        amount: vt_sent_to_cdp
                    }],
                });
                msgs.push(send_deposit_to_yield_msg);
            }

            //Create Response
            let res = Response::new()
                .add_attribute("method", "handle_enter_vault_reply")
                .add_attribute("vault_tokens_sent_to_cdp", vt_sent_to_cdp)
                .add_attribute("vault_tokens_kept", vt_kept)
                .add_messages(msgs);

            return Ok(res);

        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
}

fn get_buffer_amounts(
    querier: QuerierWrapper,
    config: Config,
    contract_address: String,
) -> StdResult<(Decimal, Uint128, Uint128, Uint128)> {
    /////Send the vault tokens to the yield strategy///
    //Find the vault token balance in the contract
    let contract_balance_of_deposit_vault_tokens = querier.query_balance(contract_address, config.deposit_token.clone().vault_token)?.amount;

    //Calculate ratio of vault tokens in the contract to the total vault tokens
    let ratio_of_tokens_in_contract = decimal_division(Decimal::from_ratio(contract_balance_of_deposit_vault_tokens, Uint128::one()), Decimal::from_ratio(max(config.total_nonleveraged_vault_tokens, Uint128::one()), Uint128::one()))?;
    
    //Calculate what is sent and what is kept
    let mut vt_sent_to_cdp: Uint128 = Uint128::zero();
    let mut vt_kept: Uint128 = Uint128::zero();            
    let desired_ratio_tokens = decimal_multiplication(Decimal::from_ratio(config.total_nonleveraged_vault_tokens, Uint128::one()), config.withdrawal_buffer)?;

    //If the ratio of tokens in the contract is less than the withdrawal_buffer, calculate the amount of vault tokens to send to the yield strategy
    if ratio_of_tokens_in_contract < config.withdrawal_buffer {
        //Calculate the amount of vault tokens that would make the ratio equal to the withdrawal buffer
        let tokens_to_fill_ratio = desired_ratio_tokens.to_uint_floor() - contract_balance_of_deposit_vault_tokens;
        //How much do we send to the yield strategy
        if tokens_to_fill_ratio >= contract_balance_of_deposit_vault_tokens {
            vt_kept = contract_balance_of_deposit_vault_tokens;
        } else {
            vt_sent_to_cdp = contract_balance_of_deposit_vault_tokens - tokens_to_fill_ratio;
            vt_kept = tokens_to_fill_ratio;
        }
    } else
    //If the ratio to keep is past the threshold then calculate how much to send to the CDP
    {
        //Calculate the amount of vault tokens that would make the ratio equal to the withdrawal buffer
        let tokens_to_lower_ratio = contract_balance_of_deposit_vault_tokens - desired_ratio_tokens.to_uint_floor() ;
        //How much do we send to the yield strategy
        if tokens_to_lower_ratio >= contract_balance_of_deposit_vault_tokens {
            vt_sent_to_cdp = contract_balance_of_deposit_vault_tokens;
        } else {
            vt_kept = contract_balance_of_deposit_vault_tokens - tokens_to_lower_ratio;
            vt_sent_to_cdp = tokens_to_lower_ratio;
        }                
    }

    Ok(( ratio_of_tokens_in_contract, contract_balance_of_deposit_vault_tokens, vt_kept, vt_sent_to_cdp ))
}


#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, env: Env, _msg: MigrateMsg) -> Result<Response, TokenFactoryError> {
    //Delete old UNLOOP_PROPS
    // UNLOOP_PROPS.remove(deps.storage);
    //New struct will save on the next exit vault

    Ok(Response::default())
}