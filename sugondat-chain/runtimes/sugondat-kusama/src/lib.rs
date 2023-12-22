#![cfg_attr(not(feature = "std"), no_std)]
// `construct_runtime!` does a lot of recursion and requires us to increase the limit to 256.
#![recursion_limit = "256"]

// Make the WASM binary available.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

mod constants;
mod weights;
pub mod xcm_config;

use cumulus_pallet_parachain_system::RelayNumberStrictlyIncreases;
use cumulus_primitives_core::{AggregateMessageOrigin, ParaId};
use pallet_transaction_payment::{Multiplier, MultiplierUpdate};
use polkadot_runtime_common::xcm_sender::NoPriceForMessageDelivery;
use sp_api::impl_runtime_apis;
use sp_core::{crypto::KeyTypeId, OpaqueMetadata};
use sp_runtime::{
    create_runtime_str, generic, impl_opaque_keys,
    traits::{
        AccountIdLookup, BlakeTwo256, Block as BlockT, Bounded, Convert, SaturatedConversion,
        Saturating,
    },
    transaction_validity::{TransactionSource, TransactionValidity},
    ApplyExtrinsicResult, FixedPointNumber, Perquintill,
};

use sp_std::prelude::*;
#[cfg(feature = "std")]
use sp_version::NativeVersion;
use sp_version::RuntimeVersion;

use constants::consensus::*;
use constants::kusama::{currency::*, fee::WeightToFee};
use frame_support::{
    construct_runtime,
    dispatch::DispatchClass,
    genesis_builder_helper::{build_config, create_default_config},
    parameter_types,
    traits::{ConstBool, ConstU32, ConstU64, ConstU8, EitherOfDiverse, Get, TransformOrigin},
    weights::{ConstantMultiplier, Weight},
    PalletId,
};
use frame_system::{
    limits::{BlockLength, BlockWeights},
    EnsureRoot,
};
use pallet_xcm::{EnsureXcm, IsVoiceOfBody};
use parachains_common::message_queue::{NarrowOriginToSibling, ParaIdToSibling};

use sugondat_primitives::{
    AccountId, AuraId, Balance, BlockNumber, Nonce, Signature, MAXIMUM_BLOCK_LENGTH,
};

use pallet_transaction_payment::TargetedFeeAdjustment;
pub use sp_runtime::{MultiAddress, Perbill, Permill};
use xcm_config::{KusamaLocation, XcmOriginToTransactDispatchOrigin};

#[cfg(any(feature = "std", test))]
pub use sp_runtime::BuildStorage;

// Polkadot imports
use polkadot_runtime_common::BlockHashCount;

use weights::{BlockExecutionWeight, ExtrinsicBaseWeight, RocksDbWeight};

// XCM Imports
use xcm::latest::prelude::BodyId;

pub use pallet_sugondat_blobs;

/// A hash of some data used by the chain.
pub type Hash = sp_core::H256;

/// The address format for describing accounts.
pub type Address = MultiAddress<AccountId, ()>;

/// Block header type as expected by this runtime.
pub type Header = generic::Header<BlockNumber, BlakeTwo256>;

/// Block type as expected by this runtime.
pub type Block = generic::Block<Header, UncheckedExtrinsic>;

/// A Block signed with a Justification
pub type SignedBlock = generic::SignedBlock<Block>;

/// BlockId type as expected by this runtime.
pub type BlockId = generic::BlockId<Block>;

/// The SignedExtension to the basic transaction logic.
pub type SignedExtra = (
    frame_system::CheckNonZeroSender<Runtime>,
    frame_system::CheckSpecVersion<Runtime>,
    frame_system::CheckTxVersion<Runtime>,
    frame_system::CheckGenesis<Runtime>,
    frame_system::CheckEra<Runtime>,
    frame_system::CheckNonce<Runtime>,
    frame_system::CheckWeight<Runtime>,
    pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
    pallet_sugondat_blobs::PrevalidateBlobs<Runtime>,
);

/// Unchecked extrinsic type as expected by this runtime.
pub type UncheckedExtrinsic =
    generic::UncheckedExtrinsic<Address, RuntimeCall, Signature, SignedExtra>;

/// Executive: handles dispatch to the various modules.
pub type Executive = frame_executive::Executive<
    Runtime,
    Block,
    frame_system::ChainContext<Runtime>,
    Runtime,
    AllPalletsWithSystem,
>;

impl_opaque_keys! {
    pub struct SessionKeys {
        pub aura: Aura,
    }
}

