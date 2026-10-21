//! Database access, transactions, and models for Risk Engine.

pub mod events;
pub mod outbox;
pub mod positions;
pub mod prices;
pub mod state;

pub use events::*;
pub use outbox::*;
pub use positions::*;
pub use prices::*;
pub use state::*;

use sqlx::postgres::{PgPool, PgPoolOptions};

/// Initializes the Postgres connection pool.
pub async fn init_db(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(database_url)
        .await?;

    Ok(pool)
}
