# zeekoe

*Zeekoe* is the Dutch word for the sea-cow, otherwise known as the manatee, a friendly coastal
aquatic mammal. The word "zeekoe" also has the fortuitous coincidence of being one of the few words
whose first and only two consonants are the letters "ZK": and hence the gentle manatee is the mascot
of the **zkChannels protocol** for zero-knowledge layer-2 based transactions.

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
and is either accepted or rejected by the merchant. Thanks to the power of zero-knowledge proofs,
the merchant can validate that the customer has the requisite balance in some open channel, without
learning the payment or real-world identity of the customer. After some number of payments and/or
refunds, the customer or merchant may *close* the channel, which distributes the current channel
balances from the on-chain escrow account to the customer and merchant.

This L2 protocol has a significant privacy advantage over many prior approaches. 
At every point from establishment through closing, the merchant is only able to
correlate the customer's on-chain payment identity with starting and ending balances of the channel,
and explicitly does not gain the ability to connect the customer's identity with the quantity,
price, or nature of the payments any individual customer has made (so long as there are sufficiently
many customers that the merchant can't draw statistical or timing correlations between their
on-chain and off-chain actions).

## Current project status

In general, the zkChannels protocol and most of the underlying software stack are compatible with
any blockchain or other escrow arbiter that supports the verification of various blind signature
constructs. The version of the `zkchannel` application in this repository is specialized to the
Tezos blockchain. Future work will generalize to other escrow arbiters.

⚠️ **Warning:** At this time, this software should be considered a **technology demonstration only**.
In particular, we have not yet prevented resource exhaustion and slow-loris style DoS attacks,
timing-based de-anonymization attacks, and our transport layer does not incorporate Tor, which means 
that the customer may be tracked based on IP address. **We strongly caution against using the software to
handle real currency.**

## Setting up the project

To build the project, you will need: 

  - A recent version of nightly Rust. This project has been tested with 1.56.0-nightly. You can set this with:
  ```
  $ rustup override set nightly-2021-08-31
  ```
