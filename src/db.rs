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

pub async fn remove_wallet(pool: &PgPool, proxy_wallet: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM watched_wallets WHERE proxy_wallet = $1")
        .bind(proxy_wallet.to_lowercase())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
