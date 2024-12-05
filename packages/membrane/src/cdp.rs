use cosmwasm_std::{Addr, Decimal, Uint128, StdResult, Api, StdError};
use cosmwasm_schema::cw_serde;

use crate::types::{
    cAsset, Asset, AssetInfo, InsolventPosition, RevenueDestination,
    SupplyCap, MultiAssetSupplyCap, TWAPPoolInfo, UserInfo, PoolType, Basket, equal, PremiumInfo,
};

#[cw_serde]
pub struct InstantiateMsg {
    /// Contract Owner
    pub owner: Option<String>,
    /// Seconds until oracle failure is accepted
    pub oracle_time_limit: u64, 
    /// Minimum debt value per position to ensure liquidatibility 
    pub debt_minimum: Uint128, 
    /// Protocol liquidation fee to restrict self liquidations
    pub liq_fee: Decimal,
    /// Timeframe for Collateral TWAPs in minutes
    pub collateral_twap_timeframe: u64, 
    /// Timeframe for Credit TWAP in minutes
    pub credit_twap_timeframe: u64,    
    /// Interest rate slope multiplier
    pub rate_slope_multiplier: Decimal, 
    /// Base debt cap multiplier
    pub base_debt_cap_multiplier: Uint128,
    /// Stability Pool contract
    pub stability_pool: Option<String>,
    /// Apollo DEX Router contract
    pub dex_router: Option<String>,
    /// MBRN Staking contract
    pub staking_contract: Option<String>,
    /// Oracle contract
    pub oracle_contract: Option<String>,
    /// Osmosis Proxy contract
    pub osmosis_proxy: Option<String>,
    /// Debt Auction contract
    pub debt_auction: Option<String>,
    /// Liquidity Check contract
    pub liquidity_contract: Option<String>,
    /// System Discounts contract    
    pub discounts_contract: Option<String>,
    /// Basket Creation struct
    pub create_basket: CreateBasket,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Update the contract config
    UpdateConfig(UpdateConfig),
    /// Deposit collateral into a Position
    Deposit {
        /// Position ID to deposit into.
        /// If the user wants to create a new/separate position, no position id is passed.
        position_id: Option<Uint128>, 
        /// Position owner.
        /// Defaults to the sender.
        position_owner: Option<String>,
    },
    /// Increase debt of a Position
    IncreaseDebt {
        /// Position ID to increase debt of
        position_id: Uint128,
        /// Amount of debt to increase
        amount: Option<Uint128>,
        /// LTV to borrow up to
        LTV: Option<Decimal>,
        /// Mint debt tokens to this address
        mint_to_addr: Option<String>,
    },
    /// Withdraw collateral from a Position
    Withdraw {
        /// Position ID to withdraw from
        position_id: Uint128,
        /// Asset to withdraw
        assets: Vec<Asset>,
        /// Send withdrawn assets to this address if not the sender
        send_to: Option<String>,
    },
    /// Repay debt of a Position
    Repay {
        /// Position ID to repay debt of
        position_id: Uint128,
        /// Position owner to repay debt of if not the sender
        position_owner: Option<String>, 
        /// Send excess assets to this address if not the sender
        send_excess_to: Option<String>, 
    },
    /// Repay message for the Stability Pool during liquidations
    LiqRepay {},
    /// Liquidate a Position
    Liquidate {
        /// Position ID to liquidate
        position_id: Uint128,
        /// Position owner to liquidate
        position_owner: String,
    },
    /// Redeem CDT for collateral
    /// Redemption limit based on Position owner buy-in
    RedeemCollateral {
        /// Max % premium on the redeemed collateral`
        max_collateral_premium: Option<u128>,
    },
    /// Edit Redeemability for owned Positions
    EditRedeemability {
        /// Position IDs to edit
        position_ids: Vec<Uint128>,
        /// Add or remove redeemability
        redeemable: Option<bool>,
        /// Edit premium on the redeemed collateral.
        /// Can't set a 100% premium, as that would be a free loan repayment.
        premium: Option<u128>,
        /// Edit Max loan repayment %
        max_loan_repayment: Option<Decimal>,
        /// Restricted collateral assets.
        /// These are restricted from use in redemptions.
        /// Swaps the full list.
        restricted_collateral_assets: Option<Vec<String>>,
    },
    /// Accrue interest for a Position
    Accrue { 
        /// Positon owner to accrue interest for, defaults to sender
        position_owner: Option<String>, 
        /// Positon ID to accrue interest for
        position_ids: Vec<Uint128>
    },
    // Close a Position by selling collateral and repaying debt
    // ClosePosition {
    //     /// Position ID to close
    //     position_id: Uint128,
    //     /// Max spread for the sale of collateral
    //     max_spread: Decimal,
    //     /// Send excess assets to this address if not the Position owner
    //     send_to: Option<String>,
    // },
    /// Edit the contract's Basket
    EditBasket(EditBasket),
    /// Edit a cAsset in the contract's Basket
    EditcAsset {
        /// cAsset to edit
        asset: AssetInfo,
        /// Max users can borrow up to
        max_borrow_LTV: Option<Decimal>, 
        /// Point of liquidation
        max_LTV: Option<Decimal>,
    },
    //Callbacks; Only callable by the contract
    Callback(CallbackMsg),
}


