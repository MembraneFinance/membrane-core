#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use cosmwasm_std::testing::{
        mock_env, mock_info, mock_dependencies, MOCK_CONTRACT_ADDR,
    };
    use cosmwasm_std::{
        coins, from_binary, Uint128, StdError, Addr, Decimal
    };
    use membrane::math::decimal_multiplication;
    use membrane::types::Owner;

    use membrane::osmosis_proxy::{InstantiateMsg, ExecuteMsg, QueryMsg, Config};

    use crate::TokenFactoryError;
    use crate::contract::{instantiate, execute, query};

    const DENOM_NAME: &str = "mydenom";
    const DENOM_PREFIX: &str = "factory";
   
    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies();
        panic!("{:?}", (decimal_multiplication(Decimal::from_str(&"1994647108.168933398069024127").unwrap(), Decimal::percent(50)).unwrap()
        * Uint128::new(10u64.pow(18 as u32) as u128)).to_string() );

        // let msg = InstantiateMsg {};
        // let info = mock_info("creator", &coins(1000, "uosmo"));

        // let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        // assert_eq!(0, res.messages.len());
    }

    // #[test]
    // fn update_config() {
    //     let mut deps = mock_dependencies();

    //     let msg = InstantiateMsg {};
    //     let info = mock_info("creator", &coins(1000, "uosmo"));

    //     let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    //     let msg = ExecuteMsg::UpdateConfig { 
    //         owners: Some(vec![ Owner {
    //             owner: Addr::unchecked("new_owner2"),
    //             total_minted: Uint128::zero(),
    //             stability_pool_ratio: Some(Decimal::zero()),
    //             non_token_contract_auth: true, 
    //             is_position_contract: false,
    //         },
    //         Owner {
    //             owner: Addr::unchecked("new_owner"),
    //             total_minted: Uint128::zero(),
    //             stability_pool_ratio: Some(Decimal::zero()),
    //             non_token_contract_auth: true, 
    //             is_position_contract: false,
    //         }]),
    //         liquidity_multiplier: Some(Decimal::zero()),
    //         add_owner: Some(true), 
    //         debt_auction: Some(String::from("debt_auction")),
    //         positions_contract: Some(String::from("positions_contract")),
    //         liquidity_contract: Some(String::from("liquidity_contract")),
    //         oracle_contract: Some(String::from("oracle_contract")),
    //     };
    //     let info = mock_info("creator", &coins(2, "token"));
    //     execute(deps.as_mut(), mock_env(), info, msg).unwrap();       

    //     //Set expected_config
    //     let expected_config = Config {
    //         owners: vec![ Owner {
    //             owner: Addr::unchecked("creator"),
    //             total_minted: Uint128::zero(),
    //             stability_pool_ratio: Some(Decimal::zero()),
    //             non_token_contract_auth: true, 
    //             is_position_contract: false,
    //         },
    //         Owner {
    //             owner: Addr::unchecked("new_owner2"),
    //             total_minted: Uint128::zero(),
    //             stability_pool_ratio: Some(Decimal::zero()),
    //             non_token_contract_auth: true, 
    //             is_position_contract: false,
    //         },
    //         Owner {
    //             owner: Addr::unchecked("new_owner"),
    //             total_minted: Uint128::zero(),
    //             stability_pool_ratio: Some(Decimal::zero()),
    //             non_token_contract_auth: true, 
    //             is_position_contract: false,
    //         }],
    //         liquidity_multiplier: Some(Decimal::zero()),
    //         debt_auction: Some(Addr::unchecked("debt_auction")),
    //         positions_contract: Some(Addr::unchecked("positions_contract")),
    //         liquidity_contract: Some(Addr::unchecked("liquidity_contract")),
    //         oracle_contract: Some(Addr::unchecked("oracle_contract")),
    //     };

    //     let response = query(deps.as_ref(), mock_env(), QueryMsg::Config {  }).unwrap();
    //     let config: Config = from_binary(&response).unwrap();
    //     assert_eq!(
    //         config,
    //         expected_config
    //     );

    // }

    // #[test]
    // fn msg_create_denom_invalid_subdenom() {
    //     let mut deps = mock_dependencies();

    //     let msg = InstantiateMsg {};
    //     let info = mock_info("creator", &coins(1000, "uosmo"));
    //     let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        

    //     let subdenom: String = String::from("");

    //     let msg = ExecuteMsg::CreateDenom { 
    //         subdenom, 
    //         max_supply: Some(Uint128::new(10)), 
    //     };
    //     let info = mock_info("creator", &coins(2, "token"));
    //     let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    //     assert_eq!(
    //         TokenFactoryError::InvalidSubdenom {
    //             subdenom: String::from("")
    //         },
    //         err
    //     );
    // }

    // #[test]
    // fn msg_change_admin_empty_address() {
    //     let mut deps = mock_dependencies();

    //     let msg = InstantiateMsg {};
    //     let info = mock_info("creator", &coins(1000, "uosmo"));

    //     let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        

    //     const EMPTY_ADDR: &str = "";

    //     let info = mock_info("creator", &coins(2, "token"));

    //     let msg = ExecuteMsg::ChangeAdmin {
    //         denom: String::from(DENOM_NAME),
    //         new_admin_address: String::from(EMPTY_ADDR),
    //     };
    //     let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    //     match err {
    //         TokenFactoryError::Std(StdError::GenericErr { msg, .. }) => {
    //             assert!(msg.contains("human address too short"))
    //         }
    //         e => panic!("Unexpected error: {:?}", e),
    //     }
    // }

    // #[test]
    // fn msg_change_admin_invalid_denom() {
    //     let mut deps = mock_dependencies();

    //     let msg = InstantiateMsg {};
    //     let info = mock_info("creator", &coins(1000, "uosmo"));

    //     let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        

    //     const NEW_ADMIN_ADDR: &str = "newadmin";

    //     let info = mock_info("creator", &coins(2, "token"));

    //     // too many parts in denom
    //     let full_denom_name: &str = &format!(
    //         "{}/{}/{}/invalid",
    //         DENOM_PREFIX, MOCK_CONTRACT_ADDR, DENOM_NAME
    //     )[..];

    //     let msg = ExecuteMsg::ChangeAdmin {
    //         denom: String::from(full_denom_name),
    //         new_admin_address: String::from(NEW_ADMIN_ADDR),
    //     };
    //     let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();

    //     let expected_error = TokenFactoryError::InvalidDenom {
    //         denom: String::from(full_denom_name),
    //         message: String::from("denom must have 3 parts separated by /, had 4"),
    //     };

    //     assert_eq!(expected_error, err);
    // }


    // #[test]
    // fn msg_mint_invalid_denom() {
    //     let mut deps = mock_dependencies();

    //     let msg = InstantiateMsg {};
    //     let info = mock_info("creator", &coins(1000, "uosmo"));
    //     let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        
    //     const NEW_ADMIN_ADDR: &str = "newadmin";

    //     let mint_amount = Uint128::new(100_u128);

    //     let info = mock_info("creator", &coins(2, "token"));

    //     let full_denom_name: &str = &format!("{}/{}", DENOM_PREFIX, MOCK_CONTRACT_ADDR)[..];
    //     let msg = ExecuteMsg::MintTokens {
    //         denom: String::from(full_denom_name),
    //         amount: mint_amount,
    //         mint_to_address: String::from(NEW_ADMIN_ADDR),
    //     };
    //     let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    //     let expected_error = TokenFactoryError::InvalidDenom {
    //         denom: String::from(full_denom_name),
    //         message: String::from("denom must have 3 parts separated by /, had 2"),
    //     };

    //     assert_eq!(expected_error, err);
    // }

    // #[test]
    // fn mint_limits() {
    //     let mut deps = mock_dependencies();

    //     let msg = InstantiateMsg {};
    //     let info = mock_info("creator", &vec![]);
    //     let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    //     const NEW_ADMIN_ADDR: &str = "newadmin";
    //     let mint_amount = Uint128::new(100_u128);
    //     let info = mock_info("creator", &vec![]);
    //     let full_denom_name: &str = &format!("{}/{}/addr", DENOM_PREFIX, DENOM_NAME);

    //     //Successful Mint
    //     let msg = ExecuteMsg::MintTokens {
    //         denom: String::from(full_denom_name),
    //         amount: mint_amount,
    //         mint_to_address: String::from(NEW_ADMIN_ADDR),
    //     };

    //     let res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();
    //     assert_eq!(res.attributes[3].value, String::from("100"));

    //     //Minting more than max supply: Error
    //     let msg = ExecuteMsg::MintTokens {
    //         denom: String::from(full_denom_name),
    //         amount: mint_amount,
    //         mint_to_address: String::from(NEW_ADMIN_ADDR),
    //     };

    //     let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    // }

}