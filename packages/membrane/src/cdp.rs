use cosmwasm_std::{Addr, Decimal, Uint128, StdResult, Api};
use cosmwasm_schema::cw_serde;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::{
    cAsset, Asset, AssetInfo, InsolventPosition, Position, PositionUserInfo,
    SupplyCap, MultiAssetSupplyCap, TWAPPoolInfo, UserInfo, PoolType, Basket, equal,
};

#[cw_serde]
pub struct InstantiateMsg {
    /// Contract Owner
    pub owner: Option<String>,
    /// Seconds until oracle failure is acceoted
    pub oracle_time_limit: u64, 
    /// Minimum debt per position to ensure liquidatibility 
    pub debt_minimum: Uint128, 
    /// Protocol liquidation fee to restrict self liquidations
    pub liq_fee: Decimal,
    /// Timeframe for Collateral TWAPs in minutes
    pub collateral_twap_timeframe: u64, 
    /// Timeframe for Credit TWAP in minutes
    pub credit_twap_timeframe: u64,     
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
    /// Close a Position by selling collateral and repaying debt
    ClosePosition {
        /// Position ID to close
        position_id: Uint128,
        /// Max spread for the sale of collateral
        max_spread: Decimal,
        /// Send excess assets to this address if not the Position owner
        send_to: Option<String>,
    },
    /// Accrue interest for a Position
    Accrue { 
        /// Positon owner to accrue interest for
        /// Only the Stability Pool can call for unowned Positions
        position_owner: Option<String>, 
        /// Positon ID to accrue interest for
        position_id: Uint128
    },
    /// Mint Basket pending revenue
    MintRevenue {
        /// Send minted tokens to this address if not the sender
        send_to: Option<String>, 
        /// Repay for a position w/ the revenue
        repay_for: Option<UserInfo>, 
        /// Amount of revenue to mint
        amount: Option<Uint128>,
    },
    /// Create the contract's Basket
    CreateBasket {
        /// Basket ID
        basket_id: Uint128,
        /// Collateral asset types.
        /// Note: Also used to tally asset amounts for ease of calculation of Basket ratios
        collateral_types: Vec<cAsset>,
        /// Creates native denom for credit_asset
        credit_asset: Asset, 
        /// Credit redemption price
        credit_price: Decimal,
        /// Base collateral interest rate.
        /// Used to calculate the interest rate for each collateral type.
        base_interest_rate: Option<Decimal>,
        /// To measure liquidity for the credit asset
        credit_pool_infos: Vec<PoolType>, 
        /// Liquidation queue for collateral assets
        liq_queue: Option<String>,
    },
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
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
    /// Returns Positions owned by a user
    GetUserPositions {
        /// User to query
        user: String,
        /// Response limiter
        limit: Option<u32>,
    },
    /// Returns a single Position
    GetPosition {
        //Position ID to query
        position_id: Uint128,
        //Position owner to query
        position_owner: String,
    },
    /// Returns Positions in the contract's Basket
    GetBasketPositions {
        /// Start after this user address
        start_after: Option<String>,
        /// Response limiter
        limit: Option<u32>,
    },
    /// Returns the contract's Basket
    GetBasket { }, 
    /// Returns Basket collateral debt caps
    GetBasketDebtCaps { },
    /// Returns Positions with bad debt in the Basket
    GetBasketBadDebt { },
    /// Returns insolvency status of a Position
    GetPositionInsolvency {
        /// Position ID to query
        position_id: Uint128,
        /// Position owner to query
        position_owner: String,
    },
    /// Returns credit redemption rate
    GetCreditRate { },
    /// Returns Basket collateral interest rates
    GetCollateralInterest { },
    /// Used internally to test state propagation
    Propagation {},
}

#[cw_serde]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Stability Pool contract address
    pub stability_pool: Option<Addr>,
    /// Apollo DEX router contract address.
    /// Note: Will need to change msg types if the router provider changes
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
}

impl UpdateConfig {
    pub fn update_config(
        self,
        api: &dyn Api,
        config: &mut Config,
    ) -> StdResult<()>{
        //Set Optionals
        if let Some(owner) = self.owner {
            config.owner = api.addr_validate(&owner)?;
        }
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
            config.liq_fee = liq_fee;
        }
        if let Some(debt_minimum) = self.debt_minimum {
            config.debt_minimum = debt_minimum;
        }
        if let Some(base_debt_cap_multiplier) = self.base_debt_cap_multiplier {
            config.base_debt_cap_multiplier = base_debt_cap_multiplier;
        }
        if let Some(oracle_time_limit) = self.oracle_time_limit {
            config.oracle_time_limit = oracle_time_limit;
        }
        if let Some(collateral_twap_timeframe) = self.collateral_twap_timeframe {
            config.collateral_twap_timeframe = collateral_twap_timeframe;
        }
        if let Some(credit_twap_timeframe) = self.credit_twap_timeframe {
            config.credit_twap_timeframe = credit_twap_timeframe;
        }
        if let Some(cpc_multiplier) = self.cpc_multiplier {
            config.cpc_multiplier = cpc_multiplier;
        }
        if let Some(rate_slope_multiplier) = self.rate_slope_multiplier {
            config.rate_slope_multiplier = rate_slope_multiplier;
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
    /// Osmosis Pool info for credit TWAP price
    /// Non-USD denominated baskets don't work due to the debt minimum
    pub credit_asset_twap_price_source: Option<TWAPPoolInfo>,
    /// Toggle allowance negative redemption rate
    pub negative_rates: Option<bool>, 
    /// Margin of error for difference in TWAP price and redemption price
    pub cpc_margin_of_error: Option<Decimal>,
    /// Toggle basket freezing
    pub frozen: Option<bool>,
    /// Toggle Basket revenue to stakers
    pub rev_to_stakers: Option<bool>,
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
        basket.oracle_set = oracle_set;

        Ok(())
    }
} 

/// Response for Positions
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
    /// Basket ID
    pub basket_id: Uint128,
    /// Average borrow LTV of collateral assets
    pub avg_borrow_LTV: Decimal,
    /// Average max LTV of collateral assets
    pub avg_max_LTV: Decimal,
}

#[cw_serde]
pub struct PositionsResponse {
    /// Position user
    pub user: String,
    /// List of Positions
    pub positions: Vec<Position>,
}

#[cw_serde]
pub struct BadDebtResponse {
    /// List of Positions with bad debt 
    pub has_bad_debt: Vec<(PositionUserInfo, Uint128)>,
}

#[cw_serde]
pub struct InsolvencyResponse {
    /// List of insolvent Positions
    pub insolvent_positions: Vec<InsolventPosition>,
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
