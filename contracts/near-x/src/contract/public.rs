use crate::constants::{ACCOUNTS_MAP, REWARD_FEE_SET_WAIT_TIME, VALIDATOR_MAP};
use crate::errors::*;
use crate::events::Event;
use crate::{contract::*, state::*};
use near_sdk::json_types::U64;
use near_sdk::near_bindgen;
use near_sdk::{assert_one_yocto, require, ONE_NEAR};

#[near_bindgen]
impl NearxPool {
    #[init]
    pub fn new(
        owner_account_id: AccountId,
        operator_account_id: AccountId,
        treasury_account_id: AccountId,
    ) -> Self {
        require!(
            owner_account_id != operator_account_id,
            ERROR_OWNER_OPERATOR_SAME
        );
        require!(
            owner_account_id != treasury_account_id,
            ERROR_OWNER_TREASURY_SAME
        );
        require!(
            operator_account_id != treasury_account_id,
            ERROR_OPERATOR_TREASURY_SAME
        );
        require!(
            operator_account_id != env::current_account_id(),
            ERROR_OPERATOR_CURRENT_CONTRACT_SAME
        );
        require!(
            treasury_account_id != env::current_account_id(),
            ERROR_TREASURY_CURRENT_CONTRACT_SAME
        );
        require!(
            owner_account_id != env::current_account_id(),
            ERROR_OWNER_CURRENT_CONTRACT_SAME
        );

        Self {
            owner_account_id,
            operator_account_id,
            accumulated_staked_rewards: 0,
            user_amount_to_stake_in_epoch: 0,
            user_amount_to_unstake_in_epoch: 0,
            reconciled_epoch_stake_amount: 0,
            reconciled_epoch_unstake_amount: 0,
            total_stake_shares: 0,
            accounts: UnorderedMap::new(ACCOUNTS_MAP.as_bytes()),
            min_deposit_amount: ONE_NEAR,
            validator_info_map: UnorderedMap::new(VALIDATOR_MAP.as_bytes()),
            total_staked: 0,
            rewards_fee: Fraction::new(0, 1),
            last_reconcilation_epoch: 0,
            temp_owner: None,
            temp_operator: None,
            temp_treasury: None,
            temp_reward_fee: None,
            last_reward_fee_set_epoch: 0,
            operations_control: OperationControls {
                stake_paused: false,
                direct_stake_paused: false,
                unstaked_paused: false,
                withdraw_paused: false,
                staking_epoch_paused: false,
                unstaking_epoch_paused: false,
                withdraw_epoch_paused: false,
                autocompounding_epoch_paused: false,
                sync_validator_balance_paused: false,
                ft_transfer_paused: false,
                ft_transfer_call_paused: false,
            },
            treasury_account_id,
            total_validator_weight: 0,
            rewards_buffer: 0,
            accumulated_rewards_buffer: 0,
            min_storage_reserve: 0,
        }
    }

    /*
       Main staking pool api
    */
    #[payable]
    pub fn update_rewards_buffer(&mut self) {
        self.assert_operator_or_owner();
        self.internal_update_rewards_buffer(env::attached_deposit())
    }

    // the difference b/w manager_deposit_and_stake and direct_deposit_and_stake is that direct
    // deposit_and_stake only allows the user to directly stake into a private validator. manager_deposit
    // _and_stake allows manager to deposit to any validator directly. For private validators, we keep
    // track of user activity offchain and update the max unstakable limit if the tracked user has does certain blacklisted actions
    #[payable]
    pub fn manager_deposit_and_stake(&mut self, validator: AccountId) {
        self.assert_operator_or_owner();
        self.internal_manager_deposit_and_stake(env::attached_deposit(), validator);
    }

    #[payable]
    pub fn direct_deposit_and_stake(&mut self, validator: AccountId) {
        self.internal_direct_deposit_and_stake(env::attached_deposit(), validator);
    }

    #[payable]
    pub fn deposit_and_stake(&mut self) {
        self.internal_deposit_and_stake(env::attached_deposit());
    }

