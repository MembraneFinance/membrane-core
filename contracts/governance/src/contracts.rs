//Fork of: https://github.com/astroport-fi/astroport-governance/tree/main/contracts/assembly
//Proposal Msg tutorial: https://blog.astroport.fi/post/tutorial-structuring-executable-messages-for-assembly-proposals-part-2-adding-proxy-contracts

use cosmwasm_std::{
    attr, entry_point, to_binary, Addr, Binary, CosmosMsg, Decimal, Deps, DepsMut, Env,
    MessageInfo, Order, QueryRequest, Response, StdResult, Uint128, Uint64, WasmMsg, WasmQuery, Storage, QuerierWrapper,
};
use osmosis_std::types::osmosis::incentives::{MsgCreateGauge, MsgAddToGauge};
use cw2::set_contract_version;
use cw_storage_plus::Bound;

use membrane::helpers::{query_staking_totals, asset_to_coin, query_basket};
use membrane::math::decimal_multiplication;
use membrane::types::{SupplyCap, AssetInfo, Asset, Basket, BidInput};
use membrane::vesting::{AllocationResponse, QueryMsg as VestingQueryMsg, RecipientsResponse};
use membrane::governance::helpers::validate_links;
use membrane::governance::{
    Config, ExecuteMsg, InstantiateMsg, Proposal, ProposalListResponse, ProposalMessage,
    ProposalResponse, ProposalStatus, ProposalVoteOption, ProposalVotesResponse, QueryMsg,
    UpdateConfig, BLOCKS_PER_DAY, MigrateMsg
};
use membrane::cdp::{ExecuteMsg as CDP_ExecuteMsg, EditBasket, QueryMsg as CDP_QueryMsg, Config as CDP_Config};
use membrane::staking::{
    Config as StakingConfig, DelegationResponse, ExecuteMsg as Staking_ExecuteMsg, QueryMsg as StakingQueryMsg, StakedResponse, StakerResponse, TotalStakedResponse
};
use membrane::liq_queue::{Config as LQ_Config, ExecuteMsg as LQ_ExecuteMsg, QueryMsg as LQ_QueryMsg, QueueResponse};
use membrane::osmosis_proxy::ExecuteMsg as OP_ExecuteMsg;

use core::panic;
use std::cmp::min;
use std::str::FromStr;

use crate::error::ContractError;
use crate::state::{CONFIG, PROPOSALS, PROPOSAL_COUNT, PENDING_PROPOSALS};

// Contract name and version used for migration.
const CONTRACT_NAME: &str = "mbrn-governance";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Default pagination constants
const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 30;
const DEFAULT_VOTERS_LIMIT: u32 = 100;
const MAX_VOTERS_LIMIT: u32 = 250;

//External query limit
const STAKE_QUERY_LIMIT: u32 = 1024;


#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let mbrn_denom = deps
        .querier
        .query::<StakingConfig>(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: deps
                .api
                .addr_validate(&msg.mbrn_staking_contract_addr)?
                .to_string(),
            msg: to_binary(&StakingQueryMsg::Config {})?,
        }))?
        .mbrn_denom;

    let config = Config {
        mbrn_denom,
        minimum_total_stake: Uint128::new(1_000_000_000_000),  //1M MBRN
        staking_contract_addr: deps.api.addr_validate(&msg.mbrn_staking_contract_addr)?,
        vesting_contract_addr: deps.api.addr_validate(&msg.vesting_contract_addr)?,
        vesting_voting_power_multiplier: msg.vesting_voting_power_multiplier,
        proposal_voting_period: msg.proposal_voting_period,
        expedited_proposal_voting_period: msg.expedited_proposal_voting_period,
        proposal_effective_delay: msg.proposal_effective_delay,
        proposal_expiration_period: msg.proposal_expiration_period,
        proposal_required_stake: msg.proposal_required_stake,
        proposal_required_quorum: Decimal::from_str(&msg.proposal_required_quorum)?,
        proposal_required_threshold: Decimal::from_str(&msg.proposal_required_threshold)?,
        whitelisted_links: msg.whitelisted_links,
        quadratic_voting: true,
    };

    config.validate()?;

    CONFIG.save(deps.storage, &config)?;

    PROPOSAL_COUNT.save(deps.storage, &Uint64::zero())?;

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
        ExecuteMsg::SubmitProposal {
            title,
            description,
            link,
            messages,
            recipient,
            expedited,
        } => submit_proposal(
            deps,
            env,
            info,
            title,
            description,
            link,
            messages,
            recipient,
            expedited,
        ),
        ExecuteMsg::CastVote {
            proposal_id,
            vote,
            recipient,
        } => cast_vote(deps, env, info, proposal_id, vote, recipient),
        ExecuteMsg::EndProposal { proposal_id } => end_proposal(deps, env, proposal_id),
        ExecuteMsg::ExecuteProposal { proposal_id } => execute_proposal(deps, env, proposal_id),
        ExecuteMsg::CheckMessages { messages, msg_switch } => check_messages(deps, env, messages, msg_switch),
        ExecuteMsg::CheckMessagesPassed { error } => passed_messages(deps, env, error),
        ExecuteMsg::RemoveCompletedProposal { proposal_id } => {
            remove_completed_proposal(deps, env, info, proposal_id)
        }
        ExecuteMsg::UpdateConfig(config) => update_config(deps, env, info, config),
        ExecuteMsg::CreateOsmosisGauge { gauge_msg } => create_gauge(info, env, gauge_msg),
        ExecuteMsg::AddToOsmosisGauge { gauge_msg } => add_to_gauge(info, env, gauge_msg),
        ExecuteMsg::FreezePositions { frozen, freeze_these_assets } => freeze_positions(info, env, frozen, freeze_these_assets),
    }
}

