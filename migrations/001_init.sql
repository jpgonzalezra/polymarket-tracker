CREATE TABLE IF NOT EXISTS watched_wallets (
    proxy_wallet TEXT PRIMARY KEY,
    alias TEXT,
    added_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
