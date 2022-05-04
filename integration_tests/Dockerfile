ARG zeekoe_base
FROM ${zeekoe_base}

ARG tezos_uri
RUN tezos-client config reset
RUN tezos-client --endpoint ${tezos_uri} bootstrapped
RUN tezos-client --endpoint ${tezos_uri} config update
RUN tezos-client import secret key alice unencrypted:edsk3QoqBuvdamxouPhin7swCvkQNgq4jP5KZPbwWNnwdZpSpJiEbq
RUN tezos-client import secret key bob unencrypted:edsk3RFfvaFaxbHx8BMtEW1rKQcPtDML3LXjNqMNLCzC3wLC1bWbAt

ARG branch
RUN git fetch
RUN git reset --hard origin/${branch} && cargo test --all-features --tests integration_tests --verbose -- ${tezos_uri}
