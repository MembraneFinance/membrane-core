use crate::contract::{execute, instantiate, query};
use crate::ContractError;

use membrane::liq_queue::{
    BidResponse, Config, ExecuteMsg, InstantiateMsg, QueryMsg, QueueResponse, ClaimsResponse,
};
use membrane::math::{Decimal256, Uint256};
use membrane::cdp::ExecuteMsg as CDP_ExecuteMsg;
use membrane::types::{AssetInfo, BidInput};

use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
use cosmwasm_std::{
    attr, from_binary, to_binary, BankMsg, Coin, CosmosMsg, Decimal, StdError, SubMsg, Uint128,
    WasmMsg, Addr,
};

#[test]
fn proper_initialization() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        owner: None, //Defaults to sender
        positions_contract: String::from("positions_contract"),
        waiting_period: 60u64,
    };

    let info = mock_info("addr0000", &[]);

    // we can just call .unwrap() to assert this was a success
    let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // it worked, let's query the state
    let value: Config =
        from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(
        value,
        Config {
            owner: Addr::unchecked("addr0000"),
            positions_contract: Addr::unchecked("positions_contract"),
            waiting_period: 60u64,
            added_assets: Some(vec![]),
            bid_asset: AssetInfo::NativeToken {
                denom: String::from("cdt"),
            },
        }
    );
}

