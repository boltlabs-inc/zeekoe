# zeekoe

## Development

```bash
# generate localhost.crt and localhost.key
$ ./dev/generate-certificates

# initialize the database
$ ./dev/create-database

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
