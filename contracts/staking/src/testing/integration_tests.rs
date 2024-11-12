#[cfg(test)]
mod tests {
    use membrane::oracle::PriceResponse;
    use membrane::staking::{ExecuteMsg, InstantiateMsg, QueryMsg, FeeEventsResponse, RewardsResponse};
    use membrane::types::{AssetInfo, StakeDistribution, Asset, UserInfo, Basket, FeeEvent, LiqAsset, Allocation, VestingPeriod};

    use cosmwasm_std::{
        coin, to_binary, BlockInfo, Addr, Binary, Decimal, Empty, Response, StdResult, Uint128, CosmosMsg, Coin, WasmMsg,
    };
    use cw_multi_test::{App, AppBuilder, BankKeeper, Contract, ContractWrapper, Executor};
    use membrane::vesting::{RecipientsResponse, RecipientResponse};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    //const USER: &str = "user";
    const ADMIN: &str = "admin";

    /// StakingContract is a wrapper around Addr that provides a lot of helpers
    /// for working with this.
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
    pub struct StakingContract(pub Addr);
    
    impl StakingContract {
        #[allow(dead_code)]
        pub fn addr(&self) -> Addr {
            self.0.clone()
        }

        #[allow(dead_code)]
        pub fn call<T: Into<ExecuteMsg>>(&self, msg: T, funds: Vec<Coin>) -> StdResult<CosmosMsg> {
            let msg = to_binary(&msg.into())?;
            Ok(WasmMsg::Execute {
                contract_addr: self.addr().into(),
                msg,
                funds,
            }
            .into())
        }
    }

