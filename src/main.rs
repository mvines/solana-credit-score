mod latitude;
mod notifier;
use {
    crate::latitude::Latitude,
    clap::{crate_description, crate_name, value_parser, Arg, Command},
    log::*,
    notifier::*,
    solana_clap_v3_utils::{
        input_parsers::parse_url_or_moniker, input_validators::normalize_to_url_if_moniker,
    },
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_credit_score::get_validator_status,
    solana_sdk::{
        account::from_account,
        commitment_config::CommitmentConfig,
        inflation::Inflation,
        native_token::Sol,
        sysvar::stake_history::{self, StakeHistory},
    },
};

fn app_version() -> String {
    let tag = option_env!("GITHUB_REF")
        .and_then(|github_ref| github_ref.strip_prefix("refs/tags/").map(|s| s.to_string()));

    tag.unwrap_or_else(|| match option_env!("GITHUB_SHA") {
        None => "devbuild".to_string(),
        Some(commit) => commit[..8].to_string(),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app_version = &*app_version();
    let matches = Command::new(crate_name!())
        .about(crate_description!())
        .version(app_version)
        .arg({
            let arg = Arg::new("config_file")
                .short('C')
                .long("config")
                .value_name("PATH")
                .takes_value(true)
                .global(true)
                .help("Configuration file to use");
            if let Some(ref config_file) = *solana_cli_config::CONFIG_FILE {
                arg.default_value(config_file)
            } else {
                arg
            }
        })
        .arg(
            Arg::new("json_rpc_url")
                .short('u')
                .long("url")
                .value_name("URL")
                .takes_value(true)
                .validator(|s| parse_url_or_moniker(s))
                .help("JSON RPC URL for the cluster [default: value from configuration file]"),
        )
        .arg(
            Arg::new("num")
                .short('n')
                .long("num")
                .value_name("N")
                .takes_value(true)
                .value_parser(value_parser!(usize))
                .help("Limit output to the top N validators [default: all validators]"),
        )
        .arg(
            Arg::new("max_percentile")
                .short('p')
                .long("percentile")
                .value_name("P")
                .takes_value(true)
                .value_parser(value_parser!(u8))
                .default_value("0")
                .help("Limit output to the validators in the Pth percentile [default: all validators]"),
        )
        .arg(
            Arg::new("ignore_commission")
                .short('i')
                .long("ignore-commission")
                .help("Ignore validator commission")
        )
        .arg(
            Arg::with_name("identity")
                .short('k')
                .long("identity")
                .value_name("IDENTITY")
                .global(true)
                .takes_value(true)
                .help("Validator identity string"),
        )
        .arg(
            Arg::with_name("reason")
                .short('r')
                .long("reason")
                .value_name("REASON")
                .global(true)
                .takes_value(true)
                .help("Reason for calling the program [hourly, daily, new_epoch]"),
        )
        .arg(
            Arg::new("epoch")
                .index(1)
                .value_name("EPOCH")
                .takes_value(true)
                .value_parser(value_parser!(i64))
                .help("Epoch to process. Negative values are permitted, e.g. -1 means the previous epoch \
                      [default: the current, incomplete, epoch]"),
        )
        .get_matches();

    let cli_config = if let Some(config_file) = matches.value_of("config_file") {
        solana_cli_config::Config::load(config_file).unwrap_or_default()
    } else {
        solana_cli_config::Config::default()
    };

    let json_rpc_url = normalize_to_url_if_moniker(
        matches
            .value_of("json_rpc_url")
            .unwrap_or(&cli_config.json_rpc_url),
    );
    let epoch = matches.value_of("epoch").map(|s| s.parse::<i64>().unwrap());
    let num = matches
        .value_of("num")
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(usize::MAX);
    let max_percentile: u8 = *matches.get_one("max_percentile").unwrap();
    let ignore_commission = matches.is_present("ignore_commission");

    solana_logger::setup_with_default("warn");
    let notifier = Notifier::default();

    info!("JSON RPC URL: {}", json_rpc_url);

    let validator_vote_id = matches.value_of("identity");

    let rpc_client =
        RpcClient::new_with_commitment(json_rpc_url.clone(), CommitmentConfig::finalized());

    let epoch_info = rpc_client.get_epoch_info().await?;

    let epoch = match epoch {
        Some(epoch) if epoch < 0 => epoch_info
            .epoch
            .checked_sub(epoch.unsigned_abs())
            .ok_or_else(|| format!("Invalid relative epoch value: {}", epoch))?,
        Some(epoch) => epoch as u64,
        None => epoch_info.epoch,
    };

    println!("Epoch {}", epoch);

    let latitude = Latitude::default();

    if let Some(validator_vote_id) = validator_vote_id {
        let status =
            get_validator_status(&rpc_client, validator_vote_id, &epoch_info, epoch).await?;
        // println!("result {:?}", status);
        let status = if let Some(status) = status {
            status
        } else {
            notifier
                .send(&format!(
                    "```Alert!! validator not found {}```",
                    validator_vote_id
                ))
                .await;
            return Ok(());
        };

        let bandwidth_usage = latitude.get_bandwidth_usage().unwrap_or_default();

        let msg = if let Some(reason) = matches.value_of("reason") {
            match reason {
                "hourly" => {
                    if status.is_delinquent
                        || status.skip_rate > 4.0
                        || status.vote_distance > 40
                        || bandwidth_usage.inbound_usage > 70
                        || bandwidth_usage.outbound_usage > 70
                    {
                        format!(
                            "```Alert!!\
                            \n{:?}\
                            \nNetwork status\
                            \n{:?}\
                            ```",
                            status, bandwidth_usage,
                        )
                    } else {
                        "".to_string()
                    }
                }
                "daily" => {
                    format!(
                        "```Current epoch status\
                            \n{:?}\
                            \nNetwork status\
                            \n{:?}\
                            ```",
                        status, bandwidth_usage,
                    )
                }
                "new_epoch" => {
                    let last_epoch_status = get_validator_status(
                        &rpc_client,
                        validator_vote_id,
                        &epoch_info,
                        epoch.saturating_sub(1),
                    )
                    .await?;
                    // println!("result {:?}", last_epoch_status);
                    if let Some(last_epoch_status) = last_epoch_status {
                        format!(
                            "```Last epoch status\
                            \n{:?}\
                            \nCurrent epoch status\
                            \n{:?}\
                            \nNetwork status\
                            \n{:?}\
                            ```",
                            last_epoch_status, status, bandwidth_usage,
                        )
                    } else {
                        format!("```Alert!! validator not found {}```", validator_vote_id)
                    }
                }
                _ => "".to_string(),
            }
        } else {
            "".to_string()
        };
        println!("{}", msg);
        if msg.len() != 0 {
            notifier.send(&msg).await;
        }

        return Ok(());
    }

    let inflation = {
        let rpc_inflation_governor = rpc_client.get_inflation_governor().await?;

        let mut inflation = Inflation::default();
        inflation.initial = rpc_inflation_governor.initial;
        inflation.terminal = rpc_inflation_governor.terminal;
        inflation.taper = rpc_inflation_governor.taper;
        inflation.foundation = rpc_inflation_governor.foundation;
        inflation.foundation_term = rpc_inflation_governor.foundation_term;
        inflation
    };

    let stake_history = {
        let stake_history_account = rpc_client.get_account(&stake_history::id()).await?;
        from_account::<StakeHistory, _>(&stake_history_account)
            .ok_or("Failed to deserialize stake history")?
    };

    let estimated_total_supply = rpc_client.supply().await?.value.total;

    let slots_per_year = 78892314.984; // hard coded value from genesis.tar.gz
    let inflation_activation_slot = 64800004; // slot when the `7XRJcS5Ud5vxGB54JbK9N2vBZVwnwdBNeJW1ibRgD9gx`
                                              // feature was activated on mainnet

    let first_slot_in_epoch = epoch_info.absolute_slot - epoch_info.slot_index;

    let inflation_num_slots =
        first_slot_in_epoch - (inflation_activation_slot - epoch_info.slots_in_epoch);

    let inflation_rate = inflation.validator(inflation_num_slots as f64 / slots_per_year);
    let epoch_duration_in_years = epoch_info.slots_in_epoch as f64 / slots_per_year;
    let estimated_total_epoch_reward =
        (inflation_rate * estimated_total_supply as f64 * epoch_duration_in_years) as u64;

    let validators_by_staker_credits = solana_credit_score::get_validators_by_credit_score(
        &rpc_client,
        &epoch_info,
        epoch,
        ignore_commission,
    )
    .await?;

    let staker_credits = validators_by_staker_credits
        .iter()
        .map(|(staker_credits, ..)| *staker_credits as f64)
        .collect::<Vec<_>>();

    let total_activated_stake = if epoch == epoch_info.epoch {
        validators_by_staker_credits
            .iter()
            .map(|(.., activated_stake)| *activated_stake as u64)
            .sum::<u64>()
    } else {
        stake_history
            .get(epoch)
            .ok_or_else(|| format!("Stake history unavailable for epoch {}", epoch))?
            .effective
    };

    let total_points = validators_by_staker_credits
        .iter()
        .map(|(staker_credits, .., activated_stake)| {
            u128::from(*staker_credits) * u128::from(*activated_stake)
        })
        .sum::<u128>();

    let top_staker_credits = staker_credits.first().copied().unwrap_or_default();

    let staker_credits = criterion_stats::Distribution::from(staker_credits.into_boxed_slice());
    let staker_credit_percentiles = staker_credits.percentiles();

    let mut p = 100u8;
    let msg = validators_by_staker_credits
        .into_iter()
        .take(num)
        .enumerate()
        .filter_map(|(i, (staker_credits, vote_pubkey, activated_stake))| {
            while p > 0 {
                let percentile_credits = staker_credit_percentiles.at(p as f64);
                if staker_credits as f64 >= percentile_credits {
                    break;
                }
                p = p.saturating_sub(1);
            }

            if p < max_percentile {
                None
            } else {
                let percent_of_top_staker = staker_credits as f64 * 100. / top_staker_credits;

                let credits_behind =
                    (top_staker_credits.floor() as u64).saturating_sub(staker_credits);

                let current_epoch_estimates = if epoch_info.epoch == epoch {
                    let points = u128::from(activated_stake) * u128::from(staker_credits);

                    let estimated_epoch_reward =
                        (estimated_total_epoch_reward as u128 * points / total_points) as u64;

                    let expected_epoch_reward = u128::from(estimated_total_epoch_reward)
                        * u128::from(activated_stake)
                        / u128::from(total_activated_stake);

                    format!(
                        "| estimated {} (expected: {})",
                        Sol(estimated_epoch_reward),
                        Sol(expected_epoch_reward as u64),
                    )
                } else {
                    "".into()
                };

                #[allow(clippy::to_string_in_format_args)]
                let vote_pubkey_str = vote_pubkey.to_string();

                Some(format!(
                    "{:>4}. {:<44} ({:>6.2}%) ({:>3}th percentile){} {}",
                    i + 1,
                    vote_pubkey_str,
                    percent_of_top_staker,
                    p,
                    if credits_behind > 0 {
                        format!(" [-{} credits]", credits_behind)
                    } else {
                        "".into()
                    },
                    current_epoch_estimates,
                ))
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    println!("{}", msg);
    notifier.send(&format!("```{}```", msg)).await;
    Ok(())
}