/// Submit a proposal to the governance contract. 
/// Total stake must surpass the minimum.
/// Only the vesting contract can submit expedited proposals.
pub fn submit_proposal(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    title: String,
    description: String,
    link: Option<String>,
    messages: Option<Vec<ProposalMessage>>,
    recipient: Option<String>,
    mut expedited: bool,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    //Assert minimum total stake from staking contract
    let non_vested_total: Uint128 = match query_staking_totals(deps.querier, config.staking_contract_addr.to_string()){
        Ok(totals) => totals.stakers,
        Err(_) => {
            //On error do regular query for totals
            let res: TotalStakedResponse = deps.querier.query_wasm_smart(
                config.clone().staking_contract_addr,
                &StakingQueryMsg::TotalStaked {  },
            )?;
            res.total_not_including_vested
        },
    };
    
    if non_vested_total < config.minimum_total_stake {
        return Err(ContractError::InsufficientTotalStake { minimum: config.minimum_total_stake.into() });
    }   
   
    //Validate voting power
    let voting_power = calc_voting_power(
        deps.storage,
        deps.querier,
        Some(non_vested_total),
        info.sender.to_string(),
        env.block.time.seconds(),
        &mut expedited,
        recipient.clone(),
        false, //No quadratic for proposal submissions
    )?;
    if voting_power.is_zero() {
        return Err(ContractError::NoVotingPower {});
    }
    
    // Update the proposal count
    let count = PROPOSAL_COUNT.update(deps.storage, |c| -> StdResult<_> {
        Ok(c.checked_add(Uint64::new(1))?)
    })?;

    let mut submitter: Option<Addr> = None;
    if let Some(recipient) = recipient.clone() {
        submitter = Some(deps.api.addr_validate(&recipient)?);
    }

    //Set end_block 
    let end_block: u64 = {
        //Config is all in blocks, the 'times 6' converts it to 6 seconds per block
        if expedited {
            env.block.time.seconds() + (config.expedited_proposal_voting_period * 6) 
        } else if messages.is_some() && config.proposal_voting_period <  (7 * BLOCKS_PER_DAY){ //Proposals with executables have to be at least 7 days
            env.block.time.seconds() + (7 * BLOCKS_PER_DAY * 6)
        } else {
            env.block.time.seconds() + (config.proposal_voting_period * 6) 
        }
    };

    let mut proposal = Proposal {
        voting_power: vec![],
        proposal_id: count,
        submitter: submitter.unwrap_or_else(|| info.sender.clone()),
        status: ProposalStatus::Active,
        aligned_power: voting_power,
        for_power: Uint128::zero(),
        against_power: Uint128::zero(),
        amendment_power: Uint128::zero(),
        removal_power: Uint128::zero(),
        aligned_voters: vec![info.sender.clone()],
        for_voters: Vec::new(),
        against_voters: Vec::new(),
        amendment_voters: Vec::new(),
        removal_voters: Vec::new(),
        start_block: env.block.height,
        start_time: env.block.time.seconds(),
        end_block,
        delayed_end_block: end_block //*6 is bc the config is in 6 sec block times but the end_block is now in seconds
            + (config.proposal_effective_delay * 6),
        expiration_block: end_block
            + (config.proposal_effective_delay * 6)
            + (config.proposal_expiration_period * 6),
        title,
        description,
        link,
        messages,
    };

    proposal.validate(config.whitelisted_links)?;

    if proposal.aligned_power >= config.proposal_required_stake && config.quadratic_voting {
        //Calc difference
        let mut difference = proposal.aligned_power.checked_sub(config.proposal_required_stake)?;
        //Square root it
        difference = Decimal::from_ratio(difference, Uint128::one()).sqrt().to_uint_floor();
        //Set aligned power to threshold
        proposal.aligned_power = config.proposal_required_stake;
        //Add difference to proposal
        proposal.aligned_power = proposal.aligned_power.checked_add(difference)?;
    }
    
    //If proposal has insufficient alignment, send to pending
    if proposal.aligned_power < config.proposal_required_stake {
        //Set end block to 1 day from now
        proposal.end_block = env.block.time.seconds() + (BLOCKS_PER_DAY * 6); //6 seconds per block
        PENDING_PROPOSALS.save(deps.storage, count.to_string(), &proposal)?;
    } else {
        PROPOSALS.save(deps.storage, count.to_string(), &proposal)?;
    }


    Ok(Response::new().add_attributes(vec![
        attr("action", "submit_proposal"),
        attr(
            "submitter",
            recipient.unwrap_or_else(|| info.sender.to_string()),
        ),
        attr("proposal_id", count),
        attr(
            "proposal_end_time",
            (proposal.end_block).to_string(),
        ),
    ]))
}

/// Cast a vote on an active proposal.
/// 
/// Warning: There is a chance that changing voting to non-quadratic with a large number of voting tokens could cause an overflow.
pub fn cast_vote(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    proposal_id: u64,
    vote_option: ProposalVoteOption,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    let mut pending = false;
    let mut saved = false;

    //Load proposal
    let mut proposal = match PROPOSALS.load(deps.storage, proposal_id.to_string()){
        Ok(proposal) => proposal,
        Err(_) => match PENDING_PROPOSALS.load(deps.storage, proposal_id.to_string()){
            Ok(proposal) => {
                pending = true;
                proposal
            },
            Err(err) => return Err(ContractError::Std(err)),
        }
    };

    //Can't vote on your own proposal
    if proposal.submitter == info.sender {
        return Err(ContractError::Unauthorized {});
    } else if let Some(recipient) = recipient.clone() {
        //validate recipient
        deps.api.addr_validate(&recipient)?;

        if proposal.submitter == recipient {
            return Err(ContractError::Unauthorized {});
        }
    }

    //Checking as if end_block is in seconds
    if env.block.time.seconds() > proposal.end_block {
        return Err(ContractError::VotingPeriodEnded {});
    }

    //Get voting power from Proposal struct
    let mut voting_power: Uint128 = calc_voting_power(
        deps.storage, 
        deps.querier,
        None,
        info.sender.to_string(), 
        proposal.start_time,
        &mut false, 
        recipient.clone(), 
        config.quadratic_voting,
    )?;
    if voting_power.is_zero() {
        return Err(ContractError::NoVotingPower {});
    }

    //Remove previous vote
    if let Some((vote, _)) = proposal.for_voters.clone().into_iter().enumerate().find(|(_, voter)| voter == &info.sender) {
        proposal.for_voters.remove(vote);
        proposal.for_power = proposal.for_power.checked_sub(voting_power)?;

    } else if let Some((vote, _)) = proposal.against_voters.clone().into_iter().enumerate().find(|(_, voter)| voter == &info.sender) {
        proposal.against_voters.remove(vote);
        proposal.against_power = proposal.against_power.checked_sub(voting_power)?;

    } else if let Some((vote, _)) = proposal.amendment_voters.clone().into_iter().enumerate().find(|(_, voter)| voter == &info.sender) {
        proposal.amendment_voters.remove(vote);
        proposal.amendment_power = proposal.amendment_power.checked_sub(voting_power)?;

    } else if let Some((vote, _)) = proposal.removal_voters.clone().into_iter().enumerate().find(|(_, voter)| voter == &info.sender) {
        proposal.removal_voters.remove(vote);
        proposal.removal_power = proposal.removal_power.checked_sub(voting_power)?;
    } else if let Some((vote, _)) = proposal.aligned_voters.clone().into_iter().enumerate().find(|(_, voter)| voter == &info.sender) {
        proposal.aligned_voters.remove(vote);
        proposal.aligned_power = proposal.aligned_power.checked_sub(voting_power)?;
    }

    match vote_option {
        ProposalVoteOption::For => {
            if pending {
                return Err(ContractError::ProposalNotActive {});
            }
            proposal.for_power = proposal.for_power.checked_add(voting_power)?;
            proposal.for_voters.push(info.sender.clone());
        }
        ProposalVoteOption::Against => {
            if pending {
                return Err(ContractError::ProposalNotActive {});
            }
            proposal.against_power = proposal.against_power.checked_add(voting_power)?;
            proposal.against_voters.push(info.sender.clone());
        }
        ProposalVoteOption::Amend => {
            if pending {
                return Err(ContractError::ProposalNotActive {});
            }
            proposal.amendment_power = proposal.amendment_power.checked_add(voting_power)?;
            proposal.amendment_voters.push(info.sender.clone());
        }
        ProposalVoteOption::Remove => {
            if pending {
                return Err(ContractError::ProposalNotActive {});
            }
            proposal.removal_power = proposal.removal_power.checked_add(voting_power)?;
            proposal.removal_voters.push(info.sender.clone());
        }
        ProposalVoteOption::Align => {
            //Remove quadratic voting for alignment if not reached yet
            if config.quadratic_voting && proposal.aligned_power < config.proposal_required_stake {
                //Square it                
                voting_power = 
                decimal_multiplication(
                    Decimal::from_ratio(voting_power, Uint128::one()), 
                    Decimal::from_ratio(voting_power, Uint128::one())
                )?.to_uint_ceil();                

                //Adding voting power to proposal
                proposal.aligned_power = proposal.aligned_power.checked_add(voting_power)?;            
                //Add voter to aligned voters
                proposal.aligned_voters.push(info.sender.clone());

                //If this addition pushes the proposal over the threshold, square root the difference & add to aligned_power.
                ///
                //Aligned power must be subject to the config's quadratic voting setting past the threshold
                //or reaching quorum becomes trival when quadratic voting is enabled
                if proposal.aligned_power >= config.proposal_required_stake {
                    //Calc difference
                    let mut difference = proposal.aligned_power.checked_sub(config.proposal_required_stake)?;
                    //Square root it
                    difference = Decimal::from_ratio(difference, Uint128::one()).sqrt().to_uint_floor();
                    //Set aligned power to threshold
                    proposal.aligned_power = config.proposal_required_stake;
                    //Add difference to proposal
                    proposal.aligned_power = proposal.aligned_power.checked_add(difference)?;
                }
            } else 
            //If quadratic voting is disabled or the threshold has been reached, add voting power to proposal
            {
                //Adding voting power to proposal
                proposal.aligned_power = proposal.aligned_power.checked_add(voting_power)?;            
                //Add voter to aligned voters
                proposal.aligned_voters.push(info.sender.clone());
            }
            //If alignment is reached, move to active proposal state
            if proposal.aligned_power >= config.proposal_required_stake {

                saved = true;
                //Remove from pending proposals
                PENDING_PROPOSALS.remove(deps.storage, proposal_id.to_string());
                //Add to active proposals
                PROPOSALS.save(deps.storage, proposal_id.to_string(), &proposal)?;
            }
        }
    };

    //Save proposal
    if !saved {
        if !pending {
            PROPOSALS.save(deps.storage, proposal_id.to_string(), &proposal)?;
        } else {
            PENDING_PROPOSALS.save(deps.storage, proposal_id.to_string(), &proposal)?;
        }   
    }

    Ok(Response::new().add_attributes(vec![
        attr("action", "cast_vote"),
        attr("proposal_id", proposal_id.to_string()),
        attr("voter", recipient.unwrap_or_else(|| info.sender.to_string())),
        attr("vote", vote_option.to_string()),
        attr("voting_power", voting_power),
    ]))
}

