# zeekoe

*Zeekoe* is the Dutch word for the sea-cow, otherwise known as the manatee, a friendly coastal
aquatic mammal. The word "zeekoe" also has the fortuitous coincidence of being one of the few words
whose first and only two consonants are the letters "ZK": and hence the gentle manatee is the mascot
of the **zkChannels protocol** for zero-knowledge layer-2 payment channels.

![photograph of a mother manatee and calf](https://upload.wikimedia.org/wikipedia/commons/thumb/d/da/Mother_manatee_and_calf.jpg/800px-Mother_manatee_and_calf.jpg)

[Source: public domain image by Sam Farkas (NOAA Photo Library), via Wikimedia](https://en.wikipedia.org/wiki/File:Mother_manatee_and_calf.jpg)

## What is a zkChannel?

This repository contains the source for the `zkchannels` application, which can serve as both the
"merchant" or the "customer" end of the asymmetric zkChannels protocol.

In this protocol, the customer *establishes* a channel with the merchant by placing a certain amount
of funds in escrow on the blockchain. After the channel is established, the customer may choose to
*pay* the merchant some number of times on the channel, and receive some digital artifact in
exchange (the nature of this good or service is not fixed by the protocol). The customer may also
request a *refund* from the merchant, up to the maximum refund made possible by the payments that
have occurred on the channel. The customer is always the party to initiate a payment (or refund),
and is either accepted or rejected by the merchant. Thanks to the power of zero -knowledge proofs,
the merchant can validate that the customer has the requisite balance in some open channel, without
learning the payment or real-world identity of the customer. After some number of payments and/or
refunds, the customer or merchant may *close* the channel, which distributes the current channel
balances from the on-chain escrow account to the customer and merchant.

This payment channel protocol has a significant privacy advantage over many prior layer-2 payment
channel approaches. At every point from establishment through closing, the merchant is only able to
correlate the customer's on-chain payment identity with starting and ending balances of the channel,
and explicitly does not gain the ability to connect the customer's identity with the quantity,
price, or nature of the payments any individual customer has made (so long as there are sufficiently
many customers that the merchant can't draw statistical or timing correlations between their
on-chain and off-chain actions).

## Current project status

This project is currently a technology demonstration. It has not yet been formally audited for
correctness and is not yet ready for use with currency that holds real value.

In general, the zkChannels protocol and most of the underlying software stack are compatible with
any blockchain or other escrow arbiter that supports the verification of various blind signature
constructs. The version of the `zkchannel` application in this repository is specialized to the
Tezos blockchain. Future work will generalize to other escrow arbiters.

## On-chain integration demo

In this repository (KWF: LINK HERE) you will also find a first demonstration of the compatibility of
the zkChannels protocol with Tezos test-net as implemented by the Zeekoe project. This demo
illustrates how zkChannels can be used to establish a payment channel linked to funds in escrow on
the Tezos test-net, make several payments on that channel, and close the channel, disbursing the
final channel balances to the merchant and customer.

At present time of writing, Zeekoe does not fully _internally_ integrate with the Tezos network, but
our highest priority of development is completing this integration. This demonstration shows that
the implementation of the zkChannels protocol produces correct outputs which can be used in the
on-chain zkChannels contract to correctly fulfill the protocol, but it does not yet emit those
transactions to the blockchain from within the `zkchannel` application itself. Instead, during this
demo, transactions are separately submitted after off-chain establishment or closing using a script
based on the PyTezos framework.

A full integration with Tezos test-net, including contract origination, funding, and closing, as
well as active monitoring for salient events, is targeted for completion soon. This integration will
use the emerging [`tezedge-client`](https://github.com/tezedge/tezedge-client) for the Tezos RPC
protocol to interact with the network directly from within the `zkchannel` application.

## Setting up the project

To build the project, you will need a recent version of stable Rust. This project has been tested on
Rust version 1.52.1.

```bash
cargo build --features "allow_explicit_certificate_trust"
```

Notice also that we specify the build option `allow_explicit_certificate_trust`. Without this
option, only certificates rooted at the webpki roots of trust would be trusted, and the customer
would reject the connection to the merchant due to the bad certificate. Because this decreases the
trustworthiness of the authentication between the merchant and customer, this is only intended for
use in testing, and cannot be enabled in release builds.

For development and testing purposes, however, the certificate and private key can be generated
using a provided script, which places them in the `./dev` folder:

```bash
./dev/generate-certificates
```

## Running the `zkchannel` merchant and customer in off-chain mode

First, let's run the merchant server. If we were to install the `zkchannel` binary, it would look
for its `Merchant.toml` configuration file in the idiomatic configuration directory for the current
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
