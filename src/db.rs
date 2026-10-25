//! SQLite schema, migrations and queries.

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime};
use rusqlite::{Connection, OptionalExtension, Row, params};

// Same on-disk format SQLAlchemy used for SQLite DATETIME columns, so
// databases created by the old Flask app keep working.
const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.6f";

pub fn format_dt(dt: NaiveDateTime) -> String {
    dt.format(DATETIME_FORMAT).to_string()
}

fn parse_dt(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f").ok()
}

/// Brings any existing database (including the Flask one) up to the current
/// schema. Only ever adds tables and columns.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS problem (
            id INTEGER NOT NULL,
            name VARCHAR(100) NOT NULL,
            link VARCHAR(200) NOT NULL,
            due_date DATETIME,
            repetitions_left INTEGER,
            current_interval INTEGER,
            PRIMARY KEY (id)
        );",
    )?;

    let existing: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('problem')")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let new_columns = [
        ("difficulty", "TEXT"),
        ("notes", "TEXT NOT NULL DEFAULT ''"),
        ("base_interval", "INTEGER"),
        ("total_reps", "INTEGER"),
        ("created_at", "DATETIME"),
        ("mastered_at", "DATETIME"),
    ];
    for (name, ddl) in new_columns {
        if !existing.iter().any(|c| c == name) {
            conn.execute_batch(&format!("ALTER TABLE problem ADD COLUMN {name} {ddl}"))?;
        }
    }

    conn.execute_batch(
        // Problems added by the Flask app always started at 14 days.
        "UPDATE problem SET base_interval = 14 WHERE base_interval IS NULL;
        UPDATE problem SET total_reps = repetitions_left WHERE total_reps IS NULL;

        CREATE TABLE IF NOT EXISTS review (
            id INTEGER PRIMARY KEY,
            problem_id INTEGER NOT NULL,
            reviewed_at DATETIME NOT NULL,
            result TEXT NOT NULL,
            prev_due_date DATETIME,
            prev_interval INTEGER,
            prev_reps_left INTEGER
        );
        CREATE INDEX IF NOT EXISTS review_problem_id ON review (problem_id);",
    )
}

pub struct Problem {
    pub id: i64,
    pub name: String,
    pub link: String,
    pub difficulty: Option<String>,
    pub notes: String,
    pub due: NaiveDateTime,
    pub reps_left: i64,
    pub total_reps: i64,
    pub interval: i64,
    pub base_interval: i64,
    pub created_at: Option<NaiveDateTime>,
    pub mastered_at: Option<NaiveDateTime>,
}

const PROBLEM_COLUMNS: &str = "id, name, link, difficulty, notes, due_date, repetitions_left,
    total_reps, current_interval, base_interval, created_at, mastered_at";

fn problem_from_row(row: &Row) -> rusqlite::Result<Problem> {
    let dt = |i: usize| -> rusqlite::Result<Option<NaiveDateTime>> {
        Ok(row.get::<_, Option<String>>(i)?.as_deref().and_then(parse_dt))
    };
    let reps_left = row.get::<_, Option<i64>>(6)?.unwrap_or(3);
    Ok(Problem {
        id: row.get(0)?,
        name: row.get(1)?,
        link: row.get(2)?,
        difficulty: row.get(3)?,
        notes: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        due: dt(5)?.unwrap_or_else(crate::now),
        reps_left,
        total_reps: row.get::<_, Option<i64>>(7)?.unwrap_or(reps_left),
        interval: row.get::<_, Option<i64>>(8)?.unwrap_or(1),
        base_interval: row.get::<_, Option<i64>>(9)?.unwrap_or(14),
        created_at: dt(10)?,
        mastered_at: dt(11)?,
    })
}

pub fn list_problems(conn: &Connection) -> rusqlite::Result<Vec<Problem>> {
    conn.prepare(&format!(
        "SELECT {PROBLEM_COLUMNS} FROM problem ORDER BY due_date, id"
    ))?
    .query_map([], problem_from_row)?
    .collect()
}

pub fn get_problem(conn: &Connection, id: i64) -> rusqlite::Result<Option<Problem>> {
    conn.query_row(
        &format!("SELECT {PROBLEM_COLUMNS} FROM problem WHERE id = ?1"),
        [id],
        problem_from_row,
    )
    .optional()
}

pub fn find_active_by_link(conn: &Connection, link: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM problem WHERE link = ?1 AND mastered_at IS NULL",
        [link],
        |row| row.get(0),
    )
    .optional()
}

pub struct NewProblem<'a> {
    pub name: &'a str,
    pub link: &'a str,
    pub difficulty: Option<&'a str>,
    pub notes: &'a str,
    pub reps: i64,
    pub first_interval: i64,
    pub now: NaiveDateTime,
}

