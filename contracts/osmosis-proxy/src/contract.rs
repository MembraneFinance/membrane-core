//Token factory fork
//https://github.com/osmosis-labs/bindings/blob/main/contracts/tokenfactory

use std::convert::TryInto;
use std::str::FromStr;

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, to_binary, Addr, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, QuerierWrapper,
    QueryRequest, Reply, Response, StdError, StdResult, Uint128, SubMsg, CosmosMsg, BankMsg, Coin, coins,
};
use cw2::set_contract_version;
use osmosis_std::types::osmosis::gamm::v1beta1::GammQuerier;

use crate::error::TokenFactoryError;
use crate::state::{TokenInfo, CONFIG, TOKENS};
use membrane::osmosis_proxy::{
    Config, ExecuteMsg, GetDenomResponse, InstantiateMsg, QueryMsg, TokenInfoResponse,
};
use membrane::types::{Pool, PoolStateResponse};
use osmosis_std::types::osmosis::tokenfactory::v1beta1::{self as TokenFactory, QueryDenomsFromCreatorResponse};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:osmosis-proxy";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const CREATE_DENOM_REPLY_ID: u64 = 1u64;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    _msg: InstantiateMsg,
) -> Result<Response, TokenFactoryError> {
    let config = Config {
        owners: vec![info.sender.clone()],
        debt_auction: None,
    };
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("method", "instantiate")
        .add_attribute("owner", info.sender))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, TokenFactoryError> {
    match msg {
        ExecuteMsg::CreateDenom {
            subdenom,
            basket_id,
            max_supply,
            liquidity_multiplier,
        } => create_denom(
            deps,
            env,
            info,
            subdenom,
            basket_id,
            max_supply,
            liquidity_multiplier,
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
        ExecuteMsg::EditTokenMaxSupply { denom, max_supply } => {
            edit_token_max(deps, info, denom, max_supply)
        }
        ExecuteMsg::UpdateConfig {
            owner,
            add_owner,
            debt_auction,
        } => update_config(deps, info, owner, debt_auction, add_owner),
    }
}

fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    debt_auction: Option<String>,
    add_owner: bool,
) -> Result<Response, TokenFactoryError> {

    let mut config = CONFIG.load(deps.storage)?;

    let mut attrs = vec![
        attr("method", "edit_owners"),
        attr("add_owner", add_owner.to_string()),
    ];

    if !validate_authority(config.clone(), info) {
        return Err(TokenFactoryError::Unauthorized {});
    }

    //Edit Owner
    if let Some(owner) = owner {
        if add_owner {
            config.owners.push(deps.api.addr_validate(&owner)?);
        } else {
            deps.api.addr_validate(&owner)?;
            //Filter out owner
            config.owners = config
                .clone()
                .owners
                .into_iter()
                .filter(|stored_owner| *stored_owner != owner)
                .collect::<Vec<Addr>>();
        }
        attrs.push(attr("owner", owner));
    }

    //Edit Debt Auction
    if let Some(debt_auction) = debt_auction {
        config.debt_auction = Some(deps.api.addr_validate(&debt_auction)?);
    }

    //Save Config
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes(attrs))
}

fn validate_authority(config: Config, info: MessageInfo) -> bool {
    //Owners or Debt Auction have contract authority
    match config
        .owners
        .into_iter()
        .find(|owner| *owner == info.sender)
    {
        Some(_owner) => true,
        None => {
            if let Some(debt_auction) = config.debt_auction {
                info.sender == debt_auction
            } else {
                false
            }
        }
    }
}

