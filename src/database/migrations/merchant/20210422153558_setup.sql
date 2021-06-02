CREATE TABLE nonces (
  id SERIAL PRIMARY KEY,
  data BLOB NOT NULL
);
CREATE UNIQUE INDEX nonces_data ON nonces (data);

CREATE TABLE revocations (
  id SERIAL PRIMARY KEY,
  lock VARCHAR(256) NOT NULL,
  secret VARCHAR(256)
);
CREATE INDEX revocations_lock on revocations (lock);
