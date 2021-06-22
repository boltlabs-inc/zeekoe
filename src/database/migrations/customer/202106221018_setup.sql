CREATE TABLE customer_channels (
  id SERIAL PRIMARY KEY,
  label TEXT NOT NULL UNIQUE,
  address BLOB NOT NULL,
  state BLOB,
  clean BOOLEAN NOT NULL
);
CREATE UNIQUE INDEX customer_channels_label on customer_channels (label);