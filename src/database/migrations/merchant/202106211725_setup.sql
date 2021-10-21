CREATE TABLE nonces (
  id INTEGER PRIMARY KEY,
  data BLOB NOT NULL
);
CREATE UNIQUE INDEX nonces_data ON nonces (data);

CREATE TABLE revocations (
  id INTEGER PRIMARY KEY,
  lock BLOB NOT NULL,
  secret BLOB
);
CREATE INDEX revocations_lock ON revocations (lock);

CREATE TABLE merchant_config (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  signing_keypair BLOB NOT NULL,
  revocation_commitment_parameters BLOB NOT NULL,
  range_constraint_parameters BLOB NOT NULL
);

CREATE TABLE merchant_channels (
  id INTEGER PRIMARY KEY,
  channel_id TEXT NOT NULL,
  contract_id BLOB NOT NULL,
  merchant_deposit BLOB NOT NULL,
  customer_deposit BLOB NOT NULL,
  status TEXT NOT NULL
    CHECK (status IN (
      "originated",
      "customer_funded",
      "merchant_funded",
      "active",
      "pending_expiry",
      "pending_close",
      "pending_mutual_close",
      "pending_merchant_claim",
      "dispute",
      "closed"
    )),
  closing_balances BLOB NOT NULL
);

CREATE INDEX merchant_channels_channel_id ON merchant_channels (channel_id);