    /// Unstakes all staked balance from the inner account of the predecessor.
    /// The new total unstaked balance will be available for withdrawal in four epochs.
    pub fn unstake_all(&mut self) {
        let account_id = env::predecessor_account_id();
        let account = self.internal_get_account(&account_id);
        let amount = self.staked_amount_from_num_shares_rounded_down(account.stake_shares);
        self.internal_unstake(amount);
    }

    /// Unstakes the given amount from the inner account of the predecessor.
    /// The inner account should have enough staked balance.
    /// The new total unstaked balance will be available for withdrawal in four epochs.
    pub fn unstake(&mut self, amount: U128) {
        let amount: Balance = amount.into();
        self.internal_unstake(amount);
    }

    /// Withdraws the entire unstaked balance from the predecessor account.
    /// It's only allowed if the `unstake` action was not performed in the four most recent epochs.
    pub fn withdraw_all(&mut self) {
        let account_id = env::predecessor_account_id();
        let account = self.internal_get_account(&account_id);
        self.internal_withdraw(account.unstaked_amount);
    }

    /// Withdraws the non staked balance for given account.
    /// It's only allowed if the `unstake` action was not performed in the four most recent epochs.
    pub fn withdraw(&mut self, amount: U128) {
        let amount: Balance = amount.into();
        self.internal_withdraw(amount);
    }

    /*
       Validator pool addition and deletion
    */
    #[payable]
    pub fn pause_validator(&mut self, validator: AccountId) {
        self.assert_operator_or_owner();
        assert_one_yocto();

        let mut validator_info = self.internal_get_validator(&validator);

        // Need to check for this as drain_withdraw and epoch_withdraw are not the same
        // drain_withdraw places the withdrawn amount back to the batched deposit
        require!(
            !validator_info.pending_unstake_release(),
            ERROR_VALIDATOR_UNSTAKE_STILL_UNBONDING
        );

        let current_validator_weight = validator_info.weight;
        self.total_validator_weight -= current_validator_weight;
        validator_info.weight = 0;
        self.internal_update_validator(&validator, &validator_info);

        Event::ValidatorPaused {
            account_id: validator,
            old_weight: current_validator_weight,
        }
        .emit();
    }

    #[payable]
    pub fn remove_validator(&mut self, validator: AccountId) {
        self.assert_operator_or_owner();
        assert_one_yocto();

        let validator_info = self.internal_get_validator(&validator);

        require!(validator_info.is_empty(), ERROR_INVALID_VALIDATOR_REMOVAL);

        self.total_validator_weight -= validator_info.weight;
        self.validator_info_map.remove(&validator);

        Event::ValidatorRemoved {
            account_id: validator,
        }
        .emit();
    }

    #[payable]
    pub fn add_validator(&mut self, validator: AccountId, weight: u16) {
        self.assert_operator_or_owner();
        assert_one_yocto();
        if self.validator_info_map.get(&validator).is_some() {
            panic!("{}", ERROR_VALIDATOR_IS_ALREADY_PRESENT);
        }
        self.validator_info_map.insert(
            &validator,
            &ValidatorInfo::new(validator.clone(), weight).into(),
        );
        self.total_validator_weight += weight;

        Event::ValidatorAdded {
            account_id: validator,
            weight,
        }
        .emit();
    }

    #[payable]
    pub fn update_validator(&mut self, validator: AccountId, weight: u16) {
        self.assert_operator_or_owner();
        assert_one_yocto();

        let mut validator_info = self.internal_get_validator(&validator);

        if weight == 0 {
            require!(false, ERROR_INVALID_VALIDATOR_WEIGHT);
        }

        // update total weight
        self.total_validator_weight = self.total_validator_weight + weight - validator_info.weight;
        validator_info.weight = weight;

        self.internal_update_validator(&validator, &validator_info);
        // self.validator_info_map.insert(&validator, &validator_info);

        Event::ValidatorUpdated {
            account_id: validator,
            weight,
        }
        .emit();
    }

