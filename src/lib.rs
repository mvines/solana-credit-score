use {
    log::*,
    solana_client::{
        nonblocking::rpc_client::RpcClient,
        rpc_config::{
            RpcBlockConfig, RpcBlockProductionConfig, RpcBlockProductionConfigRange,
            RpcGetVoteAccountsConfig, RpcLeaderScheduleConfig,
        },
        rpc_custom_error,
    },
    solana_sdk::{
        clock::Epoch, epoch_info::EpochInfo, native_token::LAMPORTS_PER_SOL, pubkey::Pubkey,
        reward_type::RewardType,
    },
    solana_transaction_status::Reward,
    std::{
        collections::BTreeMap,
        fmt::{Debug, Formatter},
    },
};

async fn get_epoch_commissions(
    rpc_client: &RpcClient,
    epoch_info: &EpochInfo,
    epoch: Epoch,
) -> Result<BTreeMap<Pubkey, u8>, Box<dyn std::error::Error>> {
    if epoch > epoch_info.epoch {
        return Err(format!("Future epoch, {}, requested", epoch).into());
    }

    let first_slot_in_epoch = epoch_info
        .absolute_slot
        .saturating_sub(epoch_info.slot_index)
        - (epoch_info.epoch - epoch) * epoch_info.slots_in_epoch;

    let mut first_block_in_epoch = first_slot_in_epoch;
    loop {
        info!("fetching block in slot {}", first_block_in_epoch);
        match rpc_client
            .get_block_with_config(first_block_in_epoch, RpcBlockConfig::rewards_only())
            .await
        {
            Ok(block) => {
                return Ok(block
                    .rewards
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|reward| match reward {
                        Reward {
                            reward_type: Some(RewardType::Voting),
                            commission: Some(commission),
                            pubkey,
                            ..
                        } => Some((pubkey.parse::<Pubkey>().unwrap_or_default(), commission)),
                        _ => None,
                    })
                    .collect());
            }
            Err(err) => {
                if matches!(
                        err.kind(),
                        solana_client::client_error::ClientErrorKind::RpcError(solana_client::rpc_request::RpcError::RpcResponseError {
                            code: rpc_custom_error::JSON_RPC_SERVER_ERROR_SLOT_SKIPPED |
                            rpc_custom_error::JSON_RPC_SERVER_ERROR_LONG_TERM_STORAGE_SLOT_SKIPPED,
                            ..
                        })
                    ) {
                        info!("slot {} skipped",first_block_in_epoch);
                        first_block_in_epoch += 1;
                        continue;
                    }
                return Err(format!(
                    "Failed to fetch the block for slot {}: {:?}",
                    first_block_in_epoch, err
                )
                .into());
            }
        }
    }
}

pub struct ValidatorStatus {
    pub epoch: Epoch,
    pub epoch_progress: u64,
    pub credits: u64,
    pub vote_distance: u64,
    pub delegated_stake: u64,
    pub leader_slots_count: usize,
    pub leader_slots_elapsed: usize,
    pub blocks_produced: usize,
    pub skip_rate: f64,
    pub is_delinquent: bool,
}

impl Debug for ValidatorStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "\tEpoch {} is {}% over\n\
            \tThe node is {}\n\
            \t{} SOLs are staked\n\
            \t{} total leader slots\n\
            \t{} produced out of {}\n\
            \t{:.2}% skip rate\n\
            \t{} vote distance\n\
            \t{} vote credits\n",
            self.epoch,
            self.epoch_progress,
            self.is_delinquent
                .then_some("!! delinquent !!")
                .or_else(|| Some("not delinquent"))
                .unwrap(),
            self.delegated_stake,
            self.leader_slots_count,
            self.blocks_produced,
            self.leader_slots_elapsed,
            self.skip_rate,
            self.vote_distance,
            self.credits,
        )
    }
}

