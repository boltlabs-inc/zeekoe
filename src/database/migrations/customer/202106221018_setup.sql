CREATE TABLE customer_channels (
  id INTEGER PRIMARY KEY,
  label TEXT NOT NULL UNIQUE,
  address BLOB NOT NULL,
  merchant_deposit BLOB NOT NULL,
  customer_deposit BLOB NOT NULL,
  state BLOB NOT NULL,
  closing_balances BLOB NOT NULL,
  merchant_tezos_public_key TEXT NOT NULL,
  contract_id TEXT,
  config_id INTEGER NOT NULL,
  FOREIGN KEY (config_id)
    REFERENCES configs (id)
);

CREATE UNIQUE INDEX customer_channels_label on customer_channels (label);

CREATE TABLE configs (
  id INTEGER PRIMARY KEY,
  data BLOB NOT NULL
);
