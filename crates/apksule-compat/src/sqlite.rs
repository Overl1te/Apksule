//! Minimal SQLite / SupportSQLite surface for Room (M4).

#![allow(
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::case_sensitive_file_extension_comparisons
)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params_from_iter};

use crate::error::{CompatError, Result};
use crate::storage::AppStorage;

/// Open databases under [`AppStorage::databases_dir`].
#[derive(Debug, Default, Clone)]
pub struct SqliteRegistry {
    inner: Arc<Mutex<HashMap<u32, OpenDatabase>>>,
    next_id: Arc<Mutex<u32>>,
}

#[derive(Debug)]
struct OpenDatabase {
    path: PathBuf,
    connection: Connection,
}

impl SqliteRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&self, storage: &AppStorage, name: &str) -> Result<u32> {
        let safe = sanitize_db_name(name)?;
        let path = storage.resolve_database(&safe)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| CompatError::Io { path: parent.to_path_buf(), source })?;
        }
        let connection = Connection::open(&path)
            .map_err(|error| CompatError::Sqlite(error.to_string()))?;
        let mut next = self.next_id.lock().map_err(|_| CompatError::Sqlite("lock".into()))?;
        let id = *next;
        *next = next.saturating_add(1);
        drop(next);
        self.inner
            .lock()
            .map_err(|_| CompatError::Sqlite("lock".into()))?
            .insert(id, OpenDatabase { path, connection });
        Ok(id)
    }

    pub fn exec_sql(&self, db_id: u32, sql: &str) -> Result<()> {
        with_db(self, db_id, |db| {
            db.connection.execute_batch(sql).map_err(|error| CompatError::Sqlite(error.to_string()))
        })
    }

    pub fn insert(
        &self,
        db_id: u32,
        table: &str,
        columns: &[String],
        values: &[SqliteValue],
    ) -> Result<i64> {
        let placeholders = (1..=columns.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        let sql = format!(
            "INSERT INTO {table} ({}) VALUES ({placeholders})",
            columns.join(", ")
        );
        with_db(self, db_id, |db| {
            let binds = values.iter().map(to_sql).collect::<Vec<_>>();
            db.connection
                .execute(&sql, params_from_iter(binds))
                .map_err(|error| CompatError::Sqlite(error.to_string()))?;
            Ok(db.connection.last_insert_rowid())
        })
    }

    pub fn query_scalar_string(
        &self,
        db_id: u32,
        sql: &str,
        binds: &[SqliteValue],
    ) -> Result<Option<String>> {
        with_db(self, db_id, |db| {
            let mut stmt = db
                .connection
                .prepare(sql)
                .map_err(|error| CompatError::Sqlite(error.to_string()))?;
            let binds = binds.iter().map(to_sql).collect::<Vec<_>>();
            let value = stmt
                .query_row(params_from_iter(binds), |row| row.get::<_, String>(0))
                .optional()
                .map_err(|error| CompatError::Sqlite(error.to_string()))?;
            Ok(value)
        })
    }

    pub fn query_rows(
        &self,
        db_id: u32,
        sql: &str,
        binds: &[SqliteValue],
    ) -> Result<Vec<Vec<SqliteValue>>> {
        with_db(self, db_id, |db| {
            let mut stmt = db
                .connection
                .prepare(sql)
                .map_err(|error| CompatError::Sqlite(error.to_string()))?;
            let column_count = stmt.column_count();
            let binds = binds.iter().map(to_sql).collect::<Vec<_>>();
            let mut rows = stmt
                .query(params_from_iter(binds))
                .map_err(|error| CompatError::Sqlite(error.to_string()))?;
            let mut out = Vec::new();
            while let Some(row) =
                rows.next().map_err(|error| CompatError::Sqlite(error.to_string()))?
            {
                let mut values = Vec::with_capacity(column_count);
                for index in 0..column_count {
                    values.push(from_sql_row(row, index)?);
                }
                out.push(values);
            }
            Ok(out)
        })
    }

    pub fn path(&self, db_id: u32) -> Result<PathBuf> {
        with_db(self, db_id, |db| Ok(db.path.clone()))
    }

    pub fn close(&self, db_id: u32) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| CompatError::Sqlite("lock".into()))?
            .remove(&db_id);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SqliteValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

fn with_db<T>(
    registry: &SqliteRegistry,
    db_id: u32,
    f: impl FnOnce(&mut OpenDatabase) -> Result<T>,
) -> Result<T> {
    let mut guard = registry.inner.lock().map_err(|_| CompatError::Sqlite("lock".into()))?;
    let db = guard.get_mut(&db_id).ok_or_else(|| CompatError::Sqlite(format!("unknown db {db_id}")))?;
    f(db)
}

fn to_sql(value: &SqliteValue) -> rusqlite::types::Value {
    match value {
        SqliteValue::Null => rusqlite::types::Value::Null,
        SqliteValue::Integer(v) => rusqlite::types::Value::Integer(*v),
        SqliteValue::Real(v) => rusqlite::types::Value::Real(*v),
        SqliteValue::Text(v) => rusqlite::types::Value::Text(v.clone()),
        SqliteValue::Blob(v) => rusqlite::types::Value::Blob(v.clone()),
    }
}

fn from_sql_row(row: &rusqlite::Row<'_>, index: usize) -> Result<SqliteValue> {
    let value = row
        .get_ref(index)
        .map_err(|error| CompatError::Sqlite(error.to_string()))?;
    Ok(match value {
        rusqlite::types::ValueRef::Null => SqliteValue::Null,
        rusqlite::types::ValueRef::Integer(v) => SqliteValue::Integer(v),
        rusqlite::types::ValueRef::Real(v) => SqliteValue::Real(v),
        rusqlite::types::ValueRef::Text(v) => {
            SqliteValue::Text(String::from_utf8_lossy(v).into_owned())
        }
        rusqlite::types::ValueRef::Blob(v) => SqliteValue::Blob(v.to_vec()),
    })
}

fn sanitize_db_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains("..") || trimmed.contains('/') || trimmed.contains('\\')
    {
        return Err(CompatError::Sqlite(format!("invalid database name {name}")));
    }
    Ok(if trimmed.ends_with(".db") { trimmed.to_owned() } else { format!("{trimmed}.db") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::AppStorage;

    #[test]
    fn sqlite_roundtrip_note() {
        let unique = format!(
            "apksule-sqlite-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let base = std::env::temp_dir().join(unique);
        let storage = AppStorage::for_package_at(&base, "org.example.notes").expect("storage");
        let registry = SqliteRegistry::new();
        let db = registry.open(&storage, "NotallyDatabase").expect("open");
        registry
            .exec_sql(
                db,
                "CREATE TABLE IF NOT EXISTS BaseNote (id INTEGER PRIMARY KEY, title TEXT, body TEXT);",
            )
            .expect("create");
        registry
            .insert(
                db,
                "BaseNote",
                &["title".into(), "body".into()],
                &[SqliteValue::Text("hello".into()), SqliteValue::Text("world".into())],
            )
            .expect("insert");
        let title = registry
            .query_scalar_string(db, "SELECT title FROM BaseNote WHERE id = 1", &[])
            .expect("query");
        assert_eq!(title.as_deref(), Some("hello"));
        let _ = std::fs::remove_dir_all(base);
    }
}