#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: create_runtime_str!("blobchain-kusama"),
    impl_name: create_runtime_str!("blobchain-kusama"),
    authoring_version: 1,
    spec_version: 1,
    impl_version: 0,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 1,
    state_version: 1,
};

/// The version information used to identify this runtime when compiled natively.
#[cfg(feature = "std")]
pub fn native_version() -> NativeVersion {
    NativeVersion {
        runtime_version: VERSION,
        can_author_with: Default::default(),
    }
}

parameter_types! {
    pub const Version: RuntimeVersion = VERSION;

    // This part is copied from Substrate's `bin/node/runtime/src/lib.rs`.
    //  The `RuntimeBlockLength` and `RuntimeBlockWeights` exist here because the
    // `DeletionWeightLimit` and `DeletionQueueDepth` depend on those to parameterize
    // the lazy contract deletion.
    pub RuntimeBlockLength: BlockLength =
        BlockLength::max_with_normal_ratio(MAXIMUM_BLOCK_LENGTH, NORMAL_DISPATCH_RATIO);
    pub RuntimeBlockWeights: BlockWeights = BlockWeights::builder()
        .base_block(BlockExecutionWeight::get())
        .for_class(DispatchClass::all(), |weights| {
            weights.base_extrinsic = ExtrinsicBaseWeight::get();
        })
        .for_class(DispatchClass::Normal, |weights| {
            weights.max_total = Some(NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT);
        })
        .for_class(DispatchClass::Operational, |weights| {
            weights.max_total = Some(MAXIMUM_BLOCK_WEIGHT);
            // Operational transactions have some extra reserved space, so that they
            // are included even if block reached `MAXIMUM_BLOCK_WEIGHT`.
            weights.reserved = Some(
                MAXIMUM_BLOCK_WEIGHT - NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT
            );
        })
        .avg_block_initialization(AVERAGE_ON_INITIALIZE_RATIO)
        .build_or_panic();
    pub const SS58Prefix: u16 = 2; // Kusama
}

/// Call filter that allows all kinds of transactions except for balance transfers.
pub struct NoBalanceTransfers;

impl frame_support::traits::Contains<RuntimeCall> for NoBalanceTransfers {
    fn contains(x: &RuntimeCall) -> bool {
        match *x {
            RuntimeCall::Balances(pallet_balances::Call::transfer_allow_death { .. })
            | RuntimeCall::Balances(pallet_balances::Call::transfer_keep_alive { .. })
            | RuntimeCall::Balances(pallet_balances::Call::transfer_all { .. }) => false,
            _ => true,
        }
    }
}

// Configure FRAME pallets to include in runtime.

impl frame_system::Config for Runtime {
    /// The identifier used to distinguish between accounts.
    type AccountId = AccountId;
    /// The aggregated dispatch type that is available for extrinsics.
    type RuntimeCall = RuntimeCall;
    /// The lookup mechanism to get account ID from whatever is passed in dispatchers.
    type Lookup = AccountIdLookup<AccountId, ()>;
    /// The index type for storing how many extrinsics an account has signed.
    type Nonce = Nonce;
    /// The type for hashing blocks and tries.
    type Hash = Hash;
    /// The hashing algorithm used.
    type Hashing = BlakeTwo256;
    /// The block type.
    type Block = Block;
    /// The ubiquitous event type.
    type RuntimeEvent = RuntimeEvent;
    /// The ubiquitous origin type.
    type RuntimeOrigin = RuntimeOrigin;
    /// Maximum number of block number to block hash mappings to keep (oldest pruned first).
    type BlockHashCount = BlockHashCount;
    /// Runtime version.
    type Version = Version;
    /// Converts a module to an index of this module in the runtime.
    type PalletInfo = PalletInfo;
    /// The data to be stored in an account.
    type AccountData = pallet_balances::AccountData<Balance>;
    /// What to do if a new account is created.
    type OnNewAccount = ();
    /// What to do if an account is fully reaped from the system.
    type OnKilledAccount = ();
    /// The weight of database operations that the runtime can invoke.
    type DbWeight = RocksDbWeight;
    /// The basic call filter to use in dispatchable.
    type BaseCallFilter = NoBalanceTransfers;
    /// Weight information for the extrinsics of this pallet.
    type SystemWeightInfo = ();
    /// Block & extrinsics weights: base values and limits.
    type BlockWeights = RuntimeBlockWeights;
    /// The maximum length of a block (in bytes).
    type BlockLength = RuntimeBlockLength;
    /// This is used as an identifier of the chain. 42 is the generic substrate prefix.
    type SS58Prefix = SS58Prefix;
    /// The action to take on a Runtime Upgrade
    type OnSetCode = cumulus_pallet_parachain_system::ParachainSetCode<Self>;
    type MaxConsumers = frame_support::traits::ConstU32<16>;
}

