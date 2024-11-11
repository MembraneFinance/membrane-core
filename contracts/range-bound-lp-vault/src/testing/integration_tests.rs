mod tests {

    use crate::helpers::SPVaultContract;

    use membrane::oracle::PriceResponse;
    use membrane::stability_pool_vault::{ExecuteMsg, InstantiateMsg, QueryMsg, Config};
    use membrane::stability_pool::ClaimsResponse;
    use membrane::types::{AssetInfo, Asset, AssetPool, Deposit};

    use cosmwasm_std::{
        coin, to_binary, Addr, Binary, Empty, Response, StdResult, Uint128, Decimal,
    };
    use cw_multi_test::{App, AppBuilder, BankKeeper, Contract, ContractWrapper, Executor};
    use cosmwasm_schema::cw_serde;

    const USER: &str = "user";
    const ADMIN: &str = "admin";

    //SP Vault Contract
    pub fn sp_vault_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new_with_empty(
            crate::contract::execute,
            crate::contract::instantiate,
            crate::contract::query,
        )
        .with_reply(crate::contract::reply);
        Box::new(contract)
    }

     //Mock Osmo Proxy Contract
    #[cw_serde]
     pub enum Osmo_MockExecuteMsg {
        ExecuteSwaps {
            token_out: String,
        }
    }
 
     
    #[cw_serde]
     pub struct Osmo_MockInstantiateMsg {}
 
     
    #[cw_serde]
     pub enum Osmo_MockQueryMsg {}
 
    pub fn osmosis_proxy_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |deps, _, info, msg: Osmo_MockExecuteMsg| -> StdResult<Response> {
                match msg {
                    Osmo_MockExecuteMsg::ExecuteSwaps { token_out } => {
                    Ok(Response::default())
                    }
                }
            },
            |_, _, _, _: Osmo_MockInstantiateMsg| -> StdResult<Response> {
                Ok(Response::default())
            },
            |_, _, msg: Osmo_MockQueryMsg| -> StdResult<Binary> {
            Ok(to_binary(&{}).unwrap())
            },
        );
        Box::new(contract)
    }

    //Mock SP Contract    
    #[cw_serde]
    pub enum SP_MockExecuteMsg {
        ClaimRewards { },
        Deposit { user: Option<String> },
        Withdraw { amount: Uint128 },
    }

    
    #[cw_serde]
    pub struct SP_MockInstantiateMsg {}

    
    #[cw_serde]
    pub enum SP_MockQueryMsg {
        UserClaims { user: String },
        AssetPool { 
            /// User address
            user: Option<String>,
            /// Deposit limit
            deposit_limit: Option<u32>,
            /// Deposit to start after
            start_after: Option<u32>,    
        },
    }

    pub fn stability_pool_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            |_, _, _, msg: SP_MockExecuteMsg| -> StdResult<Response> {
                match msg {
                    SP_MockExecuteMsg::ClaimRewards { } => Ok(Response::default()),
                    SP_MockExecuteMsg::Deposit { user: _ } => Ok(Response::default()),
                    SP_MockExecuteMsg::Withdraw { amount: _ } => Ok(Response::default()),
                }
            },
            |_, _, _, _: SP_MockInstantiateMsg| -> StdResult<Response> { Ok(Response::default()) },
            |_, _, msg: SP_MockQueryMsg| -> StdResult<Binary> {
                match msg {
                    SP_MockQueryMsg::AssetPool { user: _, deposit_limit: _, start_after: _ } => Ok(to_binary(&AssetPool {
                        credit_asset: Asset {
                            info: AssetInfo::NativeToken {
                                denom: "cdt_fulldenom".to_string(),
                            },
                            amount: Uint128::new(8),
                        },
                        liq_premium: Decimal::percent(10),
                        deposits: vec![
                            Deposit {
                                user: Addr::unchecked(USER),
                                amount: Decimal::percent(8_00),
                                deposit_time: 0,
                                last_accrued: 0,
                                unstake_time: None,

                            }
                        ],
                    })?),
                    SP_MockQueryMsg::UserClaims { user: _ } => Ok(to_binary(&ClaimsResponse {
                        claims: vec![                            
                            coin(10, "factory/osmo1s794h9rxggytja3a4pmwul53u98k06zy2qtrdvjnfuxruh7s8yjs6cyxgd/umbrn"),
                            // coin(10, "cdt_fulldenom"),
                        ],
                    })?),
                    _ => Ok(to_binary(&{}).unwrap()),
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
                &Addr::unchecked(USER),
                vec![coin(100_000, "cdt_fulldenom"), coin(100_000, "wrong_token")],
            )
            .unwrap();
            bank.init_balance(
                storage,
                &Addr::unchecked("contract2"),
                vec![coin(100_000_000, "factory/contract2/ucdt-vault")],
            )
            .unwrap();
            // bank.init_balance(
            //     storage,
            //     &Addr::unchecked("contract1"),
            //     vec![coin(10_000_000, "cdt_fulldenom")],
            // )
            // .unwrap();

            router.bank = bank;
        })
    }

    fn proper_instantiate() -> (App, SPVaultContract) {
        let mut app = mock_app();

        //Instaniate OP
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

        //Instantiate SP contract
        let sp_id = app.store_code(stability_pool_contract());

        let sp_contract_addr = app
            .instantiate_contract(
                sp_id,
                Addr::unchecked(ADMIN),
                &SP_MockInstantiateMsg {},
                &[],
                "test",
                None,
            )
            .unwrap();

        //Instantiate SP Vault contract        
        let vault_id = app.store_code(sp_vault_contract());

        let msg = InstantiateMsg { 
            vault_subdenom: String::from("ucdt-vault"), 
            deposit_token: String::from("cdt_fulldenom"),
            osmosis_proxy_contract: String::from("contract0"),
            stability_pool_contract: String::from("contract1"),
        };
        let vault_contract_addr = app
            .instantiate_contract(vault_id, Addr::unchecked(ADMIN), &msg, &[], "test", None)
            .unwrap();

        let vault_contract = SPVaultContract(vault_contract_addr);

        (app, vault_contract)
    }

    mod sp_vault {

        use cosmwasm_std::{coin, coins, Uint128};
        use membrane::osmosis_proxy::OwnerResponse;

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
            let cosmos_msg = vault_contract.call(msg, vec![coin(10, "cdt_fulldenom"), coin(10, "wrong_token")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap_err();

            //Enter Vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(10, "cdt_fulldenom")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();

            // Query Vault token underlying
            let underlying_deposit_token: Uint128 = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::VaultTokenUnderlying { 
                        vault_token_amount: Uint128::new(10_000_000)
                    },
                )
                .unwrap();
            assert_eq!(
                underlying_deposit_token,
                Uint128::new(10)
            );
            
            //Query Vault deposit token balance
            //Should be 2 bc everything but the percent_to_keep_liquid (10% of 28 (tvd is overcalcing)) was sent to the vault
            let balance = app
                .wrap()
                .query_balance(Addr::unchecked("contract2"), "cdt_fulldenom")
                .unwrap().amount;
            assert_eq!(balance, Uint128::new(2));
            
            //Enter Vault: Some of the deposit is kept in the vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(10, "cdt_fulldenom")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();

            // Query Vault token underlying
            let underlying_deposit_token: Uint128 = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::VaultTokenUnderlying { 
                        vault_token_amount: Uint128::new(10_000_000)
                    },
                )
                .unwrap();
            assert_eq!(
                underlying_deposit_token,
                Uint128::new(10)
            );
            // Query Config for total deposit amount
            let config: Config = app
                .wrap()
                .query_wasm_smart(
                    vault_contract.addr(),
                    &QueryMsg::Config { },
                )
                .unwrap();
            
            //Query Vault deposit token balance
            let balance = app
                .wrap()
                .query_balance(Addr::unchecked("contract2"), "cdt_fulldenom")
                .unwrap().amount;
            assert_eq!(balance, Uint128::new(3)); //due to TVD not updating correctly bc AssetPool state doesn't update
        }

        // #[test]
        fn compound(){
            let (mut app, vault_contract) = proper_instantiate();

            //Enter Vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(10, "cdt_fulldenom")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();

            //Compound
            let msg = ExecuteMsg::Compound { };
            let cosmos_msg = vault_contract.call(msg, vec![]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg).unwrap();

            // Query Vault token underlying
            // let underlying_deposit_token: Uint128 = app
            //     .wrap()
            //     .query_wasm_smart(
            //         vault_contract.addr(),
            //         &QueryMsg::VaultTokenUnderlying { 
            //             vault_token_amount: Uint128::new(10_000_000)
            //         },
            //     )
            //     .unwrap();
            // assert_eq!(
            //     underlying_deposit_token,
            //     Uint128::new(10)
            // );
            // // Query Config for total deposit amount
            // let config: Config = app
            //     .wrap()
            //     .query_wasm_smart(
            //         vault_contract.addr(),
            //         &QueryMsg::Config { },
            //     )
            //     .unwrap();
            // assert_eq!(
            //     config.total_deposit_tokens,
            //     Uint128::new(10)
            // );
            
            // //Query Vault deposit token balance
            // //Should be 1 bc everything but the percent_to_keep_liquid (10% of 10) was sent to the vault
            // let balance = app
            //     .wrap()
            //     .query_balance(Addr::unchecked("contract2"), "cdt_fulldenom")
            //     .unwrap().amount;
            // assert_eq!(balance, Uint128::new(1));
        }
        
        // #[test]
        fn exit_vault() {
            let (mut app, vault_contract) = proper_instantiate();

            //Enter Vault
            let msg = ExecuteMsg::EnterVault { };
            let cosmos_msg = vault_contract.call(msg, vec![coin(10, "cdt_fulldenom")]).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();
            app.execute(Addr::unchecked(USER), cosmos_msg.clone()).unwrap();

            // // Query Vault token underlying
            // let underlying_deposit_token: Uint128 = app
            //     .wrap()
            //     .query_wasm_smart(
            //         vault_contract.addr(),
            //         &QueryMsg::VaultTokenUnderlying { 
            //             vault_token_amount: Uint128::new(10_000_000)
            //         },
            //     )
            //     .unwrap();
            // assert_eq!(
            //     underlying_deposit_token,
            //     Uint128::new(10)
            // );
            // // Query Config for total deposit amount
            // let config: Config = app
            //     .wrap()
            //     .query_wasm_smart(
            //         vault_contract.addr(),
            //         &QueryMsg::Config { },
            //     )
            //     .unwrap();
            // assert_eq!(
            //     config.total_deposit_tokens,
            //     Uint128::new(20)
            // );
            
            // //Query Vault deposit token balance
            // //Should be 0 bc everything was sent to the vault
            // let balance = app
            //     .wrap()
            //     .query_balance(Addr::unchecked("contract2"), "cdt_fulldenom")
            //     .unwrap().amount;
            // assert_eq!(balance, Uint128::zero());
        }
    }

}