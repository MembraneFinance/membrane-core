use cosmwasm_std::{Decimal, Uint128, Env, QuerierWrapper, Storage, StdResult, StdError, Addr};

use membrane::cdp::Config;
use membrane::stability_pool::QueryMsg as SP_QueryMsg;
use membrane::types::{Basket, Asset, cAsset, SupplyCap, AssetPool};
use membrane::helpers::{get_asset_liquidity, get_owner_liquidity_multiplier, get_stability_pool_liquidity};
use membrane::math::decimal_multiplication; 

use crate::rates::transform_caps_based_on_volatility;
use crate::state::{CONFIG, BASKET};
use crate::query::{get_cAsset_ratios, get_cAsset_ratios_imut};
use crate::error::ContractError;

/// Asserts that the assets provided are valid collateral assets in the basket
pub fn assert_basket_assets(
    storage: &mut dyn Storage,
    _querier: QuerierWrapper,
    _env: Env,
    assets: Vec<Asset>,
) -> Result<Vec<cAsset>, ContractError> {
    let basket: Basket = BASKET.load(storage)?;

    //Checking if Assets for the position are available collateral assets in the basket
    let collateral_assets = assets
        .into_iter()
        .map(|asset| {
            let cAsset = basket
                .collateral_types
                .iter()
                .find(|cAsset| cAsset.asset.info.equal(&asset.info))
                .ok_or(ContractError::InvalidCollateral {})?;
            Ok(cAsset {
                asset: asset.clone(),
                ..cAsset.clone()
            })
        })
        .collect::<Result<Vec<cAsset>, ContractError>>()?;   

    Ok(collateral_assets)
}