impl pallet_timestamp::Config for Runtime {
    /// A timestamp: milliseconds since the unix epoch.
    type Moment = u64;
    type OnTimestampSet = Aura;
    type MinimumPeriod = ConstU64<{ SLOT_DURATION / 2 }>;
    type WeightInfo = ();
}

impl pallet_authorship::Config for Runtime {
    type FindAuthor = pallet_session::FindAccountFromAuthorIndex<Self, Aura>;
    type EventHandler = (CollatorSelection,);
}

parameter_types! {
    pub const ExistentialDeposit: Balance = EXISTENTIAL_DEPOSIT;
}

impl pallet_balances::Config for Runtime {
    type MaxLocks = ConstU32<50>;
    /// The type for recording an account's balance.
    type Balance = Balance;
    /// The ubiquitous event type.
    type RuntimeEvent = RuntimeEvent;
    type DustRemoval = ();
    type ExistentialDeposit = ExistentialDeposit;
    type AccountStore = System;
    type WeightInfo = pallet_balances::weights::SubstrateWeight<Runtime>;
    type MaxReserves = ConstU32<50>;
    type ReserveIdentifier = [u8; 8];
    type RuntimeHoldReason = RuntimeHoldReason;
    type RuntimeFreezeReason = RuntimeFreezeReason;
    type FreezeIdentifier = ();
    type MaxHolds = ConstU32<0>;
    type MaxFreezes = ConstU32<0>;
}

parameter_types! {
    /// Relay Chain `TransactionByteFee` / 10
    pub const TransactionByteFee: Balance = MILLICENTS;

    // parameters used by BlobsFeeAdjustment
    // to update NextFeeMultiplier and NextLengthMultiplier
    //
    // Common constants used in all runtimes for SlowAdjustingFeeUpdate
    /// The portion of the `NORMAL_DISPATCH_RATIO` that we adjust the fees with. Blocks filled less
    /// than this will decrease the weight and more will increase.
    pub storage TargetBlockFullness: Perquintill = Perquintill::from_percent(25);

    /// The adjustment variable of the runtime. Higher values will cause `TargetBlockFullness` to
    /// change the fees more rapidly.
    pub AdjustmentVariableBlockFullness: Multiplier = Multiplier::saturating_from_rational(75, 1000_000);
    /// that combined with `AdjustmentVariable`, we can recover from the minimum.
    /// See `multiplier_can_grow_from_zero`.
    pub MinimumMultiplierBlockFullness: Multiplier = Multiplier::saturating_from_rational(1, 10u128);
    /// The maximum amount of the multiplier.
    pub MaximumMultiplierBlockFullness: Multiplier = Bounded::max_value();


    pub storage NextLengthMultiplier: Multiplier = Multiplier::saturating_from_integer(1);
    pub storage TargetBlockSize: Perquintill = Perquintill::from_percent(16); // 0.8MiB
    // TODO: update those value accordingly with https://github.com/thrumdev/blobs/issues/16
    pub AdjustmentVariableBlockSize: Multiplier = Multiplier::saturating_from_rational(75, 1000_000);
    pub MinimumMultiplierBlockSize: Multiplier = Multiplier::saturating_from_rational(1, 10u128);
    pub MaximumMultiplierBlockSize: Multiplier = Bounded::max_value();
}

/// Currently pallet_transaction_payment use the following formula:
///
/// ```ignore
/// inclusion_fee = base_fee + length_fee + [targeted_fee_adjustment * weight_fee];
/// ```
///
/// Letting us able to update `targeted_fee_adjustment` at the end of each block
/// thanks to `FeeMultiplierUpdate`, this associated type is called inside the `on_finalize`
/// of the transaction_payment pallet with the aim of converting the before `targeted_fee_adjustment`
/// to a new one based on the congestion of the network
///
/// What this struct does is this PLUS a side effect, the goal is to reach a different formula to
/// calculate fees:
///
/// ```ignore
/// inclusion_fee = base_fee + [targeted_length_fee_adjustment * length_fee] + [targeted_weight_fee_adjustment * weight_fee];
/// ```
///
/// As you can see `targeted_fee_adjustment` becomes `targeted_weight_fee_adjustment` but the behavior
/// remains the same, the side effect is the changing to the value `targeted_length_fee_adjustment`,
/// this formula is achievable because inside pallet_transaction_payment the function `compute_fee_raw`
/// that just computes the final fee associated with an extrinsic uses the associated type `LengthToFee`
/// that converts the length of an extrinsic to a fee.
///
/// By default the implementation is a constant multiplication but we want to achieve a dynamic formula
/// that can adapt based on the usage of the network, this can't solely be done by this struct but needs
/// to be bundled with a custom implementation of `LengthToFee`.
///
/// This struct ONLY provide a dynamic update of `targeted_length_fee_adjustment` and `targeted_weight_fee_adjustment`
/// based on the congestion and usage of the blocks, while the formula si effectively implemented like
/// explained above only thanks to `LengthToFee`
pub struct BlobsFeeAdjustment<T: frame_system::Config>(core::marker::PhantomData<T>);