pub fn create_denom(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    subdenom: String,
    basket_id: String,
    max_supply: Option<Uint128>,
    liquidity_multiplier: Option<Decimal>,
) -> Result<Response, TokenFactoryError> {

    let config = CONFIG.load(deps.storage)?;
    //Assert Authority
    if !validate_authority(config, info) {
        return Err(TokenFactoryError::Unauthorized {});
    }

    if subdenom.eq("") {
        return Err(TokenFactoryError::InvalidSubdenom { subdenom });
    }    

    let msg = TokenFactory::MsgCreateDenom { sender: env.contract.address.to_string(), subdenom: subdenom.clone() };

    let create_denom_msg = SubMsg::reply_on_success(msg, CREATE_DENOM_REPLY_ID );

    let res = Response::new()
        .add_attribute("method", "create_denom")
        .add_attribute("sub_denom", subdenom)
        .add_attribute("max_supply", max_supply.unwrap_or_else(Uint128::zero))
        .add_attribute("basket_id", basket_id)
        .add_attribute(
            "liquidity_multiplier",
            liquidity_multiplier
                .unwrap_or_else(Decimal::zero)
                .to_string(),
        )
        .add_submessage(create_denom_msg);

    Ok(res)
}

pub fn change_admin(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    denom: String,
    new_admin_address: String,
) -> Result<Response, TokenFactoryError> {

    let config = CONFIG.load(deps.storage)?;
    //Assert Authority
    if !validate_authority(config, info) {
        return Err(TokenFactoryError::Unauthorized {});
    }

    deps.api.addr_validate(&new_admin_address)?;

    validate_denom(deps.querier, denom.clone())?;

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

fn edit_token_max(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
    max_supply: Uint128,
) -> Result<Response, TokenFactoryError> {

    let config = CONFIG.load(deps.storage)?;
    //Assert Authority
    if !validate_authority(config, info) {
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
    if !validate_authority(config.clone(), info.clone()) {
        return Err(TokenFactoryError::Unauthorized {});
    }

    deps.api.addr_validate(&mint_to_address)?;

    if amount.eq(&Uint128::new(0_u128)) {
        return Result::Err(TokenFactoryError::ZeroAmount {});
    }

    validate_denom(deps.querier, denom.clone())?;

    //Debt Auction can mint over max supply
    let mut mint_allowed = false;
    if let Some(debt_auction) = config.debt_auction {
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
    let mint_tokens_msg = TokenFactory::MsgMint{
        sender: env.contract.address.to_string(), 
        amount: Some(osmosis_std::types::cosmos::base::v1beta1::Coin{
            denom: denom.clone(),
            amount: amount.to_string(),
        }), 
    }.into();    

    //Send minted assets to mint_to_address
    let send_msg = CosmosMsg::Bank(BankMsg::Send { 
        to_address: mint_to_address.clone(),
        amount: coins(amount.u128(), denom.clone()),
    });

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
            .add_messages(vec![mint_tokens_msg, send_msg])
            ;
    }

    Ok(res)
}

pub fn burn_tokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    denom: String,
    amount: Uint128,
    burn_from_address: String,
) -> Result<Response, TokenFactoryError> {
    
    let config = CONFIG.load(deps.storage)?;
    //Assert Authority
    if !validate_authority(config, info) {
        return Err(TokenFactoryError::Unauthorized {});
    }

    if amount.eq(&Uint128::new(0_u128)) {
        return Result::Err(TokenFactoryError::ZeroAmount {});
    }

    validate_denom(deps.querier, denom.clone())?;

    //Update Token Supply
    TOKENS.update(
        deps.storage,
        denom.clone(),
        |token_info| -> Result<TokenInfo, TokenFactoryError> {
            match token_info {
                Some(mut token_info) => {
                    token_info.current_supply -= amount;
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
        QueryMsg::GetDenom {
            creator_address,
            subdenom,
        } => to_binary(&get_denom(deps, creator_address, subdenom)?),
        QueryMsg::PoolState { id } => to_binary(&get_pool_state(deps, id)?),
        // QueryMsg::ArithmeticTwapToNow {
        //     id,
        //     quote_asset_denom,
        //     base_asset_denom,
        //     start_time,
        // } => to_binary(&get_arithmetic_twap_to_now(
        //     deps,
        //     id,
        //     quote_asset_denom,
        //     base_asset_denom,
        //     start_time,
        // )?),
        QueryMsg::GetTokenInfo { denom } => to_binary(&get_token_info(deps, denom)?),
        QueryMsg::Config {} => to_binary(&CONFIG.load(deps.storage)?),
    }
}

fn get_token_info(deps: Deps, denom: String) -> StdResult<TokenInfoResponse> {
    let token_info = TOKENS.load(deps.storage, denom.clone())?;
    Ok(TokenInfoResponse {
        denom,
        current_supply: token_info.current_supply,
        max_supply: token_info.max_supply.unwrap_or_else(Uint128::zero),
    })
}

// fn get_arithmetic_twap_to_now(
//     deps: Deps<OsmosisQuery>,
//     id: u64,
//     quote_asset_denom: String,
//     base_asset_denom: String,
//     start_time: i64,
// ) -> StdResult<ArithmeticTwapToNowResponse> {

//     osmosis_std::types::osmosis::

//     let msg =
//         OsmosisQuery::arithmetic_twap_to_now(id, quote_asset_denom, base_asset_denom, start_time);
//     let request: QueryRequest<OsmosisQuery> = OsmosisQuery::into(msg);

//     let response: ArithmeticTwapToNowResponse = deps.querier.query(&request)?;

//     Ok(response)
// }

fn get_pool_state(
    deps: Deps,
    pool_id: u64,
) -> StdResult<PoolStateResponse> {
    let res = GammQuerier::new(&deps.querier).pool(pool_id)?;
    
    let pool: Pool = res.pool
        .ok_or_else(|| StdError::NotFound {
            kind: "pool".to_string(),
        })?
        // convert `Any` to `Pool`
        .try_into()?;

    Ok(pool.into_pool_state_response())
    
}

fn get_denom(deps: Deps, creator_addr: String, subdenom: String) -> StdResult<GetDenomResponse> {

    let response: QueryDenomsFromCreatorResponse = TokenFactory::TokenfactoryQuerier::new(&deps.querier).denoms_from_creator(creator_addr)?;

    let denom = if let Some(denom) = response.denoms.into_iter().find(|denoms| denoms.contains(&subdenom)){
        denom
    } else {
        return Err(StdError::GenericErr { msg: String::from("Can'r find subdenom in list of contract denoms") })
    };

    Ok(GetDenomResponse {
        denom,
    })
}

pub fn validate_denom(
    querier: QuerierWrapper,
    denom: String,
) -> Result<(), TokenFactoryError> {
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
    let creator_address = tokenfactory_denom_parts[1];
    let subdenom = tokenfactory_denom_parts[2];

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
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
}

fn handle_create_denom_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {
    match msg.result.into_result() {
        Ok(result) => {
            let instantiate_event = result
                .events
                .into_iter()
                .find(|e| e.attributes.iter().any(|attr| attr.key == "subdenom"))
                .ok_or_else(|| {
                    StdError::generic_err("unable to find create_denom event".to_string())
                })?;

            let subdenom = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "subdenom")
                .unwrap()
                .value;

            let max_supply = &instantiate_event
                .attributes
                .iter()
                .find(|attr| attr.key == "max_supply")
                .unwrap()
                .value;

            /// Query all denoms created by this contract
            let tq = TokenFactory::TokenfactoryQuerier::new(&deps.querier);
            let res: QueryDenomsFromCreatorResponse = tq.denoms_from_creator(env.contract.address.into_string())?;
            let denom = if let Some(denom) = res.denoms.into_iter().find(|denom| denom.contains(subdenom)){
                denom
            } else { return Err(StdError::GenericErr { msg: String::from("Cannot find created denom") }) };
           

            let max_supply = {
                if Uint128::from_str(max_supply)?.is_zero() {
                    None
                } else {
                    Some(Uint128::from_str(max_supply)?)
                }
            };
            TOKENS.save(
                deps.storage,
                denom,
                &TokenInfo {
                    current_supply: Uint128::zero(),
                    max_supply,
                },
            )?;
        } //We only reply on success
        Err(err) => return Err(StdError::GenericErr { msg: err }),
    }
    Ok(Response::new())
}