/// Update SupplyCap objects in Basket 
pub fn update_basket_tally(
    storage: &mut dyn Storage,
    querier: QuerierWrapper,
    env: Env,
    basket: &mut Basket,
    collateral_assets: Vec<cAsset>,
    full_positions_assets: Vec<cAsset>,
    add_to_cAsset: bool,
    config: Config,
    from_liquidation: bool,
) -> Result<(), ContractError> {    
    //Update SupplyCap objects 
    for cAsset in collateral_assets.clone() {
        if let Some((index, mut cap)) = basket.clone().collateral_supply_caps
            .into_iter()
            .enumerate()
            .find(|(_x, cap)| cap.asset_info.equal(&cAsset.asset.info))
        {
            if add_to_cAsset {
                cap.current_supply += cAsset.asset.amount;
            } else {                
                cap.current_supply = match cap.current_supply.checked_sub(cAsset.asset.amount){
                    Ok(diff) => diff,
                    Err(_) => Uint128::zero(),
                }; 
            }

            //Update
            basket.collateral_supply_caps[index] = cap.clone();
            basket.collateral_types[index].asset.amount = cap.current_supply;
        }    
    }

    //Transform supply caps based on asset volatility
    //This doesn't alter multi-asset caps
    let supply_caps = match transform_caps_based_on_volatility(storage, basket.clone()){
        Ok(supply_caps) => supply_caps,
        Err(_err) => basket.clone().collateral_supply_caps
    };
    
    if !from_liquidation {
        let (new_basket_ratios, _) =
            get_cAsset_ratios(storage, env, querier, basket.clone().collateral_types, config, Some(basket.clone()))?;

        
        //Assert new ratios aren't above Collateral Supply Caps. If so, conditionally error.
        for (i, ratio) in new_basket_ratios.clone().into_iter().enumerate() {
            //Initialize in_position to check if the position has these assets
            let mut in_position = false;
            
            if add_to_cAsset {
                //Check if the depositing assets are part of this cap
                if let Some((_i, _cAsset)) = collateral_assets.clone().into_iter().enumerate().find(|(_i, cAsset)| cAsset.asset.info.equal(&supply_caps[i].asset_info)){
                    in_position = true;
                }
            } else {
                //Check if the position has these assets if ur withdrawing
                //So if a withdrawal would push an asset over cap that isn't being withdrawn currently but is in the position, it errors
                if let Some((_i, _cAsset)) = full_positions_assets.clone().into_iter().enumerate().find(|(_i, cAsset)| cAsset.asset.info.equal(&supply_caps[i].asset_info)){
                    in_position = true;
                }
                //If the position is withdrawing the asset, set to false.
                //User Flow: If a user fully withdraws an asset that is over cap BUT....
                //..doesn't completely pull it under cap, we don't want to block withdrawals
                if let Some((_i, _withdrawn_cAsset)) = collateral_assets.clone().into_iter().enumerate().find(|(_i, cAsset)| cAsset.asset.info.equal(&supply_caps[i].asset_info)){
                    //Check if its being fully withdrawn from the position or if its the only asset in the position
                    if let Some((_i, _position_cAsset)) = full_positions_assets.clone().into_iter().enumerate().find(|(_i, cAsset)| cAsset.asset.info.equal(&supply_caps[i].asset_info)){
                        //If the asset is still in the position, it must be the only remaining asset
                        if full_positions_assets.len() > 1 {
                            in_position = true;                     
                        } else {
                            //You can withdraw the only asset freely
                            in_position = false;
                        }
                    } else {
                        //This means the asset was fully withdrawn
                        in_position = false;
                    }
                    
                }
            }

            if basket.collateral_supply_caps != vec![] && ratio > supply_caps[i].supply_cap_ratio && in_position {
                
                return Err(ContractError::CustomError {
                    val: format!(
                        "Supply cap ratio for {} is over the limit ({} > {})",
                        basket.collateral_supply_caps[i].asset_info,
                        ratio,
                        supply_caps[i].supply_cap_ratio
                    ),
                });            
            }
        }

        //Assert for Multi-asset caps as well
        if basket.multi_asset_supply_caps != vec![]{
            for multi_asset_cap in basket.clone().multi_asset_supply_caps {

                //Initialize total_ratio
                let mut total_ratio = Decimal::zero();
                //Initialize in_position to check if the position has these assets
                let mut in_position = false;
                
                //Find & add ratio for each asset
                for asset in multi_asset_cap.clone().assets {
                    if let Some((i, _cap)) = basket.clone().collateral_supply_caps.into_iter().enumerate().find(|(_i, cap)| cap.asset_info.equal(&asset)){
                        total_ratio += new_basket_ratios[i];
                    }
                    if add_to_cAsset {
                        //Check if the depositing assets are part of this cap
                        if let Some((_i, _cAsset)) = collateral_assets.clone().into_iter().enumerate().find(|(_i, cAsset)| cAsset.asset.info.equal(&asset)){
                            in_position = true;
                        }
                    } else {
                        //Check if the position has these assets if ur withdrawing
                        //So if a withdrawal would push an asset over cap, it errors
                        if let Some((_i, _cAsset)) = full_positions_assets.clone().into_iter().enumerate().find(|(_i, cAsset)| cAsset.asset.info.equal(&asset)){
                            in_position = true;
                        }
                        //If the position is withdrawing the asset, set to false.
                        //User Flow: If a user fully withdraws an asset that is over cap BUT....
                        //..doesn't completely pull it under cap, we don't want to block withdrawals
                        if let Some((_i, _withdrawn_cAsset)) = collateral_assets.clone().into_iter().enumerate().find(|(_i, cAsset)| cAsset.asset.info.equal(&asset)){
                            //Check if its being fully withdrawn from the position or if its the only asset in the position
                            if let Some((_i, _position_cAsset)) = full_positions_assets.clone().into_iter().enumerate().find(|(_i, cAsset)| cAsset.asset.info.equal(&asset)){
                                //If the asset is still in the position, it must be the only remaining asset
                                if full_positions_assets.len() > 1 {
                                    in_position = true;                        
                                } else {
                                    //You can withdraw the only asset freely
                                    in_position = false;
                                }
                            } else {
                                //This means the asset was fully withdrawn
                                in_position = false;
                            }
                            
                        }
                    }
                }
                                
                //Error if over cap
                if total_ratio > multi_asset_cap.supply_cap_ratio && in_position {
                    return Err(ContractError::CustomError {
                        val: format!(
                            "Multi-Asset supply cap ratio for {:?} is over the limit ({} > {})",
                            multi_asset_cap.assets,
                            total_ratio,
                            multi_asset_cap.supply_cap_ratio,
                        ),
                    });
                }

            }
        }
    }

    Ok(())
}