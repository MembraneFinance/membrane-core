use core::panic;

use cosmwasm_std::{
    attr, entry_point, to_json_binary, Addr, Attribute, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Order, QuerierWrapper, QueryRequest, Reply, ReplyOn, Response, StdError, StdResult, Storage, SubMsg, Uint128, WasmMsg, WasmQuery
};
use cw2::set_contract_version;

use cw_storage_plus::Bound;
use membrane::oracle::PriceResponse;
use membrane::points_system::{ClaimCheck, Config, ExecuteMsg, InstantiateMsg, QueryMsg, UserStats, UserStatsResponse};
use membrane::math::decimal_multiplication;
use membrane::cdp::{ExecuteMsg as CDP_ExecuteMsg, MigrateMsg, QueryMsg as CDP_QueryMsg};
use membrane::stability_pool::{QueryMsg as SP_QueryMsg, ClaimsResponse};
use membrane::liq_queue::{QueryMsg as LIQ_QueryMsg, ClaimsResponse as LQ_ClaimsResponse};
use membrane::governance::{QueryMsg as GOV_QueryMsg, Proposal};
use membrane::oracle::QueryMsg as Oracle_QueryMsg;
use membrane::osmosis_proxy::ExecuteMsg as OP_ExecuteMsg;
use membrane::stability_pool_vault::QueryMsg as Vault_QueryMsg;
use membrane::types::{Asset, AssetInfo, Basket, UserInfo};

use crate::error::ContractError;
use crate::state::{LiquidationPropagation, VaultConversionRate, VaultInfo, CLAIM_CHECK, CONFIG, LIQ_PROPAGATION, OWNERSHIP_TRANSFER, USER_STATS, VAULT_INFO};

// Contract name and version used for migration.
const CONTRACT_NAME: &str = "points_system";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Seconds per day
const SECONDS_PER_DAY: u64 = 86_400;

// Pagination defaults
const PAGINATION_DEFAULT_LIMIT: u64 = 30;

//Reply IDs
const LIQUIDATION_REPLY_ID: u64 = 1u64;
const ACCRUE_REPLY_ID: u64 = 2u64;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let config = Config {
        owner: info.sender,
        cdt_denom: msg.cdt_denom,
        oracle_contract: deps.api.addr_validate(&msg.oracle_contract)?,
        positions_contract: deps.api.addr_validate(&msg.positions_contract)?,
        stability_pool_contract: deps.api.addr_validate(&msg.stability_pool_contract)?,
        liq_queue_contract: deps.api.addr_validate(&msg.liq_queue_contract)?,
        governance_contract: deps.api.addr_validate(&msg.governance_contract)?,
        osmosis_proxy_contract: deps.api.addr_validate(&msg.osmosis_proxy_contract)?,
        mbrn_per_point: Decimal::from_ratio(1_000_000u128, 1u128), //1
        total_mbrn_distribution: Uint128::zero(), 
        max_mbrn_distribution: Uint128::new(100_000_000000u128), //100_000
        points_per_dollar: Decimal::one(),
    };    

    // let config = Config {
    //     owner: Addr::unchecked("osmo1wk0zlag50ufu5wrsfyelrylykfe3cw68fgv9s8xqj20qznhfm44qgdnq86"),
    //     cdt_denom: String::from("factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/ucdt"),
    //     oracle_contract: Addr::unchecked("osmo16sgcpe0hcs42qk5vumk06jzmstkpka9gjda9tfdelwn65ksu3l7s7d4ggs"),
    //     positions_contract: Addr::unchecked("osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr"),
    //     stability_pool_contract: Addr::unchecked("osmo1326cxlzftxklgf92vdep2nvmqffrme0knh8dvugcn9w308ya9wpqv03vk8"),
    //     liq_queue_contract: Addr::unchecked("osmo1ycmtfa7h0efexjxuaw7yh3h3qayy5lspt9q4n4e3stn06cdcgm8s50zmjl"),
    //     governance_contract: Addr::unchecked("osmo1wk0zlag50ufu5wrsfyelrylykfe3cw68fgv9s8xqj20qznhfm44qgdnq86"),
    //     osmosis_proxy_contract: Addr::unchecked("osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd"),
    //     mbrn_per_point: Decimal::from_ratio(1_000_000u128, 1u128), //1
    //     total_mbrn_distribution: Uint128::zero(), 
    //     max_mbrn_distribution: Uint128::new(100_000_000000u128), //100_000
    //     points_per_dollar: Decimal::one(),
    // };    

    //Save Config
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
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
        ExecuteMsg::UpdateConfig { owner, cdt_denom, oracle_contract, positions_contract, stability_pool_contract, liq_queue_contract, governance_contract, osmosis_proxy_contract, mbrn_per_point, max_mbrn_distribution, points_per_dollar } => update_config(deps, info, owner, cdt_denom, oracle_contract, positions_contract, stability_pool_contract, liq_queue_contract, governance_contract, osmosis_proxy_contract, mbrn_per_point, max_mbrn_distribution, points_per_dollar),
        ExecuteMsg::Liquidate { position_id, position_owner } => liquidate_for_user(deps, env, info, position_id, position_owner),
        ExecuteMsg::CheckClaims { cdp_repayment, sp_claims, lq_claims, vote } => check_claims(deps, env, info, cdp_repayment, sp_claims, lq_claims, vote),
        ExecuteMsg::GivePoints { cdp_repayment, sp_claims, lq_claims, vote } => give_points(deps, env, info, cdp_repayment, sp_claims, lq_claims, vote),
        ExecuteMsg::ClaimMBRN {} => claim_mbrn_from_points(deps, env, info),
    }
}