impl<T: frame_system::Config> Convert<Multiplier, Multiplier> for BlobsFeeAdjustment<T>
where
    T: frame_system::Config,
{
    /// This function should be a pure function used to update NextFeeMultiplier
    /// but will also has the side effect of update NextLengthMultiplier
    fn convert(previous_fee_multiplier: Multiplier) -> Multiplier {
        // Update NextLengthMultiplier

        // To update the value will be used the same formula as TargetedFeeAdjustment,
        // described here: https://research.web3.foundation/Polkadot/overview/token-economics#2-slow-adjusting-mechanism
        //
        // so this is mainly a copy paste of that function because it works on normalized mesurments,
        // so if it is ref_time, proof_size or length of the extrinsic the mutliplier will be evaluated properly.
        // The main problem is that TargetedFeeAdjustment::convert uses directly a call to the storage to extract
        // the weight of the current block so there is no way to pass the length as input argument,
        // here I will copy paste all the needed part to update properly NextLengthMultiplier

        // Defensive only. The multiplier in storage should always be at most positive. Nonetheless
        // we recover here in case of errors, because any value below this would be stale and can
        // never change.

        let previous_len_multiplier = NextLengthMultiplier::get();
        let min_multiplier = MinimumMultiplierBlockSize::get();
        let max_multiplier = MaximumMultiplierBlockSize::get();
        let previous_len_multiplier = previous_len_multiplier.max(min_multiplier);

        // Pick the limiting dimension. (from TargetedFeeAdjustment::convert)
        //
        // In this case it is the length of all extrinsic, always
        let (normal_limiting_dimension, max_limiting_dimension) = (
            <frame_system::Pallet<T>>::all_extrinsics_len(),
            MAXIMUM_BLOCK_LENGTH as u64,
        );

        let target_block_size = TargetBlockSize::get();
        let adjustment_variable = AdjustmentVariableBlockSize::get();

        let target_size = (target_block_size * max_limiting_dimension) as u128;
        let block_size = normal_limiting_dimension as u128;

        // determines if the first_term is positive
        let positive = block_size >= target_size;
        let diff_abs = block_size.max(target_size) - block_size.min(target_size);

        // defensive only, a test case assures that the maximum weight diff can fit in Multiplier
        // without any saturation.
        let diff = Multiplier::saturating_from_rational(diff_abs, max_limiting_dimension.max(1));
        let diff_squared = diff.saturating_mul(diff);

        let v_squared_2 = adjustment_variable.saturating_mul(adjustment_variable)
            / Multiplier::saturating_from_integer(2);

        let first_term = adjustment_variable.saturating_mul(diff);
        let second_term = v_squared_2.saturating_mul(diff_squared);

        let new_len_multiplier = if positive {
            let excess = first_term
                .saturating_add(second_term)
                .saturating_mul(previous_len_multiplier);
            previous_len_multiplier
                .saturating_add(excess)
                .clamp(min_multiplier, max_multiplier)
        } else {
            // Defensive-only: first_term > second_term. Safe subtraction.
            let negative = first_term
                .saturating_sub(second_term)
                .saturating_mul(previous_len_multiplier);
            previous_len_multiplier
                .saturating_sub(negative)
                .clamp(min_multiplier, max_multiplier)
        };

        NextLengthMultiplier::set(&new_len_multiplier);

        // Update NextFeeMultiplier
        //
        // Here is the tricky part, this method return the new value associated with
        // NextFeeMultiplier (in the old fashion) because weight dynamic adjustment is battle tested
        // while previously have updated the `NextLengthMultiplier` used in `LengthToWeight`
        TargetedFeeAdjustment::<
            T,
            TargetBlockFullness,
            AdjustmentVariableBlockFullness,
            MinimumMultiplierBlockFullness,
            MaximumMultiplierBlockFullness,
        >::convert(previous_fee_multiplier)
    }
}