/// End a proposal and determine the result.
pub fn end_proposal(deps: DepsMut, env: Env, proposal_id: u64) -> Result<Response, ContractError> {
    let mut proposal = PROPOSALS.load(deps.storage, proposal_id.to_string())?;

    if proposal.status != ProposalStatus::Active {
        return Err(ContractError::ProposalNotActive {});
    }
    
    //Checking as if end_block is in seconds
    if env.block.time.seconds() <= proposal.end_block {
        return Err(ContractError::VotingPeriodNotEnded {});
    }

    let mut config = CONFIG.load(deps.storage)?;

    let for_votes = proposal.for_power;
    let against_votes = proposal.against_power;
    let amend_votes = proposal.amendment_power;
    let removal_votes = proposal.removal_power;

    let total_votes = for_votes + against_votes + amend_votes + removal_votes;

    let total_voting_power =
        match calc_total_voting_power_at(
            deps.as_ref(), 
            config.quadratic_voting,
            proposal.start_time,
        ){
            Ok(voting_power) => voting_power,
            Err(_) => Uint128::one(), //if cast_vote works but this doesn't, assume 1 so we can still end the proposal
        
        };

    let mut proposal_quorum: Decimal = Decimal::zero();
    let mut for_threshold: Decimal = Decimal::zero();
    let mut amend_threshold: Decimal = Decimal::zero();
    let mut removal_threshold: Decimal = Decimal::zero();

    if !total_voting_power.is_zero() {
        if config.quadratic_voting {
            //Subtract the non-quadradic voting power from the alignment threshold
            proposal.aligned_power = proposal.aligned_power.checked_sub(config.proposal_required_stake)?;
            //Square root the alignment threshold & add to aligned power
            proposal.aligned_power += Decimal::from_ratio(config.proposal_required_stake, Uint128::one()).sqrt().to_uint_floor();
        }
        //Calc proposal quorum
        proposal_quorum = Decimal::from_ratio(total_votes+proposal.aligned_power, total_voting_power);
        //If aligned_power isn't added, proposals made by large holders can potentially never reach quorum
    }

    if !total_votes.is_zero() {
        for_threshold = Decimal::from_ratio(for_votes, total_votes);
        amend_threshold = Decimal::from_ratio(for_votes + amend_votes, total_votes);
        removal_threshold = Decimal::from_ratio(removal_votes, total_votes);

        //Set config.proposal_required_threshold to 50 if the proposal has no executables
        if proposal.messages.is_none() || proposal.messages.clone().unwrap().is_empty() {
            config.proposal_required_threshold = Decimal::percent(50);
        }
    }
    
    let mut removed = false;
    // Determine the proposal result
    proposal.status = if proposal_quorum >= config.proposal_required_quorum {
        //For check
        if for_threshold >= config.proposal_required_threshold {
            ProposalStatus::Passed
        } //Amend check
        else if amend_threshold >= config.proposal_required_threshold {
            ProposalStatus::AmendmentDesired
        } // Removal check
        else if removal_threshold >= config.proposal_required_threshold {            
            //Remove from state
            PROPOSALS.remove(deps.storage, proposal_id.to_string());
            removed = true;

            ProposalStatus::Rejected
        } else {
            ProposalStatus::Rejected
        }
    } //If didn't hit quorum & is expedited, extend end block to the normal voting period
    else if proposal.end_block - proposal.start_time == (config.expedited_proposal_voting_period * 6) { //6 seconds per block
        proposal.end_block = proposal.start_time + (config.proposal_voting_period * 6); //6 seconds per block
        
        //Reverse actions from line 492
        if config.quadratic_voting {
            //ADd the non-quadradic voting power from the alignment threshold
            proposal.aligned_power = proposal.aligned_power.checked_add(config.proposal_required_stake)?;
            //Square root the alignment threshold & subtract from aligned power
            proposal.aligned_power -= Decimal::from_ratio(config.proposal_required_stake, Uint128::one()).sqrt().to_uint_floor();
        }
        
        ProposalStatus::Active
    } else {
        ProposalStatus::Rejected
    };


    //Update proposal if still in state
    if !removed {
        PROPOSALS.save(deps.storage, proposal_id.to_string(), &proposal)?;    
    }

    let response = Response::new().add_attributes(vec![
        attr("action", "end_proposal"),
        attr("proposal_id", proposal_id.to_string()),
        attr("proposal_result", proposal.status.to_string()),
    ]);

    Ok(response)
}

