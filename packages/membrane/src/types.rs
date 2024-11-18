use prost::Message;
use core::fmt;
use std::{str::FromStr, convert::TryFrom};

use crate::{math::{Decimal256, Uint256}, liq_queue::QueueResponse, oracle::PriceResponse};

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128, StdError};
use cw_coins::Coins;

use osmosis_std::types::osmosis::poolmanager::v1beta1::SwapAmountInRoute;
use osmosis_std::types::cosmos::base::v1beta1::Coin;

/// Stability Pool
#[cw_serde]
pub struct PositionUserInfo {
    /// Position ID
    pub position_id: Option<Uint128>,
    /// User address
    pub position_owner: Option<String>,
}

#[cw_serde]
pub struct LiqAsset {
    /// Asset info
    pub info: AssetInfo,
    /// Asset amount
    pub amount: Decimal,
}

impl fmt::Display for LiqAsset {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.info)
    }
}

#[cw_serde]
pub struct UserRatio {
    /// Address
    pub user: Addr,
    /// Ratio
    pub ratio: Decimal,
}

#[cw_serde]
pub struct Deposit {
    /// User address
    pub user: Addr,
    /// Deposit amount
    pub amount: Decimal,
    /// Deposit time in seconds
    pub deposit_time: u64,
    /// Last accrued time in seconds
    pub last_accrued: u64,
    /// Unstake time in seconds
    pub unstake_time: Option<u64>,
}

impl fmt::Display for Deposit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.user, self.amount)
    }
}

impl Deposit {
    /// Equality check for Deposit's amount/time/user
    pub fn equal(&self, deposits: &[Deposit]) -> bool {
        let mut check = false;
        for deposit in deposits.iter() {
            if self.amount == deposit.amount && self.user == deposit.user && self.deposit_time == deposit.deposit_time{
                check = true;
            }
        }

        check
    }
}

#[cw_serde]
pub struct AssetPool {
    /// Credit asset
    pub credit_asset: Asset,
    /// Liquidation premium
    pub liq_premium: Decimal,
    /// Asset deposits
    pub deposits: Vec<Deposit>,
}

impl fmt::Display for AssetPool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.credit_asset)
    }
}

/// Liquidation Queue
#[cw_serde]
pub struct Queue {
    /// Bid for asset
    pub bid_asset: Asset,
    /// Max premium.
    /// A slot for each premium is created when queue is created.
    pub max_premium: Uint128,
    /// Premium slots
    pub slots: Vec<PremiumSlot>,
    /// Current bid ID
    pub current_bid_id: Uint128,
    /// Minimum bid amount in the queue before waiting period is set to 0. Threshold should be larger than the largest single liquidation amount.
    pub bid_threshold: Uint256,
}

impl Queue {
    pub fn into_queue_response(self) -> QueueResponse {
        QueueResponse {
            bid_asset: self.bid_asset,
            max_premium: self.max_premium,
            current_bid_id: self.current_bid_id,
            bid_threshold: self.bid_threshold,
        }
    }
}

#[cw_serde]
pub struct BidInput {
    /// Bid for asset
    pub bid_for: AssetInfo,
    /// Liquidation premium within range of Queue's max_premium
    pub liq_premium: u8,
}

impl fmt::Display for BidInput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.bid_for, self.liq_premium)
    }
}

#[cw_serde]
pub struct Bid {
    /// Bidder address
    pub user: Addr,
    /// Bid ID
    pub id: Uint128,
    /// Bid amount
    pub amount: Uint256,
    /// Liquidation premium
    pub liq_premium: u8,
    /// Product snapshot
    pub product_snapshot: Decimal256,
    /// Sum snapshot
    pub sum_snapshot: Decimal256,
    /// Pending liquidated collateral
    pub pending_liquidated_collateral: Uint256,
    /// End of waiting period in seconds
    pub wait_end: Option<u64>,
    /// Epoch snapshot
    pub epoch_snapshot: Uint128,
    /// Scale snapshot
    pub scale_snapshot: Uint128,
}

impl fmt::Display for Bid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.user, self.amount)
    }
}

