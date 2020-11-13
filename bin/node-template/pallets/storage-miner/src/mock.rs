use super::*;
use crate::crypto;
use frame_support::{impl_outer_event, impl_outer_origin, parameter_types, weights::Weight};
use frame_system as system;
use pallet_balances;
use sp_core::H256;
use sp_runtime::{
    testing::Header,
    traits::{BlakeTwo256, IdentityLookup},
    Perbill,
};
type Balance = u128;

#[derive(Clone, Eq, PartialEq)]
pub struct Test;
parameter_types! {
    pub const ExistentialDeposit: Balance = 100;
    pub const BlockHashCount: u64 = 250;
    pub const MaximumBlockWeight: Weight = 1024;
    pub const MaximumBlockLength: u32 = 2 * 1024;
    pub const AvailableBlockRatio: Perbill = Perbill::from_percent(75);
    pub const StorageFee: Balance = 200_000_000_000_000;
    pub const RefundFee: Balance = 100_000_000_000_000;
}

impl system::Trait for Test {
    type BaseCallFilter = ();
    type Origin = Origin;
    type Call = ();
    type Index = u64;
    type BlockNumber = u64;
    type Hash = H256;
    type Hashing = BlakeTwo256;
    type AccountId = u64;
    type Lookup = IdentityLookup<Self::AccountId>;
    type Header = Header;
    type Event = TestEvent;
    type BlockHashCount = BlockHashCount;
    type MaximumBlockWeight = MaximumBlockWeight;
    type DbWeight = ();
    type BlockExecutionWeight = ();
    type ExtrinsicBaseWeight = ();
    type MaximumExtrinsicWeight = MaximumBlockWeight;
    type MaximumBlockLength = MaximumBlockLength;
    type AvailableBlockRatio = AvailableBlockRatio;
    type Version = ();
    type PalletInfo = ();
    type AccountData = pallet_balances::AccountData<Balance>;
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = ();
}
impl pallet_balances::Trait for Test {
    type Balance = Balance;
    type DustRemoval = ();
    type Event = TestEvent;
    type ExistentialDeposit = ExistentialDeposit;
    type AccountStore = system::Module<Test>;
    type WeightInfo = ();
}

impl Trait for Test {
    type AuthorityId = crypto::TestAuthId;
    type Call = Call;
    type StorageFee = StorageFee;
    type RefundFee = RefundFee;
    type Event = TestEvent;
    type Currency = Balances;
}

impl_outer_event! {
    pub enum TestEvent for Test {
        system<T>,
        pallet_balances<T>,
    }
}

impl_outer_origin! {
    pub enum Origin for Test where system = frame_system {}
}

pub type Balances = pallet_balances::Module<Test>;
pub type System = frame_system::Module<Test>;
pub type StorageMiner = Module<Test>;

pub fn new_test_ext() -> sp_io::TestExternalities {
    let mut t = frame_system::GenesisConfig::default()
        .build_storage::<Test>()
        .unwrap();
    let genesis = pallet_balances::GenesisConfig::<Test> {
        balances: vec![(0, 100_000_000_000_000), (1, 100_000_000_000_000)],
    };
    genesis.assimilate_storage(&mut t).unwrap();
    t.into()
}
