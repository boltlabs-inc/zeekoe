# zeekoe testing

## Development

```bash
# generate localhost.crt and localhost.key
$ ./dev/generate-certificates

# initialize the database
$ ./dev/create-database

# running the server
$ cargo run --bin zeekoe-server

# running the client
$ ZEEKOE_TRUST_EXPLICIT_CERTIFICATE=$PWD/dev/localhost.crt \
    cargo run \
      ---features allow_explicit_certificate_trust \
      --bin zeekoe-client
```