/// Execute Proposal Msgs
pub fn execute_proposal(
    deps: DepsMut,
    env: Env,
    proposal_id: u64,
) -> Result<Response, ContractError> {
    let mut proposal = PROPOSALS.load(deps.storage, proposal_id.to_string())?;

    if proposal.status != ProposalStatus::Passed {
        return Err(ContractError::ProposalNotPassed {});
    }

    if env.block.time.seconds() < proposal.delayed_end_block { //Checking as if end_block is in seconds
        return Err(ContractError::ProposalDelayNotEnded {});
    }

    if env.block.time.seconds() > proposal.expiration_block { //Checking as if block is in seconds
        return Err(ContractError::ExecuteProposalExpired {});
    }

    proposal.status = ProposalStatus::Executed;

    PROPOSALS.save(deps.storage, proposal_id.to_string(), &proposal)?;

    let mut messages = match proposal.messages {
        Some(mut messages) => {
            messages.sort_by(|a, b| a.order.cmp(&b.order));
            messages.into_iter().map(|message| message.msg).collect()
        }
        None => vec![],
    };
    
    //Use this msg to test proposability before committing to the chain
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_binary(&ExecuteMsg::CheckMessagesPassed { error: Some(false) })?,
        funds: vec![],
    }));

    Ok(Response::new()
        .add_attribute("action", "execute_proposal")
        .add_attribute("proposal_id", proposal_id.to_string())
        .add_messages(messages))
}

/// Checks that proposal messages are correct.
/// Returns [`ContractError`] on failure, otherwise returns a [`Response`] with the specified
/// attributes. The last message will always fail to prevent committing into blockchain.
pub fn check_messages(
    deps: DepsMut,
    env: Env,
    mut messages: Vec<ProposalMessage>,
    // Additional contract check messages
    msg_switch: Option<u64>,
) -> Result<Response, ContractError> {

    messages.sort_by(|a, b| a.order.cmp(&b.order));

    let mut messages: Vec<_> = messages.into_iter().map(|message| message.msg).collect();

    //Create additional check messages for system contracts\
    //Set contracts
    let cdp_contract = "osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr".to_string();
    let lq_contract = "osmo1ycmtfa7h0efexjxuaw7yh3h3qayy5lspt9q4n4e3stn06cdcgm8s50zmjl".to_string();
    let op_contract = "osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd".to_string();
    //CDP
    if msg_switch == Some(0) {
        //Query CDP basket
        let basket = match query_basket(deps.querier, cdp_contract.clone()){
            Ok(basket) => basket,
            Err(_) => 
                deps.querier.query_wasm_smart::<Basket>(
                cdp_contract.clone(),
                &CDP_QueryMsg::GetBasket {}
                )?,
            };    
        //Check if the contract has uosmo && deposit it all if it does
        let uosmo_balance = match deps.querier.query_balance(env.clone().contract.address, "uosmo"){
            Ok(balance) => balance.amount,
            Err(_) => Uint128::zero(),
        };
        if !uosmo_balance.is_zero() {
            //Get the next position id
            let position_id = basket.current_position_id;
            //Deposit all uosmo
            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cdp_contract.clone(),
                msg: to_binary(&CDP_ExecuteMsg::Deposit {
                    position_id: None,
                    position_owner: None,
                })?,
                funds: vec![
                    asset_to_coin(
                        Asset {
                            info: AssetInfo::NativeToken {
                                denom: String::from("uosmo"),
                            },
                            amount: uosmo_balance,
                        }
                    )?
                ],
            }));
            //Query CDP Config
            let cdp_config = deps.querier.query_wasm_smart::<CDP_Config>(
                cdp_contract.clone(),
                &CDP_QueryMsg::Config {}
            )?;


            //Mint minimum CDT, this may error if there isn't enough OSMO in the contract
            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cdp_contract.clone(),
                msg: to_binary(&CDP_ExecuteMsg::IncreaseDebt { 
                    position_id, 
                    amount: Some(cdp_config.debt_minimum * Uint128::new(1_000_000)), 
                    LTV: None, 
                    mint_to_addr: None,
                    mint_intent: None
                })?,
                funds: vec![],
            }));

            //Repay CDT
            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cdp_contract.clone(),
                msg: to_binary(&CDP_ExecuteMsg::Repay { 
                    position_id, 
                    position_owner: None, 
                    send_excess_to: None
                })?,
                funds: vec![
                    asset_to_coin(
                        Asset {
                            info: basket.credit_asset.info,
                            amount:( cdp_config.debt_minimum * Uint128::new(1_000_000)),
                        }
                    )?
                ],
            }));

            //Withdraw all OSMO
            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cdp_contract.clone(),
                msg: to_binary(&CDP_ExecuteMsg::Withdraw { 
                    position_id, 
                    assets: vec![
                        Asset {
                            info: AssetInfo::NativeToken {
                                denom: String::from("uosmo"),
                            },
                            amount: uosmo_balance,
                        }
                    ],
                    send_to: None
                })?,
                funds: vec![],
            }));
        }        
    }
    //Staking
    if msg_switch == Some(1) {
        let config = CONFIG.load(deps.storage)?;
        //Set minimum stake
        let minimum_stake = Uint128::new(1_000_000); //1 MBRN

        //Unstake MBRN if the contract already has stake
        //Unstake the MBRN (claimable)
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.clone().staking_contract_addr.to_string(),
            msg: to_binary(&Staking_ExecuteMsg::Unstake { 
                mbrn_amount: None,
            })?,
            funds: vec![],
        }));

        //Stake minimum MBRN from the unstake
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.clone().staking_contract_addr.to_string(),
            msg: to_binary(&Staking_ExecuteMsg::Stake { 
                user: None,
            })?,
            funds: vec![
                asset_to_coin(
                    Asset {
                        info: AssetInfo::NativeToken {
                            denom: config.clone().mbrn_denom,
                        },
                        amount: minimum_stake,
                    }
                )?
            ],
        }));

        //Start the unstake for the recently staked MBRN
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.clone().staking_contract_addr.to_string(),
            msg: to_binary(&Staking_ExecuteMsg::Unstake { 
                mbrn_amount: None,
            })?,
            funds: vec![],
        }));
    }
    //LQ    
    if msg_switch == Some(2) {
        //Query CDP basket
        let basket = match query_basket(deps.querier, cdp_contract.clone()){
            Ok(basket) => basket,
            Err(_) => 
                deps.querier.query_wasm_smart::<Basket>(
                cdp_contract.clone(),
                &CDP_QueryMsg::GetBasket {}
                )?,
            };    
        //Check if the contract has cdt
        let cdt_balance = match deps.querier.query_balance(env.clone().contract.address, basket.clone().credit_asset.info.to_string()){
            Ok(balance) => balance.amount,
            Err(_) => Uint128::zero(),
        };
        //Query LQ config 
        let lq_config: LQ_Config = deps.querier.query_wasm_smart::<LQ_Config>(
            lq_contract.clone(),
            &LQ_QueryMsg::Config {}
        )?;
        //if CDT is less than the minimum, mint minimum CDT
        if cdt_balance < lq_config.clone().minimum_bid {
            //Mint minimum CDT from Osmosis Proxy
            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: op_contract.clone(),
                msg: to_binary(&OP_ExecuteMsg::MintTokens { 
                    denom: basket.clone().credit_asset.info.to_string(), 
                    amount: lq_config.clone().minimum_bid, 
                    mint_to_address: env.clone().contract.address.to_string() 
                })?,
                funds: vec![],
            }));
        }

        //Query Queue to get the bid_id
        let uosmo_queue: QueueResponse = deps.querier.query_wasm_smart::<QueueResponse>(
            lq_contract.clone(),
            &LQ_QueryMsg::Queue { bid_for: AssetInfo::NativeToken { denom: String::from("uosmo") } 
            }
        )?;
        let bid_id = uosmo_queue.current_bid_id;

        //Minimum CDT Bid
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: lq_contract.clone(),
            msg: to_binary(&LQ_ExecuteMsg::SubmitBid { 
                bid_input: BidInput {
                    bid_for: AssetInfo::NativeToken { denom: String::from("uosmo") },
                    liq_premium: 0u8
                },
                bid_owner: None,
            })?,
            funds: vec![
                asset_to_coin(
                    Asset {
                        info: basket.credit_asset.info,
                        amount: lq_config.clone().minimum_bid,
                    }
                )?
            ],
        }));

        //Retract bid
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: lq_contract.clone(),
            msg: to_binary(&LQ_ExecuteMsg::RetractBid { 
                bid_id,
                bid_for: AssetInfo::NativeToken { denom: String::from("uosmo") },
                amount: None,
            })?,
            funds: vec![],
        }));        
    }

    //Guarantee that the last message will fail
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_binary(&ExecuteMsg::CheckMessagesPassed { error: Some(true) })?,
        funds: vec![],
    }));

    Ok(Response::new()
        .add_attribute("action", "check_messages")
        .add_messages(messages))
}