impl<T: frame_system::Config> MultiplierUpdate for BlobsFeeAdjustment<T> {
    fn min() -> Multiplier {
        MinimumMultiplierBlockFullness::get()
    }
    fn max() -> Multiplier {
        MaximumMultiplierBlockFullness::get()
    }
    fn target() -> Perquintill {
        TargetBlockFullness::get()
    }
    fn variability() -> Multiplier {
        AdjustmentVariableBlockFullness::get()
    }
}

pub struct BlobsLengthToFee<T: frame_system::Config>(core::marker::PhantomData<T>);

impl<T: frame_system::Config> sp_weights::WeightToFee for BlobsLengthToFee<T> {
    type Balance = Balance;

    fn weight_to_fee(weight: &Weight) -> Self::Balance {
        // really weird but weght.ref_time will contain the length of the extrinsic
        let length_fee = Self::Balance::saturated_from(weight.ref_time())
            .saturating_mul(TransactionByteFee::get());
        let multiplier = NextLengthMultiplier::get();

        // final adjusted length fee
        multiplier.saturating_mul_int(length_fee)
    }
}

impl pallet_transaction_payment::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type OnChargeTransaction = pallet_transaction_payment::CurrencyAdapter<Balances, ()>;
    type WeightToFee = WeightToFee;
    type LengthToFee = BlobsLengthToFee<Self>;
    type FeeMultiplierUpdate = BlobsFeeAdjustment<Self>;
    type OperationalFeeMultiplier = ConstU8<5>;
}

impl pallet_sudo::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type RuntimeCall = RuntimeCall;
    type WeightInfo = ();
}

parameter_types! {
    pub const ReservedXcmpWeight: Weight = MAXIMUM_BLOCK_WEIGHT.saturating_div(4);
    pub const ReservedDmpWeight: Weight = MAXIMUM_BLOCK_WEIGHT.saturating_div(4);
    pub const RelayOrigin: AggregateMessageOrigin = AggregateMessageOrigin::Parent;
}

impl cumulus_pallet_parachain_system::Config for Runtime {
    type WeightInfo = ();
    type RuntimeEvent = RuntimeEvent;
    type OnSystemEvent = ();
    type SelfParaId = parachain_info::Pallet<Runtime>;
    type OutboundXcmpMessageSource = XcmpQueue;
    type DmpQueue = frame_support::traits::EnqueueWithOrigin<MessageQueue, RelayOrigin>;
    type ReservedDmpWeight = ReservedDmpWeight;
    type XcmpMessageHandler = XcmpQueue;
    type ReservedXcmpWeight = ReservedXcmpWeight;
    type CheckAssociatedRelayNumber = RelayNumberStrictlyIncreases;
    type ConsensusHook = cumulus_pallet_aura_ext::FixedVelocityConsensusHook<
        Runtime,
        RELAY_CHAIN_SLOT_DURATION_MILLIS,
        BLOCK_PROCESSING_VELOCITY,
        UNINCLUDED_SEGMENT_CAPACITY,
    >;
}

impl parachain_info::Config for Runtime {}

parameter_types! {
    pub MessageQueueServiceWeight: Weight = Perbill::from_percent(35) * RuntimeBlockWeights::get().max_block;
}

impl pallet_message_queue::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = ();
    #[cfg(feature = "runtime-benchmarks")]
    type MessageProcessor = pallet_message_queue::mock_helpers::NoopMessageProcessor<
        cumulus_primitives_core::AggregateMessageOrigin,
    >;
    #[cfg(not(feature = "runtime-benchmarks"))]
    type MessageProcessor = xcm_builder::ProcessXcmMessage<
        AggregateMessageOrigin,
        xcm_executor::XcmExecutor<xcm_config::XcmConfig>,
        RuntimeCall,
    >;
    type Size = u32;
    // The XCMP queue pallet is only ever able to handle the `Sibling(ParaId)` origin:
    type QueueChangeHandler = NarrowOriginToSibling<XcmpQueue>;
    type QueuePausedQuery = NarrowOriginToSibling<XcmpQueue>;
    type HeapSize = sp_core::ConstU32<{ 64 * 1024 }>;
    type MaxStale = sp_core::ConstU32<8>;
    type ServiceWeight = MessageQueueServiceWeight;
}

impl cumulus_pallet_aura_ext::Config for Runtime {}