impl Bid {
    pub fn equal(&self, bids: &[Bid]) -> bool {
        let mut check = false;
        for bid in bids.iter() {
            if self.amount == bid.amount && self.user == bid.user {
                check = true;
            }
        }

        check
    }
}

#[cw_serde]
pub struct User {
    /// Claimable assets earned from liquidations
    pub claimable_assets: Coins,
}

#[cw_serde]
pub struct PremiumSlot {
    /// Bids in the slot
    pub bids: Vec<Bid>,    
    /// Waiting bids in the slot
    pub waiting_bids: Vec<Bid>,
    /// Liquidation premium
    pub liq_premium: Decimal256,
    /// Sum snapshot
    pub sum_snapshot: Decimal256,
    /// Product snapshot
    pub product_snapshot: Decimal256,
    /// Total bid amount
    pub total_bid_amount: Uint256,
    /// Last time the bids have been totaled, in seconds
    pub last_total: u64, 
    /// Current epoch
    pub current_epoch: Uint128,
    /// Current scale
    pub current_scale: Uint128,
    /// Residue collateral
    pub residue_collateral: Decimal256,
    /// Residue bid
    pub residue_bid: Decimal256,
}

/// Staking
#[cw_serde]
pub struct StakeDeposit {
    /// Staker address
    pub staker: Addr,
    /// Amount of stake
    pub amount: Uint128,
    /// Time of stake in seconds
    pub stake_time: u64,
    /// Time of unstake in seconds
    pub unstake_start_time: Option<u64>,
    /// last_accrued time in seconds
    pub last_accrued: Option<u64>,
}

impl fmt::Display for StakeDeposit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.staker, self.amount)
    }
}

#[cw_serde]
pub struct OldStakeDeposit {
    /// Staker address
    pub staker: Addr,
    /// Amount of stake
    pub amount: Uint128,
    /// Time of stake in seconds
    pub stake_time: u64,
    /// Time of unstake in seconds
    pub unstake_start_time: Option<u64>,
}

#[cw_serde]
pub struct Delegation {
    /// Delegate address
    pub delegate: Addr,
    /// Amount of stake
    pub amount: Uint128,
    /// Fluidity toggle
    /// true: delegation can be redelegated by the delegate
    pub fluidity: bool,
    /// Delegate voting power as well as commission
    pub voting_power_delegation: bool,
    /// Time of delegation in seconds
    pub time_of_delegation: u64,
    /// last_accrued time in seconds
    pub last_accrued: Option<u64>,
}

#[cw_serde]
pub struct OldDelegation {
    /// Delegate address
    pub delegate: Addr,
    /// Amount of stake
    pub amount: Uint128,
    /// Fluidity toggle
    /// true: delegation can be redelegated by the delegate
    pub fluidity: bool,
    /// Delegate voting power as well as commission
    pub voting_power_delegation: bool,
    /// Time of delegation in seconds
    pub time_of_delegation: u64,
}

#[cw_serde]
pub struct DelegationInfo {    
    /// Delegated stake
    pub delegated: Vec<Delegation>,
    /// Stake delagated to staker
    pub delegated_to: Vec<Delegation>,
    /// Commission %
    pub commission: Decimal,
}
#[cw_serde]
pub struct OldDelegationInfo {    
    /// Delegated stake
    pub delegated: Vec<OldDelegation>,
    /// Stake delagated to staker
    pub delegated_to: Vec<OldDelegation>,
    /// Commission %
    pub commission: Decimal,
}

#[cw_serde]
pub struct Delegate {
    /// Delegate address
    pub delegate: Addr,
    /// Alias
    pub alias: Option<String>,
    /// Discord username
    pub discord_username: Option<String>,
    /// Twitter username
    pub twitter_username: Option<String>,
    /// Some URL
    pub url: Option<String>,
}

#[cw_serde]
pub struct FeeEvent {
    /// Time of event in seconds
    pub time_of_event: u64,
    /// Fee asset    
    pub fee: LiqAsset,
}

#[cw_serde]
pub struct StakeDistribution {
    /// Distribution rate
    pub rate: Decimal,
    /// Duration of distribution in days
    pub duration: u64,
}

#[cw_serde]
pub struct StakeDistributionLog {
    /// Distribution strategy
    pub ownership_distribution: StakeDistribution,
    /// Distribution start time in seconds
    pub start_time: u64,
}