pub async fn get_validator_status(
    rpc_client: &RpcClient,
    vote_pubkey: &str,
    epoch_info: &EpochInfo,
    epoch: Epoch,
) -> Result<Option<ValidatorStatus>, Box<dyn std::error::Error>> {
    let vote_accounts = rpc_client
        .get_vote_accounts_with_config(RpcGetVoteAccountsConfig {
            vote_pubkey: Some(vote_pubkey.to_string()),
            commitment: Some(rpc_client.commitment()),
            keep_unstaked_delinquents: Some(false),
            ..RpcGetVoteAccountsConfig::default()
        })
        .await?;

    let is_delinquent = vote_accounts.current.is_empty();
    let account = if is_delinquent {
        vote_accounts.delinquent.first()
    } else {
        vote_accounts.current.first()
    };

    let account = if let Some(account) = account {
        account
    } else {
        return Ok(None);
    };

    let vote_distance = account.last_vote - account.root_slot;
    let delegated_stake = account.activated_stake / LAMPORTS_PER_SOL;
    let credits = account
        .epoch_credits
        .iter()
        .find(|(e, _, _)| e == &epoch)
        .map(|(_, credits, prev_credits)| credits.saturating_sub(*prev_credits))
        .unwrap_or_default();

    let identity = &account.node_pubkey;

    let first_slot_in_epoch = epoch_info
        .absolute_slot
        .saturating_sub(epoch_info.slot_index)
        - (epoch_info.epoch - epoch) * epoch_info.slots_in_epoch;
    let last_slot = first_slot_in_epoch
        .saturating_add(epoch_info.slots_in_epoch)
        .min(epoch_info.absolute_slot);

    let (leader_slots_elapsed, blocks_produced, skip_rate) = rpc_client
        .get_block_production_with_config(RpcBlockProductionConfig {
            identity: Some(identity.clone()),
            range: Some(RpcBlockProductionConfigRange {
                first_slot: first_slot_in_epoch,
                last_slot: Some(last_slot),
            }),
            ..RpcBlockProductionConfig::default()
        })
        .await?
        .value
        .by_identity
        .into_iter()
        .find(|(leader, (_, _))| leader == identity)
        .map(|(_, (leader_slots, blocks_produced))| {
            (
                leader_slots,
                blocks_produced,
                100. * (leader_slots.saturating_sub(blocks_produced)) as f64 / leader_slots as f64,
            )
        })
        .unwrap_or_default();

    let leader_schedule = rpc_client
        .get_leader_schedule_with_config(
            Some(first_slot_in_epoch),
            RpcLeaderScheduleConfig {
                identity: Some(identity.clone()),
                commitment: None,
            },
        )
        .await?;

    let leader_slots_count = if let Some(schedule) = leader_schedule {
        schedule.get(identity).map(|v| v.len()).unwrap_or_default()
    } else {
        0
    };

    let epoch_progress = if epoch_info.epoch > epoch {
        100
    } else {
        epoch_info.slot_index * 100 / epoch_info.slots_in_epoch
    };

    Ok(Some(ValidatorStatus {
        epoch,
        epoch_progress,
        credits,
        vote_distance,
        delegated_stake,
        leader_slots_count,
        leader_slots_elapsed,
        blocks_produced,
        skip_rate,
        is_delinquent,
    }))
}

/// Returns a `Vec` of ("epoch staker credits earned", "validator vote account address"), ordered
/// by epoch staker credits earned.
pub async fn get_validators_by_credit_score(
    rpc_client: &RpcClient,
    epoch_info: &EpochInfo,
    epoch: Epoch,
    ignore_commission: bool,
) -> Result<
    Vec<(
        /* credits: */ u64,
        /* vote_pubkey: */ Pubkey,
        /* activated_stake_for_current_epoch: */ u64,
    )>,
    Box<dyn std::error::Error>,
> {
    let epoch_commissions = if epoch == epoch_info.epoch {
        None
    } else {
        Some(get_epoch_commissions(rpc_client, epoch_info, epoch).await?)
    };

    let vote_accounts = rpc_client
        .get_vote_accounts_with_config(RpcGetVoteAccountsConfig {
            commitment: Some(rpc_client.commitment()),
            keep_unstaked_delinquents: Some(false),
            ..RpcGetVoteAccountsConfig::default()
        })
        .await?;

    let mut list = vote_accounts
        .current
        .into_iter()
        .chain(vote_accounts.delinquent)
        .filter_map(|vai| {
            vai.vote_pubkey.parse::<Pubkey>().ok().map(|vote_pubkey| {
                let staker_credits = vai
                    .epoch_credits
                    .iter()
                    .find(|ec| ec.0 == epoch)
                    .map(|(_, credits, prev_credits)| {
                        let (epoch_commission, epoch_credits) = {
                            let epoch_commission = if ignore_commission {
                                0
                            } else {
                                match &epoch_commissions {
                                    Some(epoch_commissions) => {
                                        *epoch_commissions.get(&vote_pubkey).unwrap()
                                    }
                                    None => vai.commission,
                                }
                            };
                            let epoch_credits = credits.saturating_sub(*prev_credits);
                            (epoch_commission, epoch_credits)
                        };

                        let staker_credits = (u128::from(epoch_credits)
                            * u128::from(100 - epoch_commission)
                            / 100) as u64;
                        debug!(
                            "{}: total credits {}, staker credits {} in epoch {}",
                            vote_pubkey, epoch_credits, staker_credits, epoch,
                        );
                        staker_credits
                    })
                    .unwrap_or_default();

                (staker_credits, vote_pubkey, vai.activated_stake)
            })
        })
        .collect::<Vec<_>>();

    list.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(list)
}
