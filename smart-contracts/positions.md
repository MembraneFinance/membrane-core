# Positions

The Positions contract implements the logic for Collateralized Debt Positions (CDPs), through which users can receive debt tokens against their deposited collateral.\
\
Collateral parameters are held in the cAsset object, which also holds the address needed for its oracle in the Oracle Contract.

The contract also contains the logic for initiating liquidations of CDPs and the sell wall but external debt repayment logic goes through the **Queue** and **Stability Pool** contracts.

## InitMsg

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: Option<String>,
    pub collateral_types: Option<Vec<cAsset>>,
    pub credit_asset: Option<Asset>,
    pub credit_price: Option<Decimal>,
    pub credit_interest: Option<Decimal>,
    pub basket_owner: Option<String>,
}


pub struct Asset{
    pub info: AssetInfo,
    pub amount: Uint128,
}

pub enum AssetInfo {
    Token{
        address: Addr,
    },
    NativeToken{
        denom: String,
    },
}
```

| Key                 | Type         | Description                                                     |
| ------------------- | ------------ | --------------------------------------------------------------- |
| `*owner`            | String       | Contract owner that defaults to info.sender                     |
| `*collateral_types` | Vec\<cAsset> | Accepted cAssets for an initial basket                          |
| `*credit_asset`     | Asset        | Credit asset for an initial basket                              |
| `*credit_price`     | Decimal      | Credit price for an initial basket                              |
| `*credit_interest`  | Decimal      | Credit interest for an initial basket                           |
| `*basket_owner`     | String       | Basket owner for an initial basket that defaults to info.sender |

\* = optional

## ExecuteMsg

### `Receive`

Can be called during a CW20 token transfer when the Positions contract is the recipient. Allows the token transfer to execute a [Receive Hook](positions.md#receive-hook) as a subsequent action within the same transaction.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Receive(Cw20ReceiveMsg)
}

pub struct Cw20ReceiveMsg {
    pub sender: String,
    pub amount: Uint128,
    pub msg: Binary,
}
```

| Key      | Type    | Description                                                             |
| -------- | ------- | ----------------------------------------------------------------------- |
| `sender` | String  | Sender of the token transfer                                            |
| `amount` | Uint128 | Amount of tokens received                                               |
| `msg`    | Binary  | Base64-encoded string of JSON of [Receive Hook](positions.md#undefined) |

### `Deposit`

{% hint style="info" %}
Used for depositing native assets as collateral. For depositing Cw20 collateral to a CDP, you need to use the [Receive Hook variant](positions.md#undefined).
{% endhint %}

Deposits basket accepted collateral to a new or existing position.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Deposit {
        assets: Vec<Asset>,
        position_owner: Option<String>,
        basket_id: Uint128,
        position_id: Option<Uint128>,
    },
}
```

| Key                | Type        | Description                                                              |
| ------------------ | ----------- | ------------------------------------------------------------------------ |
| `assets`           | Vec\<Asset> | Asset objects to deposit                                                 |
| \*`position_owner` | String      | Owner of the position, defaults to info.sender                           |
| `basket_id`        | Uint128     | Basket ID to deposit to.                                                 |
| \*`position_id`    | Uint128     | Position ID to deposit to. If none is passed, a new position is created. |

\* = optional

### `IncreaseDebt`

Increase debt of a position. Only callable by the position owner and limited by the position's max borrow LTV.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    IncreaseDebt { 
        basket_id: Uint128,
        position_id: Uint128,
        amount: Uint128,
    }, 
}
```

| Key           | Type    | Description                      |
| ------------- | ------- | -------------------------------- |
| `basket_id`   | Uint128 | ID of basket the position is in  |
| `position_id` | Uint128 | ID of position                   |
| `amount`      | Uint128 | Amount to increase debt by       |

### `Withdraw`

Withdraw assets from the caller's position as long as it leaves the position solvent in relation to the max borrow LTV

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Withdraw {
        basket_id: Uint128,
        position_id: Uint128,
        assets: Vec<Asset>,
    },
}
```

| Key           | Type        | Description                          |
| ------------- | ----------- | ------------------------------------ |
| `basket_id`   | Uint128     | ID of basket the position is in      |
| `position_id` | Uint128     | ID of position                       |
| `assets`      | Vec\<Asset> | Assets to withdraw from the position |

### `Repay`

{% hint style="info" %}
Used for repaying native assets as collateral. For repaying Cw20 credit assets, you need to use the [Receive Hook variant](positions.md#undefined).
{% endhint %}

Repay outstanding debt for a position, not exclusive to the position owner.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Repay {
        basket_id: Uint128,
        position_id: Uint128,
        position_owner: Option<String>, 
        credit_asset: Asset,
    },
}
```

| Key               | Type    | Description                     |
| ----------------- | ------- | ------------------------------- |
| `basket_id`       | Uint128 | ID of basket the Position is in |
| `position_id`     | Uint128 | ID of Position                  |
| `*position_owner` | String  | Owner of Position to repay      |
| `credit_asset`    | Asset   | Asset object for repayment info |

\* = optional

### `LiqRepay`