/// Note: Since CallbackMsg are always sent by the contract itself, we assume all types are already
/// validated and don't do additional checks. E.g. user addresses are Addr instead of String
#[cw_serde]
pub enum CallbackMsg {
    /// Bad debt check post liquidation
    BadDebtCheck {
        /// Position ID to check
        position_id: Uint128,
        /// Position owner to check
        position_owner: Addr,
    },
}


#[cw_serde]
pub enum QueryMsg {
    /// Returns the contract's config
    Config {},
    /// Get Basket redeemability
    GetBasketRedeemability {
        /// Position owner to query.
        position_owner: Option<String>,
        /// Premium to start after 
        start_after: Option<u128>,
        /// Response limiter
        limit: Option<u32>,
    },
    /// Returns Positions in the contract's Basket
    GetBasketPositions {
        /// Start after this user address
        start_after: Option<String>,
        /// Response limiter
        limit: Option<u32>,
        /// Single position
        user_info: Option<UserInfo>,
        /// Single user
        user: Option<String>,
    },
    /// Returns the contract's Basket
    GetBasket { }, 
    /// Returns credit redemption rate
    GetCreditRate { },
    /// Returns Basket collateral interest rates
    GetCollateralInterest { },
    /// Simulate LTV mint for a Position
    SimulateMint {
        /// Position ID to simulate mint for
        position_info: UserInfo,
        /// LTV to mint to
        LTV: Decimal,
    },
    // Returns insolvency status of a Position
    // GetPositionInsolvency {
    //     /// Position ID to query
    //     position_id: Uint128,
    //     /// Position owner to query
    //     position_owner: String,
    // },
    // Used internally to test state propagation
    // Propagation {},
}

#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Stability Pool contract address
    pub stability_pool: Option<Addr>,
    /// Apollo DEX router contract address.
    /// Note: Will need to change msg types if the router provider changes.
    /// Unused. The contract uses the Osmosis Proxy for swaps.
    pub dex_router: Option<Addr>,
    /// Staking contract address
    pub staking_contract: Option<Addr>,
    /// Osmosis Proxy contract address
    pub osmosis_proxy: Option<Addr>,
    /// Debt auction contract address
    pub debt_auction: Option<Addr>,
    /// Oracle contract address
    pub oracle_contract: Option<Addr>,
    /// Liquidity Check contract address
    pub liquidity_contract: Option<Addr>,
    /// System Discounts contract address
    pub discounts_contract: Option<Addr>,
    /// Liquidation fee as percent
    pub liq_fee: Decimal,
    /// Collateral TWAP time frame in minutes
    pub collateral_twap_timeframe: u64, 
    /// Credit TWAP time frame in minutes
    pub credit_twap_timeframe: u64,
    /// Seconds until oracle failure is accepted. Think of it as how many blocks you allow the oracle to fail for.
    pub oracle_time_limit: u64, 
    /// Augment the rate of increase per % difference for the redemption rate
    pub cpc_multiplier: Decimal,
    /// Debt minimum value per position.
    /// This needs to be large enough so that USDC positions are profitable to liquidate.
    //1-2% of liquidated debt (max -> borrow_LTV) needs to be more than gas fees assuming ~96% LTV.
    pub debt_minimum: Uint128, 
    /// Debt minimum multiplier for base debt cap.
    /// How many users do we want at 0 credit liquidity?
    pub base_debt_cap_multiplier: Uint128,
    /// Interest rate 2nd Slope multiplier
    pub rate_slope_multiplier: Decimal,
    /// Rate hike rate
    pub rate_hike_rate: Option<Decimal>,
    /// Redemption Fee
    //This is only optional for backwards compatibility & should never be None as we do a bare unwrap() call in redeem_for_collateral()
    pub redemption_fee: Option<Decimal>, 
}