///Errors to prevent checked messages from being executed
/// Tests staking queries necessary for proposal execution
pub fn passed_messages(deps: DepsMut, env: Env, error: Option<bool>) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    //Assert minimum total stake from staking contract
    match query_staking_totals(deps.querier, config.staking_contract_addr.to_string()){
        Ok(totals) => totals.stakers,
        Err(_) => {
            //On error do regular query for totals
            let res: TotalStakedResponse = deps.querier.query_wasm_smart(
                config.clone().staking_contract_addr,
                &StakingQueryMsg::TotalStaked {  },
            )?;
            res.total_not_including_vested
        },
    };

    //Query stake from before Proposal's start_time
    deps.querier
        .query::<StakerResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: config.staking_contract_addr.to_string(),
            msg: to_binary(&StakingQueryMsg::UserStake { staker: env.contract.address.to_string() })?,
        }))?;
    
    deps.querier
        .query::<StakedResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: config.staking_contract_addr.to_string(),
            msg: to_binary(&StakingQueryMsg::Staked {
                limit: Some(STAKE_QUERY_LIMIT),
                start_after: None,
                end_before: Some(env.block.time.seconds()),
                unstaking: false,
            })?,
        }))?
        .stakers;

    //Query a vesting recipient
    deps.querier.query::<AllocationResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: config.vesting_contract_addr.to_string(),
        msg: to_binary(&VestingQueryMsg::Allocation { recipient: String::from("osmo1988s5h45qwkaqch8km4ceagw2e08vdw28mwk4n") })?,
    }))?;

    //Query the contracts config to ensure we don't migrate to a code_id that is not a Gov contract
    deps.querier.query::<Config>(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: env.contract.address.to_string(),
        msg: to_binary(&QueryMsg::Config {})?,
    }))?;

    //Query delegations, this is last incase this addr ever runs out of delegations & that causes the error
    deps.querier.query::<Vec<DelegationResponse>>(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: config.staking_contract_addr.to_string(),
        msg: to_binary(&StakingQueryMsg::Delegations {
            limit: Some(1),
            start_after: None,
            end_before: None,
            user: Some("osmo13gu58hzw3e9aqpj25h67m7snwcjuccd7v4p55w".to_string()),
        })?,
    }))?;

    if let Some(true) = error {
        return Ok(Response::new().add_message(CosmosMsg::Wasm(WasmMsg::ClearAdmin { contract_addr: env.contract.address.to_string() })))
    } else {
        return Ok(Response::new())
    }
}

/// Remove completed Proposals
pub fn remove_completed_proposal(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    proposal_id: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut aligned = false;

    //Load proposal
    let mut proposal = match PROPOSALS.load(deps.storage, proposal_id.to_string()){
        Ok(proposal) => {
            aligned = true;
            proposal
        },
        Err(_) => match PENDING_PROPOSALS.load(deps.storage, proposal_id.to_string()){
            Ok(proposal) => proposal,
            Err(err) => return Err(ContractError::Std(err)),
        }
    };

    if aligned {
        
        if env.block.time.seconds()
        > (proposal.end_block + ((config.proposal_effective_delay + config.proposal_expiration_period) * 6))
        {
            proposal.status = ProposalStatus::Expired;
        }
    } //If pending, expiration starts at end_block
    else {
        if env.block.time.seconds() > proposal.end_block {
            proposal.status = ProposalStatus::Expired;
        }
    }

    //If pending proposal is expired, remove
    if proposal.status == ProposalStatus::Expired && !aligned{
        PENDING_PROPOSALS.remove(deps.storage, proposal_id.to_string());    
    }
    //If proposal is expired or rejected, remove
    else if proposal.status == ProposalStatus::Expired || proposal.status == ProposalStatus::Rejected {
        PROPOSALS.remove(deps.storage, proposal_id.to_string());
    } else if info.sender == proposal.submitter {
        PROPOSALS.remove(deps.storage, proposal_id.to_string());
    } else {
        return Err(ContractError::CantRemove {});
    }
    

    Ok(Response::new()
        .add_attribute("action", "remove_completed_proposal")
        .add_attribute("proposal_id", proposal_id.to_string()))
}