//CheckClaims & GivePoints are used to sandwich executable msgs to check for claims before giving points
//1) CDP Repayment: Save CDP's pending revenue to check its difference in GivePoints
//2) SP Claims: Save SP's pending claims to check its difference in GivePoints
//3) LQ Claims: Save LQ's pending claims to check its difference in GivePoints
//4) Governance Votes: Save unvoted proposals to check for votes in GivePoints
fn check_claims(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,    
    cdp_repayment: Option<UserInfo>,
    sp_claims: bool,
    lq_claims: bool,
    vote: Option<Vec<u64>>,
) -> Result<Response, ContractError>{
    //Load config
    let config: Config = CONFIG.load(deps.storage)?;

    if cdp_repayment.is_none() && !sp_claims && !lq_claims && vote.is_none() {
        return Err(ContractError::Std(StdError::generic_err("No claims to check")));
    }

    let mut msgs: Vec<SubMsg> = vec![];
    
    //1) Check CDP repayment?
    if let Some(user_info) = cdp_repayment.clone() {
        //Create accrue message
        let accrue_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute { 
            contract_addr: config.clone().positions_contract.to_string(), 
            msg: to_json_binary(&CDP_ExecuteMsg::Accrue {
                position_ids: vec![ user_info.position_id ],
                position_owner: Some(user_info.position_owner),
            })?,
            funds: vec![] 
        });
        let sub_msg = SubMsg::reply_on_success(accrue_msg, ACCRUE_REPLY_ID);
        msgs.push(sub_msg);
    }

    let mut pending_sp_claims: ClaimsResponse = ClaimsResponse {
        claims: vec![],
    };
    //2) Check SP claims?
    if sp_claims {
        //Get SP's pending claims
        let sp_claims: ClaimsResponse = match deps.querier.query::<ClaimsResponse>(&QueryRequest::Wasm(WasmQuery::Smart { 
            contract_addr: config.clone().stability_pool_contract.to_string(), 
            msg: to_json_binary(&SP_QueryMsg::UserClaims { user: info.clone().sender.to_string() })?
        })){
            Ok(claims) => claims,
            //The SP errors if you have empty claims
            Err(_) => ClaimsResponse {
                claims: vec![],
            },
        };
        pending_sp_claims = sp_claims;
    }

    let mut pending_lq_claims: Vec<LQ_ClaimsResponse> = vec![];
    //3) Check Liquidation claims?
    if lq_claims {
        //Get Liquidation's pending claims
        let lq_claims: Vec<LQ_ClaimsResponse> = deps.querier.query::<Vec<LQ_ClaimsResponse>>(&QueryRequest::Wasm(WasmQuery::Smart { 
            contract_addr: config.clone().liq_queue_contract.to_string(), 
            msg: to_json_binary(&LIQ_QueryMsg::UserClaims { user: info.clone().sender.to_string() })?
        }))?;
        //Filter out any that are 0
        let lq_claims = lq_claims.into_iter().filter(|x| !x.pending_liquidated_collateral.is_zero()).collect::<Vec<LQ_ClaimsResponse>>();

        pending_lq_claims = lq_claims;
    }

    let mut unvoted_proposals: Vec<u64> = vec![];
    //4) Check Governance votes?
    if let Some(votes) = vote {
        for id in votes {
            //Query proposal
            let proposal: Proposal = deps.querier.query::<Proposal>(&QueryRequest::Wasm(WasmQuery::Smart { 
                contract_addr: config.clone().governance_contract.to_string(), 
                msg: to_json_binary(&GOV_QueryMsg::Proposal { proposal_id: id })?
            }))?;

            //Check queried proposal & add to the unvoted proposals list if the user hasn't voted
            let mut has_voted = false;
            //Check if user has voted
            if proposal.for_voters.contains(&info.clone().sender) 
            || proposal.against_voters.contains(&info.clone().sender) 
            || proposal.aligned_voters.contains(&info.clone().sender) 
            || proposal.removal_voters.contains(&info.clone().sender) 
            || proposal.amendment_voters.contains(&info.clone().sender) {
                has_voted = true;
            }
            if !has_voted {
                unvoted_proposals.push(id);
            }
        }
    }

    //Save Claim Check
    CLAIM_CHECK.save(deps.storage, &
        ClaimCheck {
            user: info.clone().sender,
            cdp_pending_revenue: Uint128::zero(),
            lq_pending_claims: pending_lq_claims.clone(),
            sp_pending_claims: pending_sp_claims.clone().claims,
            vote_pending: unvoted_proposals.clone(),
            check_time: env.block.time.seconds(),
        }
    )?;  

    //Set attributes
    let mut attrs: Vec<Attribute> = vec![];
        attrs.push(attr("accrual_msg_sent", cdp_repayment.is_some().to_string()));
        attrs.push(attr("sp_pending_claims", format!("{:?}", pending_sp_claims.clone().claims)));
        attrs.push(attr("lq_pending_claims", format!("{:?}", pending_lq_claims)));
        attrs.push(attr("vote_pending", format!("{:?}", unvoted_proposals)));   



    Ok(Response::new().add_attributes(attrs).add_submessages(msgs))
}


