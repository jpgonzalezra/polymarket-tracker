use sqlx::postgres::{PgConnectOptions, PgPool};
use sqlx::{Connection, Row};

#[derive(Debug, Clone)]
pub struct WatchedWallet {
    pub proxy_wallet: String,
    pub alias: Option<String>,
    pub last_synced_timestamp: Option<i64>,
}

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    // Single direct connection for migrations (no prepared statement cache for PgBouncer compatibility)
    let migrate_opts = database_url
        .parse::<PgConnectOptions>()?
        .statement_cache_capacity(0);
    let mut conn = sqlx::postgres::PgConnection::connect_with(&migrate_opts).await?;
    sqlx::migrate!("./migrations").run(&mut conn).await?;
    conn.close().await?;

    // Main pool for the app
    let pool = PgPool::connect(database_url).await?;
    tracing::info!("database connected and migrations applied");
    Ok(pool)
}

pub async fn list_wallets(pool: &PgPool) -> Result<Vec<WatchedWallet>, sqlx::Error> {
    let rows = sqlx::query("SELECT proxy_wallet, alias, last_synced_timestamp FROM watched_wallets ORDER BY added_at")
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| WatchedWallet {
            proxy_wallet: row.get("proxy_wallet"),
            alias: row.get("alias"),
            last_synced_timestamp: row.get("last_synced_timestamp"),
        })
        .collect())
}

pub async fn add_wallet(
    pool: &PgPool,
    proxy_wallet: &str,
    alias: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO watched_wallets (proxy_wallet, alias) VALUES ($1, $2) ON CONFLICT (proxy_wallet) DO UPDATE SET alias = $2",
    )
    .bind(proxy_wallet.to_lowercase())
    .bind(alias)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_last_synced_timestamp(
    pool: &PgPool,
    proxy_wallet: &str,
    timestamp: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE watched_wallets SET last_synced_timestamp = $1 WHERE proxy_wallet = $2",
    )
    .bind(timestamp)
    .bind(proxy_wallet)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_wallet_alias(pool: &PgPool, proxy_wallet: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query("SELECT alias FROM watched_wallets WHERE proxy_wallet = $1")
        .bind(proxy_wallet.to_lowercase())
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.get("alias")))
}

pub async fn remove_wallet(pool: &PgPool, proxy_wallet: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM watched_wallets WHERE proxy_wallet = $1")
        .bind(proxy_wallet.to_lowercase())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn wallet_exists(pool: &PgPool, proxy_wallet: &str) -> Result<bool, sqlx::Error> {
    let row = sqlx::query("SELECT 1 FROM watched_wallets WHERE proxy_wallet = $1")
        .bind(proxy_wallet.to_lowercase())
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub async fn load_registered_chats(pool: &PgPool) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query("SELECT chat_id FROM registered_chats")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.get("chat_id")).collect())
}

pub async fn insert_registered_chat(pool: &PgPool, chat_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO registered_chats (chat_id) VALUES ($1) ON CONFLICT (chat_id) DO NOTHING",
    )
    .bind(chat_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct TradeFilters {
    pub min_amount: Option<f64>,
    pub min_liquidity: Option<f64>,
}

pub async fn load_trade_filters(pool: &PgPool) -> Result<TradeFilters, sqlx::Error> {
    let rows = sqlx::query("SELECT key, value FROM trade_filters")
        .fetch_all(pool)
        .await?;

    let mut filters = TradeFilters {
        min_amount: None,
        min_liquidity: None,
    };

    for row in rows {
        let key: String = row.get("key");
        let value: f64 = row.get("value");
        match key.as_str() {
            "min_amount" => filters.min_amount = Some(value),
            "min_liquidity" => filters.min_liquidity = Some(value),
            _ => {}
        }
    }

    Ok(filters)
}

pub async fn set_trade_filter(pool: &PgPool, key: &str, value: f64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO trade_filters (key, value, updated_at) VALUES ($1, $2, now()) \
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_trade_filter(pool: &PgPool, key: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM trade_filters WHERE key = $1")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
