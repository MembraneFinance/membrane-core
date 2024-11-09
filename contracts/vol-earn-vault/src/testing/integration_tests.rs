mod tests {

    use crate::helpers::EarnVaultContract;
    use std::str::FromStr;

    use membrane::stable_earn_vault::{ExecuteMsg, InstantiateMsg, QueryMsg, Config};
    // use membrane::mars_redbank::{Market, UserCollateralResponse, InterestRateModel};
    use membrane::types::{ClaimTracker, VTClaimCheckpoint, AssetInfo, Asset, VaultInfo, AssetPool, cAsset, UserInfo, Basket};
    use membrane::cdp::{PositionResponse, BasketPositionsResponse, CollateralInterestResponse, InterestResponse};
    use membrane::oracle::PriceResponse;
    use cosmwasm_std::{
        attr, coin, to_json_binary, Addr, Binary, Empty, Response, StdResult, Uint128, Decimal,
    };
    use cw_multi_test::{App, AppBuilder, BankKeeper, Contract, ContractWrapper, Executor};
    use cosmwasm_schema::cw_serde;

    const USER: &str = "user";
    const ADMIN: &str = "admin";

    //Earn Vault Contract
    pub fn earn_vault_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new_with_empty(
            crate::contract::execute,
            crate::contract::instantiate,
            crate::contract::query,
        ).with_reply(crate::contract::reply);
        Box::new(contract)
    }

    
    //Mock Osmo Proxy Contract
    #[cw_serde]
    pub enum Osmo_MockExecuteMsg {
        ExecuteSwaps {
            token_out: String,
            max_slippage: Decimal,
        },
    }
    
    #[cw_serde]
    pub struct Osmo_MockInstantiateMsg {}
    
    #[cw_serde]
    pub enum Osmo_MockQueryMsg { }

    #[cw_serde]    
    pub struct MockResponse {}

    pub fn osmosis_proxy_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |_, _, _, msg: Osmo_MockExecuteMsg| -> StdResult<Response> {
                match msg {
                    Osmo_MockExecuteMsg::ExecuteSwaps {
                        token_out,
                        max_slippage,
                    } => {
                        Ok(Response::new())
                    } 
                }
            },
            |_, _, _, _: Osmo_MockInstantiateMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, msg: Osmo_MockQueryMsg| -> StdResult<Binary> {
                Ok(to_json_binary(&MockResponse {})?)
            },
        );
        Box::new(contract)
    }

    
    //Mock Mars Vault Contract
    #[cw_serde]
    pub enum Vault_MockExecuteMsg {
        EnterVault { },
        ExitVault { },
        CrankAPR { },
    }
    
    #[cw_serde]
    pub struct Vault_MockInstantiateMsg {}
    
    #[cw_serde]
    pub enum Vault_MockQueryMsg {
        VaultTokenUnderlying {
            vault_token_amount: Uint128,
        },
        DepositTokenConversion {
            deposit_token_amount: Uint128,
        },
    }

    pub fn mars_vault_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |_, _, _, msg: Vault_MockExecuteMsg| -> StdResult<Response> {
                match msg {
                    Vault_MockExecuteMsg::EnterVault { } => {
                        Ok(Response::default())
                    },
                    Vault_MockExecuteMsg::ExitVault { } => {
                        Ok(Response::default())
                    },
                    Vault_MockExecuteMsg::CrankAPR { } => {
                        Ok(Response::default())
                    }
                }
            },
            |_, _, _, _: Vault_MockInstantiateMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, msg: Vault_MockQueryMsg| -> StdResult<Binary> {
                match msg {
                    Vault_MockQueryMsg::VaultTokenUnderlying {
                        vault_token_amount,
                    } => {
                        Ok(to_json_binary(&Uint128::new(5))?)
                    },
                    Vault_MockQueryMsg::DepositTokenConversion {
                        deposit_token_amount,
                    } => {
                        Ok(to_json_binary(&Uint128::new(5))?)
                    }
                }
            },
        );
        Box::new(contract)
    }

    //Mock Oracle Contract
    #[cw_serde]    
    pub enum Oracle_MockExecuteMsg { }

     #[cw_serde]    
    pub struct Oracle_MockInstantiateMsg {}

     #[cw_serde]    
    pub enum Oracle_MockQueryMsg {
        Price {
            asset_info: AssetInfo,
            twap_timeframe: u64,
            oracle_time_limit: u64,
            basket_id: Option<Uint128>,
        },
        Prices {
            asset_infos: Vec<AssetInfo>,
            twap_timeframe: u64,
            oracle_time_limit: u64,
        },
    }

    pub fn oracle_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |deps, _, info, msg: Oracle_MockExecuteMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, _, _: Oracle_MockInstantiateMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, msg: Oracle_MockQueryMsg| -> StdResult<Binary> {
                match msg {
                    Oracle_MockQueryMsg::Price {
                        asset_info,
                        twap_timeframe,
                        oracle_time_limit,
                        basket_id
                    } => {
                        let mut prices = vec![];
                        if asset_info.to_string() == String::from("cdt") {
                            prices.push(PriceResponse {
                                prices: vec![],
                                price: Decimal::from_str("0.993").unwrap(),
                                decimals: 6,
                            });
                        } else {
                            prices.push(PriceResponse {
                                prices: vec![],
                                price: Decimal::one(),
                                decimals: 6,
                            });
                        }
                        
                        
                        Ok(to_json_binary(&prices)?)                        
                    },
                    Oracle_MockQueryMsg::Prices {
                        asset_infos,
                        twap_timeframe,
                        oracle_time_limit,
                    } => {
                        Ok(to_json_binary(&vec![PriceResponse {
                            prices: vec![],
                            price: Decimal::one(),
                            decimals: 6,
                        }, PriceResponse {
                            prices: vec![],
                            price: Decimal::one(),
                            decimals: 6,
                        }])?)
                    }
                }
            },
        );
        Box::new(contract)
    }
    //Mock Positions Contract
    #[cw_serde]    
    pub enum CDP_MockExecuteMsg {
        Deposit {
            position_id: Option<Uint128>, 
            position_owner: Option<String>,
        },        
        Withdraw {
            position_id: Uint128,
            assets: Vec<Asset>,
            send_to: Option<String>,
        },
        IncreaseDebt {
            position_id: Uint128,
            amount: Option<Uint128>,
            LTV: Option<Decimal>,
            mint_to_addr: Option<String>,
        },
        Repay {
            position_id: Uint128,
            position_owner: Option<String>,
            send_excess_to: Option<String>,
        },
        EditRedeemability {
            position_ids: Vec<Uint128>,
            redeemable: Option<bool>,
            premium: Option<u128>,
            max_loan_repayment: Option<Decimal>,
            restricted_collateral_assets: Option<Vec<String>>,
        },
    }

    #[cw_serde]    
    pub struct CDP_MockInstantiateMsg {}

    #[cw_serde]    
    pub enum CDP_MockQueryMsg {
        GetBasketPositions {
            start_after: Option<String>,
            user: Option<String>,
            user_info: Option<UserInfo>,
            limit: Option<u32>,
        },
        GetBasket {},
        GetCollateralInterest {},
        GetCreditRate {},
    }

    pub fn cdp_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |deps, _, info, msg: CDP_MockExecuteMsg| -> StdResult<Response> {
                match msg {
                    CDP_MockExecuteMsg::Deposit {
                        position_id,
                        position_owner
                    } => Ok(Response::default().add_attributes(vec![
                                attr("position_id", position_id.unwrap_or_else(|| Uint128::one())),
                                attr("position_owner", position_owner.unwrap_or_else(|| String::from(USER))),
                            ])),
                    CDP_MockExecuteMsg::Withdraw {
                        position_id,
                        assets,
                        send_to,
                    } => Ok(
                        Response::new()
                            .add_attributes(vec![
                                attr("total_loan", "1000"),
                                attr("increased_by", "2000000"),
                                attr("user", USER),
                            ])
                    ),
                    CDP_MockExecuteMsg::IncreaseDebt {
                        position_id,
                        amount,
                        LTV,
                        mint_to_addr,
                    } => Ok(
                        Response::new()
                            .add_attributes(vec![
                                attr("total_loan", "1000"),
                                attr("increased_by", "2000000"),
                                attr("user", USER),
                            ])
                    ),
                    CDP_MockExecuteMsg::Repay {
                        position_id,
                        position_owner,
                        send_excess_to,
                    } => Ok(
                        Response::new()
                            .add_attributes(vec![
                                attr("total_loan", "1000"),
                                attr("increased_by", "2000000"),
                                attr("user", USER),
                            ])
                    ),
                    CDP_MockExecuteMsg::EditRedeemability {
                        position_ids,
                        redeemable,
                        premium,
                        max_loan_repayment,
                        restricted_collateral_assets,
                    } => Ok(
                        Response::new()
                            .add_attributes(vec![
                                attr("total_loan", "1000"),
                                attr("increased_by", "2000000"),
                                attr("user", USER),
                            ])
                    ),
                }
            },
            |_, _, _, _: CDP_MockInstantiateMsg| -> StdResult<Response> { Ok(Response::default()) },
            |_, _, msg: CDP_MockQueryMsg| -> StdResult<Binary> { 
                match msg {
                    CDP_MockQueryMsg::GetBasketPositions { 
                        start_after,
                        user,
                        user_info,
                        limit,
                    } => {
                        Ok(to_json_binary(&vec![BasketPositionsResponse {
                            user: String::from("contract4"),
                            positions:
                            vec![PositionResponse {
                            position_id: Uint128::new(1),
                            collateral_assets: vec![
                                cAsset {
                                    asset: Asset {
                                        info: AssetInfo::NativeToken { denom: String::from("factory/contract3/mars-usdc-vault") },
                                        amount: Uint128::new(1_000_000_000_000),
                                    },
                                    max_borrow_LTV: Decimal::zero(), 
                                    max_LTV: Decimal::zero(),
                                    rate_index: Decimal::zero(), 
                                    pool_info: None,
                                    hike_rates: Some(true),
                                    
                                }
                            ],
                            credit_amount: Uint128::new(101_000_000),  
                            cAsset_ratios: vec![],
                            avg_borrow_LTV: Decimal::zero(),
                            avg_max_LTV: Decimal::zero(),                              
                        }]}])?)
                    },
                    CDP_MockQueryMsg::GetBasket { } => {
                        Ok(to_json_binary(&Basket {
                            basket_id: Uint128::zero(),
                            current_position_id: Uint128::zero(),
                            collateral_types: vec![
                                cAsset {
                                    asset: Asset {
                                        info: AssetInfo::NativeToken { denom: String::from("factory/contract3/mars-usdc-vault") },
                                        amount: Uint128::zero(),
                                    },
                                    max_borrow_LTV: Decimal::zero(), 
                                    max_LTV: Decimal::zero(),
                                    rate_index: Decimal::zero(), 
                                    pool_info: None,
                                    hike_rates: Some(true),
                                    
                                }
                            ],
                            collateral_supply_caps: vec![],
                            lastest_collateral_rates: vec![],
                            credit_asset: Asset { info: AssetInfo::NativeToken { denom: String::from("credit") }, amount: Uint128::zero() },
                            credit_price: PriceResponse { 
                                prices: vec![], 
                                price: Decimal::one(), 
                                decimals: 6
                            },
                            liq_queue: None,
                            base_interest_rate: Decimal::zero(),
                            pending_revenue: Uint128::zero(),
                            negative_rates: false,
                            cpc_margin_of_error: Decimal::zero(),
                            multi_asset_supply_caps: vec![],
                            frozen: false,
                            rev_to_stakers: true,
                            credit_last_accrued: 0,
                            rates_last_accrued: 0,
                            oracle_set: true,
                            revenue_destinations: Some(vec![]),
                        })?)
                    },
                    CDP_MockQueryMsg::GetCollateralInterest { } => {
                        Ok(to_json_binary(&CollateralInterestResponse {
                            rates: vec![Decimal::percent(1)],
                        })?)
                    },
                    CDP_MockQueryMsg::GetCreditRate { } => {
                        Ok(to_json_binary(&InterestResponse {
                            credit_interest: Decimal::zero(),
                            negative_rate: true,
                        })?)
                    },
                }
            },
        );
        Box::new(contract)
    }
    pub fn cdp_contract_unprofitable() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |deps, _, info, msg: CDP_MockExecuteMsg| -> StdResult<Response> {
                match msg {
                    CDP_MockExecuteMsg::Deposit {
                        position_id,
                        position_owner
                    } => Ok(Response::default().add_attributes(vec![
                                attr("position_id", position_id.unwrap_or_else(|| Uint128::one())),
                                attr("position_owner", position_owner.unwrap_or_else(|| String::from(USER))),
                            ])),
                    CDP_MockExecuteMsg::Withdraw {
                        position_id,
                        assets,
                        send_to,
                    } => Ok(
                        Response::new()
                            .add_attributes(vec![
                                attr("total_loan", "1000"),
                                attr("increased_by", "2000000"),
                                attr("user", USER),
                            ])
                    ),
                    CDP_MockExecuteMsg::IncreaseDebt {
                        position_id,
                        amount,
                        LTV,
                        mint_to_addr,
                    } => Ok(
                        Response::new()
                            .add_attributes(vec![
                                attr("total_loan", "1000"),
                                attr("increased_by", "2000000"),
                                attr("user", USER),
                            ])
                    ),
                    CDP_MockExecuteMsg::Repay {
                        position_id,
                        position_owner,
                        send_excess_to,
                    } => Ok(
                        Response::new()
                            .add_attributes(vec![
                                attr("total_loan", "1000"),
                                attr("increased_by", "2000000"),
                                attr("user", USER),
                            ])
                    ),
                    CDP_MockExecuteMsg::EditRedeemability {
                        position_ids,
                        redeemable,
                        premium,
                        max_loan_repayment,
                        restricted_collateral_assets,
                    } => Ok(
                        Response::new()
                            .add_attributes(vec![
                                attr("total_loan", "1000"),
                                attr("increased_by", "2000000"),
                                attr("user", USER),
                            ])
                    ),
                }
            },
            |_, _, _, _: CDP_MockInstantiateMsg| -> StdResult<Response> { Ok(Response::default()) },
            |_, _, msg: CDP_MockQueryMsg| -> StdResult<Binary> { 
                match msg {
                    CDP_MockQueryMsg::GetBasketPositions { 
                        start_after,
                        user,
                        user_info,
                        limit,
                    } => {
                        Ok(to_json_binary(&vec![BasketPositionsResponse {
                            user: String::from("contract4"),
                            positions:
                            vec![PositionResponse {
                            position_id: Uint128::new(1),
                            collateral_assets: vec![
                                cAsset {
                                    asset: Asset {
                                        info: AssetInfo::NativeToken { denom: String::from("factory/contract3/mars-usdc-vault") },
                                        amount: Uint128::new(1_000_000_000_000),
                                    },
                                    max_borrow_LTV: Decimal::zero(), 
                                    max_LTV: Decimal::zero(),
                                    rate_index: Decimal::zero(), 
                                    pool_info: None,
                                    hike_rates: Some(true),
                                    
                                }
                            ],
                            credit_amount: Uint128::new(101_000_000),  
                            cAsset_ratios: vec![],
                            avg_borrow_LTV: Decimal::zero(),
                            avg_max_LTV: Decimal::zero(),                              
                        }]}])?)
                    },
                    CDP_MockQueryMsg::GetBasket { } => {
                        Ok(to_json_binary(&Basket {
                            basket_id: Uint128::zero(),
                            current_position_id: Uint128::zero(),
                            collateral_types: vec![
                                cAsset {
                                    asset: Asset {
                                        info: AssetInfo::NativeToken { denom: String::from("factory/contract3/mars-usdc-vault") },
                                        amount: Uint128::zero(),
                                    },
                                    max_borrow_LTV: Decimal::zero(), 
                                    max_LTV: Decimal::zero(),
                                    rate_index: Decimal::zero(), 
                                    pool_info: None,
                                    hike_rates: Some(true),
                                    
                                }
                            ],
                            collateral_supply_caps: vec![],
                            lastest_collateral_rates: vec![],
                            credit_asset: Asset { info: AssetInfo::NativeToken { denom: String::from("credit") }, amount: Uint128::zero() },
                            credit_price: PriceResponse { 
                                prices: vec![], 
                                price: Decimal::one(), 
                                decimals: 6
                            },
                            liq_queue: None,
                            base_interest_rate: Decimal::zero(),
                            pending_revenue: Uint128::zero(),
                            negative_rates: false,
                            cpc_margin_of_error: Decimal::zero(),
                            multi_asset_supply_caps: vec![],
                            frozen: false,
                            rev_to_stakers: true,
                            credit_last_accrued: 0,
                            rates_last_accrued: 0,
                            oracle_set: true,
                            revenue_destinations: Some(vec![]),
                        })?)
                    },
                    CDP_MockQueryMsg::GetCollateralInterest { } => {
                        Ok(to_json_binary(&CollateralInterestResponse {
                            rates: vec![Decimal::one()],
                        })?)
                    },
                    CDP_MockQueryMsg::GetCreditRate { } => {
                        Ok(to_json_binary(&InterestResponse {
                            credit_interest: Decimal::zero(),
                            negative_rate: true,
                        })?)
                    },
                }
            },
        );
        Box::new(contract)
    }

    fn mock_app() -> App {
        AppBuilder::new().build(|router, _, storage| {
            let bank = BankKeeper::new();

            bank.init_balance(
                storage,
                &Addr::unchecked("god"),
                vec![coin(1_000_000_000_000_000, "factory/contract4/stable-earn-vault"), coin(100_000_000_000_000_000, "uusdc"), coin(1_000_000_000_000_000, "factory/contract3/mars-usdc-vault"), coin(1_000_000_000_000_000, "cdt")],
            )
            .unwrap();
            bank.init_balance(
                storage,
                &Addr::unchecked(USER),
                vec![coin(14, "uusdc"), coin(100_000, "wrong_token"), coin(100_000_000, "factory/contract3/mars-usdc-vault")],
            )
            .unwrap();
            bank.init_balance(
                storage,
                &Addr::unchecked(ADMIN),
                vec![coin(1_000_000_000_000, "factory/contract3/mars-usdc-vault")],
            ).unwrap();
            bank.init_balance(
                storage,
                &Addr::unchecked("contract4"),
                vec![coin(0, "factory/contract3/mars-usdc-vault")],
            )
            .unwrap();

            router.bank = bank;
        })
    }

    fn proper_instantiate() -> (App, EarnVaultContract) {
        let mut app = mock_app();

        //Instaniate Osmosis Proxy
        let op_id = app.store_code(osmosis_proxy_contract());

        let op_contract_addr = app
            .instantiate_contract(
                op_id,
                Addr::unchecked(ADMIN),
                &Osmo_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();
        //Oracle 
        let oracle_id = app.store_code(oracle_contract());
        let oracle_contract_addr = app
            .instantiate_contract(
                oracle_id,
                Addr::unchecked(ADMIN),
                &Oracle_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();

        //Instantiate CDP contract
        let cdp_id = app.store_code(cdp_contract());
        let cdp_contract_addr = app
            .instantiate_contract(
                cdp_id,
                Addr::unchecked(ADMIN),
                &CDP_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();

        //Instantiate Mars Vault contract
        let vault_id = app.store_code(mars_vault_contract());
        let vault_contract_addr = app
            .instantiate_contract(
                vault_id,
                Addr::unchecked(ADMIN),
                &Vault_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();

        //Instantiate Earn Vault contract        
        let vault_id = app.store_code(earn_vault_contract());

        let msg = InstantiateMsg { 
            cdt_denom: String::from("cdt"),
            vault_subdenom: String::from("stable-earn-vault"), 
            deposit_token: VaultInfo {
                deposit_token: String::from("uusdc"),
                vault_token: String::from("factory/contract3/mars-usdc-vault"),
                vault_addr: vault_contract_addr,
            },
            cdp_contract_addr: cdp_contract_addr.to_string(),
            osmosis_proxy_contract_addr: op_contract_addr.to_string(),
            oracle_contract_addr: oracle_contract_addr.to_string(),
        };
        let vault_contract_addr = app
            .instantiate_contract(vault_id, Addr::unchecked(ADMIN), &msg, &[coin(1_000_000_000_000, "factory/contract3/mars-usdc-vault")], "test", None)
            .unwrap();

        let vault_contract = EarnVaultContract(vault_contract_addr);

        
        //Instantiate CDP contract
        let cdp_id = app.store_code(cdp_contract_unprofitable());
        let cdp_contract_unprofitable_addr = app
            .instantiate_contract(
                cdp_id,
                Addr::unchecked(ADMIN),
                &CDP_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();

        (app, vault_contract)
    }

    mod earn_vault {

        use std::str::FromStr;

        use cosmwasm_std::{coin, coins, BlockInfo, Uint128};
        use membrane::types::APR;

        use super::*;

        
        #[test]
        fn enter_vault() {
            let (mut app, vault_contract) = proper_instantiate();

            //Enter Vault: Error, wrong token
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(10, "wrong_token")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap_err();
            //Enter Vault: Error, multiple token
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(10, "uusdc"), coin(10, "wrong_token")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap_err();

            //Enter Vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(5, "uusdc")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();

            // Query Vault token underlying
            let underlying_deposit_token: Uint128 = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::VaultTokenUnderlying { 
                        vault_token_amount: Uint128::new(5_000_000)
                    },
                )
                .unwrap();
            assert_eq!(
                underlying_deposit_token,
                Uint128::new(2)
            );
            
            //Query Vault deposit token balance
            //Should be 0 bc everything was sent to the vault
            let balance = app
                .wrap()
                .query_balance(Addr::unchecked("contract4"), "uusdc")
                .unwrap().amount;
            assert_eq!(balance, Uint128::zero());

            //Send contract4 more factory/contract3/mars-usdc-vault to mimic a vault mint
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(5, "factory/contract3/mars-usdc-vault")]).unwrap();
            
            //Enter Vault: None of the deposit is kept in the vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(5, "uusdc")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();

            // Query Vault token underlying
            let underlying_deposit_token: Uint128 = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::VaultTokenUnderlying { 
                        vault_token_amount: Uint128::new(5_000_000)
                    },
                )
                .unwrap();
            assert_eq!(
                underlying_deposit_token,
                Uint128::new(1)// this keeps going down bc the total dt is always returning 5 and we're minint gnew vts each time there is a deposit
            );
            
            //Query Vault deposit token balance
            //Should be 0 bc everything was sent to the vault
            let balance = app
                .wrap()
                .query_balance(Addr::unchecked("contract4"), "uusdc")
                .unwrap().amount;
            assert_eq!(balance, Uint128::new(0));

            //Skip 8 days to test APR
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86400*8),
                chain_id: app.block_info().chain_id,
            });
            //Crank APR
            let msg = ExecuteMsg::CrankRealizedAPR { };
            let cosmos_msg = vault_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(ADMIN), cosmos_msg).unwrap();

            //Query historical claims state
            let claim_tracker: ClaimTracker = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::ClaimTracker {},
                )
                .unwrap();
            assert_eq!(
                claim_tracker.vt_claim_checkpoints,
                vec![
                    VTClaimCheckpoint {
                        vt_claim_of_checkpoint: Uint128::new(308641),
                        time_since_last_checkpoint: 691200,
                    }
                ]
            );

            //Skip 8 days to test Claim conditional time update 
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86400*8),
                chain_id: app.block_info().chain_id,
            });
            //Crank APR
            let msg = ExecuteMsg::CrankRealizedAPR { };
            let cosmos_msg = vault_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(ADMIN), cosmos_msg).unwrap();
            
            //Query historical claims state
            let claim_tracker: ClaimTracker = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::ClaimTracker {},
                )
                .unwrap();
            assert_eq!(
                claim_tracker.vt_claim_checkpoints,
                vec![
                    VTClaimCheckpoint {
                        vt_claim_of_checkpoint: Uint128::new(308641),
                        time_since_last_checkpoint: 1382400,
                    }
                ]
            );
        }

        #[test]
        fn loop_vault() {
            let (mut app, vault_contract) = proper_instantiate();

            //Enter Vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(5, "uusdc")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();

            //Loop prep: 
            //- Send the contract cdt to mimic a mint
            //- Send the contract usdc to mimic a swap
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(899999999999, "cdt")]).unwrap();
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(895499999999, "uusdc")]).unwrap();
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(6, "factory/contract3/mars-usdc-vault")]).unwrap();

            //Loop 
            let msg = ExecuteMsg::LoopCDP { max_mint_amount: None };
            let cosmos_msg = vault_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();

            //Assert contract4 balance is non-zero bc the CDP deposit is precise
            let balance = app
                .wrap()
                .query_balance(Addr::unchecked("contract4"), "factory/contract3/mars-usdc-vault")
                .unwrap().amount;
            assert_eq!(balance, Uint128::new(1));

            //Change to an unprofitable CDP rate contract using update config
            let msg = ExecuteMsg::UpdateConfig {
                owner: None,
                cdp_contract_addr: Some("contract5".to_string()),
                mars_vault_addr: None,
                osmosis_proxy_contract_addr: None,
                oracle_contract_addr: None,
                deposit_cap: None,
                swap_slippage: None,
                vault_cost_index: None,
                withdrawal_buffer: None,
            };
            let cosmos_msg = vault_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(ADMIN), cosmos_msg).unwrap();

            //Loop: Error, unprofitable
            let msg = ExecuteMsg::LoopCDP { max_mint_amount: None};
            let cosmos_msg = vault_contract.call(msg, vec![]).unwrap(); 
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap_err();         

        }

        #[test]
        fn unloop_vault() {
            let (mut app, vault_contract) = proper_instantiate();

            //Enter Vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(5, "uusdc")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();
            
            //Unloop: Error bc profitable
            // let msg = ExecuteMsg::UnloopCDP {
            //     desired_collateral_withdrawal: None
            // };
            // let cosmos_msg = vault_contract.call(msg, vec![]).unwrap();
            // app.execute(Addr::unchecked("contract4"), cosmos_msg).unwrap_err();

            //Change to an unprofitable CDP rate contract using update config
            let msg = ExecuteMsg::UpdateConfig {
                owner: None,
                cdp_contract_addr: Some("contract5".to_string()),
                mars_vault_addr: None,
                osmosis_proxy_contract_addr: None,
                oracle_contract_addr: None,
                deposit_cap: None,
                swap_slippage: None,
                vault_cost_index: None,
                withdrawal_buffer: None,
            };
            let cosmos_msg = vault_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(ADMIN), cosmos_msg).unwrap();            

            //Unloop prep: 
            //- Send the contract cdt to mimic a mint
            //- Send the contract usdc to mimic a swap
            //- Send the contract factory/contract3/mars-usdc-vault to mimic an exit from the deposit token vault
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(9950000000, "cdt")]).unwrap();
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(5, "uusdc")]).unwrap();
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(101505000, "factory/contract3/mars-usdc-vault")]).unwrap();

            /////Test withdrawals
            /// 
            //Unloop 
            let msg = ExecuteMsg::UnloopCDP {
                desired_collateral_withdrawal: Uint128::new(1_000_000_000_000)
            };
            let cosmos_msg = vault_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("contract4"), cosmos_msg).unwrap();

            //Query to assert the config total wasn't changed 
            //Exit vault handles the state updates
            let config: Config = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::Config {},
                )
                .unwrap();
            assert_eq!(
                config.total_nonleveraged_vault_tokens,
                Uint128::new(1000000000004)
            );
        }
        
        #[test]
        fn exit_vault() {
            let (mut app, vault_contract) = proper_instantiate();

            //Enter Vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(5, "uusdc")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();

            // Query Vault token underlying
            // let underlying_deposit_token: Uint128 = app
            //     .wrap()
            //     .query_wasm_smart(
            //         vault_contract.addr(),
            //         &QueryMsg::VaultTokenUnderlying { 
            //             vault_token_amount: Uint128::new(5_000_000)
            //         },
            //     )
            //     .unwrap();
            // assert_eq!(
            //     underlying_deposit_token,
            //     Uint128::new(8)
            // );

            
            //Send the vault token to the user to enable exit
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked(USER), &vec![coin(10000000, "factory/contract4/stable-earn-vault")]).unwrap();
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(4, "uusdc")]).unwrap();
            app.send_tokens(Addr::unchecked("god"), Addr::unchecked("contract4"), &vec![coin(101505000, "factory/contract3/mars-usdc-vault")]).unwrap();

            //Query user balance
            let balance = app
                .wrap()
                .query_balance(Addr::unchecked(USER), "uusdc")
                .unwrap().amount;
            assert_eq!(balance, Uint128::new(9));

            //Exit Vault: THE RATE ASSURANCE ERRORS BC WE CAN'T REMOVE TOKENS FROM THE MARS CONTRACT MID-EXECUTION
            let msg = ExecuteMsg::ExitVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(9000000, "factory/contract4/stable-earn-vault")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();

            // Query Vault token underlying
            // let underlying_deposit_token: Uint128 = app
            //     .wrap()
            //     .query_wasm_smart(
            //         vault_contract.addr(),
            //         &QueryMsg::VaultTokenUnderlying { 
            //             vault_token_amount: Uint128::new(5_000_000)
            //         },
            //     )
            //     .unwrap();
            // assert_eq!(
            //     underlying_deposit_token,
            //     Uint128::new(8)
            // );

            //Query user balance
            let balance = app
                .wrap()
                .query_balance(Addr::unchecked(USER), "uusdc")
                .unwrap().amount;
            assert_eq!(balance, Uint128::new(13));

            //Exit vault handles the state updates
            let config: Config = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::Config {},
                )
                .unwrap();
            assert_eq!(
                config.total_nonleveraged_vault_tokens,
                Uint128::new(101504995)
            );
        }
    }

}