//1) CDP Repayment: Calc difference btwn CDP's pending revenue to validate how much was repaid
//- The sequence for points valid repayment is: accrue, claim_check, repay, give_points.
// Otherwise we'll account for less revenue than you may have paid if any.
//2) SP Claims: Check difference btwn present & pending claims & allocate points inline with value
//3) LQ Claims: Check difference btwn present & pending claims & allocate points inline with value
//4) Governance Votes: Give points for every unvoted proposal saved in CheckClaims that is now voted on
fn give_points(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,    
    cdp_repayment: bool,
    sp_claims: bool,
    lq_claims: bool,
    vote: Option<Vec<u64>>,
) -> Result<Response, ContractError>{
    //Load config
    let config: Config = CONFIG.load(deps.storage)?;
    //Load Claim Check
    let claim_check: ClaimCheck = CLAIM_CHECK.load(deps.storage)?;

    //Assert the caller is the same as the claim check user
    if info.clone().sender != claim_check.user {
        return Err(ContractError::Unauthorized {});
    }

    //Give points needs to be in the same block as the claim check
    if env.block.time.seconds() != claim_check.check_time {
        return Err(ContractError::Std(StdError::generic_err("Claim check is outdated")));
    }

    //Get CDP Basket    
    let basket: Basket = deps.querier.query::<Basket>(&QueryRequest::Wasm(WasmQuery::Smart { 
        contract_addr: config.clone().positions_contract.to_string(), 
        msg: to_json_binary(&CDP_QueryMsg::GetBasket {  })?
    }))?;
    
    let mut revenue_paid: Uint128 = Uint128::zero();
    //1) Check CDP repayment?
    if cdp_repayment {
        //Get CDP's pending revenue
        revenue_paid = match claim_check.cdp_pending_revenue.checked_sub(basket.pending_revenue){
            Ok(amount) => amount,
            Err(_) => return Err(ContractError::Std(StdError::generic_err("The sequence for points valid repayment is: accrue, claim_check, repay, give_points. Otherwise we'll account for less revenue than you may have paid if any."))),
        };
    }

    let mut sp_claim_diff: Vec<Coin> = vec![];
    //2) Check SP claims?
    if sp_claims {
        //Get SP's pending claims
        let sp_current_claims: ClaimsResponse = match deps.querier.query::<ClaimsResponse>(&QueryRequest::Wasm(WasmQuery::Smart { 
            contract_addr: config.clone().stability_pool_contract.to_string(), 
            msg: to_json_binary(&SP_QueryMsg::UserClaims { user: info.clone().sender.to_string() })?
        })){
            Ok(claims) => claims,
            //The SP errors if you have empty claims
            Err(_) => ClaimsResponse {
                claims: vec![],
            },
        };
        //Check difference in claims from the query & claim check
        for previous_claim in claim_check.sp_pending_claims.clone() {
            let mut found = false;             
            for current_claim in sp_current_claims.claims.clone() {
                if current_claim.denom == previous_claim.denom {
                    found = true;
                    let diff = match previous_claim.amount.checked_sub(current_claim.amount){
                        Ok(amount) => amount,
                        Err(_) => Uint128::zero(),
                    };
                    if diff > Uint128::zero() {
                        sp_claim_diff.push(Coin {
                            denom: current_claim.denom,
                            amount: diff,
                        });
                    }
                    continue;
                }
            }
            //Not found means it was fully claimed
            if !found {
                sp_claim_diff.push(previous_claim);
            }
        }
    }

    let mut lq_claim_diff: Vec<Coin> = vec![];
    //3) Check Liquidation claims?
    if lq_claims {
        //Get Liquidation's pending claims
        let lq_current_claims: Vec<LQ_ClaimsResponse> = deps.querier.query::<Vec<LQ_ClaimsResponse>>(&QueryRequest::Wasm(WasmQuery::Smart { 
            contract_addr: config.clone().liq_queue_contract.to_string(), 
            msg: to_json_binary(&LIQ_QueryMsg::UserClaims { user: info.clone().sender.to_string() })?
        }))?;        
        //Filter out any that are 0
        let lq_current_claims = lq_current_claims.into_iter().filter(|x| !x.pending_liquidated_collateral.is_zero()).collect::<Vec<LQ_ClaimsResponse>>();

        //Check difference in claims from the query & claim check
        for prev_claim in claim_check.lq_pending_claims.clone() {
            let mut found = false; 
            for current_claim in lq_current_claims.clone() {
                if current_claim.bid_for == prev_claim.bid_for {
                    found = true;
                    let diff = match prev_claim.pending_liquidated_collateral.0.checked_sub(current_claim.pending_liquidated_collateral.0){
                        Some(amount) => Uint128::new(amount.as_u128()),
                        None => Uint128::zero(),
                    };
                    if diff > Uint128::zero() {
                        lq_claim_diff.push(Coin {
                            denom: current_claim.bid_for,
                            amount: diff,
                        });
                    }
                    continue;
                }
            }
            if !found {
                lq_claim_diff.push(Coin {
                    denom: prev_claim.bid_for,
                    amount: Uint128::new(prev_claim.pending_liquidated_collateral.0.as_u128()),
                });
            }
        }
    }

    let mut newly_voted_proposals: Vec<u64> = vec![];
    //4) Check Governance votes?
    if let Some(votes) = vote {
        //Filter out proposal IDs that aren't in the claim check
        let votes = votes.into_iter().filter(|x| claim_check.vote_pending.contains(x)).collect::<Vec<u64>>();

        for id in votes {
            //Query proposal
            let proposal: Proposal = deps.querier.query::<Proposal>(&QueryRequest::Wasm(WasmQuery::Smart { 
                contract_addr: config.clone().governance_contract.to_string(), 
                msg: to_json_binary(&GOV_QueryMsg::Proposal { proposal_id: id })?
            }))?;

            //If the proposal was within the last hour, continue.
            //This blocks attempts of farming proposals since they r free.
            if proposal.start_time >= env.block.time.seconds() - SECONDS_PER_DAY/24 {
                continue;
            }

            //Check queried proposal & add to the unvoted proposals list if the user has voted
            let mut has_voted = false;
            //Check if user has voted
            if proposal.for_voters.contains(&info.clone().sender) 
            || proposal.against_voters.contains(&info.clone().sender) 
            || proposal.aligned_voters.contains(&info.clone().sender) 
            || proposal.removal_voters.contains(&info.clone().sender) 
            || proposal.amendment_voters.contains(&info.clone().sender) {
                has_voted = true;
            }
            if has_voted {
                newly_voted_proposals.push(id);
            }
        }
    }

    //Delete Claim Check
    CLAIM_CHECK.remove(deps.storage);

    //Allocate points
    allocate_points(
        deps.storage, 
        deps.querier, 
        config.clone(), 
        info.sender.clone(), 
        basket.clone().credit_price, 
        revenue_paid, 
        sp_claim_diff.clone(), 
        lq_claim_diff.clone(), 
        newly_voted_proposals.clone()
    )?;

    //Set attributes
    let mut attrs: Vec<Attribute> = vec![];
        attrs.push(attr("revenue_paid", revenue_paid));
        attrs.push(attr("sp_claim_diff", format!("{:?}", sp_claim_diff)));
        attrs.push(attr("lq_claim_diff", format!("{:?}", lq_claim_diff)));
        attrs.push(attr("newly_voted_proposals", format!("{:?}", newly_voted_proposals)));



    Ok(Response::new().add_attributes(attrs))
}