/// Create the contract's Basket
#[cw_serde]
pub struct CreateBasket {
    /// Basket ID
    pub basket_id: Uint128,
    /// Collateral asset types.
    /// Note: Also used to tally asset amounts for ease of calculation of Basket ratios
    pub collateral_types: Vec<cAsset>,
    /// Creates native denom for credit_asset
    pub credit_asset: Asset, 
    /// Credit redemption price
    pub credit_price: Decimal,
    /// Base collateral interest rate.
    /// Used to calculate the interest rate for each collateral type.
    pub base_interest_rate: Option<Decimal>,
    /// To measure liquidity for the credit asset
    pub credit_pool_infos: Vec<PoolType>, 
    /// Liquidation queue for collateral assets
    pub liq_queue: Option<String>,
}

#[cw_serde]
pub struct UpdateConfig {
    /// Contract owner
    pub owner: Option<String>,
    /// Stability Pool contract address
    pub stability_pool: Option<String>,
    /// Apollo DEX router contract address.
    pub dex_router: Option<String>,
    /// Staking contract address
    pub staking_contract: Option<String>,
    /// Osmosis Proxy contract address
    pub osmosis_proxy: Option<String>,
    /// Debt auction contract address
    pub debt_auction: Option<String>,
    /// Oracle contract address
    pub oracle_contract: Option<String>,
    /// Liquidity Check contract address
    pub liquidity_contract: Option<String>,
    /// System Discounts contract address
    pub discounts_contract: Option<String>,
    /// Liquidation fee as percent
    pub liq_fee: Option<Decimal>,
    /// Collateral TWAP time frame in minutes
    pub collateral_twap_timeframe: Option<u64>,
    /// Credit TWAP time frame in minutes
    pub credit_twap_timeframe: Option<u64>,
    /// Seconds until oracle failure is accepted
    pub oracle_time_limit: Option<u64>,
    /// Augment the rate of increase per % difference for the redemption rate
    pub cpc_multiplier: Option<Decimal>,
    /// Debt minimum value per position.
    pub debt_minimum: Option<Uint128>,
    /// Debt minimum multiplier for base debt cap.
    /// How many users do we want at 0 credit liquidity?
    pub base_debt_cap_multiplier: Option<Uint128>,
    /// Interest rate 2nd Slope multiplier
    pub rate_slope_multiplier: Option<Decimal>,
    /// Rate hike rate
    pub rate_hike_rate: Option<Decimal>,
    /// Redemption Fee
    pub redemption_fee: Option<Decimal>,
}

