use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::error::NoteDeckError;
use crate::models::{Account, NormalizedNote, StoredServer};

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

/// A row from the ogp_cache table, mapped to structured fields.
#[derive(Debug, Clone)]
pub struct SummaryRow {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    pub sitename: Option<String>,
    pub icon: Option<String>,
    pub player_url: Option<String>,
    pub player_width: Option<u32>,
    pub player_height: Option<u32>,
    pub player_allow: Option<String>,
    pub final_url: Option<String>,
    pub sensitive: bool,
    pub medias_json: Option<String>,
}

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, NoteDeckError> {
        let mut conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;\
             PRAGMA foreign_keys=ON;\
             PRAGMA synchronous=NORMAL;\
             PRAGMA mmap_size=67108864;\
             PRAGMA cache_size=-2000;\
             PRAGMA temp_store=MEMORY;",
        )?;

        // Run numbered migrations (V1, V2, ...)
        embedded::migrations::runner().run(&mut conn).map_err(|e| {
            NoteDeckError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some(format!("Migration failed: {e}")),
            ))
        })?;

        // One-time FTS rebuild for existing databases upgraded before FTS5 was added
        Self::rebuild_fts_if_needed(&conn)?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.cleanup_cache()?;
        Ok(db)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, NoteDeckError> {
        self.conn.lock().map_err(|_| {
            NoteDeckError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_LOCKED),
                Some("Database lock poisoned".to_string()),
            ))
        })
    }

    /// Populate FTS index from existing data if empty (one-time upgrade path).
    fn rebuild_fts_if_needed(conn: &Connection) -> Result<(), NoteDeckError> {
        let needs_rebuild: bool = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM notes_fts) = 0
                AND (SELECT COUNT(*) FROM notes_cache WHERE text IS NOT NULL) > 0",
            [],
            |row| row.get(0),
        )?;
        if needs_rebuild {
            conn.execute_batch("INSERT INTO notes_fts(notes_fts) VALUES('rebuild')")?;
        }
        Ok(())
    }

    // --- Accounts ---

    fn row_to_account(row: &rusqlite::Row) -> rusqlite::Result<Account> {
        Ok(Account {
            id: row.get(0)?,
            host: row.get(1)?,
            token: row.get(2)?,
            user_id: row.get(3)?,
            username: row.get(4)?,
            display_name: row.get(5)?,
            avatar_url: row.get(6)?,
            software: row.get(7)?,
        })
    }

    pub fn load_accounts(&self) -> Result<Vec<Account>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, host, token, user_id, username, display_name, avatar_url, software FROM accounts ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], Self::row_to_account)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_account(&self, account: &Account) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO accounts (id, host, token, user_id, username, display_name, avatar_url, software)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(host, user_id) DO UPDATE SET
                 token = excluded.token,
                 username = excluded.username,
                 display_name = excluded.display_name,
                 avatar_url = excluded.avatar_url,
                 software = excluded.software",
            params![
                account.id,
                account.host,
                account.token,
                account.user_id,
                account.username,
                account.display_name,
                account.avatar_url,
                account.software,
            ],
        )?;
        Ok(())
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, host, token, user_id, username, display_name, avatar_url, software FROM accounts WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], Self::row_to_account)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn get_account_by_host(&self, host: &str) -> Result<Option<Account>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, host, token, user_id, username, display_name, avatar_url, software FROM accounts WHERE host = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![host], Self::row_to_account)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn get_account_by_host_user(
        &self,
        host: &str,
        user_id: &str,
    ) -> Result<Option<Account>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, host, token, user_id, username, display_name, avatar_url, software FROM accounts WHERE host = ?1 AND user_id = ?2",
        )?;
        let mut rows = stmt.query_map(params![host, user_id], Self::row_to_account)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Clear the token column in DB (after migration to keychain)
    pub fn clear_token(&self, id: &str) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        conn.execute("UPDATE accounts SET token = '' WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn delete_account(&self, id: &str) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
        Ok(())
    }

    // --- Servers ---

    pub fn load_servers(&self) -> Result<Vec<StoredServer>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT host, software, version, features_json, updated_at FROM servers",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(StoredServer {
                host: row.get(0)?,
                software: row.get(1)?,
                version: row.get(2)?,
                features_json: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_server(&self, host: &str) -> Result<Option<StoredServer>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT host, software, version, features_json, updated_at FROM servers WHERE host = ?1",
        )?;
        let mut rows = stmt.query_map(params![host], |row| {
            Ok(StoredServer {
                host: row.get(0)?,
                software: row.get(1)?,
                version: row.get(2)?,
                features_json: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    // --- Notes cache ---

    pub fn cache_notes(
        &self,
        notes: &[NormalizedNote],
        timeline_type: &str,
    ) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO notes_cache (note_id, account_id, server_host, created_at, text, note_json, cached_at, timeline_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(note_id, account_id) DO UPDATE SET
                     note_json = excluded.note_json,
                     cached_at = excluded.cached_at,
                     timeline_type = excluded.timeline_type",
            )?;
            for note in notes {
                let json = serde_json::to_string(note).unwrap_or_default();
                stmt.execute(params![
                    note.id,
                    note.account_id,
                    note.server_host,
                    note.created_at,
                    note.text,
                    json,
                    now,
                    timeline_type,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn cache_note(
        &self,
        note: &NormalizedNote,
        timeline_type: &str,
    ) -> Result<(), NoteDeckError> {
        self.cache_notes(std::slice::from_ref(note), timeline_type)
    }

    pub fn search_cached_notes(
        &self,
        account_id: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        self.search_cached_notes_advanced(account_id, query, limit, None, None, false)
    }

    pub fn search_cached_notes_advanced(
        &self,
        account_id: &str,
        query: &str,
        limit: i64,
        since_date: Option<&str>,
        until_date: Option<&str>,
        ascending: bool,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let conn = self.lock()?;
        let order = if ascending { "ASC" } else { "DESC" };
        let has_query = !query.is_empty();

        let mut conditions = vec!["nc.account_id = ?1".to_string()];
        let mut param_idx = 2u32;

        let fts_query;
        let like_pattern;
        let use_fts = has_query && query.chars().count() >= 3;
        let use_like = has_query && !use_fts;

        if use_fts {
            let escaped = query.replace('"', "\"\"");
            fts_query = format!("\"{escaped}\"");
            conditions.push(format!(
                "nc.rowid IN (SELECT rowid FROM notes_fts WHERE notes_fts MATCH ?{param_idx})"
            ));
            param_idx += 1;
        } else {
            fts_query = String::new();
        }
        if use_like {
            like_pattern = format!("%{query}%");
            conditions.push(format!("nc.text LIKE ?{param_idx}"));
            param_idx += 1;
        } else {
            like_pattern = String::new();
        }

        if since_date.is_some() {
            conditions.push(format!("nc.created_at >= ?{param_idx}"));
            param_idx += 1;
        }
        if until_date.is_some() {
            conditions.push(format!("nc.created_at <= ?{param_idx}"));
            param_idx += 1;
        }

        let sql = format!(
            "SELECT nc.note_json FROM notes_cache nc WHERE {} ORDER BY nc.created_at {order} LIMIT ?{param_idx}",
            conditions.join(" AND "),
        );

        let mut stmt = conn.prepare(&sql)?;

        let mut dynamic_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        dynamic_params.push(Box::new(account_id.to_string()));
        if use_fts {
            dynamic_params.push(Box::new(fts_query));
        }
        if use_like {
            dynamic_params.push(Box::new(like_pattern));
        }
        if let Some(d) = since_date {
            dynamic_params.push(Box::new(d.to_string()));
        }
        if let Some(d) = until_date {
            dynamic_params.push(Box::new(d.to_string()));
        }
        dynamic_params.push(Box::new(limit));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            dynamic_params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let json_str: String = row.get(0)?;
                Ok(json_str)
            })?
            .filter_map(|r| r.ok())
            .collect::<Vec<String>>();

        Ok(rows
            .into_iter()
            .filter_map(|json_str| serde_json::from_str::<NormalizedNote>(&json_str).ok())
            .collect())
    }

    pub fn get_cached_timeline(
        &self,
        account_id: &str,
        timeline_type: &str,
        limit: i64,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT note_json FROM notes_cache
             WHERE account_id = ?1 AND timeline_type = ?2
             ORDER BY created_at DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, timeline_type, limit], |row| {
            let json_str: String = row.get(0)?;
            Ok(json_str)
        })?;
        let mut notes = Vec::new();
        for row in rows {
            let json_str = row?;
            if let Ok(note) = serde_json::from_str::<NormalizedNote>(&json_str) {
                notes.push(note);
            }
        }
        Ok(notes)
    }

    /// Retained for API compatibility. Notes are now kept indefinitely.
    pub fn cleanup_cache(&self) -> Result<(), NoteDeckError> {
        Ok(())
    }

    /// Delete a single note from the cache (e.g. when a deletion event is received).
    pub fn delete_cached_note(&self, note_id: &str) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM notes_cache WHERE note_id = ?1",
            params![note_id],
        )?;
        Ok(())
    }

    /// Return (note_count, db_size_bytes).
    pub fn cache_stats(&self) -> Result<(i64, i64), NoteDeckError> {
        let conn = self.lock()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM notes_cache", [], |row| row.get(0))?;
        let page_count: i64 =
            conn.query_row("SELECT page_count FROM pragma_page_count", [], |row| {
                row.get(0)
            })?;
        let page_size: i64 =
            conn.query_row("SELECT page_size FROM pragma_page_size", [], |row| {
                row.get(0)
            })?;
        Ok((count, page_count * page_size))
    }

    /// Fetch cached notes created at or before the given ISO 8601 datetime.
    pub fn get_cached_timeline_before(
        &self,
        account_id: &str,
        timeline_type: &str,
        before: &str,
        limit: i64,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT note_json FROM notes_cache
             WHERE account_id = ?1 AND timeline_type = ?2 AND created_at <= ?3
             ORDER BY created_at DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![account_id, timeline_type, before, limit], |row| {
            let json_str: String = row.get(0)?;
            Ok(json_str)
        })?;
        let mut notes = Vec::new();
        for row in rows {
            let json_str = row?;
            if let Ok(note) = serde_json::from_str::<NormalizedNote>(&json_str) {
                notes.push(note);
            }
        }
        Ok(notes)
    }

    /// Get the date range (min, max) of cached notes for a timeline.
    pub fn get_cache_date_range(
        &self,
        account_id: &str,
        timeline_type: &str,
    ) -> Result<Option<(String, String)>, NoteDeckError> {
        let conn = self.lock()?;
        let result: (Option<String>, Option<String>) = conn.query_row(
            "SELECT MIN(created_at), MAX(created_at) FROM notes_cache
             WHERE account_id = ?1 AND timeline_type = ?2",
            params![account_id, timeline_type],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        match result {
            (Some(min), Some(max)) => Ok(Some((min, max))),
            _ => Ok(None),
        }
    }

    // --- OGP / Summary cache ---

    pub fn cache_summary(
        &self,
        url: &str,
        row: &SummaryRow,
        ttl_secs: i64,
    ) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        conn.execute(
            "INSERT INTO ogp_cache (url, title, description, image, site_name, icon, player_url, player_width, player_height, player_allow, final_url, sensitive, medias_json, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(url) DO UPDATE SET
                 title = excluded.title,
                 description = excluded.description,
                 image = excluded.image,
                 site_name = excluded.site_name,
                 icon = excluded.icon,
                 player_url = excluded.player_url,
                 player_width = excluded.player_width,
                 player_height = excluded.player_height,
                 player_allow = excluded.player_allow,
                 final_url = excluded.final_url,
                 sensitive = excluded.sensitive,
                 medias_json = excluded.medias_json,
                 expires_at = excluded.expires_at",
            params![
                url,
                row.title,
                row.description,
                row.thumbnail,
                row.sitename,
                row.icon,
                row.player_url,
                row.player_width,
                row.player_height,
                row.player_allow,
                row.final_url,
                row.sensitive as i32,
                row.medias_json,
                now + ttl_secs
            ],
        )?;
        Ok(())
    }

    pub fn get_cached_summary(&self, url: &str) -> Result<Option<SummaryRow>, NoteDeckError> {
        let conn = self.lock()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let result = conn.query_row(
            "SELECT title, description, image, site_name, icon, player_url, player_width, player_height, player_allow, final_url, sensitive, medias_json
             FROM ogp_cache WHERE url = ?1 AND expires_at > ?2",
            params![url, now],
            |row| Self::row_to_summary(url, row),
        );
        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn load_summary_cache(&self, limit: usize) -> Result<Vec<SummaryRow>, NoteDeckError> {
        let conn = self.lock()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let mut stmt = conn.prepare_cached(
            "SELECT url, title, description, image, site_name, icon, player_url, player_width, player_height, player_allow, final_url, sensitive, medias_json
             FROM ogp_cache WHERE expires_at > ?1 LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![now, limit as i64], |row| {
                let url: String = row.get(0)?;
                Self::row_to_summary_offset(&url, row, 1)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Map a DB row (without url column) into SummaryRow. Columns start at index 0.
    fn row_to_summary(url: &str, row: &rusqlite::Row) -> rusqlite::Result<SummaryRow> {
        Self::row_to_summary_offset(url, row, 0)
    }

    /// Map a DB row into SummaryRow with column offset (for queries with/without url column).
    fn row_to_summary_offset(
        url: &str,
        row: &rusqlite::Row,
        off: usize,
    ) -> rusqlite::Result<SummaryRow> {
        let sensitive_i: i32 = row.get(off + 10)?;
        Ok(SummaryRow {
            url: url.to_string(),
            title: row.get(off)?,
            description: row.get(off + 1)?,
            thumbnail: row.get(off + 2)?,
            sitename: row.get(off + 3)?,
            icon: row.get(off + 4)?,
            player_url: row.get(off + 5)?,
            player_width: row.get(off + 6)?,
            player_height: row.get(off + 7)?,
            player_allow: row.get(off + 8)?,
            final_url: row.get(off + 9)?,
            sensitive: sensitive_i != 0,
            medias_json: row.get(off + 11)?,
        })
    }

    pub fn cleanup_expired_ogp(&self) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        conn.execute("DELETE FROM ogp_cache WHERE expires_at <= ?1", params![now])?;
        Ok(())
    }

    pub fn upsert_server(&self, server: &StoredServer) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO servers (host, software, version, features_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(host) DO UPDATE SET
                 software = excluded.software,
                 version = excluded.version,
                 features_json = excluded.features_json,
                 updated_at = excluded.updated_at",
            params![
                server.host,
                server.software,
                server.version,
                server.features_json,
                server.updated_at,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Account, NormalizedNote, NormalizedUser, StoredServer};
    use std::collections::HashMap;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        (dir, db)
    }

    // --- Migration tests ---

    #[test]
    fn migration_creates_all_tables() {
        let (_dir, db) = temp_db();
        let conn = db.lock().unwrap();

        // Verify all expected tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"accounts".to_string()));
        assert!(tables.contains(&"servers".to_string()));
        assert!(tables.contains(&"notes_cache".to_string()));
        assert!(tables.contains(&"ogp_cache".to_string()));
        assert!(tables.contains(&"refinery_schema_history".to_string()));
    }

    #[test]
    fn migration_creates_fts5_virtual_table() {
        let (_dir, db) = temp_db();
        let conn = db.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='notes_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // First open
        let db = Database::open(&db_path).unwrap();
        drop(db);

        // Second open: migrations re-run without error
        let db = Database::open(&db_path).unwrap();
        assert!(db.load_accounts().unwrap().is_empty());
    }

    #[test]
    fn migration_tracks_schema_version() {
        let (_dir, db) = temp_db();
        let conn = db.lock().unwrap();
        let version: i32 = conn
            .query_row(
                "SELECT MAX(version) FROM refinery_schema_history",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(version >= 1);
    }

    #[test]
    fn ogp_cache_has_summaly_columns() {
        let (_dir, db) = temp_db();
        let conn = db.lock().unwrap();
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('ogp_cache')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        for expected in &[
            "icon",
            "player_url",
            "player_width",
            "player_height",
            "player_allow",
            "final_url",
            "sensitive",
            "medias_json",
        ] {
            assert!(
                columns.contains(&expected.to_string()),
                "Missing column: {expected}"
            );
        }
    }

    #[test]
    fn notes_cache_has_timeline_type_column() {
        let (_dir, db) = temp_db();
        let conn = db.lock().unwrap();
        let has: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('notes_cache') WHERE name='timeline_type'",
            )
            .unwrap()
            .query_row([], |row| row.get(0))
            .unwrap();
        assert!(has);
    }

    // --- Account CRUD tests ---

    fn sample_account() -> Account {
        Account {
            id: "acc-1".to_string(),
            host: "misskey.io".to_string(),
            token: "test-token".to_string(),
            user_id: "user-1".to_string(),
            username: "alice".to_string(),
            display_name: Some("Alice".to_string()),
            avatar_url: None,
            software: "misskey".to_string(),
        }
    }

    #[test]
    fn account_upsert_and_load() {
        let (_dir, db) = temp_db();
        db.upsert_account(&sample_account()).unwrap();

        let accounts = db.load_accounts().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].username, "alice");
        assert_eq!(accounts[0].host, "misskey.io");
    }

    #[test]
    fn account_get_by_id() {
        let (_dir, db) = temp_db();
        db.upsert_account(&sample_account()).unwrap();

        let acc = db.get_account("acc-1").unwrap().unwrap();
        assert_eq!(acc.username, "alice");

        assert!(db.get_account("nonexistent").unwrap().is_none());
    }

    #[test]
    fn account_delete() {
        let (_dir, db) = temp_db();
        db.upsert_account(&sample_account()).unwrap();
        db.delete_account("acc-1").unwrap();
        assert!(db.load_accounts().unwrap().is_empty());
    }

    #[test]
    fn account_clear_token() {
        let (_dir, db) = temp_db();
        db.upsert_account(&sample_account()).unwrap();
        db.clear_token("acc-1").unwrap();

        let acc = db.get_account("acc-1").unwrap().unwrap();
        assert!(acc.token.is_empty());
    }

    // --- Server CRUD tests ---

    fn sample_server() -> StoredServer {
        StoredServer {
            host: "misskey.io".to_string(),
            software: "misskey".to_string(),
            version: "2025.3.0".to_string(),
            features_json: "{}".to_string(),
            updated_at: 1700000000,
        }
    }

    #[test]
    fn server_upsert_and_load() {
        let (_dir, db) = temp_db();
        db.upsert_server(&sample_server()).unwrap();

        let servers = db.load_servers().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].host, "misskey.io");
    }

    #[test]
    fn server_get_by_host() {
        let (_dir, db) = temp_db();
        db.upsert_server(&sample_server()).unwrap();

        let s = db.get_server("misskey.io").unwrap().unwrap();
        assert_eq!(s.version, "2025.3.0");

        assert!(db.get_server("nonexistent").unwrap().is_none());
    }

    // --- Notes cache tests ---

    fn sample_note(id: &str, text: &str) -> NormalizedNote {
        NormalizedNote {
            id: id.to_string(),
            account_id: "acc-1".to_string(),
            server_host: "misskey.io".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            text: Some(text.to_string()),
            cw: None,
            user: NormalizedUser {
                id: "user-1".to_string(),
                username: "alice".to_string(),
                host: None,
                name: None,
                avatar_url: None,
                emojis: HashMap::new(),
                is_bot: false,
                avatar_decorations: Vec::new(),
                instance: None,
            },
            visibility: "public".to_string(),
            emojis: HashMap::new(),
            reaction_emojis: HashMap::new(),
            reactions: HashMap::new(),
            my_reaction: None,
            renote_count: 0,
            replies_count: 0,
            files: Vec::new(),
            poll: None,
            reply_id: None,
            renote_id: None,
            channel_id: None,
            reaction_acceptance: None,
            uri: None,
            url: None,
            updated_at: None,
            local_only: false,
            visible_user_ids: Vec::new(),
            is_favorited: false,
            mode_flags: HashMap::new(),
            reply: None,
            renote: None,
        }
    }

    #[test]
    fn cache_note_and_retrieve() {
        let (_dir, db) = temp_db();
        let note = sample_note("note-1", "Hello world");
        db.cache_notes(&[note], "home").unwrap();

        let cached = db.get_cached_timeline("acc-1", "home", 10).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].id, "note-1");
    }

    #[test]
    fn cache_note_delete() {
        let (_dir, db) = temp_db();
        db.cache_notes(&[sample_note("note-1", "test")], "home")
            .unwrap();
        db.delete_cached_note("note-1").unwrap();

        let cached = db.get_cached_timeline("acc-1", "home", 10).unwrap();
        assert!(cached.is_empty());
    }

    #[test]
    fn fts_search_finds_cached_notes() {
        let (_dir, db) = temp_db();
        db.cache_notes(
            &[
                sample_note("n1", "Rust programming language"),
                sample_note("n2", "Python scripting"),
            ],
            "home",
        )
        .unwrap();

        let results = db.search_cached_notes("acc-1", "Rust", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "n1");
    }

    #[test]
    fn cache_date_range() {
        let (_dir, db) = temp_db();
        db.cache_notes(&[sample_note("n1", "test")], "home")
            .unwrap();

        let range = db.get_cache_date_range("acc-1", "home").unwrap();
        assert!(range.is_some());
        let (oldest, newest) = range.unwrap();
        assert_eq!(oldest, newest); // single note
    }

    // --- OGP cache tests ---

    #[test]
    fn ogp_cache_store_and_retrieve() {
        let (_dir, db) = temp_db();
        let row = SummaryRow {
            url: "https://example.com".to_string(),
            title: Some("Example".to_string()),
            description: Some("A test page".to_string()),
            thumbnail: None,
            sitename: None,
            icon: None,
            player_url: None,
            player_width: None,
            player_height: None,
            player_allow: None,
            final_url: None,
            sensitive: false,
            medias_json: None,
        };
        db.cache_summary("https://example.com", &row, 3600).unwrap();

        let cached = db.get_cached_summary("https://example.com").unwrap();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().title, Some("Example".to_string()));
    }

    #[test]
    fn ogp_cache_expired_returns_none() {
        let (_dir, db) = temp_db();
        let row = SummaryRow {
            url: "https://expired.com".to_string(),
            title: Some("Old".to_string()),
            description: None,
            thumbnail: None,
            sitename: None,
            icon: None,
            player_url: None,
            player_width: None,
            player_height: None,
            player_allow: None,
            final_url: None,
            sensitive: false,
            medias_json: None,
        };
        // TTL = 0 means already expired
        db.cache_summary("https://expired.com", &row, 0).unwrap();

        // Should not return expired entry
        let cached = db.get_cached_summary("https://expired.com").unwrap();
        assert!(cached.is_none());
    }
}