#[cw_serde]
pub struct VaultTokenInfo {
    /// Vault contract address
    pub vault_contract: String,
    /// Underlying token 
    pub underlying_token: String,
}

/// Oracle
#[cw_serde]
pub struct AssetOracleInfo {
    /// Basket ID
    pub basket_id: Uint128,
    /// Pyth price feed ID
    pub pyth_price_feed_id: Option<String>,
    /// Osmosis pools for OSMO TWAP
    pub pools_for_osmo_twap: Vec<TWAPPoolInfo>,
    /// Bool to provide $1 static_price if the asset is USD-par
    pub is_usd_par: bool,
    /// LP pool info
    pub lp_pool_info: Option<PoolInfo>,
    /// Vault Info (for vault tokens only)
    pub vault_info: Option<VaultTokenInfo>,
    /// Asset decimals
    pub decimals: u64,
}

impl fmt::Display for AssetOracleInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "osmo_pools: {:?}, is_usd_par: {:?}", self.pools_for_osmo_twap, self.is_usd_par)
    }
}

#[cw_serde]
pub struct TWAPPoolInfo {
    /// Pool ID
    pub pool_id: u64,
    /// Base asset denom
    pub base_asset_denom: String,
    /// Quote asset denom
    pub quote_asset_denom: String,
}

impl fmt::Display for TWAPPoolInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "pool_id: {}, base_asset_denom: {}, quote_asset_denom: {}",
            self.pool_id, self.base_asset_denom, self.quote_asset_denom
        )
    }
}

#[cw_serde]
pub struct StoredPrice {
    /// Price
    pub price: PriceResponse,
    /// Time of price in seconds
    pub last_time_updated: u64,
}

#[cw_serde]
pub struct PriceInfo {
    /// Source of price,
    /// Chain name, Oracle Address or static
    pub source: String, 
    /// Price
    pub price: Decimal,
}

/// CDP
#[cw_serde]
pub struct cAsset {
    /// Asset data
    /// NOTE: AssetInfo denom for an Osmo LP is the shares_denom
    pub asset: Asset,
    /// Max borrow limit, aka what u can borrow up to
    pub max_borrow_LTV: Decimal, 
    /// Liquidation LTV
    pub max_LTV: Decimal,
    /// Rate index to smooth rate accrual
    pub rate_index: Decimal, 
    /// Pool Info for Osmosis LP
    pub pool_info: Option<PoolInfo>,
    /// Is this subject to rate hikes?
    pub hike_rates: Option<bool>,
}

/// Osmosis PoolInfo
#[cw_serde]
pub struct PoolInfo {
    /// Pool ID
    pub pool_id: u64,
    /// Asset Infos
    /// Includes asset decimals (https://api-osmosis.imperator.co/tokens/v2/all)
    pub asset_infos: Vec<LPAssetInfo>, 
}

#[cw_serde]
pub struct LPAssetInfo {
    /// Pool asset denom
    pub info: AssetInfo,
    /// Asset decimals
    pub decimals: u64,
    /// Asset ratio in pool
    pub ratio: Decimal,
}

#[cw_serde]
pub struct Position {
    /// Position ID
    pub position_id: Uint128,
    /// Collateral assets
    pub collateral_assets: Vec<cAsset>,
    /// Loan size
    pub credit_amount: Uint128,
}

#[cw_serde]
pub struct RedemptionInfo {
    /// Position owner 
    pub position_owner: Addr,
    /// Position redemption info of the positions to be redeemed from
    pub position_infos: Vec<PositionRedemption>,
}

#[cw_serde]
pub struct PositionRedemption {
    /// Position ID of the position to be redeemed from
    pub position_id: Uint128,
    /// Remaining available loan repayment in debt tokens
    pub remaining_loan_repayment: Uint128,
    /// Restricted collateral assets.
    /// These aren't used for redemptions.
    pub restricted_collateral_assets: Vec<String>,
}

#[cw_serde]
pub struct PremiumInfo {
    /// Premium
    pub premium: u128,
    /// IDs in the Premium
    pub users_of_premium: Vec<RedemptionInfo>,
}