impl cumulus_pallet_xcmp_queue::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type ChannelInfo = ParachainSystem;
    type VersionWrapper = ();
    // Enqueue XCMP messages from siblings for later processing.
    type XcmpQueue = TransformOrigin<MessageQueue, AggregateMessageOrigin, ParaId, ParaIdToSibling>;
    type MaxInboundSuspended = sp_core::ConstU32<1_000>;
    type ControllerOrigin = EnsureRoot<AccountId>;
    type ControllerOriginConverter = XcmOriginToTransactDispatchOrigin;
    type WeightInfo = ();
    type PriceForSiblingDelivery = NoPriceForMessageDelivery<ParaId>;
}

parameter_types! {
    pub const Period: u32 = 6 * HOURS;
    pub const Offset: u32 = 0;
}

impl pallet_session::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type ValidatorId = <Self as frame_system::Config>::AccountId;
    // we don't have stash and controller, thus we don't need the convert as well.
    type ValidatorIdOf = pallet_collator_selection::IdentityCollator;
    type ShouldEndSession = pallet_session::PeriodicSessions<Period, Offset>;
    type NextSessionRotation = pallet_session::PeriodicSessions<Period, Offset>;
    type SessionManager = CollatorSelection;
    // Essentially just Aura, but let's be pedantic.
    type SessionHandler = <SessionKeys as sp_runtime::traits::OpaqueKeys>::KeyTypeIdProviders;
    type Keys = SessionKeys;
    type WeightInfo = ();
}

impl pallet_aura::Config for Runtime {
    type AuthorityId = AuraId;
    type DisabledValidators = ();
    type MaxAuthorities = ConstU32<100_000>;
    type AllowMultipleBlocksPerSlot = ConstBool<false>;
    #[cfg(feature = "experimental")]
    type SlotDuration = pallet_aura::MinimumPeriodTimesTwo<Self>;
}

parameter_types! {
    pub const PotId: PalletId = PalletId(*b"PotStake");
    pub const SessionLength: BlockNumber = 6 * HOURS;
    // StakingAdmin pluralistic body.
    pub const StakingAdminBodyId: BodyId = BodyId::Defense;
}

/// We allow root and the StakingAdmin to execute privileged collator selection operations.
pub type CollatorSelectionUpdateOrigin = EitherOfDiverse<
    EnsureRoot<AccountId>,
    EnsureXcm<IsVoiceOfBody<KusamaLocation, StakingAdminBodyId>>,
>;

impl pallet_collator_selection::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type Currency = Balances;
    type UpdateOrigin = CollatorSelectionUpdateOrigin;
    type PotId = PotId;
    type MaxCandidates = ConstU32<0>;
    type MinEligibleCollators = ConstU32<4>;
    type MaxInvulnerables = ConstU32<20>;
    // should be a multiple of session or things will get inconsistent
    type KickThreshold = Period;
    type ValidatorId = <Self as frame_system::Config>::AccountId;
    type ValidatorIdOf = pallet_collator_selection::IdentityCollator;
    type ValidatorRegistration = Session;
    type WeightInfo = ();
}

parameter_types! {
    pub const MaxBlobs: u32 = 100 * 1024;
    pub const MaxBlobSize: u32 = 100 * 1024;
    pub const MaxTotalBlobSize: u32 = 2 * 1024 * 1024;
}

impl pallet_sugondat_blobs::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type MaxBlobs = MaxBlobs;
    type MaxBlobSize = MaxBlobSize;
    type MaxTotalBlobSize = MaxTotalBlobSize;
    type WeightInfo = pallet_sugondat_blobs::weights::SubstrateWeight<Runtime>;
}

// Create the runtime by composing the FRAME pallets that were previously configured.
construct_runtime!(
    pub enum Runtime
    {
        // System support stuff.
        System: frame_system = 0,
        ParachainSystem: cumulus_pallet_parachain_system = 1,
        Timestamp: pallet_timestamp = 2,
        ParachainInfo: parachain_info = 3,

        // Monetary stuff.
        Balances: pallet_balances = 10,
        TransactionPayment: pallet_transaction_payment = 11,

        // Governance
        Sudo: pallet_sudo = 15,

        // Collator support. The order of these 4 are important and shall not change.
        Authorship: pallet_authorship = 20,
        CollatorSelection: pallet_collator_selection = 21,
        Session: pallet_session = 22,
        Aura: pallet_aura = 23,
        AuraExt: cumulus_pallet_aura_ext = 24,

        // XCM helpers.
        XcmpQueue: cumulus_pallet_xcmp_queue = 30,
        PolkadotXcm: pallet_xcm = 31,
        CumulusXcm: cumulus_pallet_xcm = 32,
        MessageQueue: pallet_message_queue = 33,

        Blobs: pallet_sugondat_blobs = 40,
    }
);