#[test]
fn update_config() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        owner: None, //Defaults to sender
        positions_contract: String::from("positions_contract"),
        waiting_period: 60u64,
    };

    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    // update owner
    let msg = ExecuteMsg::UpdateConfig {
        owner: Some("owner0001".to_string()),
        positions_contract: None,
        waiting_period: None,
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // it worked, let's query the state
    let value: Config =
        from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(
        value,
        Config {
            owner: Addr::unchecked("owner0001"),
            positions_contract: Addr::unchecked("positions_contract"),
            waiting_period: 60u64,
            added_assets: Some(vec![]),
            bid_asset: AssetInfo::NativeToken {
                denom: String::from("cdt"),
            },
        }
    );

    // Update left items
    let info = mock_info("owner0001", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: None,
        positions_contract: None,
        waiting_period: Some(100u64),
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(0, res.messages.len());

    // it worked, let's query the state
    let value: Config =
        from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(
        value,
        Config {
            owner: Addr::unchecked("owner0001"),
            positions_contract: Addr::unchecked("positions_contract"),
            waiting_period: 60u64,
            added_assets: Some(vec![]),
            bid_asset: AssetInfo::NativeToken {
                denom: String::from("cdt"),
            },
        }
    );

    // Unauthorized err
    let info = mock_info("owner0000", &[]);
    let msg = ExecuteMsg::UpdateConfig {
        owner: Some("addr0000".to_string()),
        positions_contract: None,
        waiting_period: Some(60u64),
    };

    let res = execute(deps.as_mut(), mock_env(), info, msg);

    match res {
        Err(ContractError::Unauthorized {}) => {}
        Err(_) => {
            panic!("{}", res.err().unwrap().to_string())
        }
        _ => panic!("Msg sender is unauthorized to make this change"),
    }
}

#[test]
fn submit_bid() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        owner: None, //Defaults to sender
        positions_contract: String::from("positions_contract"),
        waiting_period: 60u64,
    };

    let info = mock_info("owner0000", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::AddQueue {
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        max_premium: Uint128::new(10u128), //A slot for each premium is created when queue is created
        bid_threshold: Uint256::from(1_000_000_000u128),
    };
    let info = mock_info("owner0000", &[]);
    execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    //Invalid bid_fpr
    let msg = ExecuteMsg::SubmitBid {
        bid_input: BidInput {
            bid_for: AssetInfo::NativeToken {
                denom: "reverse_osmo".to_string(),
            },
            liq_premium: 10u8,
        },
        bid_owner: None,
    };

    let info = mock_info("addr0000", &[]);
    let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    assert_eq!(err, ContractError::InvalidAsset {});

    //No Assets sent w/ bid
    let msg = ExecuteMsg::SubmitBid {
        bid_input: BidInput {
            bid_for: AssetInfo::NativeToken {
                denom: "osmo".to_string(),
            },
            liq_premium: 10u8,
        },
        bid_owner: None,
    };

    let info = mock_info("addr0000", &[]);
    let err = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap_err();
    assert_eq!(
        err,
        ContractError::Std(StdError::GenericErr {
            msg: "No asset provided, only bid asset allowed".to_string()
        })
    );

    //Invalid Bid Asset sent
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "osmo".to_string(),
            amount: Uint128::from(1000000u128),
        }],
    );
    let err = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap_err();
    assert_eq!(
        err,
        ContractError::Std(StdError::GenericErr {
            msg: "Invalid asset provided, only bid asset allowed".to_string()
        })
    );

    //Invalid Bid Asset sent alongside valid asset
    let info = mock_info(
        "addr0000",
        &[
            Coin {
                denom: "cdt".to_string(),
                amount: Uint128::from(1000000u128),
            },
            Coin {
                denom: "uluna".to_string(),
                amount: Uint128::from(1000000u128),
            },
        ],
    );
    let err = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap_err();
    assert_eq!(
        err,
        ContractError::Std(StdError::GenericErr {
            msg: "Invalid asset provided, only bid asset allowed".to_string()
        })
    );

    //Invalid Premium
    let invalid_msg = ExecuteMsg::SubmitBid {
        bid_input: BidInput {
            bid_for: AssetInfo::NativeToken {
                denom: "osmo".to_string(),
            },
            liq_premium: 15u8,
        },
        bid_owner: None,
    };

    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "cdt".to_string(),
            amount: Uint128::from(1000000u128),
        }],
    );

    let err = execute(deps.as_mut(), mock_env(), info.clone(), invalid_msg).unwrap_err();
    assert_eq!(err, ContractError::InvalidPremium {});

    //Successful Bid
    let env = mock_env();
    let wait_end = env.block.time.plus_seconds(60u64);
    execute(deps.as_mut(), env, info, msg).unwrap();

   
    let bid_response: BidResponse = from_binary(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Bid {
                bid_for: AssetInfo::NativeToken {
                    denom: "osmo".to_string(),
                },
                bid_id: Uint128::from(1u128),
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        bid_response,
        BidResponse {
            id: Uint128::from(1u128),
            user: "addr0000".to_string(),
            amount: Uint256::from(1000000u128),
            liq_premium: 10u8,
            product_snapshot: Decimal256::one(),
            sum_snapshot: Decimal256::zero(),
            pending_liquidated_collateral: Uint256::zero(),
            wait_end: None,
            epoch_snapshot: Uint128::zero(),
            scale_snapshot: Uint128::zero(),
        }
    );
}

#[test]
fn retract_bid() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        owner: None, //Defaults to sender
        positions_contract: String::from("positions_contract"),
        waiting_period: 60u64,
    };

    let info = mock_info("owner0000", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::AddQueue {
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        max_premium: Uint128::new(10u128), //A slot for each premium is created when queue is created
        bid_threshold: Uint256::from(1_000_000_000u128),
    };
    let info = mock_info("owner0000", &[]);
    execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::SubmitBid {
        bid_input: BidInput {
            bid_for: AssetInfo::NativeToken {
                denom: "osmo".to_string(),
            },
            liq_premium: 10u8,
        },
        bid_owner: None,
    };
    let info = mock_info(
        "addr0000",
        &[Coin {
            denom: "cdt".to_string(),
            amount: Uint128::from(1000000u128),
        }],
    );

    let env = mock_env();
    execute(deps.as_mut(), env, info, msg).unwrap();

    //Bid not found
    let msg = ExecuteMsg::RetractBid {
        bid_id: Uint128::new(0u128),
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        amount: None,
    };
    let info = mock_info("addr0000", &[]);
    let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    assert_eq!(
        err,
        ContractError::Std(StdError::GenericErr {
            msg: "Bid not found".to_string()
        })
    );

    //Successful RetractBid
    let msg = ExecuteMsg::RetractBid {
        bid_id: Uint128::new(1u128),
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        amount: None,
    };
    let info = mock_info("addr0000", &[]);
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
            to_address: "addr0000".to_string(),
            amount: vec![Coin {
                denom: "cdt".to_string(),
                amount: Uint128::from(1000000u128),
            }]
        }))]
    );

    //Bid not found after retracting
    let msg = ExecuteMsg::RetractBid {
        bid_id: Uint128::new(1u128),
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        amount: None,
    };
    let info = mock_info("addr0000", &[]);
    let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    assert_eq!(
        err,
        ContractError::Std(StdError::GenericErr {
            msg: "Bid not found".to_string()
        })
    );
}