fn save_vault_conversion_rates(
    deps: DepsMut,
    info: MessageInfo,
) -> Result<Response, ContractError>{
    //Load vault info state for all vaults
    let vault_infos: Vec<VaultInfo> = VAULT_INFO.load(deps.storage)?;
    //Query the user's balances
    let user_balances = deps.querier.query_all_balances(&info.sender)?;
    //Search for any vault tokens
    let vault_tokens = user_balances.into_iter().filter(|x| vault_infos.iter().any(|vault| vault.vault_token_denom == x.denom )).collect::<Vec<Coin>>();

    //Get conversation rates for each found vault token
    let conversion_rates = vault_tokens.into_iter().map(|token| {
        //find token in vault infos
        let vault_info = vault_infos.iter().find(|x| x.vault_token_denom == token.denom).unwrap();
        //Query for conversion rate
        let conversion_rate: Uint128 = deps.querier.query::<Uint128>(&QueryRequest::Wasm(WasmQuery::Smart { 
            contract_addr: vault_info.vault_address.clone(), 
            msg: to_json_binary(&Vault_QueryMsg::VaultTokenUnderlying {  
                vault_token_amount: vault_info.single_vault_token
            })?
        }))?;
        //Return conversion rate object
        return Ok(VaultConversionRate {
            vault_address: vault_info.vault_address.clone(),
            last_conversion_rate: Uint128::zero(),
            total_vault_tokens: Asset {
                amount: token.amount,
                info: AssetInfo::NativeToken { denom: token.denom },
            },
        });
    }).collect::<StdResult<Vec<VaultConversionRate>>>()?;

    //Load user's existing conversion rates

    //Give points for any existing conversion rates.
    //We use the minimum of the user's vault tokens to calculate points. If they got more or exited some, it won't help their points.

    //Add vault conversion rates to the user.
    //Replace any existing, add otherwise.




    Ok(Response::new())
}