    #[payable]
    pub fn make_validator_private(
        &mut self,
        validator: AccountId,
        initial_max_unstakable_limit: Option<U128>,
    ) {
        self.assert_operator_or_owner();
        assert_one_yocto();

        let mut validator_info = self.internal_get_validator(&validator);

        // don't accidently make a private validator renounce his stake!!
        require!(
            validator_info.validator_type == ValidatorType::PUBLIC,
            ERROR_VALIDATOR_IS_PRIVATE
        );

        let max_unstakable_limit =
            initial_max_unstakable_limit.unwrap_or(U128(validator_info.staked));
        require!(
            max_unstakable_limit.0 <= validator_info.staked,
            ERROR_VALIDATOR_MAX_UNSTAKABLE_LIMIT_GREATER_THAN_STAKED_AMOUNT
        );

        validator_info.validator_type = ValidatorType::PRIVATE;

        // all the public stake before is unstakable. All the private stake from now on will be not be directly unstakable
        validator_info.max_unstakable_limit = max_unstakable_limit.0;

        self.internal_update_validator(&validator, &validator_info);

        Event::MakeValidatorPrivate {
            validator_id: validator,
        }
        .emit();
    }

    #[payable]
    pub fn make_validator_public(&mut self, validator: AccountId) {
        self.assert_operator_or_owner();
        assert_one_yocto();

        let mut validator_info = self.internal_get_validator(&validator);

        require!(
            validator_info.validator_type == ValidatorType::PRIVATE,
            ERROR_VALIDATOR_IS_PUBLIC
        );

        validator_info.validator_type = ValidatorType::PUBLIC;
        // all the existing stake is available to unstake from even the private stakes
        validator_info.max_unstakable_limit = validator_info.staked;

        self.internal_update_validator(&validator, &validator_info);

        Event::MakeValidatorPublic {
            validator_id: validator,
        }
        .emit();
    }

    #[payable]
    pub fn update_validator_max_unstakeable_limit(
        &mut self,
        validator: AccountId,
        amount_unstaked: U128,
    ) {
        self.assert_operator_or_owner();
        assert_one_yocto();

        let mut validator_info = self.internal_get_validator(&validator);

        require!(
            validator_info.validator_type == ValidatorType::PRIVATE,
            ERROR_VALIDATOR_IS_PUBLIC
        );

        let new_max_unstakable_limit = validator_info.max_unstakable_limit + amount_unstaked.0;
        require!(
            new_max_unstakable_limit <= validator_info.staked,
            ERROR_VALIDATOR_MAX_UNSTAKABLE_LIMIT_GREATER_THAN_STAKED_AMOUNT
        );

        // for private validators this will be 0 initially, since we cannot unstake from them without cause.
        validator_info.max_unstakable_limit = new_max_unstakable_limit;

        self.internal_update_validator(&validator, &validator_info);

        Event::UpdateValidatorMaxUnstakableLimit {
            validator_id: validator,
            amount_unstaked,
            new_max_unstakable_limit: U128(validator_info.max_unstakable_limit),
        }
        .emit();
    }

    // Owner update methods
    #[payable]
    pub fn set_owner(&mut self, new_owner: AccountId) {
        assert_one_yocto();
        self.assert_owner_calling();

        // owner, operator, treasury and current contract address should all be different
        require!(
            new_owner != self.operator_account_id,
            ERROR_OWNER_OPERATOR_SAME
        );
        require!(
            new_owner != self.treasury_account_id,
            ERROR_OWNER_TREASURY_SAME
        );
        require!(new_owner != self.owner_account_id, ERROR_OWNER_SAME);
        require!(
            new_owner != env::current_account_id(),
            ERROR_OWNER_CURRENT_CONTRACT_SAME
        );

        self.temp_owner = Some(new_owner.clone());
        Event::SetOwner {
            old_owner: self.owner_account_id.clone(),
            new_owner,
        }
        .emit();
    }

    #[payable]
    pub fn commit_owner(&mut self) {
        assert_one_yocto();

        if let Some(temp_owner) = self.temp_owner.clone() {
            require!(
                env::predecessor_account_id() == temp_owner,
                ERROR_UNAUTHORIZED
            );
            self.owner_account_id = self.temp_owner.as_ref().unwrap().clone();
            self.temp_owner = None;
            Event::CommitOwner {
                new_owner: self.owner_account_id.clone(),
                caller: env::predecessor_account_id(),
            }
            .emit();
        } else {
            require!(false, ERROR_TEMP_OWNER_NOT_SET);
        }
    }