impl UpdateConfig {
    pub fn update_config(
        self,
        api: &dyn Api,
        config: &mut Config,
    ) -> StdResult<()>{
        //Set Optionals
        if let Some(stability_pool) = self.stability_pool {
            config.stability_pool = Some(api.addr_validate(&stability_pool)?);
        }
        if let Some(dex_router) = self.dex_router {
            config.dex_router = Some(api.addr_validate(&dex_router)?);
        }
        if let Some(osmosis_proxy) = self.osmosis_proxy {
            config.osmosis_proxy = Some(api.addr_validate(&osmosis_proxy)?);
        }
        if let Some(debt_auction) = self.debt_auction {
            config.debt_auction = Some(api.addr_validate(&debt_auction)?);
        }
        if let Some(staking_contract) = self.staking_contract {
            config.staking_contract = Some(api.addr_validate(&staking_contract)?);
        }
        if let Some(oracle_contract) = self.oracle_contract {
            config.oracle_contract = Some(api.addr_validate(&oracle_contract)?);
        }
        if let Some(liquidity_contract) = self.liquidity_contract {
            config.liquidity_contract = Some(api.addr_validate(&liquidity_contract)?);
        }
        if let Some(discounts_contract) = self.discounts_contract {
            config.discounts_contract = Some(api.addr_validate(&discounts_contract)?);
        }
        if let Some(liq_fee) = self.liq_fee {
            //Enforce 0-100% range
            if liq_fee > Decimal::percent(100) || liq_fee < Decimal::zero() {
                return Err(StdError::GenericErr{ msg: String::from("Liquidation fee must be between 0-100%") });
            }
            config.liq_fee = liq_fee;
        }
        if let Some(debt_minimum) = self.debt_minimum {
            config.debt_minimum = debt_minimum;
        }
        if let Some(base_debt_cap_multiplier) = self.base_debt_cap_multiplier {
            config.base_debt_cap_multiplier = base_debt_cap_multiplier;
        }
        if let Some(oracle_time_limit) = self.oracle_time_limit {
            //Assert oracle time limit is max the collateral_twap_timeframe
            if oracle_time_limit > config.collateral_twap_timeframe * 60 {
                return Err(StdError::GenericErr{ msg: String::from("Oracle time limit ceiling is the collateral twap timeframe")});
            }
            config.oracle_time_limit = oracle_time_limit;
        }
        if let Some(collateral_twap_timeframe) = self.collateral_twap_timeframe {
            config.collateral_twap_timeframe = collateral_twap_timeframe;
        }
        if let Some(credit_twap_timeframe) = self.credit_twap_timeframe {
            config.credit_twap_timeframe = credit_twap_timeframe;
        }
        if let Some(cpc_multiplier) = self.cpc_multiplier {
            //Enforce 0-1k%
            if cpc_multiplier > Decimal::percent(10_00) || cpc_multiplier < Decimal::zero() {
                return Err(StdError::GenericErr{ msg: String::from("CPC multiplier must be between 0-1000%") });
            }
            config.cpc_multiplier = cpc_multiplier;
        }
        if let Some(rate_slope_multiplier) = self.rate_slope_multiplier {
            //Enforce 0-1k%
            if rate_slope_multiplier > Decimal::percent(10_00) || rate_slope_multiplier < Decimal::zero() {
                return Err(StdError::GenericErr{ msg: String::from("Rate slope multiplier must be between 0-10000%") });
            }            
            config.rate_slope_multiplier = rate_slope_multiplier;
        }
        if let Some(new_rate) = self.rate_hike_rate {
            config.rate_hike_rate = Some(new_rate);
        }
        if let Some(redemption_fee) = self.redemption_fee {
            //Enforce 0-99% range
            if redemption_fee >= Decimal::percent(100) || redemption_fee < Decimal::zero() {
                return Err(StdError::GenericErr{ msg: String::from("Redemption fee must be between 0-99%") });
            }
            config.redemption_fee = Some(redemption_fee);
        }
        Ok(())
    }
}

#[cw_serde]
pub struct EditBasket {
    /// Add new cAsset
    pub added_cAsset: Option<cAsset>,
    /// Liquidation Queue
    pub liq_queue: Option<String>,
    /// Credit pool info for liquidity measuring
    pub credit_pool_infos: Option<Vec<PoolType>>, 
    /// Supply caps for each collateral
    pub collateral_supply_caps: Option<Vec<SupplyCap>>,
    /// Supply caps for asset groups
    pub multi_asset_supply_caps: Option<Vec<MultiAssetSupplyCap>>,
    /// Base interest rate
    pub base_interest_rate: Option<Decimal>,
    /// Osmosis Pool info for credit->OSMO TWAP price
    /// Non-USD denominated baskets don't work due to the debt minimum
    pub credit_asset_twap_price_source: Option<(TWAPPoolInfo)>,
    /// Toggle allowance negative redemption rate
    pub negative_rates: Option<bool>, 
    /// Margin of error for difference in TWAP price and redemption price
    pub cpc_margin_of_error: Option<Decimal>,
    /// Toggle basket freezing
    pub frozen: Option<bool>,
    /// Toggle Basket revenue to stakers
    pub rev_to_stakers: Option<bool>,
    /// Take revenue, used as a way to distribute revenue
    pub take_revenue: Option<Uint128>,
    /// Set destination ratios for revenue distribution
    pub revenue_destinations: Option<Vec<RevenueDestination>>,
}

