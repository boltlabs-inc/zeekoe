CREATE TABLE nonces (
  id SERIAL PRIMARY KEY,
  data BLOB NOT NULL
);
CREATE UNIQUE INDEX nonces_data ON nonces (data);

CREATE TABLE revocations (
  id SERIAL PRIMARY KEY,
  lock BLOB NOT NULL,
  secret BLOB
);
CREATE INDEX revocations_lock on revocations (lock);

CREATE TABLE merchant_config (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  signing_keypair BLOB NOT NULL,
  revocation_commitment_parameters BLOB NOT NULL,
  range_proof_parameters BLOB NOT NULL
);

CREATE TABLE merchant_channels (
  id SERIAL PRIMARY KEY,
  channel_id BLOB NOT NULL,
  contract_id BLOB NOT NULL,
  status TEXT NOT NULL
    CHECK (status IN (
      "originated",
      "customer_funded",
      "merchant_funded",
      "active",
      "closed"
    ))
);