    #[payable]
    pub fn set_operator_id(&mut self, new_operator_account_id: AccountId) {
        assert_one_yocto();
        self.assert_owner_calling();

        // owner, operator, treasury and current contract address should all be different
        require!(
            new_operator_account_id != self.operator_account_id,
            ERROR_OPERATOR_SAME
        );
        require!(
            new_operator_account_id != self.treasury_account_id,
            ERROR_OPERATOR_TREASURY_SAME
        );
        require!(
            new_operator_account_id != self.owner_account_id,
            ERROR_OPERATOR_OWNER_SAME
        );
        require!(
            new_operator_account_id != env::current_account_id(),
            ERROR_OPERATOR_CURRENT_CONTRACT_SAME
        );

        Event::SetOperator {
            old_operator: self.operator_account_id.clone(),
            new_operator: new_operator_account_id.clone(),
        }
        .emit();

        self.temp_operator = Some(new_operator_account_id);
    }

    #[payable]
    pub fn commit_operator_id(&mut self) {
        assert_one_yocto();

        if let Some(temp_operator) = self.temp_operator.clone() {
            require!(
                env::predecessor_account_id() == temp_operator,
                ERROR_UNAUTHORIZED
            );
            self.operator_account_id = temp_operator;
            self.temp_operator = None;

            Event::CommitOperator {
                new_operator: self.operator_account_id.clone(),
            }
            .emit();
        } else {
            require!(false, ERROR_TEMP_OPERATOR_NOT_SET);
        }
    }

    #[payable]
    pub fn set_treasury_id(&mut self, new_treasury_account_id: AccountId) {
        assert_one_yocto();
        self.assert_owner_calling();

        // owner, operator, treasury and current contract address should all be different
        require!(
            new_treasury_account_id != self.operator_account_id,
            ERROR_TREASURY_OPERATOR_SAME
        );
        require!(
            new_treasury_account_id != self.treasury_account_id,
            ERROR_TREASURY_SAME
        );
        require!(
            new_treasury_account_id != self.owner_account_id,
            ERROR_TREASURY_OWNER_SAME
        );
        require!(
            new_treasury_account_id != env::current_account_id(),
            ERROR_TREASURY_CURRENT_CONTRACT_SAME
        );

        Event::SetTreasury {
            old_treasury_account: self.treasury_account_id.clone(),
            new_treasury_account: new_treasury_account_id.clone(),
        }
        .emit();

        self.temp_treasury = Some(new_treasury_account_id);
    }

    #[payable]
    pub fn commit_treasury_id(&mut self) {
        assert_one_yocto();

        if let Some(temp_treasury) = self.temp_treasury.clone() {
            require!(
                env::predecessor_account_id() == temp_treasury,
                ERROR_UNAUTHORIZED
            );
            self.treasury_account_id = temp_treasury;
            self.temp_treasury = None;

            Event::CommitTreasury {
                new_treasury_account: self.treasury_account_id.clone(),
            }
            .emit();
        } else {
            require!(false, ERROR_TEMP_TREASURY_NOT_SET);
        }
    }

