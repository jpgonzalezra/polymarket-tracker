use sqlx::postgres::PgPool;
use sqlx::Row;

#[derive(Debug, Clone)]
pub struct WatchedWallet {
    pub proxy_wallet: String,
    pub alias: Option<String>,
}

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPool::connect(database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("database connected and migrations applied");
    Ok(pool)
}

pub async fn list_wallets(pool: &PgPool) -> Result<Vec<WatchedWallet>, sqlx::Error> {
    let rows = sqlx::query("SELECT proxy_wallet, alias FROM watched_wallets ORDER BY added_at")
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| WatchedWallet {
            proxy_wallet: row.get("proxy_wallet"),
            alias: row.get("alias"),
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

pub async fn remove_wallet(pool: &PgPool, proxy_wallet: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM watched_wallets WHERE proxy_wallet = $1")
        .bind(proxy_wallet.to_lowercase())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
