ALTER TABLE watched_wallets ADD COLUMN IF NOT EXISTS last_synced_timestamp BIGINT;