    #[payable]
    pub fn update_operations_control(
        &mut self,
        update_operations_control_request: OperationsControlUpdateRequest,
    ) {
        assert_one_yocto();
        self.assert_owner_calling();

        self.operations_control.stake_paused = update_operations_control_request
            .stake_paused
            .unwrap_or(self.operations_control.stake_paused);
        self.operations_control.direct_stake_paused = update_operations_control_request
            .direct_stake_paused
            .unwrap_or(self.operations_control.direct_stake_paused);
        self.operations_control.unstaked_paused = update_operations_control_request
            .unstake_paused
            .unwrap_or(self.operations_control.unstaked_paused);
        self.operations_control.withdraw_paused = update_operations_control_request
            .withdraw_paused
            .unwrap_or(self.operations_control.withdraw_paused);
        self.operations_control.staking_epoch_paused = update_operations_control_request
            .staking_epoch_paused
            .unwrap_or(self.operations_control.staking_epoch_paused);
        self.operations_control.unstaking_epoch_paused = update_operations_control_request
            .unstaking_epoch_paused
            .unwrap_or(self.operations_control.unstaking_epoch_paused);
        self.operations_control.withdraw_epoch_paused = update_operations_control_request
            .withdraw_epoch_paused
            .unwrap_or(self.operations_control.withdraw_epoch_paused);
        self.operations_control.autocompounding_epoch_paused = update_operations_control_request
            .autocompounding_epoch_paused
            .unwrap_or(self.operations_control.autocompounding_epoch_paused);
        self.operations_control.sync_validator_balance_paused = update_operations_control_request
            .sync_validator_balance_paused
            .unwrap_or(self.operations_control.sync_validator_balance_paused);
        self.operations_control.ft_transfer_paused = update_operations_control_request
            .ft_transfer_paused
            .unwrap_or(self.operations_control.ft_transfer_paused);
        self.operations_control.ft_transfer_call_paused = update_operations_control_request
            .ft_transfer_call_paused
            .unwrap_or(self.operations_control.ft_transfer_call_paused);

        Event::UpdateOperationsControl {
            operations_control: OperationControls {
                stake_paused: self.operations_control.stake_paused,
                direct_stake_paused: self.operations_control.direct_stake_paused,
                unstaked_paused: self.operations_control.unstaked_paused,
                withdraw_paused: self.operations_control.withdraw_paused,
                staking_epoch_paused: self.operations_control.staking_epoch_paused,
                unstaking_epoch_paused: self.operations_control.unstaking_epoch_paused,
                withdraw_epoch_paused: self.operations_control.withdraw_epoch_paused,
                autocompounding_epoch_paused: self.operations_control.autocompounding_epoch_paused,
                sync_validator_balance_paused: self
                    .operations_control
                    .sync_validator_balance_paused,
                ft_transfer_paused: self.operations_control.ft_transfer_paused,
                ft_transfer_call_paused: self.operations_control.ft_transfer_call_paused,
            },
        }
        .emit();
    }

    #[payable]
    pub fn add_min_storage_reserve(&mut self) {
        self.assert_min_deposit_amount(env::attached_deposit());

        self.min_storage_reserve += env::attached_deposit();
    }

    #[payable]
    pub fn set_reward_fee(&mut self, numerator: u32, denominator: u32) {
        self.assert_owner_calling();
        assert_one_yocto();
        require!(numerator * 10 <= denominator); // less than or equal to 10%

        let old_reward_fee = self.rewards_fee;
        let future_reward_fee = Fraction::new(numerator, denominator);
        self.temp_reward_fee = Some(future_reward_fee);
        self.last_reward_fee_set_epoch = env::epoch_height();

        Event::SetRewardFee {
            old_reward_fee,
            new_reward_fee: future_reward_fee,
        }
        .emit();
    }

    // Don't immediaitely set the rewards, wait for 4 epochs and then set the rewards to allow
    // users to opt out if they feel that the reward fee is too high
    #[payable]
    pub fn commit_reward_fee(&mut self) {
        self.assert_owner_calling();
        assert_one_yocto();

        if self.temp_reward_fee.is_some() {
            require!(
                self.last_reward_fee_set_epoch + REWARD_FEE_SET_WAIT_TIME <= env::epoch_height(),
                ERROR_TEMP_REWARD_FEE_SET_IN_WAIT_PERIOD
            );

            self.rewards_fee = self.temp_reward_fee.unwrap();
            self.temp_reward_fee = None;

            Event::CommitRewardFee {
                commited_reward_fee: self.rewards_fee,
            }
            .emit();
        } else {
            require!(false, ERROR_TEMP_REWARD_FEE_IS_NOT_SET);
        }
    }

    #[payable]
    pub fn set_min_deposit(&mut self, min_deposit: U128) {
        self.assert_owner_calling();
        assert_one_yocto();

        require!(min_deposit > U128(ONE_NEAR), ERROR_MIN_DEPOSIT_TOO_LOW);
        require!(
            min_deposit < U128(100 * ONE_NEAR),
            ERROR_MIN_DEPOSIT_TOO_HIGH
        );

        let old_min_deposit = self.min_deposit_amount;
        self.min_deposit_amount = min_deposit.0;

        Event::SetMinDeposit {
            old_min_deposit: U128(old_min_deposit),
            new_min_deposit: U128(self.min_deposit_amount),
        }
        .emit();
    }