/// Liquidate a position
/// Send fees to the caller in the reply
/// 1) Liquidator gets points for the fee
/// 2) Liquidatee gets points for what was liquidated
fn liquidate_for_user(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,    
    position_id: Uint128,
    position_owner: String,
) -> Result<Response, ContractError>{
    //Load config
    let config: Config = CONFIG.load(deps.storage)?;
    //Verify address
    let position_owner = deps.api.addr_validate(&position_owner)?;

    //Create Liquidation message
    let liquidation_msg = CosmosMsg::Wasm(WasmMsg::Execute { 
        contract_addr: config.clone().positions_contract.to_string(), 
        msg: to_json_binary(&CDP_ExecuteMsg::Liquidate {
                position_id,
                position_owner: position_owner.to_string(),
            })?,
        funds: vec![] 
    });
    //Create submsg
    let liquidation_submsg = SubMsg::reply_on_success(liquidation_msg, LIQUIDATION_REPLY_ID);

    //Get CDP's outstanding credit supply
    let basket: Basket = deps.querier.query::<Basket>(&QueryRequest::Wasm(WasmQuery::Smart { 
        contract_addr: config.clone().positions_contract.to_string(), 
        msg: to_json_binary(&CDP_QueryMsg::GetBasket {  })?
    }))?;
    //Save balances
    LIQ_PROPAGATION.save(deps.storage, &
        LiquidationPropagation {
            pre_liq_CDT: basket.credit_asset.amount,
            liquidator: info.sender.clone(),
            liquidatee: position_owner.clone(),
        }
    )?;
    //We will use the difference in CREDIT SUPPLY to calculate how much was liquidated//
    //Then we check this contract's balances to find & send fees to the caller.


    Ok(Response::new().add_submessage(liquidation_submsg))
}

