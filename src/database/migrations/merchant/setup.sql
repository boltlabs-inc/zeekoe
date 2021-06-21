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

CREATE TABLE config (
  id SERIAL PRIMARY KEY,
  signing_keypair BLOB NOT NULL,
  revocation_commitment_parameters BLOB NOT NULL,
  range_proof_parameters BLOB NOT NULL
);