    // View methods

    pub fn get_account_staked_balance(&self, account_id: AccountId) -> U128 {
        self.get_account(account_id).staked_balance
    }

    pub fn get_account_unstaked_balance(&self, account_id: AccountId) -> U128 {
        self.get_account(account_id).unstaked_balance
    }

    pub fn get_account_total_balance(&self, account_id: AccountId) -> U128 {
        let account = self.get_account(account_id);
        (account.unstaked_balance.0 + account.staked_balance.0).into()
    }

    pub fn is_account_unstaked_balance_available(&self, account_id: AccountId) -> bool {
        self.get_account(account_id).can_withdraw
    }

    pub fn get_owner_id(&self) -> AccountId {
        self.owner_account_id.clone()
    }

    pub fn get_reward_fee_fraction(&self) -> Fraction {
        self.rewards_fee
    }

    pub fn is_staking_paused(&self) -> bool {
        self.operations_control.stake_paused
    }

    pub fn get_total_staked_balance(&self) -> U128 {
        U128::from(self.total_staked)
    }

    pub fn get_staking_key(&self) -> PublicKey {
        panic!("{}", ERROR_NO_STAKING_KEY);
    }

    pub fn get_roles(&self) -> RolesResponse {
        RolesResponse {
            treasury_account: self.treasury_account_id.clone(),
            operator_account: self.operator_account_id.clone(),
            owner_account: self.owner_account_id.clone(),
            temp_owner: self.temp_owner.clone(),
            temp_operator: self.temp_operator.clone(),
            temp_treasury: self.temp_treasury.clone(),
        }
    }

    pub fn get_operations_control(&self) -> OperationControls {
        self.operations_control
    }

    pub fn get_user_account(&self, account_id: AccountId) -> AccountResponse {
        let account = self.internal_get_account(&account_id);
        AccountResponse {
            account_id,
            unstaked_balance: U128(account.unstaked_amount),
            staked_balance: self
                .staked_amount_from_num_shares_rounded_down(account.stake_shares)
                .into(),
            withdrawable_epoch: U64(account.withdrawable_epoch_height),
        }
    }

    pub fn get_account(&self, account_id: AccountId) -> HumanReadableAccount {
        let account = self.internal_get_account(&account_id);
        HumanReadableAccount {
            account_id,
            unstaked_balance: U128(account.unstaked_amount),
            staked_balance: self
                .staked_amount_from_num_shares_rounded_down(account.stake_shares)
                .into(),
            can_withdraw: account.withdrawable_epoch_height <= env::epoch_height(),
        }
    }

    pub fn get_number_of_accounts(&self) -> u64 {
        self.accounts.len()
    }

    pub fn get_snapshot_users(&self, from: usize, length: usize) -> Vec<SnapshotUser> {
        self.accounts
            .keys_as_vector()
            .iter()
            .skip(from)
            .take(length)
            .map(|account_id| SnapshotUser {
                account_id: account_id.clone(),
                nearx_balance: U128(self.internal_get_account(&account_id).stake_shares),
            })
            .collect()
    }

    pub fn get_accounts(&self, from_index: u64, limit: u64) -> Vec<HumanReadableAccount> {
        let keys = self.accounts.keys_as_vector();
        (from_index..std::cmp::min(from_index + limit, keys.len()))
            .map(|index| self.get_account(keys.get(index).unwrap()))
            .collect()
    }

    pub fn get_storage_usage(&self) -> U64 {
        U64(env::storage_usage())
    }