pub fn insert_problem(conn: &Connection, p: &NewProblem) -> rusqlite::Result<i64> {
    let due = p.now + chrono::TimeDelta::days(p.first_interval);
    conn.execute(
        "INSERT INTO problem (name, link, difficulty, notes, due_date, repetitions_left,
            total_reps, current_interval, base_interval, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?7, ?8)",
        params![
            p.name,
            p.link,
            p.difficulty,
            p.notes,
            format_dt(due),
            p.reps,
            p.first_interval,
            format_dt(p.now)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_schedule(
    conn: &Connection,
    id: i64,
    due: NaiveDateTime,
    interval: i64,
    reps_left: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE problem SET due_date = ?1, current_interval = ?2, repetitions_left = ?3,
            mastered_at = NULL
         WHERE id = ?4",
        params![format_dt(due), interval, reps_left, id],
    )?;
    Ok(())
}

pub fn mark_mastered(conn: &Connection, id: i64, now: NaiveDateTime) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE problem SET repetitions_left = 0, mastered_at = ?1 WHERE id = ?2",
        params![format_dt(now), id],
    )?;
    Ok(())
}

pub fn reactivate(conn: &Connection, id: i64, now: NaiveDateTime) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE problem SET repetitions_left = 1, total_reps = total_reps + 1,
            mastered_at = NULL, due_date = ?1
         WHERE id = ?2",
        params![format_dt(now), id],
    )?;
    Ok(())
}

pub fn update_details(
    conn: &Connection,
    id: i64,
    name: &str,
    difficulty: Option<&str>,
    notes: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE problem SET name = ?1, difficulty = ?2, notes = ?3 WHERE id = ?4",
        params![name, difficulty, notes, id],
    )?;
    Ok(())
}

pub fn delete_problem(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    conn.execute("DELETE FROM review WHERE problem_id = ?1", [id])?;
    Ok(conn.execute("DELETE FROM problem WHERE id = ?1", [id])? > 0)
}

pub fn insert_review(
    conn: &Connection,
    before: &Problem,
    result: &str,
    now: NaiveDateTime,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO review (problem_id, reviewed_at, result, prev_due_date, prev_interval,
            prev_reps_left)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            before.id,
            format_dt(now),
            result,
            format_dt(before.due),
            before.interval,
            before.reps_left
        ],
    )?;
    Ok(())
}

/// Restores the problem to how it was before its latest review and removes
/// that review. Returns false if the problem has never been reviewed.
pub fn undo_last_review(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    let last = conn
        .query_row(
            "SELECT id, prev_due_date, prev_interval, prev_reps_left FROM review
             WHERE problem_id = ?1 ORDER BY reviewed_at DESC, id DESC LIMIT 1",
            [id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((review_id, due, interval, reps_left)) = last else {
        return Ok(false);
    };
    conn.execute(
        "UPDATE problem SET due_date = ?1, current_interval = ?2, repetitions_left = ?3,
            mastered_at = NULL
         WHERE id = ?4",
        params![due, interval, reps_left, id],
    )?;
    conn.execute("DELETE FROM review WHERE id = ?1", [review_id])?;
    Ok(true)
}

pub struct Review {
    pub reviewed_at: NaiveDateTime,
    pub result: String,
}

/// Reviews grouped by problem (all problems, or just `only`), oldest first.
pub fn reviews_by_problem(
    conn: &Connection,
    only: Option<i64>,
) -> rusqlite::Result<HashMap<i64, Vec<Review>>> {
    let mut map: HashMap<i64, Vec<Review>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT problem_id, reviewed_at, result FROM review
         WHERE ?1 IS NULL OR problem_id = ?1 ORDER BY reviewed_at, id",
    )?;
    let rows = stmt.query_map([only], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (problem_id, reviewed_at, result) = row?;
        if let Some(reviewed_at) = parse_dt(&reviewed_at) {
            map.entry(problem_id).or_default().push(Review {
                reviewed_at,
                result,
            });
        }
    }
    Ok(map)
}

/// Distinct days with at least one review, newest first.
pub fn review_days(conn: &Connection) -> rusqlite::Result<Vec<NaiveDate>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT substr(reviewed_at, 1, 10) AS day FROM review ORDER BY day DESC",
    )?;
    let days = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(|r| match r {
            Ok(s) => NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok().map(Ok),
            Err(e) => Some(Err(e)),
        })
        .collect();
    days
}

pub fn review_count_since(conn: &Connection, since: NaiveDateTime) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM review WHERE reviewed_at >= ?1",
        [format_dt(since)],
        |row| row.get(0),
    )
}