impl EditBasket {    
    /// Use EditBasket to edit a Basket
    pub fn edit_basket(
        self,
        basket: &mut Basket,
        new_cAsset: cAsset,
        new_queue: Option<Addr>,
        oracle_set: bool,
    ) -> StdResult<()> {
        if self.added_cAsset.is_some() {
            basket.collateral_types.push(new_cAsset);
        }
        if self.liq_queue.is_some() {
            basket.liq_queue = new_queue;
        }
        if let Some(collateral_supply_caps) = self.collateral_supply_caps {
            //Set new cap parameters
            for new_cap in collateral_supply_caps {
                if let Some((index, _cap)) = basket.clone().collateral_supply_caps
                    .into_iter()
                    .enumerate()
                    .find(|(_x, cap)| cap.asset_info.equal(&new_cap.asset_info))
                {
                    //Set supply cap ratio
                    basket.collateral_supply_caps[index].supply_cap_ratio = new_cap.supply_cap_ratio;
                    //Set stability pool based ratio
                    basket.collateral_supply_caps[index].stability_pool_ratio_for_debt_cap = new_cap.stability_pool_ratio_for_debt_cap;
                }
            }
        }
        if let Some(multi_asset_supply_caps) = self.multi_asset_supply_caps {
            //Set new cap parameters
            for new_cap in multi_asset_supply_caps {
                if let Some((index, _cap)) = basket.clone().multi_asset_supply_caps
                    .into_iter()
                    .enumerate()
                    .find(|(_x, cap)| equal(&cap.assets, &new_cap.assets))
                {
                    //Set supply cap ratio
                    basket.multi_asset_supply_caps[index].supply_cap_ratio = new_cap.supply_cap_ratio;
                } else {
                    basket.multi_asset_supply_caps.push(new_cap);
                }
            }
        }
        if let Some(base_interest_rate) = self.base_interest_rate {
            basket.base_interest_rate = base_interest_rate;
        }
        if let Some(toggle) = self.negative_rates {
            basket.negative_rates = toggle;
        }
        if let Some(toggle) = self.frozen {
            basket.frozen = toggle;
        }
        if let Some(toggle) = self.rev_to_stakers {
            basket.rev_to_stakers = toggle;
        }
        if let Some(error_margin) = self.cpc_margin_of_error {
            basket.cpc_margin_of_error = error_margin;
        }
        if let Some(take_revenue) = self.take_revenue {
            basket.pending_revenue = match basket.pending_revenue.checked_sub(take_revenue){
                Ok(val) => val,
                Err(_) => Uint128::zero(),
            };
        }
        if let Some(revenue_destinations) = self.revenue_destinations {
            let mut current_destinations =  basket.clone().revenue_destinations.unwrap();
            for new_dest in revenue_destinations {
                if let Some((index, _dest)) = current_destinations.clone()
                    .into_iter()
                    .enumerate()
                    .find(|(_x, current_dest)| current_dest.destination == new_dest.destination)
                {
                    //Set new ratio
                    current_destinations[index].distribution_ratio = new_dest.distribution_ratio;
                } else {
                    //Add new destination
                    current_destinations.push(new_dest);
                }
            }
            //Set new destinations
            basket.revenue_destinations = Some(current_destinations);
        }
        basket.oracle_set = oracle_set;

        

        Ok(())
    }
} 

/// Response for GetUserPositions
#[cw_serde]
pub struct PositionResponse {
    /// Position ID
    pub position_id: Uint128,
    /// Position collateral assets
    pub collateral_assets: Vec<cAsset>,
    /// Collateral asset ratios
    /// Allows front ends to get ratios using the same oracles.
    /// Useful for users who want to deposit or withdraw at the current ratio.
    pub cAsset_ratios: Vec<Decimal>,
    /// Position outstanding debt
    pub credit_amount: Uint128,
    /// Average borrow LTV of collateral assets
    pub avg_borrow_LTV: Decimal,
    /// Average max LTV of collateral assets
    pub avg_max_LTV: Decimal,
}

#[cw_serde]
pub struct BasketPositionsResponse {
    /// Position user
    pub user: String,
    /// List of Positions
    pub positions: Vec<PositionResponse>,
}

/// Response for credit redemption price
#[cw_serde]
pub struct InterestResponse {
    /// Redemption rate
    pub credit_interest: Decimal,
    /// Is the redemption rate negative?
    pub negative_rate: bool,
}

#[cw_serde]
pub struct CollateralInterestResponse {
    /// Collateral interest rates in the order of the collateral types
    pub rates: Vec<Decimal>,
}

#[cw_serde]
pub struct RedeemabilityResponse {
    /// State for each premium 
    pub premium_infos: Vec<PremiumInfo>,
}
#[cw_serde]
pub struct InsolvencyResponse {
    /// List of insolvent Positions
    pub insolvent_positions: Vec<InsolventPosition>,
}
