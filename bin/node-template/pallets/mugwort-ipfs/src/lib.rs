#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Decode, Encode};
use frame_support::{debug, decl_error, decl_event, decl_module, decl_storage, weights::Weight};
use frame_system::{
    self as system, ensure_signed,
    offchain::{SendTransactionTypes, SubmitTransaction},
};
use sp_core::offchain::{Duration, IpfsRequest, IpfsResponse, Timestamp};
use sp_io::offchain::timestamp;
use sp_runtime::offchain::ipfs;
use sp_std::{str, vec::Vec};

/// The pallet's configuration trait.
pub trait Trait: system::Trait + SendTransactionTypes<Call<Self>> {
    /// The overarching event type.
    type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
    /// The overarching dispatch call type.
    type Call: From<Call<Self>>;
}

#[derive(Default, Encode, Decode, PartialEq)]
pub struct ConjurationState<AccountId> {
    owner: AccountId,
    metadata: Vec<u8>,
}

#[derive(Encode, Decode, PartialEq)]
enum DataCommand {
    AddBytes(ArtId, Vec<u8>),
}

type ArtId = u64;

// This pallet's storage items.
decl_storage! {
    trait Store for Module<T: Trait> as MugwortIpfs {
        // WIP: Offchain_worker id - Used for signing transactions and ensuring only some keys are
        // allowed.
        // OWAccountId get(fn ow_account_id) build(|config: &GenesisConfig<T>| config.ow_account_id.clone()) : <T as system::Trait>::AccountId;
        // Counter for Art work
        CurrentArtId get(fn current_art_id): ArtId;
        // A map for the conjured art states
        Artwork get(fn art_work): map hasher(blake2_128_concat) ArtId => ConjurationState<<T as
                                  system::Trait>::AccountId>;
        // A Queue to handle Ipfs data requests
        DataQueue: Vec<DataCommand>;
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
        // Spell cast by Account
        SpellCast(AccountId),
        // Artwork conjured by Account, with id and metadata cid
        Conjured(ArtId, Vec<u8>),
    }
);

// The pallet's errors
decl_error! {
    pub enum Error for Module<T: Trait> {
        Overflow,
        InvalidOW,
        CantCreateRequest,
        RequestTimeout,
        RequestFailed,
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

        // needs to be synchronized with offchain_worker actitivies
        fn on_initialize(block_number: T::BlockNumber) -> Weight {
            DataQueue::kill();
            0
        }

        /// Conjure a piece of art aka mint. This only initiates the process
        #[weight = 200_000]
        pub fn conjure_art(origin, data: Vec<u8>) {
            let who = ensure_signed(origin)?;
            let new_id = Self::current_art_id().checked_add(1).ok_or(Error::<T>::Overflow)?;
            CurrentArtId::put(new_id);
            Artwork::<T>::insert(new_id, ConjurationState::<T::AccountId>{
                owner: who.clone(),
                metadata: Vec::new(),
            });
            DataQueue::mutate(|q| q.push(DataCommand::AddBytes(new_id, data)));
            Self::deposit_event(RawEvent::SpellCast(who));
        }

        /// Offchain worker returns the cid
        #[weight = 200_000]
        fn transfigure_art(origin, id: ArtId, cid: Vec<u8>) {
            // WIP: Used for signed transactions
            //let who = ensure_signed(origin)?;
            //ensure!{
            //    who == Self::ow_account_id(),
            //    Error::<T>::InvalidOW
            //}
            Artwork::<T>::mutate(id, |art_work| {
                art_work.metadata = cid.clone();
            });
            Self::deposit_event(RawEvent::Conjured( id, cid));
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
        let data_queue = DataQueue::get();
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
                DataCommand::AddBytes(art_id, data) => {
                    match Self::ipfs_request(IpfsRequest::AddBytes(data.clone()), deadline) {
                        Ok(IpfsResponse::AddBytes(cid)) => {
                            debug::info!(
                                "IPFS: added data with Cid {}",
                                str::from_utf8(&cid)
                                    .expect("our own IPFS node can be trusted here; qed")
                            );

                            match Self::transfigure_with_cid(art_id, data) {
                                Ok(_) => debug::info!("ipfs returned cid for art_id: {}", art_id),
                                Err(e) => {
                                    debug::error!("IPFS: add error for art_id {}: {:?}", art_id, e)
                                }
                            }
                        }
                        Ok(_) => unreachable!(
                            "only AddBytes can be a response for that request type; qed"
                        ),
                        Err(e) => debug::error!("IPFS: add error: {:?}", e),
                    }
                }
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

    fn transfigure_with_cid(id: ArtId, cid: Vec<u8>) -> Result<(), &'static str> {
        // WIP: Using unsigned transactions for now
        // let signer = Signer::<T, T::AuthorityId>::all_accounts();
        // if !signer.can_sign() {
        //     return Err(
        //         "No local accounts available. Consider adding one via `author_insertKey` RPC.",
        //     )?;
        // }

        // let results =
        //     signer.send_signed_transaction(|_account| Call::transfigure_art(id, cid.clone()));

        // for (acc, res) in &results {
        //     match res {
        //         Ok(()) => debug::info!("[{:?}] Transfigured art of id: {}", acc.id, id),
        //         Err(e) => debug::error!(
        //             "[{:?}] Failed to submit for transfiguration: {:?}",
        //             acc.id,
        //             e
        //         ),
        //     }
        // }

        let call = Call::transfigure_art(id, cid);
        SubmitTransaction::<T, Call<T>>::submit_unsigned_transaction(call.into())
            .map_err(|()| "Unable to submit unsigned transaction.")?;
        Ok(())
    }
}
