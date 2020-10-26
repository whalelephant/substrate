#![cfg_attr(not(feature = "std"), no_std)]
use codec::{Decode, Encode};
use frame_support::{
    debug, decl_error, decl_event, decl_module, decl_storage, ensure,
    traits::{Currency, ExistenceRequirement},
    weights::Weight,
};
use frame_system::{
    self as system, ensure_signed,
    offchain::{CreateSignedTransaction, SendSignedTransaction, Signer},
};
use sp_core::offchain::{Duration, IpfsRequest, IpfsResponse, Timestamp};
use sp_io::offchain::timestamp;
use sp_runtime::offchain::ipfs;
use sp_std::{prelude::*, str, vec::Vec};

/// Pallet's key pair and signing algo
pub mod crypto;
pub type BalanceOf<T> =
    <<T as Trait>::Currency as Currency<<T as frame_system::Trait>::AccountId>>::Balance;

/// The pallet's configuration trait.
pub trait Trait: system::Trait + CreateSignedTransaction<Call<Self>> {
    /// The identifier type for an offchain worker.
    type AuthorityId: crypto::AppCrypto<Self::Public, Self::Signature>;
    /// The overarching event type.
    type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
    /// The overarching dispatch call type.
    type Call: From<Call<Self>>;
    /// Currency of this pallet
    type Currency: Currency<Self::AccountId>;
}

#[derive(Encode, Decode, PartialEq)]
pub struct Record<T: Trait> {
    owner: Option<<T as system::Trait>::AccountId>,
    avail: Option<u8>,
}

impl<T: Trait> Default for Record<T> {
    fn default() -> Self {
        Self {
            owner: None,
            avail: None,
        }
    }
}

#[derive(Encode, Decode, PartialEq)]
enum DataCommand {
    AddBytes(Vec<u8>),
    CatBytes(Vec<u8>),
    InsertPin(Vec<u8>),
    RemovePin(Vec<u8>),
}

// This pallet's storage items.
decl_storage! {
    trait Store for Module<T: Trait> as StorageMiner {
        // WIP: This is whitelisting a single account
        OWAccountId get(fn ow_account_id) build(|config: &GenesisConfig<T>| config.ow_account_id.clone()): <T as system::Trait>::AccountId;
        // A map for the items to be stored
        pub Storage: map hasher(blake2_128_concat) Vec<u8> => Record<T>;
        // A Queue to handle Ipfs data requests
        pub Queue: Vec<DataCommand>;
    }
    add_extra_genesis {
        config(ow_account_id): T::AccountId;
}}

// The pallet's events
decl_event!(
    pub enum Event<T>
    where
        AccountId = <T as system::Trait>::AccountId,
    {
        /// Returns initiator and QueueId
        PinRequest(AccountId),
        RemovePinRequest(AccountId, Vec<u8>),
        RefundDueToMissingData(AccountId, Vec<u8>),
        CheckRequest(AccountId, Vec<u8>),
        StorageUpdated(IpfsResponse),
        Stored(u64, Vec<u8>),
    }
);

// The pallet's errors
decl_error! {
    pub enum Error for Module<T: Trait> {
        InvalidOW,
        CantCreateRequest,
        RequestTimeout,
        RequestFailed,
        OffchainSignedTxError,
        OffchainSignerError
    }
}

// The pallet's dispatchable functions.
decl_module! {
    /// The module declaration.
    pub struct Module<T: Trait> for enum Call where origin: T::Origin {
        // Initializing errors
        type Error = Error<T>;

        // Initializing events
        fn deposit_event() = default;

        /// Add data to IPFS node and pay flat fee for persistent storage
        #[weight = 200_000]
        pub fn pin(origin, data: Vec<u8>) {
            let who = ensure_signed(origin)?;
            // Ensure has enough fees
            // Transfer fees to Miner
            // Add data to Queue
            Self::deposit_event(RawEvent::PinRequest(who));
        }

        /// Remove pinned data, current gets a constant amount of refund
        #[weight = 200_000]
        pub fn remove_pin(origin, cid: Vec<u8>) {
            let who = ensure_signed(origin)?;
            // Ensure miner has enough fees
            // Transfer fees to caler
            Self::deposit_event(RawEvent::RemovePinRequest(who, cid));
        }

        /// Check if the ipfs node has the cid data
        #[weight = 200_000]
        pub fn check_availability(origin, cid: Vec<u8>) {
            let who = ensure_signed(origin)?;
            // add to queue
            Self::deposit_event(RawEvent::CheckRequest(who, cid));
        }

        /// Report data is not available after `None` is return for avail
        #[weight = 200_000]
        pub fn report_unavailable(origin, cid: Vec<u8>) {
            let who = ensure_signed(origin)?;
            // Check Storage map if None
            // Ensure miner has enough fees
            // Refund initiator, not neccessarily caller
            // Remove cid from Storage map
            Self::deposit_event(RawEvent::RefundDueToMissingData(who, cid));
        }

        /// Offchain worker updates storage, pin, remove_pin, unavilable etc
        #[weight = 200_000]
        fn update_storage(origin, response: IpfsResponse) {
            let who = ensure_signed(origin)?;
            ensure!{
                // This is temp way to only let ocw to update pinned
                who == Self::ow_account_id(),
                Error::<T>::InvalidOW
            }
            // match dataCommand and update storage accordingly
            Self::deposit_event(RawEvent::StorageUpdated(response));
        }


        fn offchain_worker(block_number: T::BlockNumber) {

            // process Ipfs::{add, get} queues every block
            if let Err(e) = Self::handle_data_requests() {
                debug::error!("IPFS: Encountered an error while processing data requests: {:?}", e);
            }

            // display some stats every 5 blocks
            if block_number % 5.into() == 0.into() {
                if let Err(e) = Self::print_metadata() {
                    debug::error!("IPFS: Encountered an error while obtaining metadata: {:?}", e);
                }
            }
        }
    }
}