#[test]
fn execute_bid() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        owner: None, //Defaults to sender
        positions_contract: String::from("positions_contract"),
        waiting_period: 60u64,
    };

    let info = mock_info("owner0000", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::AddQueue {
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        max_premium: Uint128::new(10u128), //A slot for each premium is created when queue is created
        bid_threshold: Uint256::from(1_000_000_000u128),
    };
    let info = mock_info("owner0000", &[]);
    execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::SubmitBid {
        bid_input: BidInput {
            bid_for: AssetInfo::NativeToken {
                denom: "osmo".to_string(),
            },
            liq_premium: 1u8,
        },
        bid_owner: None,
    };
    let info = mock_info(
        "owner0000",
        &[Coin {
            denom: "cdt".to_string(),
            amount: Uint128::from(500_000u128),
        }],
    );

    let env = mock_env();
    execute(deps.as_mut(), env, info, msg).unwrap();
    /////////////////////

    // required_stable 495,000
    let liq_msg = ExecuteMsg::Liquidate {
        credit_price: Decimal::one(),
        collateral_price: Decimal::one(),
        collateral_amount: Uint256::from(500_000u128),
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        bid_with: AssetInfo::NativeToken {
            denom: "cdt".to_string(),
        },
        position_id: Uint128::new(1u128),
        position_owner: "owner01".to_string(),
    };

    // unauthorized attempt
    let unauth_info = mock_info("asset0000", &[]); // only owner can execute
    let env = mock_env();
    let err = execute(
        deps.as_mut(),
        env.clone(),
        unauth_info,
        liq_msg.clone(),
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {},);
    let info = mock_info(
        "positions_contract",
        &[Coin {
            denom: "cdt".to_string(),
            amount: Uint128::from(500_000u128),
        }],
    );
    let res = execute(deps.as_mut(), env.clone(), info.clone(), liq_msg.clone()).unwrap();
    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: "positions_contract".to_string(),
            msg: to_binary(&CDP_ExecuteMsg::Repay {
                position_id: Uint128::new(1u128),
                position_owner: Some("owner01".to_string()),
                send_excess_to: None,
            })
            .unwrap(),
            funds: vec![Coin {
                denom: "cdt".to_string(),
                amount: Uint128::from(495000u128),
            }],
        }))]
    );

    let err = execute(deps.as_mut(), env.clone(), info.clone(), liq_msg).unwrap_err();
    assert_eq!(err, ContractError::InsufficientBids {});

    let liq_msg = ExecuteMsg::Liquidate {
        credit_price: Decimal::one(),
        collateral_price: Decimal::one(),
        collateral_amount: Uint256::from(500_000_000_000u128),
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        bid_with: AssetInfo::NativeToken {
            denom: "cdt".to_string(),
        },
        position_id: Uint128::new(1u128),
        position_owner: "owner01".to_string(),
    };

    let res = execute(deps.as_mut(), env, info, liq_msg).unwrap_err();
    assert_eq!(res, ContractError::InsufficientBids {});
}