#[cw_serde]
pub struct Rate {
    /// Rate
    pub rate: Decimal,
    /// Time of rate in seconds
    pub last_time_updated: u64,
}

#[cw_serde]
pub struct Basket {
    /// Basket ID
    pub basket_id: Uint128,
    /// Position ID for next position
    pub current_position_id: Uint128,
    /// Available collateral types
    pub collateral_types: Vec<cAsset>,
    /// Collateral supply caps
    pub collateral_supply_caps: Vec<SupplyCap>, 
    /// Lastest Collateral Rates
    pub lastest_collateral_rates: Vec<Rate>,
    /// Multi collateral supply caps
    pub multi_asset_supply_caps: Vec<MultiAssetSupplyCap>,
    /// Credit asset object
    pub credit_asset: Asset, 
    /// Credit redemption price, not market price
    pub credit_price: PriceResponse,
    /// Base collateral interest rate.
    /// Enter as percent, 0.02 = 2%.
    pub base_interest_rate: Decimal,
    /// Pending revenue available to mint
    pub pending_revenue: Uint128,
    /// Last time credit price was updated, in seconds
    pub credit_last_accrued: u64,
    /// Last time rate indices for collateral_types was updated, in seconds
    pub rates_last_accrued: u64,
    /// True if the credit oracle was set. Can't update redemption price without it.
    pub oracle_set: bool, 
    /// Toggle to allow negative redemption rates
    pub negative_rates: bool, 
    /// Freeze withdrawals and debt increases to provide time to fix vulnerabilities
    pub frozen: bool, 
    /// Toggle to allow revenue to be distributed to the revenue_destinations.
    /// If false, revenue is left in pending_revenue.
    pub rev_to_stakers: bool,
    /// % difference btwn credit TWAP and redemption price before the controller is effected.
    /// Set to 100 if you want to turn off the controller.
    pub cpc_margin_of_error: Decimal,
    /// Liquidation queue contract address
    pub liq_queue: Option<Addr>,
    /// Revenue destinations distribution
    /// All destinations must have a DepositFee execute msg entrypoint.
    /// The remaining ratio space is left in pending revenue a la the 'Insurance Fund'.
    /// Only an Option for backwards compatibility, don't set to None.
    pub revenue_destinations: Option<Vec<RevenueDestination>>,
}

#[cw_serde]
pub struct RevenueDestination {
    /// Revenue destination
    pub destination: Addr,
    /// Distribution ratio
    pub distribution_ratio: Decimal,
}

#[cw_serde]
pub struct SupplyCap {
    /// Asset info
    pub asset_info: AssetInfo,
    /// Current amount of asset in Basket
    pub current_supply: Uint128,
    /// Total debt collateralized by asset
    pub debt_total: Uint128,
    /// Total supply cap ratio
    pub supply_cap_ratio: Decimal,    
    /// is LP?
    pub lp: bool,
    /// Toggle for a debt cap ratio based on Stability Pool Liquidity.
    /// If false, debt cap is based on proportion of TVL.
    pub stability_pool_ratio_for_debt_cap: Option<Decimal>,     
}

#[cw_serde]
pub struct MultiAssetSupplyCap {
    /// Asset infos
    pub assets: Vec<AssetInfo>,
    /// Target supply cap ratio
    pub supply_cap_ratio: Decimal,
}

//Used for Query Responses
#[cw_serde]
pub struct DebtCap {
    /// Asset info
    pub collateral: AssetInfo,
    /// Total debt collateralized by asset
    pub debt_total: Uint128,
    /// Debt ceiling
    pub cap: Uint128,
}

#[cw_serde]
pub struct UserInfo {
    /// Position ID
    pub position_id: Uint128,
    /// Position owner
    pub position_owner: String,
}

impl fmt::Display for UserInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "owner: {}, position: {}",
            self.position_owner, self.position_id
        )
    }
}

#[cw_serde]
pub struct InsolventPosition {
    /// Is insolvent?
    pub insolvent: bool,
    /// Position info
    pub position_info: UserInfo,
    /// Current LTV
    pub current_LTV: Decimal,
    /// Available liquidation fee
    pub available_fee: Uint128,
}

