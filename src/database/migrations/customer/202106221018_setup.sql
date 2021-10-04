CREATE TABLE customer_channels (
  id SERIAL PRIMARY KEY,
  label TEXT NOT NULL UNIQUE,
  address BLOB NOT NULL,
  merchant_deposit BLOB NOT NULL,
  customer_deposit BLOB NOT NULL,
  state BLOB NOT NULL,
  closing_balances BLOB NOT NULL,
  merchant_tezos_public_key TEXT NOT NULL,
  contract_id TEXT
);
CREATE UNIQUE INDEX customer_channels_label on customer_channels (label);