Repay function for the liquidation contracts the CDP uses ([Queue ](liquidation-queue.md)and [Stability Pool](stability-pool.md)). Used to repay insolvent positions and distribute liquidated funds to said contracts.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    LiqRepay {
        credit_asset: Asset,
        collateral_asset: Option<Asset>,
        fee_ratios: Option<Vec<RepayFee>>, 
    },
}

pub struct RepayFee {
    pub fee: Decimal,
    pub ratio: Decimal,
}
```

| Key                 | Type           | Description                                                                                         |
| ------------------- | -------------- | --------------------------------------------------------------------------------------------------- |
| `credit_asset`      | Asset          | Asset object for repayment info                                                                     |
| `*collateral_asset` | Asset          | Collateral asset to specify for distribution, used by the [Liquidation Queue](liquidation-queue.md) |
| `*fee_ratios`       | Vec\<RepayFee> | List of fee ratios used by the [Liquidation Queue](liquidation-queue.md)                            |

\* = optional

### `Liquidate`

Assert's the position is insolvent and calculates the distribution of repayment to the various liquidation modules. Does a bad debt check at the end of the procedure that starts a[ MBRN auction](mbrn-auction.md) if necessary.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Liquidate {
        basket_id: Uint128,
        position_id: Uint128,
        position_owner: String,
    },
}
```

| Key              | Type    | Description                     |
| ---------------- | ------- | ------------------------------- |
| `basket_id`      | Uint128 | ID of basket the Position is in |
| `position_id`    | Uint128 | ID of Position                  |
| `position_owner` | String  | Owner of Position               |

### `CreateBasket`

Add Basket to the Position's contract, only callable by the contract owner.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    CreateBasket {
        owner: Option<String>,
        collateral_types: Vec<cAsset>,
        credit_asset: Asset,
        credit_price: Option<Decimal>,
        credit_interest: Option<Decimal>,
    },
}
```

| Key                | Type         | Description                               |
| ------------------ | ------------ | ----------------------------------------- |
| `*owner`           | String       | Basket owner, defaults to info.sender     |
| `collateral_type`  | Vec\<cAsset> | List of accepted cAssets                  |
| `credit_asset`     | Asset        | Asset info for Basket's credit asset      |
| `*credit_price`    | Decimal      | Price of credit in basket                 |
| `*credit_interest` | Decimal      | Interest rate of credit's repayment price |

\* = optional

### `EditBasket`

Add cAsset, change owner and/or change credit\_interest of an existing Basket.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    EditBasket {
        basket_id: Uint128,
        added_cAsset: Option<cAsset>,
        owner: Option<String>,
        credit_interest: Option<Decimal>,
    }, 
}
```

| Key                | Type    | Description                                     |
| ------------------ | ------- | ----------------------------------------------- |
| `basket_id`        | Uint128 | ID of existing Basket                           |
| `*added_cAsset`    | cAsset  | cAsset object to add to accepted basket objects |
| `*owner`           | String  | New owner of Basket                             |
| `*credit_interest` | Decimal | Credit repayment price interest                 |

\* = optional

### `EditAdmin`

Edit contract owner.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    EditAdmin {
        owner: String,
    },
}
```

| Key     | Type   | Description              |
| ------- | ------ | ------------------------ |
| `owner` | String | Positions contract owner |

### `Callback`

Messages usable only by the contract to enable functionality in line with message semantics

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Callback(CallbackMsg)
}

pub enum CallbackMsg {
    BadDebtCheck {
        basket_id: Uint128,
        position_id: Uint128,
        position_owner: Addr,
    },
}
```

## Receive Hook

### `Deposit`

{% hint style="info" %}
Used for depositing `CW20` assets as collateral. For depositing native assets collateral to a CDP, you need to use the [ExecuteMsg variant](positions.md#deposit)
{% endhint %}

Deposits basket accepted collateral to a new or existing position.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Deposit {
        basket_id: Uint128,
        position_owner: Option<String>,
        position_id: Option<Uint128>,
    },
}
```

| Key                | Type    | Description                                                             |
| ------------------ | ------- | ----------------------------------------------------------------------- |
| `basket_id`        | Uint128 | Basket ID to deposit to                                                 |
| \*`position_owner` | String  | Owner of the position, defaults to info.sender                          |
| \*`position_id`    | Uint128 | Position ID to deposit to. If none is passed, a new position is created |

\* = optional

### `Repay`

{% hint style="info" %}
Used for repaying CW20 as collateral. For repaying native asset credit assets, you need to use the [ExecuteMsg variant](positions.md#repay).
{% endhint %}

Repay outstanding debt for a position, not exclusive to the position owner.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Repay {
        basket_id: Uint128,
        position_id: Uint128,
        position_owner: Option<String>, 
    },
}
```

| Key               | Type    | Description                                             |
| ----------------- | ------- | ------------------------------------------------------- |
| `basket_id`       | Uint128 | ID of basket the position is in                         |
| `position_id`     | Uint128 | ID of position                                          |
| `*position_owner` | String  | Owner of position to repay for, defaults to info.sender |

\* = optional

## CallbackMsg

