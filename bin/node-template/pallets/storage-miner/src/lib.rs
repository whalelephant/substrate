#![cfg_attr(not(feature = "std"), no_std)]
use codec::{Decode, Encode};
use frame_support::{
    debug, decl_error, decl_event, decl_module, decl_storage,
    dispatch::DispatchResult,
    ensure,
    traits::{Currency, ExistenceRequirement, Get},
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
    /// Mock flat fee for storage
    type StorageFee: Get<BalanceOf<Self>>;
    /// Mock flat refund fee
    type RefundFee: Get<BalanceOf<Self>>;
}

#[derive(Encode, Decode, PartialEq)]
pub struct Record<T: Trait> {
    initiator: Option<<T as system::Trait>::AccountId>,
    avail: Option<Timestamp>,
}

impl<T: Trait> Default for Record<T> {
    fn default() -> Self {
        Self {
            initiator: None,
            avail: None,
        }
    }
}

#[derive(Encode, Decode, PartialEq)]
enum DataCommand<T: Trait> {
    AddBytes(<T as system::Trait>::AccountId, Vec<u8>),
    CatBytes(Vec<u8>),
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
        pub Queue: Vec<DataCommand<T>>;
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
        OffchainSignerError,
        Unpinned,
        NotInitiator,
        NotInStorage,
        ReportAvailabelData,
        RefundError,
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

        fn on_initialize(block_number: T::BlockNumber) -> Weight {
            Queue::<T>::kill();
            0
        }
        /// Add data to STORAGE node and pay flat fee for persistent storage
        #[weight = 200_000]
        pub fn pin(origin, data: Vec<u8>) {
            let who = ensure_signed(origin)?;
            Self::add_and_pin_data(who.clone(), data)?;
            Self::deposit_event(RawEvent::PinRequest(who));
        }

        /// Remove pinned data, current gets a constant amount of refund
        #[weight = 200_000]
        pub fn remove_pin(origin, cid: Vec<u8>) {
            let who = ensure_signed(origin)?;
            Self::remove_pinned_data(who.clone(), cid.clone())?;
            Self::deposit_event(RawEvent::RemovePinRequest(who, cid));
        }

        /// Check if the ipfs node has the cid data
        #[weight = 200_000]
        pub fn check_availability(origin, cid: Vec<u8>) {
            let who = ensure_signed(origin)?;
            Self::check_avail(cid.clone());
            Self::deposit_event(RawEvent::CheckRequest(who, cid));
        }

        /// Report data is not available after `None` is return for avail
        #[weight = 200_000]
        pub fn report(origin, cid: Vec<u8>) {
            let who = ensure_signed(origin)?;
            Self::report_unavailable(cid.clone())?;
            Self::deposit_event(RawEvent::RefundDueToMissingData(who, cid));
        }

        /// Offchain worker updates storage, pin, remove_pin, unavilable etc
        #[weight = 200_000]
        fn update_storage(origin, initiator: Option<T::AccountId>, response: IpfsResponse, remark:
            bool) {
            let who = ensure_signed(origin)?;
            ensure!{
                // This is temp way to only let ocw to update pinned
                who == Self::ow_account_id(),
                Error::<T>::InvalidOW
            }
            match response {
                IpfsResponse::AddBytes(ref cid) => {
                    Storage::<T>::insert(cid, Record::<T> {
                        initiator: initiator,
                        avail: Some(timestamp())
                    })
                }
                // using cid here as need key to update storage
                IpfsResponse::CatBytes(ref cid) => {
                    if Storage::<T>::contains_key(cid.clone()) {
                        Storage::<T>::mutate(cid, |record| {
                            if remark {
                                record.avail = Some(timestamp());
                            } else {
                                record.avail = None;
                            }
                        })
                    }
                }
                _ =>{}
            }
            Self::deposit_event(RawEvent::StorageUpdated(response));
        }


        fn offchain_worker(block_number: T::BlockNumber) {

            // process Ipfs::{add, get} queues every block
            if let Err(e) = Self::handle_data_requests() {
                debug::error!("STORAGE: Encountered an error while processing data requests: {:?}", e);
            }

            // display some stats every 5 blocks
            if block_number % 5.into() == 0.into() {
                if let Err(e) = Self::print_metadata() {
                    debug::error!("STORAGE: Encountered an error while obtaining metadata: {:?}", e);
                }
            }
        }
    }
}

