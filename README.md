# zeekoe

## Setting up the project

You will need a recent version of stable Rust.

Notice also that we specify the build option `allow_explicit_certificate_trust`. Without this
option, only certificates rooted at the webpki roots of trust would be trusted, and the customer
would reject the connection to the merchant due to the bad certificate. Because this decreases the
trustworthiness of the authentication between the merchant and customer, this is only intended for
use in testing, and cannot be enabled in release builds.

```bash
cargo build --features "allow_explicit_certificate_trust"
```

## Running the zkchannel merchant and customer

The customer authenticates the merchant using a TLS certificate, which must be generated. For
development and testing purposes, the certificate and private key can be generated using a provided
script, which places them in the `./dev` folder:

```bash
./dev/generate-certificates
```

Now, we can run the merchant server. If we were to install the `zkchannel` binary, it would look for
its `Merchant.toml` configuration file in the idiomatic configuration directory for the current
user, but in this self-contained example we use the `--config` flag to request that it use the
configuration in `./dev`. This configuration also specifies that the merchant should store its
database in that same directory.

```bash
$ ./target/debug/zkchannel merchant --config "./dev/Merchant.toml" run
serving on: [::1]:2611
```

Leaving the merchant running, we can now act as the customer to establish a new payment channel with
the merchant, making an initial deposit of 5 XTZ. We're specifying here that we'd like to give this
channel the nickname "my-first-zkchannel", so we can keep track of it by a human-readable name.

As with the merchant, we specify a local configuration file using the `--config` flag, which
overrides the default location of the customer configuration file. This configuration file puts the
customer database in the `./dev` directory.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" \
    establish "zkchannel://localhost" \
    --label "my-first-zkchannel" \
    --deposit "5 XTZ"
Successfully established new channel with label "my-first-zkchannel"
Establishment data written to "6827e5ed90227b0f7afca7be8a8f756ce83275ed1b43744a0bcec695b43526db.establish.json"
```

The establishment data written to the file listed above can be used by an external tool in this
repository to originate and fund the contract on-chain. This is a temporary stop-gap until we very
shortly integrate with the `tezedge-client` project to originate and fund the contract from within
the `zkchannel customer establish` call itself.

Now, when we list our channels, we can see that we have an open channel with 5 XTZ available to spend.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" list
┌────────────────────┬───────┬──────────┬────────────┬──────────────────────────────────────────────┐
│ Label              ┆ State ┆ Balance  ┆ Max Refund ┆ Channel ID                                   │
╞════════════════════╪═══════╪══════════╪════════════╪══════════════════════════════════════════════╡
│ my-first-zkchannel ┆ ready ┆ 5.00 XTZ ┆ 0.00 XTZ   ┆ aCfl7ZAiew96/Ke+io91bOgyde0bQ3RKC87GlbQ1Jts= │
└────────────────────┴───────┴──────────┴────────────┴──────────────────────────────────────────────┘
```

And, on the merchant's side, a channel with the same ID has been established also.

```bash
$ ./target/debug/zkchannel merchant --config "./dev/Merchant.toml" list
┌──────────────────────────────────────────────┬────────┐
│ Channel ID                                   ┆ Status │
╞══════════════════════════════════════════════╪════════╡
│ aCfl7ZAiew96/Ke+io91bOgyde0bQ3RKC87GlbQ1Jts= ┆ active │
└──────────────────────────────────────────────┴────────┘
```

Now, we can make a payment on this channel, in this case in the amount of 0.005 XTZ.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" \
    pay "my-first-zkchannel" "0.005 XTZ"
```

We can then check the balances in our channels again to confirm that the payment went through.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" list
┌────────────────────┬───────┬───────────┬────────────┬──────────────────────────────────────────────┐
│ Label              ┆ State ┆ Balance   ┆ Max Refund ┆ Channel ID                                   │
╞════════════════════╪═══════╪═══════════╪════════════╪══════════════════════════════════════════════╡
│ my-first-zkchannel ┆ ready ┆ 4.995 XTZ ┆ 0.005 XTZ  ┆ aCfl7ZAiew96/Ke+io91bOgyde0bQ3RKC87GlbQ1Jts= │
└────────────────────┴───────┴───────────┴────────────┴──────────────────────────────────────────────┘
```

Finally, after some number of payments, we can close the channel.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" close --force my-first-zkchannel
Closing data written to "6827e5ed90227b0f7afca7be8a8f756ce83275ed1b43744a0bcec695b43526db.close.json"
```

Just as in the channel establishment protocol, an external tool in this repository can consume the
closing data to close the contract on chain and recover the current balance of the channel. This
will shortly be integrated into the functionality of `zkchannel customer close` itself.

Finally, we can see that the channel is now closed. No further payments can be made on this channel.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" list
┌────────────────────┬────────┬───────────┬────────────┬──────────────────────────────────────────────┐
│ Label              ┆ State  ┆ Balance   ┆ Max Refund ┆ Channel ID                                   │
╞════════════════════╪════════╪═══════════╪════════════╪══════════════════════════════════════════════╡
│ my-first-zkchannel ┆ closed ┆ 4.995 XTZ ┆ 0.005 XTZ  ┆ aCfl7ZAiew96/Ke+io91bOgyde0bQ3RKC87GlbQ1Jts= │
└────────────────────┴────────┴───────────┴────────────┴──────────────────────────────────────────────┘
```

The merchant server may now be stopped by pressing ^C.

## Development

While developing on the project, here are some more things you may wish to know:

### Updating `sqlx-data.json`

If you change an SQL query, you may see an error when you build:

```bash
error: failed to find data for query 30ccc281095d5d9f292125e2fd49f0c6d65d62bce30422c24cb37e2f5e2c6c33 at line 321 column 1
```

[sqlx][] uses the file `sqlx-data.json` to ensure the queries are well formed.
When you change a query, you'll need to regenerate it:

```bash
# 1. Generate the dev database. You'll only need to do this once, but it's
#    required to run step 2.
$ ./dev/create-database

# 2. Regenerate query metadata.
$ cargo sqlx prepare -- --lib

# 3. Check in the changes to sqlx-data.json.
$ git add -p sqlx-data.json
```

[sqlx]: https://github.com/launchbadge/sqlx