/// Update the contract configuration
pub fn update_config(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    updated_config: UpdateConfig,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    // Only the Governance contract is allowed to update its own parameters (through a successful proposal)
    if info.sender != env.contract.address {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(mbrn_denom) = updated_config.mbrn_denom {
        config.mbrn_denom = mbrn_denom;
    }
    if let Some(minimum_total_stake) = updated_config.minimum_total_stake {
        config.minimum_total_stake = minimum_total_stake;
    }
    if let Some(staking_contract) = updated_config.staking_contract {
        config.staking_contract_addr = deps.api.addr_validate(&staking_contract)?;
    }
    if let Some(vesting_contract_addr) = updated_config.vesting_contract_addr {
        config.vesting_contract_addr = deps.api.addr_validate(&vesting_contract_addr)?;
    }
    if let Some(proposal_voting_period) = updated_config.proposal_voting_period {
        config.proposal_voting_period = proposal_voting_period;
    }
    if let Some(expedited_proposal_voting_period) = updated_config.expedited_proposal_voting_period {
        config.expedited_proposal_voting_period = expedited_proposal_voting_period;
    }
    if let Some(proposal_effective_delay) = updated_config.proposal_effective_delay {
        config.proposal_effective_delay = proposal_effective_delay;
    }
    if let Some(proposal_expiration_period) = updated_config.proposal_expiration_period {
        config.proposal_expiration_period = proposal_expiration_period;
    }
    if let Some(proposal_required_stake) = updated_config.proposal_required_stake {
        config.proposal_required_stake = Uint128::from(proposal_required_stake);
    }
    if let Some(proposal_required_quorum) = updated_config.proposal_required_quorum {
        config.proposal_required_quorum = Decimal::from_str(&proposal_required_quorum)?;
    }
    if let Some(proposal_required_threshold) = updated_config.proposal_required_threshold {
        config.proposal_required_threshold = Decimal::from_str(&proposal_required_threshold)?;
    }

    if let Some(whitelist_add) = updated_config.whitelist_add {
        validate_links(&whitelist_add)?;

        config.whitelisted_links.append(
            &mut whitelist_add
                .into_iter()
                .filter(|link| !config.whitelisted_links.contains(link))
                .collect(),
        );
    }

    if let Some(whitelist_remove) = updated_config.whitelist_remove {
        config.whitelisted_links = config
            .whitelisted_links
            .into_iter()
            .filter(|link| !whitelist_remove.contains(link))
            .collect();

        if config.whitelisted_links.is_empty() {
            return Err(ContractError::WhitelistEmpty {});
        }
    }

    config.validate()?;

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attribute("action", "update_config"))
}

/// Calc total voting power at a specific time
pub fn calc_total_voting_power_at(
    deps: Deps,
    quadratic_voting: bool,
    proposal_start_time: u64,
) -> StdResult<Uint128> {
    let config = CONFIG.load(deps.storage)?;
    //Initialize total voting power
    let mut total: Uint128 = Uint128::zero();

    //Query stake from before Proposal's start_time
    let mut staked_mbrn = match deps.querier
        .query::<StakedResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: config.staking_contract_addr.to_string(),
            msg: to_binary(&StakingQueryMsg::Staked {
                limit: Some(STAKE_QUERY_LIMIT),
                start_after: None,
                end_before: Some(proposal_start_time),
                unstaking: false,
            })?,
        })){
            Ok(staked) => staked.stakers,
            Err(_) => vec![],
        };
        
    //This will provide the lowest total voting power if quadratic voting is enabled
    //bc the stake can be split into delegations which are individually square rooted
    for staker in staked_mbrn.clone() {
        let mut staker_total = Uint128::zero();
        let mut remove_list = vec![];

        for (i, stake) in staked_mbrn.clone().into_iter().enumerate(){
            if stake.staker == staker.staker {
                //Remove to shorten subsequent iterations
                remove_list.push(i);

                if stake.stake_time < proposal_start_time && stake.unstake_start_time.is_none() {
                    staker_total += stake.amount;
                }
            }
        };
        //Reverse remove_list so we don't break the index ordering
        remove_list.reverse();
        //Remove staker from list
        for i in remove_list {
            staked_mbrn.remove(i);
        }
        //Transform w/ quadratics if enabled
        if quadratic_voting {
            staker_total = Decimal::from_ratio(staker_total, Uint128::one()).sqrt().to_uint_ceil();
        }
        //Add to total
        total += staker_total;

    }


    /////Get vested vp/////   
    //Set staked non-vested total
    let non_vested_total: Uint128 = match query_staking_totals(deps.querier, config.staking_contract_addr.to_string()){
        Ok(totals) => totals.stakers,
        Err(_) => {
            //On error do regular query for totals
            match deps.querier.query_wasm_smart::<TotalStakedResponse>(
                config.clone().staking_contract_addr,
                &StakingQueryMsg::TotalStaked {  },
            ){
                Ok(totals) => totals.total_not_including_vested,
                Err(_) => Uint128::zero()
            }                
        },
    };    

    //Get Vesting Recipients
    let recipients = deps
        .querier
        .query::<RecipientsResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: config.vesting_contract_addr.to_string(),
            msg: to_binary(&VestingQueryMsg::Recipients {  })?,
        }))?;
    
    //Add voting power of each vesting recipient
    for recipient in recipients.recipients {
        if let Some(allocation) = recipient.allocation {
            let allocation = (allocation.amount - allocation.amount_withdrawn) * config.vesting_voting_power_multiplier;
            // Vested voting power can't be more than 19% of total voting power pre-quadratic 
            let mut vp = min(allocation, non_vested_total * Decimal::percent(19));

            // Take square root of total stake if quadratic voting is enabled
            if quadratic_voting {
                vp = Decimal::from_ratio(vp, Uint128::one()).sqrt().to_uint_floor();
            }

            total += vp;
        }            
    }
    
    Ok(total)
}

