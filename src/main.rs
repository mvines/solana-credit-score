use {
    clap::{crate_description, crate_name, crate_version, Arg, Command},
    log::*,
    solana_clap_v3_utils::input_validators::{
        is_parsable, is_url_or_moniker, normalize_to_url_if_moniker,
    },
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::commitment_config::CommitmentConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = Command::new(crate_name!())
        .about(crate_description!())
        .version(crate_version!())
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
                .validator(|s| is_url_or_moniker(s))
                .help("JSON RPC URL for the cluster [default: value from configuration file]"),
        )
        .arg(
            Arg::new("num")
                .short('n')
                .long("num")
                .value_name("N")
                .takes_value(true)
                .validator(|s| is_parsable::<usize>(s))
                .help("Limit output to the top N validators [default: all validators]"),
        )
        .arg(
            Arg::new("epoch")
                .index(1)
                .value_name("EPOCH")
                .takes_value(true)
                .validator(|s| is_parsable::<i64>(s))
                .help("Epoch to process. Negative values are permitted, e.g. -1 means the previous epoch [default: the current, incomplete, epoch]"),
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

    solana_logger::setup_with_default("warn");
    info!("JSON RPC URL: {}", json_rpc_url);

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

    let validators_by_staker_credits =
        solana_credit_score::get_validators_by_credit_score(&rpc_client, &epoch_info, epoch)
            .await?;

    let top_staker_credits = validators_by_staker_credits
        .get(0)
        .map(|(staker_credits, _)| staker_credits)
        .copied()
        .unwrap_or_default() as f64;

    for (i, (staker_credits, vote_pubkey)) in validators_by_staker_credits
        .into_iter()
        .take(num)
        .enumerate()
    {
        if i == 0 {
            println!("{}. {:<44}", i, vote_pubkey.to_string())
        } else {
            let percent_of_top_staker = staker_credits as f64 * 100. / top_staker_credits;
            println!(
                "{}. {:<44} ({:5.2}%)",
                i,
                vote_pubkey.to_string(),
                percent_of_top_staker
            );
        }
    }

    Ok(())
}
