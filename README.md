# zeekoe

## Running

```bash
# generate localhost.crt and localhost.key
$ ./dev/generate-certificates

# running the merchant server (leave this running)
$ cargo run --bin zkchannel-merchant -- --config "./dev/Merchant.toml" run

# establish a channel
$ cargo run --features "allow_explicit_certificate_trust" \
    --bin zkchannel-customer -- \
    --config "./dev/Customer.toml" \
    establish "zkchannel://localhost" \
    --label "my-first-zkchannel" \
    --deposit "5 XTZ" \
    --from somewhere

# make a payment
cargo run --features "allow_explicit_certificate_trust" \
    --bin zkchannel-customer -- \
    --config "./dev/Customer.toml" \
    pay "my-first-zkchannel" "0.005 XTZ"
```

## Development

```bash
$ cargo build
```

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