#[cfg(feature = "runtime-benchmarks")]
mod benches {
    frame_benchmarking::define_benchmarks!(
        [frame_system, SystemBench::<Runtime>]
        [pallet_balances, Balances]
        [pallet_session, SessionBench::<Runtime>]
        [pallet_timestamp, Timestamp]
        [pallet_sudo, Sudo]
        [pallet_collator_selection, CollatorSelection]
        [cumulus_pallet_xcmp_queue, XcmpQueue]
        [pallet_sugondat_blobs, Blobs]
    );
}

impl_runtime_apis! {
    impl sp_consensus_aura::AuraApi<Block, AuraId> for Runtime {
        fn slot_duration() -> sp_consensus_aura::SlotDuration {
            sp_consensus_aura::SlotDuration::from_millis(Aura::slot_duration())
        }

        fn authorities() -> Vec<AuraId> {
            Aura::authorities().into_inner()
        }
    }

    impl sp_api::Core<Block> for Runtime {
        fn version() -> RuntimeVersion {
            VERSION
        }

        fn execute_block(block: Block) {
            Executive::execute_block(block)
        }

        fn initialize_block(header: &<Block as BlockT>::Header) {
            Executive::initialize_block(header)
        }
    }

    impl sp_api::Metadata<Block> for Runtime {
        fn metadata() -> OpaqueMetadata {
            OpaqueMetadata::new(Runtime::metadata().into())
        }

        fn metadata_at_version(version: u32) -> Option<OpaqueMetadata> {
            Runtime::metadata_at_version(version)
        }

        fn metadata_versions() -> sp_std::vec::Vec<u32> {
            Runtime::metadata_versions()
        }
    }

    impl sp_block_builder::BlockBuilder<Block> for Runtime {
        fn apply_extrinsic(extrinsic: <Block as BlockT>::Extrinsic) -> ApplyExtrinsicResult {
            Executive::apply_extrinsic(extrinsic)
        }

        fn finalize_block() -> <Block as BlockT>::Header {
            Executive::finalize_block()
        }

        fn inherent_extrinsics(data: sp_inherents::InherentData) -> Vec<<Block as BlockT>::Extrinsic> {
            data.create_extrinsics()
        }

        fn check_inherents(
            block: Block,
            data: sp_inherents::InherentData,
        ) -> sp_inherents::CheckInherentsResult {
            data.check_extrinsics(&block)
        }
    }

    impl sp_transaction_pool::runtime_api::TaggedTransactionQueue<Block> for Runtime {
        fn validate_transaction(
            source: TransactionSource,
            tx: <Block as BlockT>::Extrinsic,
            block_hash: <Block as BlockT>::Hash,
        ) -> TransactionValidity {
            Executive::validate_transaction(source, tx, block_hash)
        }
    }

    impl sp_offchain::OffchainWorkerApi<Block> for Runtime {
        fn offchain_worker(header: &<Block as BlockT>::Header) {
            Executive::offchain_worker(header)
        }
    }

    impl sp_session::SessionKeys<Block> for Runtime {
        fn generate_session_keys(seed: Option<Vec<u8>>) -> Vec<u8> {
            SessionKeys::generate(seed)
        }

        fn decode_session_keys(
            encoded: Vec<u8>,
        ) -> Option<Vec<(Vec<u8>, KeyTypeId)>> {
            SessionKeys::decode_into_raw_public_keys(&encoded)
        }
    }

    impl frame_system_rpc_runtime_api::AccountNonceApi<Block, AccountId, Nonce> for Runtime {
        fn account_nonce(account: AccountId) -> Nonce {
            System::account_nonce(account)
        }
    }

    impl pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<Block, Balance> for Runtime {
        fn query_info(
            uxt: <Block as BlockT>::Extrinsic,
            len: u32,
        ) -> pallet_transaction_payment_rpc_runtime_api::RuntimeDispatchInfo<Balance> {
            TransactionPayment::query_info(uxt, len)
        }
        fn query_fee_details(
            uxt: <Block as BlockT>::Extrinsic,
            len: u32,
        ) -> pallet_transaction_payment::FeeDetails<Balance> {
            TransactionPayment::query_fee_details(uxt, len)
        }
        fn query_weight_to_fee(weight: Weight) -> Balance {
            TransactionPayment::weight_to_fee(weight)
        }
        fn query_length_to_fee(length: u32) -> Balance {
            TransactionPayment::length_to_fee(length)
        }
    }

    impl pallet_transaction_payment_rpc_runtime_api::TransactionPaymentCallApi<Block, Balance, RuntimeCall>
        for Runtime
    {
        fn query_call_info(
            call: RuntimeCall,
            len: u32,
        ) -> pallet_transaction_payment::RuntimeDispatchInfo<Balance> {
            TransactionPayment::query_call_info(call, len)
        }
        fn query_call_fee_details(
            call: RuntimeCall,
            len: u32,
        ) -> pallet_transaction_payment::FeeDetails<Balance> {
            TransactionPayment::query_call_fee_details(call, len)
        }
        fn query_weight_to_fee(weight: Weight) -> Balance {
            TransactionPayment::weight_to_fee(weight)
        }
        fn query_length_to_fee(length: u32) -> Balance {
            TransactionPayment::length_to_fee(length)
        }
    }

    impl cumulus_primitives_core::CollectCollationInfo<Block> for Runtime {
        fn collect_collation_info(header: &<Block as BlockT>::Header) -> cumulus_primitives_core::CollationInfo {
            ParachainSystem::collect_collation_info(header)
        }
    }

    #[cfg(feature = "try-runtime")]
    impl frame_try_runtime::TryRuntime<Block> for Runtime {
        fn on_runtime_upgrade(checks: frame_try_runtime::UpgradeCheckSelect) -> (Weight, Weight) {
            let weight = Executive::try_runtime_upgrade(checks).unwrap();
            (weight, RuntimeBlockWeights::get().max_block)
        }

        fn execute_block(
            block: Block,
            state_root_check: bool,
            signature_check: bool,
            select: frame_try_runtime::TryStateSelect,
        ) -> Weight {
            // NOTE: intentional unwrap: we don't want to propagate the error backwards, and want to
            // have a backtrace here.
            Executive::try_execute_block(block, state_root_check, signature_check, select).unwrap()
        }
    }

    #[cfg(feature = "runtime-benchmarks")]
    impl frame_benchmarking::Benchmark<Block> for Runtime {
        fn benchmark_metadata(extra: bool) -> (
            Vec<frame_benchmarking::BenchmarkList>,
            Vec<frame_support::traits::StorageInfo>,
        ) {
            use frame_benchmarking::{Benchmarking, BenchmarkList};
            use frame_support::traits::StorageInfoTrait;
            use frame_system_benchmarking::Pallet as SystemBench;
            use cumulus_pallet_session_benchmarking::Pallet as SessionBench;

            let mut list = Vec::<BenchmarkList>::new();
            list_benchmarks!(list, extra);

            let storage_info = AllPalletsWithSystem::storage_info();
            (list, storage_info)
        }

        fn dispatch_benchmark(
            config: frame_benchmarking::BenchmarkConfig
        ) -> Result<Vec<frame_benchmarking::BenchmarkBatch>, sp_runtime::RuntimeString> {
            use frame_benchmarking::{BenchmarkError, Benchmarking, BenchmarkBatch};

            use frame_system_benchmarking::Pallet as SystemBench;
            impl frame_system_benchmarking::Config for Runtime {
                fn setup_set_code_requirements(code: &sp_std::vec::Vec<u8>) -> Result<(), BenchmarkError> {
                    ParachainSystem::initialize_for_set_code_benchmark(code.len() as u32);
                    Ok(())
                }

                fn verify_set_code() {
                    System::assert_last_event(cumulus_pallet_parachain_system::Event::<Runtime>::ValidationFunctionStored.into());
                }
            }

            use cumulus_pallet_session_benchmarking::Pallet as SessionBench;
            impl cumulus_pallet_session_benchmarking::Config for Runtime {}

            use frame_support::traits::WhitelistedStorageKeys;
            let whitelist = AllPalletsWithSystem::whitelisted_storage_keys();

            let mut batches = Vec::<BenchmarkBatch>::new();
            let params = (&config, &whitelist);
            add_benchmarks!(params, batches);

            if batches.is_empty() { return Err("Benchmark not found for this pallet.".into()) }
            Ok(batches)
        }
    }

    impl sp_genesis_builder::GenesisBuilder<Block> for Runtime {
        fn create_default_config() -> Vec<u8> {
            create_default_config::<RuntimeGenesisConfig>()
        }

        fn build_config(config: Vec<u8>) -> sp_genesis_builder::Result {
            build_config::<RuntimeGenesisConfig>(config)
        }
    }
}

cumulus_pallet_parachain_system::register_validate_block! {
    Runtime = Runtime,
    BlockExecutor = cumulus_pallet_aura_ext::BlockExecutor::<Runtime, Executive>,
}