/// Builder Vesting
#[cw_serde]
pub struct VestingPeriod {
    /// Cliff period in days
    pub cliff: u64,
    /// Linear period in days
    pub linear: u64,
}

impl VestingPeriod {
    pub fn equal(&self, vesting_period: &VestingPeriod) -> bool {
        vesting_period.cliff == self.cliff && vesting_period.linear == self.linear
    }
}

#[cw_serde]
pub struct Recipient {
    /// Recipient address
    pub recipient: Addr,
    /// Allocation
    pub allocation: Option<Allocation>,
    /// Claimable assets
    pub claimables: Vec<Asset>,
}

#[cw_serde]
pub struct Allocation {
    /// Remaining amount of allocation
    pub amount: Uint128,
    /// Amount of asset withdrawn
    pub amount_withdrawn: Uint128,
    /// Start time of allocation in seconds
    pub start_time_of_allocation: u64, 
    /// Vesting period
    pub vesting_period: VestingPeriod,
}

/// Debt Auction
#[cw_serde]
pub struct AuctionRecipient {
    /// Amount
    pub amount: Uint128,
    /// Recipient address
    pub recipient: Addr,
}

#[cw_serde]
pub struct DebtAuction {
    /// Remaining debt to repay
    pub remaining_recapitalization: Uint128,
    /// Positions to repay
    pub repayment_positions: Vec<RepayPosition>,
    /// Capital recipients
    pub send_to: Vec<AuctionRecipient>,
    /// Auction start time
    pub auction_start_time: u64,
}

#[cw_serde]
pub struct FeeAuction {
    /// Remaining debt to repay
    pub auction_asset: Asset,
    /// Auction start time
    pub auction_start_time: u64,
}

#[cw_serde]
pub struct RepayPosition {
    /// Repayment amount
    pub repayment: Uint128,
    /// Position info
    pub position_info: UserInfo,
}

/// Liquidity Check
#[cw_serde]
pub struct LiquidityInfo {
    /// Asset info
    pub asset: AssetInfo,
    /// Pool info
    pub pool_infos: Vec<PoolType>,
}

#[cw_serde]
pub enum PoolType {
    /// Balancer pool
    Balancer { pool_id: u64 },
    /// Stableswap pool
    StableSwap { pool_id: u64 },
}

/// Lockdrop
#[cw_serde]
pub struct LPPoolInfo {
    /// LP share token asset info
    pub share_token: AssetInfo,
    /// Pool ID
    pub pool_id: u64,
}

#[cw_serde]
pub struct DebtTokenAsset {
    /// Asset info
    pub info: AssetInfo,
    /// Amount
    pub amount: Uint128,
    /// Basket ID
    pub basket_id: Uint128,
}

/// Osmosis Proxy
#[cw_serde]
pub struct Owner {
    /// Owner address
    pub owner: Addr,
    /// Total CDT minted (Unused)
    pub total_minted: Uint128,
    /// Stability pool ratio allocated to CDT mint caps
    pub stability_pool_ratio: Option<Decimal>,
    /// Authority over non-token contract messages
    pub non_token_contract_auth: bool,
    /// Is a position's contract?
    pub is_position_contract: bool,
}

#[cw_serde]
pub struct SwapRoute {
    pub token_in: String,
    pub route_out: SwapAmountInRoute,
}
/// Launch
#[cw_serde]
#[serde(rename_all = "snake_case")]
pub struct Lockdrop {
    /// Total number of incentives to distribute
    pub num_of_incentives: Uint128,
    /// Asset needed to lock
    pub locked_asset: AssetInfo,    
    /// Lock up ceiling, in days
    pub lock_up_ceiling: u64,
    /// Start time, for queries
    pub start_time: u64,
    /// End of the Deposit period window, in seconds
    pub deposit_end: u64,
    /// End of the Withdrawal period window, in seconds
    pub withdrawal_end: u64,
    /// Has the protocol launched?
    pub launched: bool,
}

#[cw_serde]
#[serde(rename_all = "snake_case")]
pub struct LockedUser {
    /// User address
    pub user: Addr,
    /// List of deposits
    pub deposits: Vec<Lock>,
    /// Total number of tickets, i.e. share of incentives distributed
    pub total_tickets: Uint128,
    /// Total number of incentives withdrawn
    pub incentives_withdrawn: Uint128,
}

