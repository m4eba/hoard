//! SQLite access: a small connection pool, the schema, and a helper that runs
//! blocking queries off the async runtime.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension};

pub use rusqlite::params;

/// Each entry is applied once, in order; `PRAGMA user_version` records how far
/// a database has got. Never edit a migration that has shipped — add one.
const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE roots (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    path         TEXT NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_scan_at INTEGER
);

-- Every directory under a root, '' being the root itself. Kept so empty
-- folders still show in the tree and can be moved into.
CREATE TABLE folders (
    root_id INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    path    TEXT NOT NULL,
    PRIMARY KEY (root_id, path)
) WITHOUT ROWID;

CREATE TABLE media (
    id          INTEGER PRIMARY KEY,
    root_id     INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    folder      TEXT NOT NULL,
    filename    TEXT NOT NULL,
    kind        TEXT NOT NULL,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    source_url  TEXT NOT NULL DEFAULT '',
    size        INTEGER NOT NULL,
    file_mtime  INTEGER NOT NULL,
    added_at    INTEGER NOT NULL,
    duration    REAL NOT NULL DEFAULT 0,
    width       INTEGER NOT NULL DEFAULT 0,
    height      INTEGER NOT NULL DEFAULT 0,
    video_codec TEXT NOT NULL DEFAULT '',
    audio_codec TEXT NOT NULL DEFAULT '',
    -- 0 waiting for probe and thumbnail, 1 done, 2 failed
    processed   INTEGER NOT NULL DEFAULT 0,
    -- bumped whenever the thumbnail is rewritten, so URLs can be cached forever
    thumb_ver   INTEGER NOT NULL DEFAULT 0,
    -- JSON describing the sprite sheet, once one has been generated
    storyboard  TEXT,
    UNIQUE (root_id, folder, filename)
);
CREATE INDEX media_added ON media (added_at);
CREATE INDEX media_folder ON media (root_id, folder);
CREATE INDEX media_pending ON media (processed) WHERE processed = 0;

CREATE TABLE media_tags (
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    tag      TEXT NOT NULL,
    PRIMARY KEY (media_id, tag)
) WITHOUT ROWID;
CREATE INDEX media_tags_tag ON media_tags (tag);