    //Staking Contract
    pub fn staking_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new_with_empty(
            crate::contract::execute,
            crate::contract::instantiate,
            crate::contract::query,
        );
        Box::new(contract)
    }

    //Mock Osmo Proxy Contract
    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Osmo_MockExecuteMsg {
        MintTokens {
            denom: String,
            amount: Uint128,
            mint_to_address: String,
        }
    }
    
    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub struct Osmo_MockInstantiateMsg {}

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Osmo_MockQueryMsg {}

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub struct MockResponse {}

    pub fn osmosis_proxy_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |deps, _, info, msg: Osmo_MockExecuteMsg| -> StdResult<Response> {
                match msg {
                    Osmo_MockExecuteMsg::MintTokens {
                        denom,
                        amount,
                        mint_to_address,
                    } => {
                        if (amount != Uint128::new(8_219) || denom != String::from("mbrn_denom") || mint_to_address != String::from("user_1")) 
                        && (amount != Uint128::new(8219) || denom != String::from("mbrn_denom") || mint_to_address != String::from("contract4")) 
                        && (amount != Uint128::new(1095) || denom != String::from("mbrn_denom") || mint_to_address != String::from("contract4"))
                        && (amount != Uint128::new(8) || denom != String::from("mbrn_denom") || mint_to_address != String::from("user_1"))
                        && (amount != Uint128::new(78082) || denom != String::from("mbrn_denom") || mint_to_address != String::from("user_1"))
                        && (amount != Uint128::new(156164) || denom != String::from("mbrn_denom") || mint_to_address != String::from("user_1"))
                        && (amount != Uint128::new(4109) || denom != String::from("mbrn_denom") || mint_to_address != String::from("governator_addr")){
                            // panic!("MintTokens called with incorrect parameters, {}, {}, {}", amount, denom, mint_to_address);
                        }
                        Ok(Response::default())
                    }
                }
            },
            |_, _, _, _: Osmo_MockInstantiateMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, msg: Osmo_MockQueryMsg| -> StdResult<Binary> {
                Ok(to_binary(&MockResponse {})?)
            },
        );
        Box::new(contract)
    }

    //Mock Vesting Contract
    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Vesting_MockExecuteMsg {}

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub struct Vesting_MockInstantiateMsg {}

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Vesting_MockQueryMsg {
        Recipients {},
    }

    pub fn vesting_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |deps, _, info, msg: Vesting_MockExecuteMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, _, _: Vesting_MockInstantiateMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, msg: Vesting_MockQueryMsg| -> StdResult<Binary> {
                match msg {
                    Vesting_MockQueryMsg::Recipients { } => Ok(to_binary(&RecipientsResponse {
                        recipients: vec![RecipientResponse {
                            recipient: String::from("recipient"),
                            allocation: Some(Allocation {
                                amount: Uint128::new(1_000_000),
                                amount_withdrawn: Uint128::zero(),
                                start_time_of_allocation: 0,
                                vesting_period: VestingPeriod { cliff: 0, linear: 0 },
                            }),
                            claimables: vec![],
                        }],
                    })?),
                }
            },
        );
        Box::new(contract)
    }

    //Mock CDP Contract
    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CDP_MockExecuteMsg {}

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub struct CDP_MockInstantiateMsg {}

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CDP_MockQueryMsg {
        GetBasket { },
    }

    pub fn cdp_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |deps, _, info, msg: CDP_MockExecuteMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, _, _: CDP_MockInstantiateMsg| -> StdResult<Response> { Ok(Response::default()) },
            |_, _, msg: CDP_MockQueryMsg| -> StdResult<Binary> {
                match msg {
                    CDP_MockQueryMsg::GetBasket { } => Ok(to_binary(&Basket {
                        basket_id: Uint128::one(),
                        current_position_id: Uint128::one(),
                        collateral_types: vec![],
                        collateral_supply_caps: vec![],
                        lastest_collateral_rates: vec![],
                        credit_asset: Asset {
                            info: AssetInfo::NativeToken {
                                denom: String::from("credit_fulldenom"),
                            },
                            amount: Uint128::zero(),
                        },
                        credit_price: PriceResponse { 
                            prices: vec![], 
                            price: Decimal::one(), 
                            decimals: 6
                        },
                        liq_queue: None,
                        base_interest_rate: Decimal::zero(),
                        pending_revenue: Uint128::zero(),
                        negative_rates: true,
                        cpc_margin_of_error: Decimal::zero(),
                        multi_asset_supply_caps: vec![],
                        frozen: false,
                        rev_to_stakers: true,
                        credit_last_accrued: 0,
                        rates_last_accrued: 0,
                        oracle_set: false,
                        revenue_destinations: Some(vec![]),
                    })?),
                }
            },
        );
        Box::new(contract)
    }

    //Mock Auction Contract
    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Auction_MockExecuteMsg {
        StartAuction {
            repayment_position_info: Option<UserInfo>,
            send_to: Option<String>,
            auction_asset: Asset,
        },
    }

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub struct Auction_MockInstantiateMsg {}

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Auction_MockQueryMsg {}


    pub fn auction_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |deps, _, info, msg: Auction_MockExecuteMsg| -> StdResult<Response> {
                match msg {
                    Auction_MockExecuteMsg::StartAuction {
                        repayment_position_info,
                        send_to,
                        auction_asset,
                    } => Ok(Response::default()),
                }
            },
            |_, _, _, _: Auction_MockInstantiateMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, msg: Auction_MockQueryMsg| -> StdResult<Binary> {
                Ok(to_binary(&MockResponse {})?)
            },
        );
        Box::new(contract)
    }


    fn mock_app() -> App {
        AppBuilder::new().build(|router, _, storage| {
            let bank = BankKeeper::new();

            bank.init_balance(
                storage,
                &Addr::unchecked("coin_God"),
                vec![coin(100_000_000, "debit"), coin(100_000_000, "2nddebit"), coin(100_000_000, "mbrn_denom")]
            )
            .unwrap();
            bank.init_balance(
                storage,
                &Addr::unchecked("user_1"),
                vec![coin(10_000_000, "mbrn_denom")],
            )
            .unwrap();
            bank.init_balance(
                storage,
                &Addr::unchecked("contract1"), //positions contract
                vec![coin(3000, "credit_fulldenom"), coin(3000, "fee_asset")],
            )
            .unwrap();
            // bank.init_balance(
            //     storage,
            //     &Addr::unchecked("contract4"), //staking contract
            //     vec![coin(1, "mbrn_denom")], //This should make claim's check reply error
            // )
            // .unwrap();

            router.bank = bank;
        })
    }

    fn proper_instantiate() -> (App, StakingContract, Addr) {
        let mut app = mock_app();

        //Instaniate Osmosis Proxy
        let proxy_id = app.store_code(osmosis_proxy_contract());

        let osmosis_proxy_contract_addr = app
            .instantiate_contract(
                proxy_id,
                Addr::unchecked(ADMIN),
                &Osmo_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();

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

        let auction_id = app.store_code(auction_contract());
        let auction_contract_addr = app
            .instantiate_contract(
                auction_id,
                Addr::unchecked(ADMIN),
                &Auction_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();

        let vesting_id = app.store_code(vesting_contract());
        let vesting_contract_addr = app
            .instantiate_contract(
                vesting_id,
                Addr::unchecked(ADMIN),
                &CDP_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();

        //Instantiate Staking contract
        let staking_id = app.store_code(staking_contract());

        let msg = InstantiateMsg {
            owner: Some(ADMIN.to_string()),
            positions_contract: Some(cdp_contract_addr.to_string()),
            auction_contract: Some(auction_contract_addr.to_string()),
            vesting_contract: Some(vesting_contract_addr.to_string()),
            governance_contract: Some("gov_contract".to_string()),
            osmosis_proxy: Some(osmosis_proxy_contract_addr.to_string()),
            incentive_schedule: Some(StakeDistribution { rate: Decimal::percent(10), duration: 90 }),
            mbrn_denom: String::from("mbrn_denom"),
            unstaking_period: None,
        };

        let staking_contract_addr = app
            .instantiate_contract(staking_id, Addr::unchecked(ADMIN), &msg, &[], "test", None)
            .unwrap();

        let staking_contract = StakingContract(staking_contract_addr);

        (app, staking_contract, auction_contract_addr)
    }

    #[cfg(test)]
    mod staking {
        use std::str::FromStr;

        use super::*;
        use membrane::staking::{TotalStakedResponse, StakerResponse};
        
        #[test]
        fn commission_claims() {
            let (mut app, staking_contract, auction_contract) = proper_instantiate();

            //Stake MBRN as user
            let msg = ExecuteMsg::Stake { user: None };
            let cosmos_msg = staking_contract.call(msg, vec![coin(10_000000, "mbrn_denom")]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Delegate MBRN to governator
            let msg = ExecuteMsg::UpdateDelegations { 
                governator_addr: Some(String::from("governator_addr")), 
                mbrn_amount: Some(Uint128::new(5_000000)),
                delegate: Some(true), 
                fluid: None, 
                voting_power_delegation: None,
                commission: None,
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Skip fee waiting period & add staking rewards
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 30u64), //Added 30 days
                chain_id: app.block_info().chain_id,
            });

            //Update delegate commission
            let msg = ExecuteMsg::UpdateDelegations { 
                governator_addr: None,
                mbrn_amount: None,
                delegate: None,
                fluid: None, 
                voting_power_delegation: None,
                commission: Some(Decimal::percent(10)),
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("governator_addr"), cosmos_msg).unwrap();

            //DepositFees
            let msg = ExecuteMsg::DepositFee {  };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1000, "credit_fulldenom")]).unwrap();
            app.execute(Addr::unchecked("contract1"), cosmos_msg).unwrap();
                       
            //Assert User Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("user_1"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 1 as usize);
            assert_eq!(resp.claimables[0], Asset {
                amount: Uint128::new(931),
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });
            assert_eq!(resp.accrued_interest, Uint128::new(78082));

            //Assert Vesting Claims to find remaining claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("contract3"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 1 as usize);
            assert_eq!(resp.claimables[0], Asset {
                amount: Uint128::new(19),
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });
            assert_eq!(resp.accrued_interest, Uint128::new(0));

            //Skip fee waiting period & add staking rewards
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 30u64), //Added 30 days
                chain_id: app.block_info().chain_id,
            });
            
            //Claim for user
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Check that the rewards were sent
            assert_eq!(
                app.wrap().query_all_balances("user_1").unwrap(),
                vec![coin(931, "credit_fulldenom")]
            );  
            
            //Assert Delegate Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("governator_addr"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 1 as usize);
            assert_eq!(resp.claimables[0], Asset {
                amount: Uint128::new(49),
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });
            assert_eq!(resp.accrued_interest, Uint128::new(8219));
            
            //Skip fee waiting period 
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(1u64),
                chain_id: app.block_info().chain_id,
            });

            //Undelegate
            let msg = ExecuteMsg::UpdateDelegations { 
                governator_addr: Some(String::from("governator_addr")), 
                mbrn_amount: Some(Uint128::new(5_000000)),
                delegate: Some(false), 
                fluid: None, 
                voting_power_delegation: None,
                commission: None,
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            
            //Assert Delegate Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("governator_addr"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 1 as usize);
            assert_eq!(resp.claimables[0], Asset {
                amount: Uint128::new(49),
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });
            assert_eq!(resp.accrued_interest, Uint128::new(8219));

            //Skip fee waiting period 
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(1u64),
                chain_id: app.block_info().chain_id,
            });

            //Claim for delegate even though they are undelegated
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("governator_addr"), cosmos_msg).unwrap();

            //Check that the rewards were sent
            assert_eq!(
                app.wrap().query_all_balances("governator_addr").unwrap(),
                vec![coin(49, "credit_fulldenom")]
            );

            ////MBRN amount doesn't change for either bc they are restaked////
                
            //Claim: Assert claim was saved and can't be double claimed
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap_err();

            //Claim: Assert claim was saved and can't be double claimed
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("governator_addr"), cosmos_msg).unwrap_err();
           
        }

        //If delegqate gets all your rewards, make sure the math doesn't break
        #[test]
        fn overflow_commission_claims() {
            let (mut app, staking_contract, auction_contract) = proper_instantiate();

            //Stake MBRN as user
            let msg = ExecuteMsg::Stake { user: None };
            let cosmos_msg = staking_contract.call(msg, vec![coin(10_000000, "mbrn_denom")]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Delegate MBRN to governator
            let msg = ExecuteMsg::UpdateDelegations { 
                governator_addr: Some(String::from("governator_addr")), 
                mbrn_amount: Some(Uint128::new(9_000000)),
                delegate: Some(true), 
                fluid: None, 
                voting_power_delegation: None,
                commission: None,
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Skip fee waiting period & add staking rewards
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 30u64), //Added 30 days
                chain_id: app.block_info().chain_id,
            });

            //Update delegate commission
            let msg = ExecuteMsg::UpdateDelegations { 
                governator_addr: None,
                mbrn_amount: None,
                delegate: None,
                fluid: None, 
                voting_power_delegation: None,
                commission: Some(Decimal::percent(10)),
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("governator_addr"), cosmos_msg).unwrap();

            //DepositFees
            let msg = ExecuteMsg::DepositFee {  };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1000, "credit_fulldenom")]).unwrap();
            app.execute(Addr::unchecked("contract1"), cosmos_msg).unwrap();

            //Unstake 50%
            let msg = ExecuteMsg::Unstake { mbrn_amount: Some(Uint128::new(10_000000)) };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64), //Added a days
                chain_id: app.block_info().chain_id,
            });
                       
            //Assert User Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("user_1"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 0 as usize);
        }

        #[test]
        fn deposit_fee_and_claim() {
            let (mut app, staking_contract, auction_contract) = proper_instantiate();
            
            //DepositFee: Unauthorized
            let msg = ExecuteMsg::DepositFee {  };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(ADMIN), cosmos_msg).unwrap_err();

            //Stake MBRN as user
            let msg = ExecuteMsg::Stake { user: None };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1_000_000, "mbrn_denom")]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Pass 1 second to enable fees
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(1u64), 
                chain_id: app.block_info().chain_id,
            });

            //DepositFees
            let msg = ExecuteMsg::DepositFee {  };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1000, "credit_fulldenom"), coin(1000, "fee_asset")]).unwrap();
            app.execute(Addr::unchecked("contract1"), cosmos_msg).unwrap();

            //Check that the fee is deposited in the auction contract
            assert_eq!(
                app.wrap().query_all_balances(auction_contract.clone()).unwrap(),
                vec![coin(1000, "fee_asset")]
            );
            
            //DepositFee from Auction
            let msg = ExecuteMsg::DepositFee {  };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1000, "fee_asset")]).unwrap();
            app.execute(auction_contract, cosmos_msg).unwrap();

            //Assert FeeEventsResponse
            let resp: FeeEventsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::FeeEvents {
                        limit: None,
                        start_after: None,
                    },
                )
                .unwrap();
            assert_eq!(resp.fee_events, vec![
                FeeEvent {
                    time_of_event: 1571797420,
                    fee: LiqAsset {
                        info: AssetInfo::NativeToken {
                            denom: String::from("credit_fulldenom")
                        },
                        amount: Decimal::from_str("0.000833333333333333").unwrap(), 
                    },
                },
                FeeEvent {
                    time_of_event: 1571797420,
                    fee: LiqAsset {
                        info: AssetInfo::NativeToken {
                            denom: String::from("fee_asset")
                        },
                        amount: Decimal::from_str("0.000833333333333333").unwrap(), 
                    },
                }
            ]);

            //Add staking rewards
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 30u64), //Added 30 days
                chain_id: app.block_info().chain_id,
            });
            
            //No stake Error for ClaimRewards
            let msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("not_a_staker"), cosmos_msg).unwrap_err();

            //Assert User Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("user_1"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 2 as usize);
            assert_eq!(resp.accrued_interest, Uint128::new(8_219));

            //User stake before Restake
            let resp_before: StakerResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserStake { staker: String::from("user_1") },
                )
                .unwrap();

            //Claim && Restake
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: true,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Check that the rewards were sent
            assert_eq!(
                app.wrap().query_all_balances("user_1").unwrap(),
                vec![coin(833, "credit_fulldenom"), coin(833, "fee_asset"), coin(9_000_000, "mbrn_denom")]
            );
                
            //Assert Vesting Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("contract3"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 2 as usize);
            assert_eq!(resp.accrued_interest, Uint128::new(0));

            //Claim Vesting
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("contract3"), cosmos_msg).unwrap();

            //Check that the rewards were sent
            assert_eq!(
                app.wrap().query_all_balances("contract3").unwrap(),
                vec![coin(166, "credit_fulldenom"), coin(166, "fee_asset")]
            );

            //Claim: Assert claim was saved and can't be double claimed
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap_err();
            //Claim: Assert claim was saved and can't be double claimed
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("contract3"), cosmos_msg).unwrap_err();

            //Add staking rewards
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 30u64), //Added 30 days
                chain_id: app.block_info().chain_id,
            });

            //User stake after Restake
            let resp_after: StakerResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserStake { staker: String::from("user_1") },
                )
                .unwrap();

            //Assert that the stake was restaked
            assert_eq!(resp_before.total_staked + Uint128::new(8_219), resp_after.total_staked);

            //Trim fee events since all were used & claimed
            let msg = ExecuteMsg::TrimFeeEvents { };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(ADMIN), cosmos_msg).unwrap();
            //Query FeeEvents
            let resp: FeeEventsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::FeeEvents {
                        limit: None,
                        start_after: None,
                    },
                )
                .unwrap();
            assert_eq!(resp.fee_events.len(), 0 as usize);
        }

        #[test]
        fn vesting_claims_multiplier() {
            let (mut app, staking_contract, auction_contract) = proper_instantiate();

            //Stake MBRN as user
            let msg = ExecuteMsg::Stake { user: None };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1_000_000, "mbrn_denom")]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();
            
            //Add stake rewards & enable fees
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 30u64), //Added 30 days
                chain_id: app.block_info().chain_id,
            });

            //DepositFees
            let msg = ExecuteMsg::DepositFee {  };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1000, "credit_fulldenom"), coin(1000, "fee_asset")]).unwrap();
            app.execute(Addr::unchecked("contract1"), cosmos_msg).unwrap();

            //Update vesting multiplier without affecting the previous DepositFee
            let msg = ExecuteMsg::UpdateConfig { 
                owner: None,
                unstaking_period: None,
                osmosis_proxy: None,
                positions_contract: None,
                auction_contract: None,
                governance_contract: None,
                mbrn_denom: None,
                vesting_contract: None,
                incentive_schedule: None,
                max_commission_rate: None,
                keep_raw_cdt: None,
                vesting_rev_multiplier: Some(Decimal::percent(50)),
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(ADMIN), cosmos_msg).unwrap();

            //Claim && Restake
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: true,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Check that the rewards were sent
            assert_eq!(
                app.wrap().query_all_balances("user_1").unwrap(),
                vec![coin(833, "credit_fulldenom"), coin(9_000_000, "mbrn_denom")]
            );
                
            //Assert Vesting Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("contract3"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 1 as usize);
            assert_eq!(resp.accrued_interest, Uint128::new(0));

            //Claim Vesting
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("contract3"), cosmos_msg).unwrap();

            //Check that the rewards were sent
            assert_eq!(
                app.wrap().query_all_balances("contract3").unwrap(),
                vec![coin(166, "credit_fulldenom")]
            );

            //Add 1 second to enable fees
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(1u64),
                chain_id: app.block_info().chain_id,
            });


            //NOW THAT VESTING HAS CLAIMED, THE MULTIPLIER IS UPDATED
            //ROUND 2

            //DepositFees
            let msg = ExecuteMsg::DepositFee {  };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1000, "credit_fulldenom"), coin(1000, "fee_asset")]).unwrap();
            app.execute(Addr::unchecked("contract1"), cosmos_msg).unwrap();            

            //Update vesting multiplier without affecting the previous DepositFee
            let msg = ExecuteMsg::UpdateConfig { 
                owner: None,
                unstaking_period: None,
                osmosis_proxy: None,
                positions_contract: None,
                auction_contract: None,
                governance_contract: None,
                mbrn_denom: None,
                vesting_contract: None,
                incentive_schedule: None,
                max_commission_rate: None,
                keep_raw_cdt: None,
                vesting_rev_multiplier: Some(Decimal::zero()),
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(ADMIN), cosmos_msg).unwrap();

            //Skip fee waiting period + excess time
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 30u64), //Added 30 days
                chain_id: app.block_info().chain_id,
            });

            //Assert User Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("user_1"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables[0], Asset {
                amount: Uint128::new(663),
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });
            assert_eq!(resp.claimables[1], Asset {
                amount: Uint128::new(5),
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });

            //Assert Vesting Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("contract3"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 1 as usize);
            assert_eq!(resp.claimables[0], Asset {
                amount: Uint128::new(331),
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });

            //Claim Vesting
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("contract3"), cosmos_msg).unwrap();

            //ROUND 3
            //vesting multiplier at 0

            //DepositFees
            let msg = ExecuteMsg::DepositFee {  };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1000, "credit_fulldenom"), coin(1000, "fee_asset")]).unwrap();
            app.execute(Addr::unchecked("contract1"), cosmos_msg).unwrap();
            

            //Skip fee waiting period + excess time
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 30u64), //Added 30 days
                chain_id: app.block_info().chain_id,
            });

            //Assert User Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("user_1"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables.len(), 2 as usize);
            assert_eq!(resp.claimables[0], Asset {
                amount: Uint128::new(1654),  //- 668 = 986
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });
            assert_eq!(resp.claimables[1], Asset {
                amount: Uint128::new(13), //986 + 13 = 999
                info: AssetInfo::NativeToken { denom: String::from("credit_fulldenom") },
            });

            //Assert Vesting Claims
            let resp: RewardsResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::UserRewards {
                        user: String::from("contract3"),
                    }
                )
                .unwrap();
            assert_eq!(resp.claimables, vec![]);
            assert_eq!(resp.accrued_interest, Uint128::new(0));

            //Claim Vesting
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: false,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("contract3"), cosmos_msg).unwrap_err();
            

        }

        /// Do we have unstake tests here or in contract_tests that test multiple unstaking deposits?
        #[test]
        fn unstaking(){
            let (mut app, staking_contract, auction_contract) = proper_instantiate();

            //Stake MBRN as user
            let msg = ExecuteMsg::Stake { user: None };
            let cosmos_msg = staking_contract.call(msg, vec![coin(1_000_000, "mbrn_denom")]).unwrap();
            app.execute(Addr::unchecked("coin_God"), cosmos_msg).unwrap();
            
            //Stake MBRN as user
            let msg = ExecuteMsg::Stake { user: None };
            let cosmos_msg = staking_contract.call(msg, vec![coin(10_000_000, "mbrn_denom")]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Query and Assert totals
            let resp: TotalStakedResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::TotalStaked {},
                )
                .unwrap();
            assert_eq!(resp.total_not_including_vested, Uint128::new(11_000_000));
            assert_eq!(resp.vested_total, Uint128::new(0));

            //Not a staker Error
            let msg = ExecuteMsg::Unstake {
                mbrn_amount: Some(Uint128::new(1u128)),
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("not_a_staker"), cosmos_msg).unwrap_err();

            //Successful Unstake all w/o withdrawals
            let msg = ExecuteMsg::Unstake {
                mbrn_amount: None
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Successful Restake to reset the deposits
            let msg = ExecuteMsg::Restake {
                mbrn_amount: Uint128::new(10_000_000u128),
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();
            //Skip unstaking period
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 4u64), //Added 4 days
                chain_id: app.block_info().chain_id,
            });

            //Successful Unstake Some, no withdrawal
            let msg = ExecuteMsg::Unstake {
                mbrn_amount: Some(Uint128::new(4_500_000u128))
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();
            //Error: Unstake Some that is more than current staked or unstakable amount
            let msg = ExecuteMsg::Unstake {
                mbrn_amount: Some(Uint128::new(6_500_000u128))
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap_err();
            //Successful Unstake Some, no withdrawal
            let msg = ExecuteMsg::Unstake {
                mbrn_amount: Some(Uint128::new(4_500_000u128))
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Assert no withdrawal to assert the Restake
            assert_eq!(
                app.wrap().query_all_balances(Addr::unchecked("user_1")).unwrap(),
                vec![]
            );

            //Skip unstaking period
            app.set_block(BlockInfo {
                height: app.block_info().height,
                time: app.block_info().time.plus_seconds(86_400u64 * 4u64), //Added 4 days
                chain_id: app.block_info().chain_id,
            });
            
            //Claiming during an unstaking period doesn't reset the unstaking period
            let claim_msg = ExecuteMsg::ClaimRewards {
                send_to: None,
                restake: true,
            };
            let cosmos_msg = staking_contract.call(claim_msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap(); 


            //Successful Unstake all w/ withdrawal
            //Would error if:
            //- staking totals isn't updated when creating a position for the accrued_interest
            //- unstaking time was reset
            let msg = ExecuteMsg::Unstake {
                mbrn_amount: Some(Uint128::new(11_000_000u128))
            };
            let cosmos_msg = staking_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked("user_1"), cosmos_msg).unwrap();

            //Assert withdrawal 
            assert_eq!(
                app.wrap().query_all_balances(Addr::unchecked("user_1")).unwrap(),
                vec![coin(9000000, "mbrn_denom")]
            );

            //Query and Assert totals
            let resp: TotalStakedResponse = app
                .wrap()
                .query_wasm_smart(
                    staking_contract.addr(),
                    &QueryMsg::TotalStaked {},
                )
                .unwrap();
            assert_eq!(resp.total_not_including_vested, Uint128::new(1_000_000));//This is from coin_God
            assert_eq!(resp.vested_total, Uint128::new(0));

        }
    }
}