#[cw_serde]
#[serde(rename_all = "snake_case")]
pub struct Lock {
    /// Deposit amount
    pub deposit: Uint128,
    /// Lock up duration, in days
    pub lock_up_duration: u64,
}

/// Discount Vault
#[cw_serde]
pub struct VaultUser {
    /// User address
    pub user: Addr,
    /// List of vaulted LPs
    pub vaulted_lps: Vec<VaultedLP>,
}

#[cw_serde]
pub struct VaultedLP {
    /// LP share token asset info
    pub gamm: AssetInfo,
    /// Amount of LP share tokens
    pub amount: Uint128,
    /// Deposit time
    pub deposit_time: u64,
}

//////////Possibly switching to cw-asset//////

#[cw_serde]
pub enum AssetInfo {
    /// Cw20 token
    Token { address: Addr },
    /// Native token
    NativeToken { denom: String },
}

impl fmt::Display for AssetInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AssetInfo::NativeToken { denom } => write!(f, "{}", denom),
            AssetInfo::Token { address } => write!(f, "{}", address),
        }
    }
}

impl AssetInfo {
    pub fn is_native_token(&self) -> bool {
        match self {
            AssetInfo::NativeToken { .. } => true,
            AssetInfo::Token { .. } => false,
        }
    }

    pub fn equal(&self, asset: &AssetInfo) -> bool {
        match self {
            AssetInfo::Token { address, .. } => {
                let self_addr = address;
                match asset {
                    AssetInfo::Token { address, .. } => self_addr == address,
                    AssetInfo::NativeToken { .. } => false,
                }
            }
            AssetInfo::NativeToken { denom, .. } => {
                let self_denom = denom;
                match asset {
                    AssetInfo::Token { .. } => false,
                    AssetInfo::NativeToken { denom, .. } => self_denom == denom,
                }
            }
        }
    }
}

pub fn equal(assets_1: &Vec<AssetInfo>, assets_2: &Vec<AssetInfo>) -> bool {

    if assets_1.len() != assets_2.len() {
        return false
    }

    for asset in assets_2{
        if assets_1.iter().find(|self_asset| asset.equal(self_asset)).is_none(){
           return false
        }
    }

    true
}

#[cw_serde]
pub struct Asset {
    /// Asset info
    pub info: AssetInfo,
    /// Amount
    pub amount: Uint128,
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.info)
    }
}

////////////////////Osmosis-std types////////////////////
pub enum Pool {
    Balancer(osmosis_std::types::osmosis::gamm::v1beta1::Pool),
    StableSwap(osmosis_std::types::osmosis::gamm::poolmodels::stableswap::v1beta1::Pool),
}

impl Pool {
    pub fn into_pool_state_response(&self) -> PoolStateResponse {
        
        match self {
            Pool::Balancer(pool) => {
                PoolStateResponse { 
                    assets: pool.clone().pool_assets.into_iter().map(|pool_asset| pool_asset.token.unwrap_or_default()).collect::<Vec<Coin>>(), 
                    shares: pool.clone().total_shares.unwrap_or_default(),
                }
            },
            Pool::StableSwap(pool) => {
                PoolStateResponse { 
                    assets: pool.clone().pool_liquidity, 
                    shares: pool.clone().total_shares.unwrap_or_default(),
                }
            },
        }
    }
}

impl TryFrom<osmosis_std::shim::Any> for Pool {
    type Error = StdError;

    fn try_from(value: osmosis_std::shim::Any) -> Result<Self, Self::Error> {
        if let Ok(pool) = osmosis_std::types::osmosis::gamm::v1beta1::Pool::decode(value.value.as_slice()) {
            return Ok(Pool::Balancer(pool));
        }
        if let Ok(pool) = osmosis_std::types::osmosis::gamm::poolmodels::stableswap::v1beta1::Pool::decode(value.value.as_slice()) {
            return Ok(Pool::StableSwap(pool));
        }
        
        Err(StdError::ParseErr {
            target_type: "Pool".to_string(),
            msg: "Unmatched pool: must be either `Balancer` or `StableSwap`.".to_string(),
        })
    }
}

