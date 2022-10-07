### Solana Credit Score

This crate orders validators from highest to lowest earned credits for their
stakers for a given epoch.

If you believe that past performance will be be indicative of future results,
delegate to the validators higher in this list for better future staking returns.

### Usage

Ensure stable Rust is installed, clone this repo, then:
```bash
$ cargo run -- --help
solana-credit-score 0.1.0
Orders Solana validators by their credit score

USAGE:
    solana-credit-score [OPTIONS] [EPOCH]

ARGS:
    <EPOCH>    Epoch to process. Negative values are permitted, e.g. -1 means the previous epoch
               [default: the current, incomplete, epoch]

OPTIONS:
    -C, --config <PATH>     Configuration file to use [default:
                            /Users/mvines/.config/solana/cli/config.yml]
    -h, --help              Print help information
    -n, --num <N>           Limit output to the top N validators [default: all validators]
    -p, --percentile <P>    Limit output to the validators in the Pth percentile [default: all
                            validators] [default: 0]
    -u, --url <URL>         JSON RPC URL for the cluster [default: value from configuration file]
    -V, --version           Print version information
```


#### Example
```
$ solana epoch-info

Block height: 139209612
Slot: 154155261
Epoch: 356
Transaction Count: 103715423715
Epoch Slot Range: [153792000..154224000)
Epoch Completed Percent: 84.088%
Epoch Completed Slots: 363261/432000 (68739 remaining)
Epoch Completed Time: 2days 3h 34m 39s/2days 9h 51m 34s (6h 16m 55s remaining)

$ cargo run -- --num 10
    Finished dev [unoptimized + debuginfo] target(s) in 0.26s
     Running `target/debug/solana-credit-score --num 10`
Epoch 356
   0. Pond1QyT1sQtiru3fi9G5LGaLRGeUpJKR1a2gdbq2u4 (100.00%) (100th percentile)
   1. HxRrsnbc6K8CdEo3LCTrSUkFaDDxv9BdJsTDzBKnUVWH ( 99.95%) ( 99th percentile)
   2. DMSuZcavta8L1w1tSiH8bALWjz6Q6KSryGG6m6Az4Qt5 ( 98.76%) ( 99th percentile)
   3. GNZ1PAAS33davY4Q1BMEpZEpVBtRtGvSpcTH5wYVkkVt ( 98.59%) ( 99th percentile)
   4. 5EYp3kCdMLq52vzZ4ucsVyYaaxQe5MKTquxahjXpcShS ( 98.43%) ( 99th percentile)
   5. GfZybqTfVXiiF7yjwnqfwWKm2iwP96sSbHsGdSpwGucH ( 98.27%) ( 99th percentile)
   6. F5b1wSUtpaYDnpjLQonCZC7iyFvizLcNqTactZbwSEXK ( 98.23%) ( 99th percentile)
   7. 5yHqB3NxovCEMUniQCboaPRMyyQ7kQQF4QqvC4vaz78z ( 98.21%) ( 99th percentile)
   8. juicQdAnksqZ5Yb8NQwCLjLWhykvXGktxnQCDvMe6Nx ( 98.21%) ( 99th percentile)
   9. GHRvDXj9BfACkJ9CoLWbpi2UkMVti9DwXJGsaFT9XDcD ( 98.20%) ( 99th percentile)
```