#[test]
fn claim_liquidations() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        owner: None, //Defaults to sender
        positions_contract: String::from("positions_contract"),
        waiting_period: 60u64,
    };

    let info = mock_info("owner0000", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::AddQueue {
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        max_premium: Uint128::new(10u128), //A slot for each premium is created when queue is created
        bid_threshold: Uint256::from(1_000_000_000u128),
    };

    execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::SubmitBid {
        bid_input: BidInput {
            bid_for: AssetInfo::NativeToken {
                denom: "osmo".to_string(),
            },
            liq_premium: 1u8,
        },
        bid_owner: None,
    };
    let submit_info = mock_info(
        "owner0000",
        &[Coin {
            denom: "cdt".to_string(),
            amount: Uint128::from(1_000_000u128),
        }],
    );

    let env = mock_env();
    execute(deps.as_mut(), env.clone(), submit_info, msg).unwrap();

    let liq_msg = ExecuteMsg::Liquidate {
        credit_price: Decimal::one(),
        collateral_price: Decimal::one(),
        collateral_amount: Uint256::from(5000u128),
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        bid_with: AssetInfo::NativeToken {
            denom: "cdt".to_string(),
        },
        position_id: Uint128::new(1u128),
        position_owner: "owner01".to_string(),
    };
    let info = mock_info("positions_contract", &[]);
    execute(deps.as_mut(), env, info, liq_msg).unwrap();
    /////////
    
    let query_msg = QueryMsg::UserClaims { user: String::from("owner0000") };
    let claims: Vec<ClaimsResponse> = from_binary(&query(deps.as_ref(), mock_env(), query_msg.clone()).unwrap()).unwrap();
    assert_eq!(
        claims,
        vec![
            ClaimsResponse { 
                bid_for: String::from("osmo"), 
                pending_liquidated_collateral: Uint256::from(5000u128),
            },
        ],
    );

    let msg = ExecuteMsg::ClaimLiquidations {
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        bid_ids: Some(vec![Uint128::new(1u128)]),
    };
    let info = mock_info("owner0000", &[]);
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(
        res.attributes,
        vec![
            attr("action", "claim_liquidations"),
            attr("collateral_token", "osmo"),
            attr("collateral_amount", "5000"),
        ]
    );
}

#[test]
fn update_queue() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        owner: None, //Defaults to sender
        positions_contract: String::from("positions_contract"),
        waiting_period: 60u64,
    };

    let info = mock_info("addr0000", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let msg = ExecuteMsg::AddQueue {
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        max_premium: Uint128::new(10u128), //A slot for each premium is created when queue is created
        bid_threshold: Uint256::from(1_000_000_000u128),
    };
    //Unauthorized
    let unauth_info = mock_info("owner0000", &[]);
    let err = execute(deps.as_mut(), mock_env(), unauth_info, msg.clone()).unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});

    //Success
    execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

    let query_msg = QueryMsg::Queue {
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
    };

    let queue_response: QueueResponse =
        from_binary(&query(deps.as_ref(), mock_env(), query_msg.clone()).unwrap()).unwrap();

    assert_eq!(queue_response.max_premium, Uint128::new(10u128).to_string());
    assert_eq!(
        queue_response.bid_threshold,
        Uint256::from(1_000_000_000u128).to_string()
    );

    let msg = ExecuteMsg::UpdateQueue {
        bid_for: AssetInfo::NativeToken {
            denom: "osmo".to_string(),
        },
        max_premium: Some(Uint128::new(20u128)),
        bid_threshold: Some(Uint256::from(500_000_000u128)),
    };
    execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let queue_response: QueueResponse =
        from_binary(&query(deps.as_ref(), mock_env(), query_msg).unwrap()).unwrap();

    assert_eq!(queue_response.max_premium, Uint128::new(20u128).to_string());
    assert_eq!(
        queue_response.bid_threshold,
        Uint256::from(500_000_000u128).to_string()
    );
}