/// Calc voting power for sender at a Proposal's start_time
pub fn calc_voting_power(
    storage: &dyn Storage,
    querier: QuerierWrapper,
    mut non_vested_total: Option<Uint128>,
    sender: String,
    start_time: u64,
    expedited: &mut bool,
    recipient: Option<String>,
    quadratic_voting: bool,
) -> StdResult<Uint128> {
    let config = CONFIG.load(storage)?;
    if non_vested_total.is_none(){
        let new_non_vested_total: Uint128 = match query_staking_totals(querier, config.staking_contract_addr.to_string()){
            Ok(totals) => totals.stakers,
            Err(_) => {
                //On error do regular query for totals
                let res: TotalStakedResponse = querier.query_wasm_smart(
                    config.clone().staking_contract_addr,
                    &StakingQueryMsg::TotalStaked {  },
                )?;
                res.total_not_including_vested
            },
        };

        non_vested_total = Some(new_non_vested_total);
    }
      
    //Query stake from before Proposal's start_time
    let mut total: Uint128 = match querier
    .query::<StakerResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: config.staking_contract_addr.to_string(),
        msg: to_binary(&StakingQueryMsg::UserStake { staker: sender.clone() })?,
    })){
        Ok(stake) => {
            //Filter out unstaking stake & stake after proposal start time
            let voting_stake = stake.clone().deposit_list.into_iter().filter(|stake| {
                stake.stake_time < start_time && stake.unstake_start_time.is_none()
            }).map(|stake| stake.amount).sum();

            voting_stake
        },
        Err(_) => Uint128::zero(),
    };
    
    //If calculating vesting voting power, we take from recipient's allocation
    if recipient.is_none() {
        //Only vesting recipients can submit expedited proposals
        *expedited = false;

    } else if recipient.is_some() {
        let recipient = recipient.clone().unwrap();
        //info.sender must equal the recipient contract
        if sender != recipient {
            return Err(cosmwasm_std::StdError::GenericErr { msg: String::from("You are not the Recipient that was passed") });
        }

        match querier
            .query::<AllocationResponse>(&QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: config.vesting_contract_addr.to_string(),
                msg: to_binary(&VestingQueryMsg::Allocation { recipient })?,
            })){
                Ok(allocation) => {  
                    total = (allocation.amount - allocation.amount_withdrawn) * config.vesting_voting_power_multiplier;            
                    // Vested voting power can't be more than 19% of total voting power pre-quadratic
                    total = min(total, non_vested_total.unwrap() * Decimal::percent(19));
                },
                Err(_) => {  
                    //Only vesting recipients can submit expedited proposals
                    *expedited = false;
                    total = Uint128::zero();
                }
            };
            
    } else {
        total = Uint128::zero();
        *expedited = false;
    }    
    
    // Take square root of total stake if quadratic voting is enabled
    //prior to adding delegated stake
    if quadratic_voting {        
        total = Decimal::from_ratio(total, Uint128::one()).sqrt().to_uint_ceil();
    }

    // Query delegations 
    let delegations: Vec<DelegationResponse> = match querier.query::<Vec<DelegationResponse>>(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: config.staking_contract_addr.to_string(),
        msg: to_binary(&StakingQueryMsg::Delegations {
            limit: None,
            start_after: None,
            end_before: Some(start_time),
            user: Some(sender.clone().to_string()),
        })?,
    })){
        Ok(delegations) => delegations,
        Err(_) => vec![], //If no delegations, set to empty vec
    };
    
    //Get user's delegation info
    //We transform the vp wrt the config's quadratic voting setting individually for each delegation.
    //This ensures the benefits of delegations: better voting power participation, easier quorum, voluntarily abstracted governance for users.
    //Otherwise delegates would get exponentially less voting power than what was delegated to them which makes quorum harder to reach with delegations vs w/o.
    match delegations.into_iter().find(|delegation| delegation.user.to_string() == sender){
        Some(delegation_info) => {
            //Get total delegated to user from before proposal start time
            let total_delegated_to_user: Uint128 = delegation_info.delegation_info.clone().delegated
                .into_iter()
                .filter(|delegation| delegation.time_of_delegation <= start_time && delegation.voting_power_delegation)
                .map(|dele| {
                    if quadratic_voting {
                        Decimal::from_ratio(dele.amount, Uint128::one()).sqrt().to_uint_ceil()
                    } else {
                        dele.amount
                    }
                })
                .sum();

            //Get total delegated away from user from before proposal start time
            let total_delegated_from_user: Uint128 = delegation_info.delegation_info.clone().delegated_to
                .into_iter()
                .filter(|delegation| delegation.time_of_delegation <= start_time && delegation.voting_power_delegation)
                .map(|dele| {
                    if quadratic_voting {
                        Decimal::from_ratio(dele.amount, Uint128::one()).sqrt().to_uint_ceil()
                    } else {
                        dele.amount
                    }
                })
                .sum();
            //Add delegated to user and subtract delegated from user
            total += total_delegated_to_user;
            total = match total.checked_sub(total_delegated_from_user){
                Ok(total) => total,
                Err(_) => Uint128::zero(),
            };
        },
        None => {}
    }
    
    Ok(total)
}

/// Create Osmosis Incentive Gauge.
/// Uses osmosis-std to make it easier for Governance to execute osmosis messages.
fn create_gauge(
    info: MessageInfo,
    env: Env,
    gauge_msg: MsgCreateGauge,
) -> Result<Response, ContractError>{

    // Only the Governance contract is allowed to utilize its assets (through a successful proposal)
    if info.sender != env.contract.address {
        return Err(ContractError::Unauthorized {});
    }
    Ok(Response::new().add_message(gauge_msg))
}

/// Add to Osmosis Incentive Gauge.
/// Uses osmosis-std to make it easier for Governance to execute osmosis messages.
fn add_to_gauge(
    info: MessageInfo,
    env: Env,
    gauge_msg: MsgAddToGauge,
) -> Result<Response, ContractError>{

    // Only the Governance contract is allowed to utilize its assets (through a successful proposal)
    if info.sender != env.contract.address {
        return Err(ContractError::Unauthorized {});
    }
    Ok(Response::new().add_message(gauge_msg))
}

