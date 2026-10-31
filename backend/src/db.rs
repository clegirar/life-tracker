use std::str::FromStr;
use std::time::Duration;

use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

pub static MIGRATOR: Migrator = sqlx::migrate!();

/// Pool à connexion différée : le serveur démarre même si la base arrive après lui.
pub fn pool(url: &str) -> Result<PgPool, sqlx::Error> {
    // Rien de ce que sqlx journaliserait n'a sa place dans nos logs.
    let options = PgConnectOptions::from_str(url)?
        .log_statements(LevelFilter::Off)
        .log_slow_statements(LevelFilter::Off, Duration::ZERO);
    Ok(PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy_with(options))
}
