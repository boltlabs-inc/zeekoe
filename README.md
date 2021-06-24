# zeekoe

## Development

```bash
# generate localhost.crt and localhost.key
$ ./dev/generate-certificates

# initialize the database
touch test.db
sqlite3 test.db < src/database/migrations/merchant/*_setup.sql
sqlite3 test.db < src/database/migrations/customer/*_setup.sql

# running the server
$ cargo run --bin zeekoe-server

# running the client
$ ZEEKOE_TRUST_EXPLICIT_CERTIFICATE=$PWD/dev/localhost.crt \
    cargo run \
      ---features allow_explicit_certificate_trust \
      --bin zeekoe-client
```
