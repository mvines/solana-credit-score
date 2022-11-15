use {
    log::*,
    solana_client::{
        nonblocking::rpc_client::RpcClient,
        rpc_config::{RpcBlockConfig, RpcGetVoteAccountsConfig},
        rpc_custom_error,
    },
    solana_sdk::{clock::Epoch, epoch_info::EpochInfo, pubkey::Pubkey, reward_type::RewardType},
    solana_transaction_status::Reward,
    std::collections::BTreeMap,
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
            keep_unstaked_delinquents: Some(true),
            ..RpcGetVoteAccountsConfig::default()
        })
        .await?;

    let mut list = vote_accounts
        .current
        .into_iter()
        .chain(vote_accounts.delinquent)
        .filter_map(|vai| {
            vai.vote_pubkey
                .parse::<Pubkey>()
                .ok()
                .map(|vote_pubkey| {
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
