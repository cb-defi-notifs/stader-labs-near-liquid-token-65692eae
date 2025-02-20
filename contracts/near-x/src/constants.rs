use near_sdk::{EpochHeight, ONE_NEAR};

pub const NO_DEPOSIT: u128 = 0;
pub const NEAR_CENT: u128 = ONE_NEAR / 100;
pub const ONE_MILLI_NEAR: u128 = ONE_NEAR / 1_000;
pub const ONE_MICRO_NEAR: u128 = ONE_NEAR / 1_000_000;
pub const TWO_NEAR: u128 = 2 * ONE_NEAR;
pub const FIVE_NEAR: u128 = 5 * ONE_NEAR;
pub const TEN_NEAR: u128 = 10 * ONE_NEAR;
pub const K_NEAR: u128 = 1_000 * ONE_NEAR;

pub const NUM_EPOCHS_TO_UNLOCK: EpochHeight = 4;
// Number of epochs to wait for reward fee to set
pub const REWARD_FEE_SET_WAIT_TIME: EpochHeight = 4;

/// Storage keys
pub const ACCOUNTS_MAP: &str = "A";
pub const VALIDATOR_MAP: &str = "B";

uint::construct_uint! {
        /// 256-bit unsigned integer.
        pub struct U256(4);
}

pub mod gas {
    use near_sdk::Gas;

    /// Gas attached to deposit call on the staking pool contract.
    pub const DEPOSIT_AND_STAKE: Gas = base_gas(3);

    /// Gas attached to stake call on the staking pool contract.
    pub const STAKE: Gas = base_gas(3);

    /// The amount of gas required to get current unstaked balance of this account from the
    /// staking pool.
    pub const GET_ACCOUNT_UNSTAKED_BALANCE: Gas = base_gas(1);

    /// The amount of gas required to get the current total balance of this account from the
    /// staking pool.
    pub const GET_ACCOUNT_TOTAL_BALANCE: Gas = base_gas(1);

    /// Gas attached to the inner callback for processing result of the deposit and stake call to
    /// the staking pool.
    pub const ON_STAKE_POOL_DEPOSIT_AND_STAKE: Gas = base_gas(1);

    pub const ON_STAKE_POOL_DEPOSIT_AND_STAKE_CB: Gas = base_gas(1);

    /// The amount of gas required to get the current staked balance of this account from the
    /// staking pool.
    pub const ON_STAKE_POOL_GET_ACCOUNT_STAKED_BALANCE: Gas = base_gas(1);

    pub const ON_STAKE_POOL_GET_ACCOUNT_STAKED_BALANCE_CB: Gas = base_gas(1);

    pub const ON_STAKE_POOL_GET_ACCOUNT_TOTAL_BALANCE: Gas = base_gas(1);

    pub const ON_STAKE_POOL_GET_ACCOUNT_TOTAL_BALANCE_CB: Gas = base_gas(1);

    /// Gas attached to the inner callback for processing result of the call to get the current total balance from the staking pool.
    pub const ON_GET_SP_STAKED_BALANCE_TO_RECONCILE: Gas = tera(5);

    pub const ON_STAKE_POOL_WITHDRAW_ALL: Gas = base_gas(3);

    pub const ON_STAKE_POOL_WITHDRAW_ALL_CB: Gas = base_gas(3);

    pub const ON_STAKE_POOL_UNSTAKE: Gas = base_gas(3);

    pub const ON_STAKE_POOL_UNSTAKE_CB: Gas = base_gas(3);

    pub const WITHDRAW_EPOCH: Gas = base_gas(3);

    pub const UNSTAKING_EPOCH: Gas = base_gas(3);

    pub const STAKING_EPOCH: Gas = base_gas(3);

    pub const AUTOCOMPOUNDING_EPOCH: Gas = base_gas(3);

    pub const SYNC_VALIDATOR_EPOCH: Gas = base_gas(3);

    pub const DRAIN_UNSTAKE: Gas = base_gas(3);

    pub const DRAIN_WITHDRAW: Gas = base_gas(3);

    pub const FT_TRANSFER_RESOLVE: Gas = tera(12);

    pub const FT_TRANSFER: Gas = tera(35);

    const fn base_gas(n: u64) -> Gas {
        Gas(1_000_000_000_000 * 25 * n)
    }

    const fn tera(n: u64) -> Gas {
        Gas(1_000_000_000_000 * n)
    }
}