impl<T: Trait> Module<T> {
    // send a request to the local STORAGE node; can only be called be an off-chain worker
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
                    debug::error!("STORAGE: request failed: {}", str::from_utf8(&err).unwrap());
                } else {
                    debug::error!("STORAGE: request failed: {:?}", e);
                }
                Error::<T>::RequestFailed
            })
    }

    fn handle_data_requests() -> Result<(), Error<T>> {
        let data_queue = Queue::<T>::get();
        let len = data_queue.len();
        if len != 0 {
            debug::info!(
                "STORAGE: {} entr{} in the data queue",
                len,
                if len == 1 { "y" } else { "ies" }
            );
        }

        let deadline = Some(timestamp().add(Duration::from_millis(1_000)));
        for cmd in data_queue.into_iter() {
            match cmd {
                DataCommand::AddBytes(initiator, data) => {
                    match Self::ipfs_request(IpfsRequest::AddBytes(data.clone()), deadline) {
                        Ok(IpfsResponse::AddBytes(cid)) => {
                            match Self::ipfs_request(
                                IpfsRequest::InsertPin(cid.clone(), false),
                                deadline,
                            ) {
                                Ok(IpfsResponse::Success) => {
                                    match Self::ocw_update_storage(
                                        Some(initiator.clone()),
                                        IpfsResponse::AddBytes(cid.clone()),
                                        false,
                                    ) {
                                        Ok(_) => debug::info!("STORAGE::pin for cid {:?}", cid),
                                        Err(e) => {
                                            debug::error!("STORAGE::pin error: {:?}", e);

                                            // try update status as need to be refunded?
                                            let _ = Self::refund(initiator, T::StorageFee::get());
                                        }
                                    }
                                }
                                Ok(_) => {
                                    unreachable!("Only Success for response for request type; qed")
                                }
                                Err(e) => debug::error!("STORAGE: pin error: {:?}", e),
                            }
                        }
                        Ok(_) => unreachable!(
                            "only AddBytes can be a response for that request type; qed"
                        ),
                        Err(e) => {
                            debug::error!("STORAGE: add error: {:?}", e);
                            // try update status as need to be refunded?
                            let _ = Self::refund(initiator, T::StorageFee::get());
                        }
                    }
                }
                DataCommand::RemovePin(cid) => {
                    match Self::ipfs_request(IpfsRequest::RemovePin(cid.clone(), false), deadline) {
                        Ok(IpfsResponse::Success) => {
                            debug::info!(
                                "STORAGE: unpinned data with Cid {}",
                                str::from_utf8(&cid)
                                    .expect("our own request can be trusted to be UTF-8; qed")
                            );
                        }
                        Ok(_) => {
                            unreachable!("only Success can be a response for request type; qed")
                        }
                        Err(e) => debug::error!("STORAGE: remove pin error: {:?}", e),
                    }
                }
                DataCommand::CatBytes(cid) => {
                    match Self::ipfs_request(IpfsRequest::CatBytes(cid.clone()), deadline) {
                        Ok(IpfsResponse::CatBytes(_)) => {
                            match Self::ocw_update_storage(
                                None,
                                IpfsResponse::CatBytes(cid.clone()),
                                true,
                            ) {
                                Ok(_) => debug::info!("STORAGE::check updated for cid {:?}", cid),
                                Err(e) => debug::error!("STORAGE::check error: {:?}", e),
                            }
                        }
                        Ok(_) => unreachable!(
                            "only CatBytes can be a response for that request type; qed"
                        ),
                        Err(e) => {
                            debug::error!("STORAGE: cat error: {:?}", e);
                            match Self::ocw_update_storage(
                                None,
                                IpfsResponse::CatBytes(cid.clone()),
                                false,
                            ) {
                                Ok(_) => debug::info!("STORAGE::check updated for cid {:?}", cid),
                                Err(e) => debug::error!("STORAGE::check error: {:?}", e),
                            }
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
            "STORAGE: currently connected to {} peer{}",
            peer_count,
            if peer_count == 1 { "" } else { "s" },
        );

        Ok(())
    }

    fn ocw_update_storage(
        initiator: Option<T::AccountId>,
        response: IpfsResponse,
        remark: bool,
    ) -> Result<(), Error<T>> {
        // We retrieve a signer and check if it is valid.
        //   Since this pallet only has one key in the keystore. We use `any_account()1 to
        //   retrieve it. If there are multiple keys and we want to pinpoint it, `with_filter()` can be chained,
        //   ref: https://substrate.dev/rustdocs/v2.0.0/frame_system/offchain/struct.Signer.html
        let signer = Signer::<T, T::AuthorityId>::any_account();

        let result = signer.send_signed_transaction(|_account| {
            Call::update_storage(initiator.clone(), response.clone(), remark)
        });

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

    fn refund(to: T::AccountId, amount: BalanceOf<T>) -> Result<(), Error<T>> {
        T::Currency::transfer(
            &Self::ow_account_id(),
            &to,
            amount,
            ExistenceRequirement::KeepAlive,
        )
        .map_err(|e| {
            debug::info! {"Refund error: {:?}", e};
            Error::<T>::RefundError
        })
    }

    pub fn add_and_pin_data(from: T::AccountId, data: Vec<u8>) -> DispatchResult {
        T::Currency::transfer(
            &from,
            &Self::ow_account_id(),
            T::StorageFee::get(),
            ExistenceRequirement::KeepAlive,
        )?;
        Queue::<T>::mutate(|q| q.push(DataCommand::<T>::AddBytes(from, data)));
        Ok(())
    }

    pub fn remove_pinned_data(from: T::AccountId, cid: Vec<u8>) -> Result<(), Error<T>> {
        let record: Record<T> = Storage::take(cid.clone());
        match record.initiator {
            Some(initiator) => {
                if initiator == from {
                    Self::refund(from, T::RefundFee::get())?;
                    Queue::<T>::mutate(|q| q.push(DataCommand::<T>::RemovePin(cid)));
                    Ok(())
                } else {
                    Err(<Error<T>>::NotInitiator)
                }
            }
            None => Err(<Error<T>>::Unpinned),
        }
    }

    pub fn check_avail(cid: Vec<u8>) {
        Queue::<T>::mutate(|q| q.push(DataCommand::<T>::CatBytes(cid)));
    }

    pub fn report_unavailable(cid: Vec<u8>) -> Result<(), Error<T>> {
        if Storage::<T>::contains_key(cid.clone()) {
            let record: Record<T> = Storage::get(cid.clone());
            match record.avail {
                Some(_) => Err(Error::<T>::ReportAvailabelData),
                None => {
                    // Existing records can only have Some
                    Self::refund(record.initiator.unwrap(), T::RefundFee::get())?;
                    Storage::<T>::remove(cid);
                    Ok(())
                }
            }
        } else {
            Err(Error::<T>::NotInStorage)
        }
    }
}