//Freeze Positions
fn freeze_positions(
    info: MessageInfo,
    _env: Env,
    //toggle frozen Basket 
    frozen: bool,
    //Set supply caps to 0
    //We only allow native tokens so we can take Strings here
    freeze_these_assets: Vec<String>,
) -> Result<Response, ContractError>{
    // Only the founder addr is allowed
    if info.sender != Addr::unchecked("osmo1988s5h45qwkaqch8km4ceagw2e08vdw28mwk4n") {
        return Err(ContractError::Unauthorized {});
    }

    //Create Supply cap instance for each "freeze_these_assets" input
    let supply_caps: Vec<SupplyCap> = freeze_these_assets.into_iter().map(|asset| {
        SupplyCap {
            asset_info: AssetInfo::NativeToken {
                denom: asset
            },
            current_supply: Uint128::zero(),
            debt_total: Uint128::zero(),
            supply_cap_ratio: Decimal::percent(0),
            lp: false,
            stability_pool_ratio_for_debt_cap: None,
        }
    }).collect();

    //If not empty, edit 
    if !supply_caps.is_empty() {
        Ok(Response::new()
            .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: String::from("osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr"),
                msg: to_binary(&CDP_ExecuteMsg::EditBasket(EditBasket {
                    frozen: Some(frozen),
                    added_cAsset: None,
                    liq_queue: None,
                    credit_pool_infos: None,
                    collateral_supply_caps: Some(supply_caps),
                    multi_asset_supply_caps: None,
                    base_interest_rate: None,
                    credit_asset_twap_price_source: None,
                    negative_rates: None,
                    cpc_margin_of_error: None,
                    rev_to_stakers: None,
                    take_revenue: None,
                    revenue_destinations: None,
                }))?,
                funds: vec![],
        })))   
    } else { //Only set freeze
        Ok(Response::new()
            .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: String::from("osmo1gy5gpqqlth0jpm9ydxlmff6g5mpnfvrfxd3mfc8dhyt03waumtzqt8exxr"),
                msg: to_binary(&CDP_ExecuteMsg::EditBasket(EditBasket {
                    frozen: Some(frozen),
                    added_cAsset: None,
                    liq_queue: None,
                    credit_pool_infos: None,
                    collateral_supply_caps: None,
                    multi_asset_supply_caps: None,
                    base_interest_rate: None,
                    credit_asset_twap_price_source: None,
                    negative_rates: None,
                    cpc_margin_of_error: None,
                    rev_to_stakers: None,
                    take_revenue: None,
                    revenue_destinations: None,
                }))?,
                funds: vec![],
        })))        
    }    
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::ActiveProposals { start, limit } => to_binary(&query_proposals(deps, start, limit)?),
        QueryMsg::PendingProposals { start, limit } => to_binary(&query_pending_proposals(deps, start, limit)?),
        QueryMsg::Proposal { proposal_id } => to_binary(&PROPOSALS.load(deps.storage, proposal_id.to_string())?),
        QueryMsg::ProposalVotes { proposal_id } => {
            to_binary(&query_proposal_votes(deps, proposal_id)?)
        }
        QueryMsg::UserVotingPower {
            user,
            proposal_id,
            vesting,
        } => {
            let proposal = PROPOSALS.load(deps.storage, proposal_id.to_string())?;
            let user = deps.api.addr_validate(&user)?;

            let recipient = if vesting {
                Some(user.to_string())
            } else {
                None
            };

            to_binary(&calc_voting_power(
                deps.storage,
                deps.querier,
                None,
                user.to_string(),
                proposal.start_time,
                &mut false,
                recipient,
                CONFIG.load(deps.storage)?.quadratic_voting,
            )?)
        }
        QueryMsg::TotalVotingPower { proposal_id } => {
            let proposal = PROPOSALS.load(deps.storage, proposal_id.to_string())?;
            to_binary(&calc_total_voting_power_at(
                deps, 
                CONFIG.load(deps.storage)?.quadratic_voting,
                proposal.start_time
            )?)
        }
        QueryMsg::ProposalVoters {
            proposal_id,
            vote_option,
            start,
            limit,
            specific_user
        } => to_binary(&query_proposal_voters(
            deps,
            proposal_id,
            vote_option,
            start,
            limit,
            specific_user
        )?),
    }
}

/// Return a list of Proposals
pub fn query_proposals(
    deps: Deps,
    start: Option<u64>,
    limit: Option<u32>,
) -> StdResult<ProposalListResponse> {
    let proposal_count = PROPOSAL_COUNT.load(deps.storage)?;

    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = start.map(|start| Bound::inclusive(start.to_string()));

    let proposal_list = PROPOSALS
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (_, proposal) = item?;
            Ok(ProposalResponse {
                voting_power: proposal.voting_power,
                proposal_id: proposal.proposal_id,
                submitter: proposal.submitter,
                status: proposal.status,
                aligned_power: proposal.aligned_power,
                for_power: proposal.for_power,
                against_power: proposal.against_power,
                amendment_power: proposal.amendment_power,
                removal_power: proposal.removal_power,
                start_block: proposal.start_block,
                start_time: proposal.start_time,
                end_block: proposal.end_block,
                delayed_end_block: proposal.delayed_end_block,
                expiration_block: proposal.expiration_block,
                title: proposal.title,
                description: proposal.description,
                link: proposal.link,
                messages: proposal.messages,
            })
        })
        .collect::<StdResult<Vec<_>>>()?;

    Ok(ProposalListResponse {
        proposal_count,
        proposal_list,
    })
}

/// Return a list of Pending Proposals
pub fn query_pending_proposals(
    deps: Deps,
    start: Option<u64>,
    limit: Option<u32>,
) -> StdResult<ProposalListResponse> {
    let proposal_count = PROPOSAL_COUNT.load(deps.storage)?;

    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = start.map(|start| Bound::inclusive(start.to_string()));

    let proposal_list = PENDING_PROPOSALS
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (_, proposal) = item?;
            Ok(ProposalResponse {
                voting_power: proposal.voting_power,
                proposal_id: proposal.proposal_id,
                submitter: proposal.submitter,
                status: proposal.status,
                aligned_power: proposal.aligned_power,
                for_power: proposal.for_power,
                against_power: proposal.against_power,
                amendment_power: proposal.amendment_power,
                removal_power: proposal.removal_power,
                start_block: proposal.start_block,
                start_time: proposal.start_time,
                end_block: proposal.end_block,
                delayed_end_block: proposal.delayed_end_block,
                expiration_block: proposal.expiration_block,
                title: proposal.title,
                description: proposal.description,
                link: proposal.link,
                messages: proposal.messages,
            })
        })
        .collect::<StdResult<Vec<_>>>()?;

    Ok(ProposalListResponse {
        proposal_count,
        proposal_list,
    })
}

/// Return a list of voters for a given proposal
pub fn query_proposal_voters(
    deps: Deps,
    proposal_id: u64,
    vote_option: ProposalVoteOption,
    start: Option<u64>,
    limit: Option<u32>,
    specific_user: Option<String>,
) -> StdResult<Vec<Addr>> {
    let limit = limit.unwrap_or(DEFAULT_VOTERS_LIMIT).min(MAX_VOTERS_LIMIT);
    let start = start.unwrap_or_default();

    let proposal = PROPOSALS.load(deps.storage, proposal_id.to_string())?;

    let voters = match vote_option {
        ProposalVoteOption::For => proposal.for_voters,
        ProposalVoteOption::Against => proposal.against_voters,
        ProposalVoteOption::Amend => proposal.amendment_voters,
        ProposalVoteOption::Remove => proposal.removal_voters,
        ProposalVoteOption::Align => proposal.aligned_voters,
    };

    if let Some(specific_user) = specific_user {
        let specific_user = deps.api.addr_validate(&specific_user)?;
        if voters.contains(&specific_user) {
            return Ok(vec![specific_user]);
        } else {
            return Err(cosmwasm_std::StdError::GenericErr { msg: format!("User did not vote for this option in proposal {}", proposal_id) })
        }
    };

    Ok(voters
        .iter()
        .skip(start as usize)
        .take(limit as usize)
        .cloned()
        .collect())
}

/// Return the voting power per option for a given proposal
pub fn query_proposal_votes(deps: Deps, proposal_id: u64) -> StdResult<ProposalVotesResponse> {
    let proposal = PROPOSALS.load(deps.storage, proposal_id.to_string())?;

    Ok(ProposalVotesResponse {
        proposal_id,
        for_power: proposal.for_power,
        against_power: proposal.against_power,
        amendment_power: proposal.amendment_power,
        removal_power: proposal.removal_power,
        aligned_power: proposal.aligned_power,
    })
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    Ok(Response::default())
}