fn claim_mbrn_from_points(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError>{
    //Load config
    let mut config: Config = CONFIG.load(deps.storage)?;
    //Load user stats
    let user_stats = match USER_STATS.load(deps.storage, info.sender.clone()){
        Ok(stats) => stats,
        Err(_) => return Err(ContractError::Std(StdError::generic_err("User has no points in our system"))),
    };

    //Calculate MBRN to claim
    let mut mbrn_to_claim = match user_stats.claimable_points.checked_mul(config.clone().mbrn_per_point){
        Ok(amount) => amount.to_uint_ceil(),
        Err(_) => return Err(ContractError::Std(StdError::generic_err("No MBRN to claim"))),
    };

    if mbrn_to_claim == Uint128::zero() {
        return Err(ContractError::Std(StdError::generic_err("No MBRN to claim")));
    }

    //Assert MBRN to claim is less than the max distribution
    if mbrn_to_claim + config.clone().total_mbrn_distribution > config.clone().max_mbrn_distribution {
        
        //Set MBRN to claim as any remaining MBRN to reach the max distribution
        mbrn_to_claim = match config.clone().max_mbrn_distribution.checked_sub(config.clone().total_mbrn_distribution){
            Ok(amount) => amount,
            Err(_) => return Err(ContractError::Std(StdError::generic_err("Claimable MBRN exceeds the max distribution")))
        };
    }

    //Update total MBRN distribution
    config.total_mbrn_distribution += mbrn_to_claim;
    CONFIG.save(deps.storage, &config)?;

    //Reset user's claimable levels
    let mut updated_stats = user_stats.clone();
    updated_stats.claimable_points = Decimal::zero();

    //Mint MBRN to user
    let mbrn_mint: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.clone().osmosis_proxy_contract.to_string(),
        msg: to_json_binary(&OP_ExecuteMsg::MintTokens { 
            denom: String::from("factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/umbrn"), 
            amount: mbrn_to_claim, 
            mint_to_address: info.sender.clone().to_string(), 
        })?,
        funds: vec![],
    });

    //Save updated stats
    USER_STATS.save(deps.storage, info.sender.clone(), &updated_stats)?;
    
    //Set attributes
    let mut attrs: Vec<Attribute> = vec![];
        attrs.push(attr("mbrn_claimed", mbrn_to_claim));
        attrs.push(attr("claimed_points", user_stats.claimable_points.to_string()));

    Ok(Response::new()
    .add_attributes(attrs)
    .add_message(mbrn_mint))
}

fn allocate_points(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    config: Config,
    user: Addr,
    cdt_price: PriceResponse,
    revenue_paid: Uint128,
    sp_claim_diff: Vec<Coin>,
    lq_claim_diff: Vec<Coin>,
    newly_voted_proposals: Vec<u64>,
) -> StdResult<()> {
    //Concat the sp & lq claims
    let mut claim_diffs: Vec<Coin> = sp_claim_diff.clone();
    claim_diffs.extend(lq_claim_diff.clone());

    //Create a list of unique denoms from the sp & lq claims
    let mut unique_denoms: Vec<AssetInfo> = vec![];
    for coin in claim_diffs.clone(){
        if !unique_denoms.contains(&AssetInfo::NativeToken { denom: coin.denom.clone() }){
            unique_denoms.push(
                AssetInfo::NativeToken { denom: coin.denom.clone() }
            );
        }
    }

    //Get the current price of each unique denom
    let denoms_prices: Vec<PriceResponse> = querier.query::<Vec<PriceResponse>>(&QueryRequest::Wasm(WasmQuery::Smart { 
        contract_addr: config.clone().oracle_contract.to_string(), 
        msg: to_json_binary(&Oracle_QueryMsg::Prices { 
            asset_infos: unique_denoms.clone(),
            twap_timeframe: 60u64,
            oracle_time_limit: 600u64,
        })?
    })).unwrap_or(vec![]);
    //It'll error if there are no denoms passed most likely so we just return an empty vec

    
    //Sum the value of each claim
    let mut total_value: Decimal = Decimal::zero();
    for coin in claim_diffs.clone(){
        //Find index of the denom in unique denoms
        if let Some(index) = unique_denoms.iter().position(|x| x == &AssetInfo::NativeToken { denom: coin.denom.clone() }){
            //Add the value of the claim to the total value
            total_value += denoms_prices[index].get_value(coin.amount)?;
        }
    }
    
    //Add CDT revenue to the total value
    total_value += cdt_price.get_value(revenue_paid)?;

    //Add $1 for each proposal voted
    total_value += Decimal::from_ratio(newly_voted_proposals.len() as u64, 1u64);

    //Calculate points
    let points = decimal_multiplication(total_value, config.clone().points_per_dollar)?;

    //Save points to user
    let mut user_stats = match USER_STATS.load(storage, user.clone()){
        Ok(stats) => stats,
        Err(_) => UserStats {
            total_points: Decimal::zero(),
            claimable_points: Decimal::zero(),
        },
    };
    user_stats.total_points += points;
    user_stats.claimable_points += points;
    //Save updated stats
    USER_STATS.save(storage, user, &user_stats)?;

    Ok(())
}


