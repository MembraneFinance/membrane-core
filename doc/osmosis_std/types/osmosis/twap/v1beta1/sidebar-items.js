window.SIDEBAR_ITEMS = {"struct":[["ArithmeticTwapRequest",""],["ArithmeticTwapResponse",""],["ArithmeticTwapToNowRequest",""],["ArithmeticTwapToNowResponse",""],["GenesisState","GenesisState defines the twap module’s genesis state."],["Params","Params holds parameters for the twap module"],["ParamsRequest",""],["ParamsResponse",""],["TwapQuerier",""],["TwapRecord","A TWAP record should be indexed in state by pool_id, (asset pair), timestamp The asset pair assets should be lexicographically sorted. Technically (pool_id, asset_0_denom, asset_1_denom, height) do not need to appear in the struct however we view this as the wrong performance tradeoff given SDK today. Would rather we optimize for readability and correctness, than an optimal state storage format. The system bottleneck is elsewhere for now."]]};