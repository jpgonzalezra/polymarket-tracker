mod config;
mod db;
mod filter;
mod health;
mod notifier;
mod poller;
mod polymarket;
mod telegram;

use config::Config;
use notifier::Notifier;
use poller::Poller;
use polymarket::PolymarketClient;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use teloxide::prelude::*;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load .env
    dotenvy::dotenv().ok();

    // 2. Init tracing
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 3. Parse config
    let config = Config::from_env()?;
    tracing::info!("config loaded");

    // 4. Connect DB + run migrations
    let pool = db::connect(&config.database_url).await?;

    // 5. Record startup timestamp
    let startup_timestamp = chrono::Utc::now().timestamp();
    let start_time = std::time::Instant::now();
    tracing::info!(startup_timestamp, "starting up");

    // 6. Shared state
    let seen_tx_hashes: Arc<RwLock<HashSet<String>>> = Arc::new(RwLock::new(HashSet::new()));
    let last_poll: Arc<RwLock<Option<std::time::Instant>>> = Arc::new(RwLock::new(None));
    let registered_chats: Arc<RwLock<HashSet<i64>>> = Arc::new(RwLock::new(HashSet::new()));

    // 7. Notifier channel
    let (notifier_tx, notifier_rx) = tokio::sync::mpsc::channel(256);

    // 8. Cancellation token
    let shutdown = CancellationToken::new();

    // 9. Create components
    let bot = Bot::new(&config.telegram_bot_token);
    let api_client = PolymarketClient::new(&config.polymarket_api_base_url);

    let poller = Poller::new(
        pool.clone(),
        api_client,
        notifier_tx,
        config.max_concurrency,
        Duration::from_secs(config.poll_interval_secs),
        startup_timestamp,
        seen_tx_hashes,
        last_poll.clone(),
    );

    let notifier = Notifier::new(bot.clone(), registered_chats.clone(), notifier_rx);

    let health_state = health::HealthState {
        pool: pool.clone(),
        last_poll: last_poll.clone(),
        poll_interval: Duration::from_secs(config.poll_interval_secs),
    };

    let bot_state = telegram::BotState {
        pool: pool.clone(),
        admin_user_ids: config.admin_user_ids.clone(),
        start_time,
        last_poll: last_poll.clone(),
        registered_chats: registered_chats.clone(),
    };

    // 10. Spawn tasks
    let shutdown_clone = shutdown.clone();
    let poller_handle = tokio::spawn(async move {
        poller.run(shutdown_clone).await;
    });

    let shutdown_clone = shutdown.clone();
    let notifier_handle = tokio::spawn(async move {
        notifier.run(shutdown_clone).await;
    });

    let health_handle = tokio::spawn(async move {
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.http_port));
        tracing::info!(%addr, "health server starting");
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, health::router(health_state))
            .await
            .unwrap();
    });

    let bot_clone = bot.clone();
    let bot_handle = tokio::spawn(async move {
        tracing::info!("telegram bot starting");
        let handler = Update::filter_message().filter_command::<telegram::Command>().endpoint(
            move |bot: Bot, msg: Message, cmd: telegram::Command| {
                let state = bot_state.clone();
                async move { telegram::handle_command(bot, msg, cmd, state).await }
            },
        );

        Dispatcher::builder(bot_clone, handler)
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;
    });

    // 11. Wait for shutdown signal
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received SIGINT, shutting down");
        }
    }

    shutdown.cancel();

    // Wait for tasks to finish
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        let _ = poller_handle.await;
        let _ = notifier_handle.await;
    })
    .await;

    health_handle.abort();
    bot_handle.abort();

    tracing::info!("shutdown complete");
    Ok(())
}