/// Update contract configuration
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    cdt_denom: Option<String>,
    oracle_contract: Option<String>,
    positions_contract: Option<String>,
    stability_pool_contract: Option<String>,
    liq_queue_contract: Option<String>,
    governance_contract: Option<String>,
    osmosis_proxy_contract: Option<String>,
    mbrn_per_point: Option<Decimal>,
    max_mbrn_distribution: Option<Uint128>,
    points_per_dollar: Option<Decimal>,  
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    let mut attrs = vec![attr("method", "update_config")];

    //Assert Authority
    if info.sender != config.owner {
        //Check if ownership transfer is in progress & transfer if so
        if info.sender == OWNERSHIP_TRANSFER.load(deps.storage)? {
            config.owner = info.sender;
        } else {
            return Err(ContractError::Unauthorized {});
        }
    }

    //Save optionals
    if let Some(addr) = owner {
        let valid_addr = deps.api.addr_validate(&addr)?;

        //Set owner transfer state
        OWNERSHIP_TRANSFER.save(deps.storage, &valid_addr)?;
        attrs.push(attr("owner_transfer", valid_addr));  
    }
    if let Some(denom) = cdt_denom {
        config.cdt_denom = denom.clone();
        attrs.push(attr("cdt_denom", denom));
    }
    if let Some(addr) = oracle_contract {
        config.oracle_contract = deps.api.addr_validate(&addr)?;
        attrs.push(attr("oracle_contract", addr));
    }
    if let Some(addr) = positions_contract {
        config.positions_contract = deps.api.addr_validate(&addr)?;
        attrs.push(attr("positions_contract", addr));
    }
    if let Some(addr) = stability_pool_contract {
        config.stability_pool_contract = deps.api.addr_validate(&addr)?;
        attrs.push(attr("stability_pool_contract", addr));
    }
    if let Some(addr) = liq_queue_contract {
        config.liq_queue_contract = deps.api.addr_validate(&addr)?;
        attrs.push(attr("liq_queue_contract", addr));
    }
    if let Some(addr) = governance_contract {
        config.governance_contract = deps.api.addr_validate(&addr)?;
        attrs.push(attr("governance_contract", addr));
    }
    if let Some(addr) = osmosis_proxy_contract {
        config.osmosis_proxy_contract = deps.api.addr_validate(&addr)?;
        attrs.push(attr("osmosis_proxy_contract", addr));
    }
    if let Some(amount) = mbrn_per_point {
        config.mbrn_per_point = amount;
        attrs.push(attr("mbrn_per_point", amount.to_string()));
    }
    if let Some(amount) = max_mbrn_distribution {
        config.max_mbrn_distribution = amount;
        attrs.push(attr("max_mbrn_distribution", amount));
    }
    if let Some(amount) = points_per_dollar {
        config.points_per_dollar = amount;
        attrs.push(attr("points_per_dollar", amount.to_string()));
    }        

    //Save Config
    CONFIG.save(deps.storage, &config)?;
    attrs.push(attr("updated_config", format!("{:?}", config)));

    Ok(Response::new().add_attributes(attrs))
}



#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.id {
        LIQUIDATION_REPLY_ID => handle_liq_reply(deps, env, msg),
        ACCRUE_REPLY_ID => handle_accrue_reply(deps, env, msg),
        id => Err(StdError::generic_err(format!("invalid reply id: {}", id))),
    }
}

