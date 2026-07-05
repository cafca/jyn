use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use p2panda_core::Timestamp;
use p2panda_store::sqlite::{run_pending_migrations, SqlitePool};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::query;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::Row;

const PROFILE_DB_FILE_NAME: &str = "profile-store.sqlite3";
const KV_TABLE: &str = "profile_store_kv_v1";

pub(crate) struct ProfileDataStore {
    pub(crate) pool: SqlitePool,
}

pub(crate) fn open_profile_data_store(data_dir: &Path) -> Result<ProfileDataStore> {
    let path = profile_data_store_path(data_dir);
    match try_open_profile_data_store(&path) {
        Ok(pool) => Ok(ProfileDataStore { pool }),
        Err(err) if path.exists() => {
            let recovered_path = move_corrupt_db_aside(&path)?;
            let pool = try_open_profile_data_store(&path)?;
            tracing::warn!(
                "recovered profile store after SQLite load failure ({err}); moved corrupt database to {}",
                recovered_path.display()
            );
            Ok(ProfileDataStore { pool })
        }
        Err(err) => Err(err),
    }
}

pub(crate) fn profile_data_store_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PROFILE_DB_FILE_NAME)
}

pub(crate) fn load_json_key<T>(pool: &SqlitePool, key: &str) -> Result<Option<T>>
where
    T: DeserializeOwned + Send + 'static,
{
    let pool = pool.clone();
    let key = key.to_owned();
    let bytes: Option<Vec<u8>> = block_on_db(async move {
        Ok(
            query(&format!("SELECT value FROM {KV_TABLE} WHERE key = ?"))
                .bind(key)
                .fetch_optional(&pool)
                .await
                .context("failed to load value from profile data store")?
                .map(|row| row.get::<Vec<u8>, _>("value")),
        )
    })?;

    bytes
        .map(|bytes| serde_json::from_slice(&bytes).context("failed to decode profile data value"))
        .transpose()
}

pub(crate) fn write_json_key<T>(pool: &SqlitePool, key: &str, value: &T) -> Result<()>
where
    T: Serialize + Send + Sync,
{
    let pool = pool.clone();
    let key = key.to_owned();
    let bytes = serde_json::to_vec(value).context("failed to encode profile data value")?;
    block_on_db(async move {
        query(&format!(
            "INSERT INTO {KV_TABLE} (key, value)
             VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value"
        ))
        .bind(key)
        .bind(bytes)
        .execute(&pool)
        .await
        .context("failed to persist value in profile data store")?;
        Ok(())
    })
}

fn try_open_profile_data_store(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create profile store directory {}",
                parent.display()
            )
        })?;
    }

    let db_path = path.to_path_buf();
    block_on_db(async move {
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .with_context(|| format!("failed to open profile database {}", db_path.display()))?;

        run_pending_migrations(&pool)
            .await
            .context("failed to run p2panda-store SQLite migrations")?;
        ensure_profile_tables(&pool).await?;

        Ok(pool)
    })
}

async fn ensure_profile_tables(pool: &SqlitePool) -> Result<()> {
    query(&format!(
        "CREATE TABLE IF NOT EXISTS {KV_TABLE} (
            key TEXT PRIMARY KEY NOT NULL,
            value BLOB NOT NULL
        )"
    ))
    .execute(pool)
    .await
    .context("failed to create profile data key-value table")?;

    Ok(())
}

fn move_corrupt_db_aside(path: &Path) -> Result<PathBuf> {
    let recovered_path =
        path.with_extension(format!("corrupt-{}.sqlite3", u64::from(Timestamp::now())));
    fs::rename(path, &recovered_path).with_context(|| {
        format!(
            "failed to move corrupt profile database {} to {}",
            path.display(),
            recovered_path.display()
        )
    })?;
    Ok(recovered_path)
}

fn block_on_db<F, T>(future: F) -> Result<T>
where
    F: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("failed to build SQLite helper runtime")
                .and_then(|runtime| runtime.block_on(future));
            let _ = tx.send(result);
        });

        return rx
            .recv()
            .context("SQLite helper thread terminated before returning a result")?;
    }

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build SQLite helper runtime")?
        .block_on(future)
}
