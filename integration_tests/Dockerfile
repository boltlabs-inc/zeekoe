ARG zeekoe_base
FROM ${zeekoe_base}

ARG tezos_uri
RUN tezos-client --endpoint http://${tezos_uri}:20000 bootstrapped
RUN tezos-client --endpoint http://${tezos_uri}:20000 config update
RUN tezos-client import secret key alice unencrypted:edsk3QoqBuvdamxouPhin7swCvkQNgq4jP5KZPbwWNnwdZpSpJiEbq
RUN tezos-client import secret key bob unencrypted:edsk3RFfvaFaxbHx8BMtEW1rKQcPtDML3LXjNqMNLCzC3wLC1bWbAt

RUN cargo test --all-features --tests integration_tests --verbose -- --nocapture