/// On success, sell for collateral composition, redeposit & call loop fn again.
/// Increment Loop number.
fn handle_liq_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response>{
    
    match msg.result.into_result() {
        Ok(_) => {
            //Load config
            let config: Config = CONFIG.load(deps.storage)?;
            //Query new CDT SUPPLY
            let basket: Basket = deps.querier.query::<Basket>(&QueryRequest::Wasm(WasmQuery::Smart { 
                contract_addr: config.clone().positions_contract.to_string(), 
                msg: to_json_binary(&CDP_QueryMsg::GetBasket {  })?
            }))?;
            let post_liq_CDT = basket.credit_asset.amount;

            //Load Liquidation Propagation
            let liquidation_propagation: LiquidationPropagation = LIQ_PROPAGATION.load(deps.storage)?;
            
            //Calculate liquidation amount
            let liquidated_amount = match liquidation_propagation.clone().pre_liq_CDT.checked_sub(post_liq_CDT.clone()){
                Ok(amount) => amount,
                Err(_) => Uint128::zero(),
            };

            //Query contract balances to find fees
            let balances = deps.querier.query_all_balances(env.contract.address.clone())?;            

            //Send fees to caller
            let fee_message: CosmosMsg = CosmosMsg::Bank(BankMsg::Send {
                to_address: liquidation_propagation.liquidator.to_string(),
                amount: balances.clone(),
            });

            //Empty Liquidation Propagation
            LIQ_PROPAGATION.remove(deps.storage);

            //Allocate points to the Vault owner who got liquidated
            allocate_points(
                deps.storage, 
                deps.querier, 
                config.clone(), 
                liquidation_propagation.clone().liquidatee, 
                basket.clone().credit_price, 
                liquidated_amount, 
                vec![],                 
                vec![],                 
                vec![]
            )?;

            //Allocate points to the Liquidation caller
            allocate_points(
                deps.storage, 
                deps.querier, 
                config.clone(), 
                liquidation_propagation.clone().liquidator, 
                basket.clone().credit_price, 
                Uint128::zero(), 
                balances.clone(),
                vec![],                 
                vec![]
            )?;
            
            Ok(Response::new()
                .add_message(fee_message)
                .add_attributes([
                    attr("pre_liquidated_CDT", liquidation_propagation.pre_liq_CDT),
                    attr("post_liquidated_CDT", post_liq_CDT),
                    attr("total_liquidated_CDT", liquidated_amount),
                    attr("fee_assets", format!("{:?}", balances)),
                    ])
            )

        },
        Err(string) => {            
            Ok(Response::new().add_attribute("we no error", string))
        }
    }
}

fn handle_accrue_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response>{
    
    match msg.result.into_result() {
        Ok(_) => {
            //Load config
            let config: Config = CONFIG.load(deps.storage)?;
            //Get CDP's pending revenue
            let basket: Basket = deps.querier.query::<Basket>(&QueryRequest::Wasm(WasmQuery::Smart { 
                contract_addr: config.clone().positions_contract.to_string(), 
                msg: to_json_binary(&CDP_QueryMsg::GetBasket {  })?
            }))?;
            let present_revenue = basket.pending_revenue;

            //Load Claim Check
            let mut claim_check: ClaimCheck = CLAIM_CHECK.load(deps.storage)?;
            claim_check.cdp_pending_revenue = present_revenue;
            CLAIM_CHECK.save(deps.storage, &claim_check)?;

            
            Ok(Response::new()
                .add_attribute("pending_revenue_post_accrual", present_revenue)
            )

        },
        Err(string) => {            
            Ok(Response::new().add_attribute("we no error", string))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::ClaimCheck {} => to_json_binary(&CLAIM_CHECK.load(deps.storage)?),
        QueryMsg::UserStats { user, limit, start_after } => to_json_binary(&query_user_stats(deps, user, limit, start_after)?),
    }
}

/// Return a list of users with their points
fn query_user_stats(
    deps: Deps,
    user: Option<String>,
    limit: Option<u64>, //User limit
    start_after: Option<String>, //user
) -> StdResult<Vec<UserStatsResponse>> {
    if let Some(user) = user {
        let user = deps.api.addr_validate(&user)?;
        let stats = USER_STATS.load(deps.storage, user.clone())?;
        return Ok(vec![
            UserStatsResponse {
            user,
            stats
        }]);
    };

    let limit = limit.unwrap_or(PAGINATION_DEFAULT_LIMIT) as usize;
    let start = if let Some(start) = start_after {
        let start_after_addr = deps.api.addr_validate(&start)?;
        Some(Bound::exclusive(start_after_addr))
    } else {
        None
    };

    let mut user_stats: Vec<UserStatsResponse> = vec![];
    for user in USER_STATS
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
    {
        let (user, stats) = user?;
        user_stats.push(
            UserStatsResponse {
            user,
            stats
        });
    }

    Ok(user_stats)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    
    Ok(Response::default())
}