### `BadDebtCheck`

After liquidations, this checks for bad debt in the liquidated position.

```
BadDebtCheck {
        basket_id: Uint128,
        position_id: Uint128,
        position_owner: Addr,
}
```

| Key              | Type    | Description                     |
| ---------------- | ------- | ------------------------------- |
| `basket_id`      | Uint128 | ID of basket the Position is in |
| `position_id`    | Uint128 | ID of Position                  |
| `position_owner` | Addr    | Owner of Position               |

## QueryMsg

### `Config`

Returns Config

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {}
}

pub struct ConfigResponse {
    pub owner: String,
    pub current_basket_id: Uint128,
    pub stability_pool: String,
    pub dex_router: String, 
    pub fee_collector: String,
    pub osmosis_proxy: String,
    pub liq_fee: Decimal, // 5 = 5%
}
```

### `GetUserPositions`

Returns all Positions from a user

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    GetUserPositions { 
        basket_id: Option<Uint128>, 
        user: String
    },
}

pub struct PositionResponse {
    pub position_id: String,
    pub collateral_assets: Vec<cAsset>,
    pub avg_borrow_LTV: String,
    pub avg_max_LTV: String,
    pub credit_amount: String,
    pub basket_id: String,
    
}
```

| Key          | Type    | Description                                                               |
| ------------ | ------- | ------------------------------------------------------------------------- |
| `*basket_id` | Uint128 | ID of Basket to limit positions to, defaults to positions from all Basket |
| `user`       | String  | Position owner to query for                                               |

\* = optional

### `GetPosition`

Returns single Position data

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    GetPosition { 
        position_id: Uint128, 
        basket_id: Uint128, 
        user: String 
    },
}

pub struct PositionResponse {
    pub position_id: String,
    pub collateral_assets: Vec<cAsset>,
    pub avg_borrow_LTV: String,
    pub avg_max_LTV: String,
    pub credit_amount: String,
    pub basket_id: String,
    
}
```

| Key           | Type    | Description                      |
| ------------- | ------- | -------------------------------- |
| `position_id` | Uint128 | ID of Position                   |
| `basket_id`   | Uint128 | ID of Basket the Position is in  |
| `user`        | String  | User that owns position          |

### `GetBasketPositions`

Returns all positions in a basket with optional limits

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    GetBasketPositions { 
        basket_id: Uint128,
        start_after: Option<String>,
        limit: Option<u32>,
    },
}

pub struct PositionsResponse{
    pub user: String,
    pub positions: Vec<Position>,
}

pub struct Position {
    pub position_id: Uint128,
    pub collateral_assets: Vec<cAsset>,
    pub avg_borrow_LTV: Decimal,
    pub avg_max_LTV: Decimal,
    pub credit_amount: Decimal,
    pub basket_id: Uint128,
}
```

| Key            | Type    | Description                 |
| -------------- | ------- | --------------------------- |
| `basket_id`    | Uint128 | ID of Basket to parse       |
| `*start_after` | String  | User address to start after |
| `*limit`       | u32     | Response output limit       |

\* = optional

### `GetBasket`

Returns Basket parameters

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    GetBasket { basket_id: Uint128 }, 
}

pub struct BasketResponse{
    pub owner: String,
    pub basket_id: String,
    pub current_position_id: String,
    pub collateral_types: Vec<cAsset>, 
    pub credit_asset: Asset, 
    pub credit_price: String,
    pub credit_interest: String,
}
```

| Key         | Type    | Description           |
| ----------- | ------- | --------------------- |
| `basket_id` | Uint128 | ID of Basket to parse |

### `GetAllBaskets`

Returns parameters for all Baskets with optional limiters

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    GetAllBaskets { 
        start_after: Option<Uint128>,
        limit: Option<u32>, 
    }, 
}

pub struct BasketResponse{
    pub owner: String,
    pub basket_id: String,
    pub current_position_id: String,
    pub collateral_types: Vec<cAsset>, 
    pub credit_asset: Asset, 
    pub credit_price: String,
    pub credit_interest: String,
}
```

| Key            | Type    | Description                 |
| -------------- | ------- | --------------------------- |
| `*start_after` | Uint128 | User address to start after |
| `*limit`       | u32     | Response output limit       |

### `GetBasketDebtCaps`

Returns a basket's debt caps per collateral asset, calculates on every call

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    GetBasketDebtCaps {
        basket_id: Uint128,
    }
}
```

| Key         | Type    | Description   |
| ----------- | ------- | ------------- |
| `basket_id` | Uint128 | ID of basket  |

### Propagation

Returns `RepayPropagation.`Used internally to test state propagation.

```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Propagation {}
}

pub struct PropResponse {
    pub liq_queue_leftovers: Decimal,
    pub stability_pool: Decimal,
    pub sell_wall_distributions: Vec<SellWallDistribution>,
    pub positions_contract: String,
    //So the sell wall knows who to repay to
    pub position_id: Uint128,
    pub basket_id: Uint128,
    pub position_owner: String,
}

pub struct SellWallDistribution {
    pub distributions: Vec<( AssetInfo, Decimal )>,
}
```