CREATE VIRTUAL TABLE media_fts USING fts5(
    title, filename, tags, description, folder,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- The search index follows the tables through triggers, so no code path that
-- writes media or tags can forget to keep it current.
CREATE TRIGGER media_ai AFTER INSERT ON media BEGIN
    INSERT INTO media_fts (rowid, title, filename, tags, description, folder)
    VALUES (new.id, new.title, new.filename, '', new.description, new.folder);
END;
CREATE TRIGGER media_au AFTER UPDATE OF title, filename, description, folder ON media BEGIN
    UPDATE media_fts SET title = new.title, filename = new.filename,
        description = new.description, folder = new.folder
    WHERE rowid = new.id;
END;
CREATE TRIGGER media_ad AFTER DELETE ON media BEGIN
    DELETE FROM media_fts WHERE rowid = old.id;
END;
CREATE TRIGGER media_tags_ai AFTER INSERT ON media_tags BEGIN
    UPDATE media_fts SET tags = (SELECT coalesce(group_concat(tag, ' '), '')
        FROM media_tags WHERE media_id = new.media_id)
    WHERE rowid = new.media_id;
END;
CREATE TRIGGER media_tags_ad AFTER DELETE ON media_tags BEGIN
    UPDATE media_fts SET tags = (SELECT coalesce(group_concat(tag, ' '), '')
        FROM media_tags WHERE media_id = old.media_id)
    WHERE rowid = old.media_id;
END;

CREATE TABLE import_jobs (
    id          INTEGER PRIMARY KEY,
    batch_id    TEXT NOT NULL,
    url         TEXT NOT NULL,
    state       TEXT NOT NULL,
    title       TEXT NOT NULL DEFAULT '',
    thumb_url   TEXT NOT NULL DEFAULT '',
    bytes_done  INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    error       TEXT NOT NULL DEFAULT '',
    tags        TEXT NOT NULL DEFAULT '[]',
    root_id     INTEGER,
    folder      TEXT NOT NULL DEFAULT '',
    media_id    INTEGER,
    created_at  INTEGER NOT NULL
);
CREATE INDEX import_jobs_batch ON import_jobs (batch_id);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE sessions (
    token      TEXT PRIMARY KEY,
    expires_at INTEGER NOT NULL
);
"#, r#"
-- Videos waiting for a storyboard. A table rather than memory, so a large
-- queue survives a restart.
CREATE TABLE storyboard_queue (
    media_id  INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    priority  INTEGER NOT NULL DEFAULT 0,
    queued_at INTEGER NOT NULL
);
"#, r#"
-- Ids are never reused. Without AUTOINCREMENT, SQLite hands a deleted
-- table's top ids to the next rows, and thumbnail and storyboard URLs, which
-- are cached for good by id and version, then showed the old file's images.
-- Rebuilt the documented way: new table, copy, drop, rename.
CREATE TABLE roots_new (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    path         TEXT NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_scan_at INTEGER
);
INSERT INTO roots_new (id, name, path, created_at, last_scan_at)
    SELECT id, name, path, created_at, last_scan_at FROM roots;
DROP TABLE roots;
ALTER TABLE roots_new RENAME TO roots;

DROP TRIGGER media_ai;
DROP TRIGGER media_au;
DROP TRIGGER media_ad;
CREATE TABLE media_new (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id     INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    folder      TEXT NOT NULL,
    filename    TEXT NOT NULL,
    kind        TEXT NOT NULL,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    source_url  TEXT NOT NULL DEFAULT '',
    size        INTEGER NOT NULL,
    file_mtime  INTEGER NOT NULL,
    added_at    INTEGER NOT NULL,
    duration    REAL NOT NULL DEFAULT 0,
    width       INTEGER NOT NULL DEFAULT 0,
    height      INTEGER NOT NULL DEFAULT 0,
    video_codec TEXT NOT NULL DEFAULT '',
    audio_codec TEXT NOT NULL DEFAULT '',
    processed   INTEGER NOT NULL DEFAULT 0,
    thumb_ver   INTEGER NOT NULL DEFAULT 0,
    storyboard  TEXT,
    UNIQUE (root_id, folder, filename)
);
INSERT INTO media_new (id, root_id, folder, filename, kind, title, description, source_url,
        size, file_mtime, added_at, duration, width, height, video_codec, audio_codec,
        processed, thumb_ver, storyboard)
    SELECT id, root_id, folder, filename, kind, title, description, source_url,
        size, file_mtime, added_at, duration, width, height, video_codec, audio_codec,
        processed, thumb_ver, storyboard FROM media;
DROP TABLE media;
ALTER TABLE media_new RENAME TO media;
CREATE INDEX media_added ON media (added_at);
CREATE INDEX media_folder ON media (root_id, folder);
CREATE INDEX media_pending ON media (processed) WHERE processed = 0;
CREATE TRIGGER media_ai AFTER INSERT ON media BEGIN
    INSERT INTO media_fts (rowid, title, filename, tags, description, folder)
    VALUES (new.id, new.title, new.filename, '', new.description, new.folder);
END;
CREATE TRIGGER media_au AFTER UPDATE OF title, filename, description, folder ON media BEGIN
    UPDATE media_fts SET title = new.title, filename = new.filename,
        description = new.description, folder = new.folder
    WHERE rowid = new.id;
END;
CREATE TRIGGER media_ad AFTER DELETE ON media BEGIN
    DELETE FROM media_fts WHERE rowid = old.id;
END;

-- Browsers may already hold another file's image under a reused id; new
-- versions give every existing thumbnail and storyboard a fresh URL.
UPDATE media SET thumb_ver = thumb_ver + 1 WHERE thumb_ver > 0;
UPDATE media SET storyboard = json_set(storyboard, '$.ver', json_extract(storyboard, '$.ver') + 1)
    WHERE storyboard IS NOT NULL;
"#, r#"
-- Images ffmpeg could not decode (animated WebP before FFmpeg 8) now have a
-- fallback; give the ones left without a thumbnail another pass.
UPDATE media SET processed = 0 WHERE kind = 'image' AND thumb_ver = 0;
"#];

#[derive(Clone)]
pub struct Db(r2d2::Pool<SqliteConnectionManager>);

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path).with_init(|c| {
            c.busy_timeout(Duration::from_secs(10))?;
            c.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;",
            )
        });
        let pool = r2d2::Pool::builder()
            .max_size(8)
            .build(manager)
            .with_context(|| format!("opening {}", path.display()))?;
        let mut conn = pool.get()?;
        migrate(&mut conn)?;
        Ok(Self(pool))
    }

    /// Runs `f` on a pooled connection on the blocking thread pool.
    pub async fn run<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let pool = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get()?;
            f(&mut conn)
        })
        .await?
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version as usize >= MIGRATIONS.len() {
        return Ok(());
    }
    // Rebuilding a table means dropping it, and with foreign keys on, the drop
    // would cascade into every table referencing it. The pragma is a no-op
    // inside a transaction, so it is switched around the whole run, and the
    // references are checked before it is switched back.
    conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    let result = (|| -> Result<()> {
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql).with_context(|| format!("migration {}", i + 1))?;
            let broken: i64 = tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))?;
            if broken > 0 {
                anyhow::bail!("migration {} left {broken} broken references", i + 1);
            }
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
            tracing::info!("applied migration {}", i + 1);
        }
        Ok(())
    })();
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    result
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?", [key], |r| r.get(0))
        .optional()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thumbnails are cached by id, so an id handed out twice shows one
    /// file's thumbnail on another.
    #[test]
    fn ids_are_never_reused() {
        let dir = std::env::temp_dir().join(format!("hoard-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("test.db")).unwrap();
        let mut conn = db.0.get().unwrap();
        let add = |c: &mut Connection| -> (i64, i64) {
            c.execute("INSERT INTO roots (name, path, created_at) VALUES ('r', '/r', 0)", []).unwrap();
            let root = c.last_insert_rowid();
            c.execute(
                "INSERT INTO media (root_id, folder, filename, kind, title, size, file_mtime, added_at)
                 VALUES (?, '', 'a.mp4', 'video', 'a', 1, 0, 0)",
                [root],
            )
            .unwrap();
            (root, c.last_insert_rowid())
        };
        let (root, media) = add(&mut conn);
        conn.execute("DELETE FROM roots WHERE id = ?", [root]).unwrap();
        let gone: i64 = conn.query_row("SELECT count(*) FROM media", [], |r| r.get(0)).unwrap();
        assert_eq!(gone, 0, "deleting a root cascades to its media");
        let (root2, media2) = add(&mut conn);
        assert!(root2 > root && media2 > media);
        drop(conn);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