- A recent version of Python. This project has been tested with Python 3.8.10. 
- Cryptographic and system dependencies for our Tezos clients:
  ```
  $ sudo apt install libsodium-dev libsecp256k1-dev libgmp-dev libudev-dev
  ```
  If you're on OSX, install dependencies via [Homebrew](https://brew.sh/):
  ```
  $ brew tap cuber/homebrew-libsecp256k1
  $ brew install libsodium libsecp256k1 gmp
  ```
  Please see the installation guides for [PyTezos](https://pytezos.org/quick_start.html) and 
  [tezedge-client](https://github.com/boltlabs-inc/tezedge-client/tree/develop) for further details.

- The PyTezos library:
  ```
  $ pip install pytezos
  ```
- This repository, installed with submodules:
  ```
  $ git clone git@github.com:boltlabs-inc/zeekoe.git --recurse-submodules
  ```

Build a test version of the project with:

```bash
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo build --features "allow_explicit_certificate_trust"
```

We specify the build option `allow_explicit_certificate_trust`. Without this
option, only certificates rooted at the webpki roots of trust would be trusted, and the customer
would reject the connection to the merchant due to the bad certificate. Because this decreases the
trustworthiness of the authentication between the merchant and customer, this is only intended for
use in testing, and cannot be enabled in release builds.

For development and testing purposes, however, the certificate and private key can be generated
using a provided script, which places them in the `./dev` folder:

```bash
./dev/generate-certificates
```

Alternately, the `Dockerfile` includes a complete Ubuntu build specification and can be run on any
machine that supports Docker. Build the container once with:
```
$ docker build -t zeekoe .
```

Then, run instances with
```
$ docker run -it --network host zeekoe
```
Host networking simplifies network specifications, especially when running the merchant server on localhost as in this example.


## Specifying a configuration

Each party has a configuration file that, among other things, specifies the Tezos network and key
material that will be used to fund the channel.
The `tezos_uri` field indicates the URI of the tezos node:
- for testnet, use `"https://rpc.tzkt.io/granadanet/"`
- for a local sandbox, use `"http://localhost:20000"`

To specify the `tezos_account` for a party, you can either specify a path to a key file like those generated by the [tezos faucet](https://faucet.tzalpha.net/) for testnet: 
```
tezos_account = "path/to/key.json"
```
or you can use an alias that the `tezos-client` application has registered. 
In particular, if you use the [Tezos agora instructions to set up a sandbox](https://wiki.tezosagora.org/build/clients/run-a-sandbox), the `tezos-client` will register users `alice` and `bob`:
```
tezos_account = { alias = "alice" }
```

The zkchannels protocol does not activate accounts on chain, reveal their public keys, or fund the accounts; the user will have to do this separately.

## Running the `zkchannel` merchant and customer

First, let's run the merchant server. If we were to install the `zkchannel` binary, it would look
for its `Merchant.toml` configuration file in the idiomatic configuration directory for the current
user, but in this self-contained example we use the `--config` flag to request that it use the
configuration in `./dev`. This configuration also specifies that the merchant should store its
database in that same directory.

```bash
$ ./target/debug/zkchannel merchant --config "./dev/Merchant.toml" run
serving on: [::1]:2611
```

This sets up the merchant server and creates a separate thread that watches the chain and reacts to
any changes in the merchant's open contracts. We must also run a customer chain watcher. These 
watchers must continue to run the entire time that a party has any open channels.
Failure to run the watcher can result in loss of funds!

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" watch
```

Once the chain watchers are running, the customer can establish a new zkChannel with
the merchant, making an initial deposit of 5 XTZ. We specify a human-readable nickname
"my-first-zkchannel" to more easily keep track of the channel.

As with the merchant, we specify a local configuration file using the `--config` flag, which
overrides the default location of the customer configuration file. This configuration file puts the
customer database in the `./dev` directory.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" \
    establish "zkchannel://localhost" \
    --label "my-first-zkchannel" \
    --deposit "5 XTZ"
<output omitted>
Successfully established new channel with label "my-first-zkchannel"

```

The default behavior of the repository sends the establish operations to testnet. The `<output 
omitted>` text above will print out the block hash for the block containing the establish 
operations. You can examine the block online at
```
https://rpc.tzkt.io/granadanet/chains/main/blocks/<block hash>
```

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

Finally, after some number of payments, either party can close the channel. When a close procedure
is initiated, no further payments can be made on the channel. If the customer initiates, it runs:

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" close --force my-first-zkchannel
<output omitted>
```

This command posts the current channel balances on chain. The merchant's chain watcher will see the
post and make sure it is valid. If it is valid, the customer's chain watcher will claim their
balance after 48 hours. If it is not, the merchant's chain watcher will immediately post proof that
the balances are outdated and claim the full channel balance.

If the merchant initiates, it runs:

```bash
$ ./target/debug/zkchannel merchant --config "./dev/Merchant.toml" \
    close --channel aCfl7ZAiew96/Ke+io91bOgyde0bQ3RKC87GlbQ1Jts=
```

The customer's chain watcher will observe the change to the contract. If necessary, it will
post the correct channel balances, and the close procedure will continue as above. Otherwise,
the merchant will claim the full balance of the channel after 48 hours.

Once these steps are complete, we can see that the channel is successfully closed. 

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" list
┌────────────────────┬────────┬───────────┬────────────┬──────────────────────────────────────────────┐
│ Label              ┆ State  ┆ Balance   ┆ Max Refund ┆ Channel ID                                   │
╞════════════════════╪════════╪═══════════╪════════════╪══════════════════════════════════════════════╡
│ my-first-zkchannel ┆ closed ┆ 4.995 XTZ ┆ 0.005 XTZ  ┆ aCfl7ZAiew96/Ke+io91bOgyde0bQ3RKC87GlbQ1Jts= │
└────────────────────┴────────┴───────────┴────────────┴──────────────────────────────────────────────┘
```

The merchant server and customer chain watcher may now be stopped by pressing ^C.

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