impl<T: Trait> Module<T> {
    // send a request to the local IPFS node; can only be called be an off-chain worker
    fn ipfs_request(
        req: IpfsRequest,
        deadline: impl Into<Option<Timestamp>>,
    ) -> Result<IpfsResponse, Error<T>> {
        let ipfs_request =
            ipfs::PendingRequest::new(req).map_err(|_| Error::<T>::CantCreateRequest)?;
        ipfs_request
            .try_wait(deadline)
            .map_err(|_| Error::<T>::RequestTimeout)?
            .map(|r| r.response)
            .map_err(|e| {
                if let ipfs::Error::IoError(err) = e {
                    debug::error!("IPFS: request failed: {}", str::from_utf8(&err).unwrap());
                } else {
                    debug::error!("IPFS: request failed: {:?}", e);
                }
                Error::<T>::RequestFailed
            })
    }

    fn handle_data_requests() -> Result<(), Error<T>> {
        let data_queue = Queue::get();
        let len = data_queue.len();
        if len != 0 {
            debug::info!(
                "IPFS: {} entr{} in the data queue",
                len,
                if len == 1 { "y" } else { "ies" }
            );
        }

        let deadline = Some(timestamp().add(Duration::from_millis(1_000)));
        for cmd in data_queue.into_iter() {
            match cmd {
                DataCommand::AddBytes(data) => {
                    match Self::ipfs_request(IpfsRequest::AddBytes(data.clone()), deadline) {
                        Ok(IpfsResponse::AddBytes(cid)) => {
                            debug::info!(
                                "IPFS: added data with Cid {}",
                                str::from_utf8(&cid)
                                    .expect("our own IPFS node can be trusted here; qed")
                            );
                            match Self::ocw_update_storage(IpfsResponse::AddBytes(cid.clone())) {
                                Ok(_) => debug::info!("ipfs returned cid {:?}", cid),
                                Err(e) => debug::error!("IPFS: add error: {:?}", e),
                            }
                        }
                        Ok(_) => unreachable!(
                            "only AddBytes can be a response for that request type; qed"
                        ),
                        Err(e) => {
                            // should return some fund to initiator
                            debug::error!("IPFS: add error: {:?}", e)
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn print_metadata() -> Result<(), Error<T>> {
        let deadline = Some(timestamp().add(Duration::from_millis(200)));

        let peers =
            if let IpfsResponse::Peers(peers) = Self::ipfs_request(IpfsRequest::Peers, deadline)? {
                peers
            } else {
                unreachable!("only Peers can be a response for that request type; qed");
            };
        let peer_count = peers.len();

        debug::info!(
            "IPFS: currently connected to {} peer{}",
            peer_count,
            if peer_count == 1 { "" } else { "s" },
        );

        Ok(())
    }

    fn ocw_update_storage(response: IpfsResponse) -> Result<(), Error<T>> {
        // We retrieve a signer and check if it is valid.
        //   Since this pallet only has one key in the keystore. We use `any_account()1 to
        //   retrieve it. If there are multiple keys and we want to pinpoint it, `with_filter()` can be chained,
        //   ref: https://substrate.dev/rustdocs/v2.0.0/frame_system/offchain/struct.Signer.html
        let signer = Signer::<T, T::AuthorityId>::any_account();

        let result =
            signer.send_signed_transaction(|_account| Call::update_storage(response.clone()));

        if let Some((_, res)) = result {
            if res.is_err() {
                debug::error!("signed tx error {:?}", res);
                return Err(Error::<T>::OffchainSignedTxError);
            }
            return Ok(());
        } else {
            // The case of `None`: no account is available for sending
            debug::error!("No local account available");
            return Err(<Error<T>>::OffchainSignerError);
        };
    }
}
