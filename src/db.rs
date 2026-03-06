use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::error::NoteDeckError;
use crate::models::{Account, NormalizedNote, StoredServer};

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
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
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

    fn migrate(&self) -> Result<(), NoteDeckError> {
        let conn = self.lock()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                host TEXT NOT NULL,
                token TEXT NOT NULL,
                user_id TEXT NOT NULL,
                username TEXT NOT NULL,
                display_name TEXT,
                avatar_url TEXT,
                software TEXT NOT NULL,
                UNIQUE(host, user_id)
            );
            CREATE TABLE IF NOT EXISTS servers (
                host TEXT PRIMARY KEY,
                software TEXT NOT NULL,
                version TEXT NOT NULL,
                features_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS notes_cache (
                note_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                server_host TEXT NOT NULL,
                created_at TEXT NOT NULL,
                text TEXT,
                note_json TEXT NOT NULL,
                cached_at INTEGER NOT NULL,
                PRIMARY KEY (note_id, account_id)
            );
            CREATE INDEX IF NOT EXISTS idx_notes_cache_timeline
                ON notes_cache (account_id, created_at DESC);

            -- FTS5 trigram index for fast substring search (CJK-friendly)
            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                text,
                content='notes_cache',
                content_rowid=rowid,
                tokenize='trigram'
            );
            CREATE TRIGGER IF NOT EXISTS notes_fts_ai
                AFTER INSERT ON notes_cache WHEN new.text IS NOT NULL BEGIN
                INSERT INTO notes_fts(rowid, text) VALUES (new.rowid, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS notes_fts_ad
                AFTER DELETE ON notes_cache WHEN old.text IS NOT NULL BEGIN
                INSERT INTO notes_fts(notes_fts, rowid, text) VALUES('delete', old.rowid, old.text);
            END;",
        )?;

        // Populate FTS from existing data (upgrade path: one-time rebuild)
        let needs_rebuild: bool = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM notes_fts) = 0
                AND (SELECT COUNT(*) FROM notes_cache WHERE text IS NOT NULL) > 0",
            [],
            |row| row.get(0),
        )?;
        if needs_rebuild {
            conn.execute_batch("INSERT INTO notes_fts(notes_fts) VALUES('rebuild')")?;
        }

        // Drop legacy index superseded by FTS5
        conn.execute_batch("DROP INDEX IF EXISTS idx_notes_cache_text")?;

        // OGP cache table
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ogp_cache (
                url TEXT PRIMARY KEY,
                title TEXT,
                description TEXT,
                image TEXT,
                site_name TEXT,
                expires_at INTEGER NOT NULL
            );",
        )?;

        // Add summaly-compatible columns to ogp_cache
        let has_icon_col: bool = conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('ogp_cache') WHERE name='icon'")?
            .query_row([], |row| row.get(0))?;
        if !has_icon_col {
            conn.execute("ALTER TABLE ogp_cache ADD COLUMN icon TEXT", [])?;
            conn.execute("ALTER TABLE ogp_cache ADD COLUMN player_url TEXT", [])?;
            conn.execute("ALTER TABLE ogp_cache ADD COLUMN player_width INTEGER", [])?;
            conn.execute("ALTER TABLE ogp_cache ADD COLUMN player_height INTEGER", [])?;
            conn.execute("ALTER TABLE ogp_cache ADD COLUMN player_allow TEXT", [])?;
            conn.execute("ALTER TABLE ogp_cache ADD COLUMN final_url TEXT", [])?;
            conn.execute(
                "ALTER TABLE ogp_cache ADD COLUMN sensitive INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            conn.execute("ALTER TABLE ogp_cache ADD COLUMN medias_json TEXT", [])?;
        }

        // Add player_allow column (for existing DBs that already have icon but not player_allow)
        let has_player_allow: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('ogp_cache') WHERE name='player_allow'",
            )?
            .query_row([], |row| row.get(0))?;
        if !has_player_allow {
            conn.execute("ALTER TABLE ogp_cache ADD COLUMN player_allow TEXT", [])?;
        }

        // Add timeline_type column for per-timeline cache isolation
        let has_tl_col: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('notes_cache') WHERE name='timeline_type'",
            )?
            .query_row([], |row| row.get(0))?;
        if !has_tl_col {
            conn.execute_batch(
                "ALTER TABLE notes_cache ADD COLUMN timeline_type TEXT NOT NULL DEFAULT '';
                 CREATE INDEX IF NOT EXISTS idx_notes_cache_tl
                     ON notes_cache (account_id, timeline_type, created_at DESC);",
            )?;
        }

        Ok(())
    }

    // --- Accounts ---

    pub fn load_accounts(&self) -> Result<Vec<Account>, NoteDeckError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, host, token, user_id, username, display_name, avatar_url, software
             FROM accounts ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], |row| {
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
        })?;
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
        let mut stmt = conn.prepare(
            "SELECT id, host, token, user_id, username, display_name, avatar_url, software
             FROM accounts WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
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
        })?;
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
        let mut stmt = conn.prepare(
            "SELECT id, host, token, user_id, username, display_name, avatar_url, software
             FROM accounts WHERE host = ?1 AND user_id = ?2",
        )?;
        let mut rows = stmt.query_map(params![host, user_id], |row| {
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
        })?;
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
        let mut stmt =
            conn.prepare("SELECT host, software, version, features_json, updated_at FROM servers")?;
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
        let mut stmt = conn.prepare(
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
        let conn = self.lock()?;

        // FTS5 trigram requires 3+ characters; fall back to LIKE for shorter queries
        let rows: Vec<String> = if query.chars().count() >= 3 {
            let escaped = query.replace('"', "\"\"");
            let fts_query = format!("\"{escaped}\"");
            let mut stmt = conn.prepare_cached(
                "SELECT nc.note_json FROM notes_cache nc
                 WHERE nc.account_id = ?1
                   AND nc.rowid IN (SELECT rowid FROM notes_fts WHERE notes_fts MATCH ?2)
                 ORDER BY nc.created_at DESC
                 LIMIT ?3",
            )?;
            let result: Vec<String> = stmt
                .query_map(params![account_id, fts_query, limit], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            result
        } else {
            let pattern = format!("%{query}%");
            let mut stmt = conn.prepare_cached(
                "SELECT note_json FROM notes_cache
                 WHERE account_id = ?1 AND text LIKE ?2
                 ORDER BY created_at DESC
                 LIMIT ?3",
            )?;
            let result: Vec<String> = stmt
                .query_map(params![account_id, pattern, limit], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            result
        };

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
        let mut stmt = conn.prepare(
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
