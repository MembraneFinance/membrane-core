window.SIDEBAR_ITEMS = {"enum":[["BondStatus","BondStatus is the status of a validator."]],"struct":[["Commission","Commission defines commission parameters for a given validator."],["CommissionRates","CommissionRates defines the initial commission rates to be used for creating a validator."],["Delegation","Delegation represents the bond with tokens held by an account. It is owned by one delegator, and is associated with the voting power of one validator."],["DelegationResponse","DelegationResponse is equivalent to Delegation except that it contains a balance in addition to shares which is more suitable for client responses."],["Description","Description defines a validator description."],["DvPair","DVPair is struct that just has a delegator-validator pair with no other data. It is intended to be used as a marshalable pointer. For example, a DVPair can be used to construct the key to getting an UnbondingDelegation from state."],["DvPairs","DVPairs defines an array of DVPair objects."],["DvvTriplet","DVVTriplet is struct that just has a delegator-validator-validator triplet with no other data. It is intended to be used as a marshalable pointer. For example, a DVVTriplet can be used to construct the key to getting a Redelegation from state."],["DvvTriplets","DVVTriplets defines an array of DVVTriplet objects."],["Params","Params defines the parameters for the staking module."],["Pool","Pool is used for tracking bonded and not-bonded token supply of the bond denomination."],["Redelegation","Redelegation contains the list of a particular delegator’s redelegating bonds from a particular source validator to a particular destination validator."],["RedelegationEntry","RedelegationEntry defines a redelegation object with relevant metadata."],["RedelegationEntryResponse","RedelegationEntryResponse is equivalent to a RedelegationEntry except that it contains a balance in addition to shares which is more suitable for client responses."],["RedelegationResponse","RedelegationResponse is equivalent to a Redelegation except that its entries contain a balance in addition to shares which is more suitable for client responses."],["UnbondingDelegation","UnbondingDelegation stores all of a single delegator’s unbonding bonds for a single validator in an time-ordered list."],["UnbondingDelegationEntry","UnbondingDelegationEntry defines an unbonding object with relevant metadata."],["ValAddresses","ValAddresses defines a repeated set of validator addresses."],["Validator","Validator defines a validator, together with the total amount of the Validator’s bond shares and their exchange rate to coins. Slashing results in a decrease in the exchange rate, allowing correct calculation of future undelegations without iterating over delegators. When coins are delegated to this validator, the validator is credited with a delegation whose number of bond shares is based on the amount of coins delegated divided by the current exchange rate. Voting power can be calculated as total bonded shares multiplied by exchange rate."]]};