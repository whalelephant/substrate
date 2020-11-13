## Simple Demo Stoage Miner Pallet

This pallet aims to use `Offchain::ipfs` features in a simple pallet where other pallets /
clients can request for the node to store some data for some fee.

### Calls

#### Pin
1. Transfer a flat storage fee from the requestor to the miner designated account
1. Starts a offchain worker to perform `Addbytes` and `InsertPin` sequentially
1. Offchain worker then sends a transaction to update chain state - `Storage` map
1. Any failure result in the miner refunding the full storage fee

#### Unpin
1. Refunds a flat refund fee from the miner to the requestor (must be same account stored in `Pin`)
   if cid is in `Storage`
1. Starts an offchain worker to perform `RemovePin`

#### CheckAvailability
1. Starts an offchain worker to perform `CatBytes`
1. Return results of Success / Failure to the chain state by a signed transaction to update the availability of
   the addressable content

#### Report
1. Refund a flat refund fee if the data for the given cid is not available

### Frontend Interactions
1. Navigate to https://polkadot.js.org/apps/
2. Connect to the Development network
3. Make sure you add the below to _Settings_ > _Developer_ so that polkadotjs knows how to decode
   the data returned as the default types are for polkadot runtime.
   ```
   {
	   "Address": "AccountId",
	   "LookupSource": "AccountId",
	   "Record": {
		   "initiator": "Option<AccountId>",
		   "avail": "Option<u64>"
	   }
   }
   ```