    // Contract state query
    pub fn get_nearx_pool_state(&self) -> NearxPoolStateResponse {
        NearxPoolStateResponse {
            owner_account_id: self.owner_account_id.clone(),
            total_staked: U128::from(self.total_staked),
            total_stake_shares: U128::from(self.total_stake_shares),
            accumulated_staked_rewards: U128::from(self.accumulated_staked_rewards),
            min_deposit_amount: U128::from(self.min_deposit_amount),
            operator_account_id: self.operator_account_id.clone(),
            rewards_fee_pct: self.rewards_fee,
            user_amount_to_stake_in_epoch: U128(self.user_amount_to_stake_in_epoch),
            user_amount_to_unstake_in_epoch: U128(self.user_amount_to_unstake_in_epoch),
            reconciled_epoch_stake_amount: U128(self.reconciled_epoch_stake_amount),
            reconciled_epoch_unstake_amount: U128(self.reconciled_epoch_unstake_amount),
            last_reconcilation_epoch: U64(self.last_reconcilation_epoch),
            temp_reward_fee: self.temp_reward_fee,
            rewards_buffer: U128(self.rewards_buffer),
            accumulated_rewards_buffer: U128(self.accumulated_rewards_buffer),
            last_reward_fee_set_epoch: self.last_reward_fee_set_epoch,
            min_storage_reserve: U128(self.min_storage_reserve),
        }
    }

    pub fn get_nearx_price(&self) -> U128 {
        if self.total_staked == 0 || self.total_stake_shares == 0 {
            return U128(ONE_NEAR);
        }

        let amount = self.staked_amount_from_num_shares_rounded_down(ONE_NEAR);
        if amount == 0 {
            U128(ONE_NEAR)
        } else {
            U128(amount)
        }
    }

    pub fn get_validator_info(&self, validator: AccountId) -> ValidatorInfoResponse {
        let wrapped_validator_info = if let Some(val_info) = self.validator_info_map.get(&validator)
        {
            val_info
        } else {
            panic!("Validator {:?} not found", validator);
        };

        let validator_info = wrapped_validator_info.into_current();

        ValidatorInfoResponse {
            account_id: validator_info.account_id.clone(),
            staked: validator_info.staked.into(),
            unstaked: U128(validator_info.unstaked_amount),
            weight: validator_info.weight,
            last_asked_rewards_epoch_height: validator_info.last_redeemed_rewards_epoch.into(),
            last_unstake_start_epoch: U64(validator_info.unstake_start_epoch),
            max_unstakable_limit: U128(validator_info.max_unstakable_limit),
            validator_type: validator_info.validator_type,
        }
    }

    pub fn get_validators(&self) -> Vec<ValidatorInfoResponse> {
        self.validator_info_map
            .iter()
            .map(|pool| -> ValidatorInfoResponse {
                let validator_info = pool.1.into_current();
                ValidatorInfoResponse {
                    account_id: validator_info.account_id.clone(),
                    staked: U128::from(validator_info.staked),
                    last_asked_rewards_epoch_height: U64(validator_info.last_redeemed_rewards_epoch),
                    last_unstake_start_epoch: U64(validator_info.unstake_start_epoch),
                    unstaked: U128(validator_info.unstaked_amount),
                    weight: validator_info.weight,
                    max_unstakable_limit: U128(validator_info.max_unstakable_limit),
                    validator_type: validator_info.validator_type,
                }
            })
            .collect()
    }

    pub fn get_total_validator_weight(&self) -> u16 {
        self.total_validator_weight
    }

    pub fn is_validator_unstake_pending(&self, validator: AccountId) -> bool {
        let validator_info = self.internal_get_validator(&validator);

        validator_info.pending_unstake_release()
    }

    pub fn get_current_epoch(&self) -> U64 {
        U64(env::epoch_height())
    }

    pub fn get_contract_summary(&self) -> ContractSummary {
        let treasury_account = self.get_account(self.treasury_account_id.clone());

        ContractSummary {
            total_staked: U128(self.total_staked),
            total_shares: U128(self.total_stake_shares),
            total_validators: U128(self.validator_info_map.len() as u128),
            treasury_staked_balance: treasury_account.staked_balance,
            treasury_unstaked_balance: treasury_account.unstaked_balance,
            nearx_price: self.get_nearx_price(),
        }
    }

    pub fn get_near_from_nearx(&self, nearx_amount: U128) -> U128 {
        U128(self.staked_amount_from_num_shares_rounded_down(nearx_amount.0))
    }
}