#[cw_serde]
#[serde(rename_all = "snake_case")]
pub struct PoolStateResponse {
    /// The various assets that be swapped. Including current liquidity.
    pub assets: Vec<Coin>,
    /// The number of lp shares and their amount
    pub shares: Coin,
}

impl PoolStateResponse {
    pub fn has_denom(&self, denom: &str) -> bool {
        self.assets.iter().any(|c| c.denom == denom)
    }

    pub fn lp_denom(&self) -> &str {
        &self.shares.denom
    }

    /// If I hold num_shares of the lp_denom, how many assets does that equate to?
    pub fn shares_value(&self, num_shares: impl Into<Uint128>) -> Vec<Coin> {
        let num_shares = num_shares.into();
        self.assets
            .iter()
            .map(|c| Coin {
                denom: c.denom.clone(),
                amount: (Uint128::from_str(&c.amount).unwrap() * num_shares / Uint128::from_str(&self.shares.amount).unwrap()).to_string(),
            })
            .collect()
    }
}

#[cw_serde]
pub struct Swap {
    /// Pool id
    pub pool_id: u64,
    /// Denom in
    pub denom_in: String,
    /// Denom out
    pub denom_out: String,
}

impl Swap {
    pub fn new(pool_id: u64, denom_in: impl Into<String>, denom_out: impl Into<String>) -> Self {
        Swap {
            pool_id,
            denom_in: denom_in.into(),
            denom_out: denom_out.into(),
        }
    }
}

#[cw_serde]
pub struct Step {
    /// Pool id
    pub pool_id: u64,
    /// Denom out
    pub denom_out: String,
}

impl Step {
    pub fn new(pool_id: u64, denom_out: impl Into<String>) -> Self {
        Step {
            pool_id,
            denom_out: denom_out.into(),
        }
    }
}

#[cw_serde]
pub enum SwapAmount {
    In(Uint128),
    Out(Uint128),
}

impl SwapAmount {
    pub fn as_in(&self) -> Uint128 {
        match self {
            SwapAmount::In(x) => *x,
            _ => panic!("was output"),
        }
    }

    pub fn as_out(&self) -> Uint128 {
        match self {
            SwapAmount::Out(x) => *x,
            _ => panic!("was input"),
        }
    }
}

#[cw_serde]
pub enum SwapAmountWithLimit {
    ExactIn { input: Uint128, min_output: Uint128 },
    ExactOut { output: Uint128, max_input: Uint128 },
}

impl SwapAmountWithLimit {
    pub fn discard_limit(self) -> SwapAmount {
        match self {
            SwapAmountWithLimit::ExactIn { input, .. } => SwapAmount::In(input),
            SwapAmountWithLimit::ExactOut { output, .. } => SwapAmount::Out(output),
        }
    }
}

///////////////VAULTS////////////////////
#[cw_serde]
pub struct VTClaimCheckpoint {
    pub vt_claim_of_checkpoint: Uint128,
    pub time_since_last_checkpoint: u64,
}

#[cw_serde]
pub struct ClaimTracker {
    pub vt_claim_checkpoints: Vec<VTClaimCheckpoint>,
    pub last_updated: u64,
}

#[cw_serde]
pub struct APR {
    pub apr: Decimal,
    pub negative: bool,
}

#[cw_serde]
pub struct DepositToken {
    pub deposit_token: String,
    pub decimal: u32,
}

/// Earn Vault
#[cw_serde]
pub struct VaultInfo {
    pub vault_addr: Addr,
    pub deposit_token: String,
    pub vault_token: String
}

/// Range Bound LP Vault
#[cw_serde]
pub struct RangeTokens {
    pub ceiling_deposit_token: String,
    pub floor_deposit_token: String
}

#[cw_serde]
pub struct RangeBounds {
    pub ceiling: RangeTicks,
    pub floor: RangeTicks,
}

#[cw_serde]
pub struct RangeTicks {
    pub lower_tick: i64,
    pub upper_tick: i64,
}

#[cw_serde]
pub struct RangePositions {
    pub ceiling: u64,
    pub floor: u64,
}
