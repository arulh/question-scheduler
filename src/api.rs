//! JSON API handlers.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::{self, NewProblem, Problem, Review};
use crate::now;
use crate::schedule::{self, Next, Outcome, RELEARNING, Schedule};

pub type Db = Arc<Mutex<Connection>>;

const DIFFICULTIES: [&str; 3] = ["easy", "medium", "hard"];

pub fn router(db: Db) -> Router {
    Router::new()
        .route("/api/problems", get(list_problems).post(create_problem))
        .route(
            "/api/problems/{id}",
            axum::routing::patch(update_problem).delete(delete_problem),
        )
        .route("/api/problems/{id}/review", post(review_problem))
        .route("/api/problems/{id}/undo", post(undo_review))
        .route("/api/problems/{id}/snooze", post(snooze_problem))
        .route("/api/problems/{id}/reactivate", post(reactivate_problem))
        .route("/api/stats", get(stats))
        .with_state(db)
}

// --- Errors ---

pub enum AppError {
    NotFound,
    BadRequest(String),
    Conflict(String),
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Db(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "Problem not found".to_string()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            AppError::Db(e) => {
                eprintln!("database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error".to_string())
            }
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, AppError>;

// --- Response types ---

#[derive(Serialize)]
pub struct ProblemDto {
    id: i64,
    name: String,
    link: String,
    difficulty: Option<String>,
    notes: String,
    /// 'overdue' | 'due' | 'upcoming' | 'mastered'
    status: &'static str,
    due_date: String,
    /// Negative when overdue.
    days_until_due: i64,
    repetitions_left: i64,
    total_reps: i64,
    current_interval: i64,
    relearning: bool,
    base_interval: i64,
    created_at: Option<String>,
    mastered_at: Option<String>,
    history: Vec<HistoryEntry>,
}

#[derive(Serialize)]
struct HistoryEntry {
    date: String,
    result: String,
}

fn today() -> NaiveDate {
    now().date()
}

fn ymd(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d").to_string()
}

fn to_dto(p: Problem, history: Vec<Review>, today: NaiveDate) -> ProblemDto {
    let days_until_due = (p.due.date() - today).num_days();
    let status = if p.mastered_at.is_some() {
        "mastered"
    } else if days_until_due < 0 {
        "overdue"
    } else if days_until_due == 0 {
        "due"
    } else {
        "upcoming"
    };
    ProblemDto {
        id: p.id,
        name: p.name,
        link: p.link,
        difficulty: p.difficulty,
        notes: p.notes,
        status,
        due_date: ymd(p.due),
        days_until_due,
        repetitions_left: p.reps_left,
        total_reps: p.total_reps,
        current_interval: p.interval,
        relearning: p.interval == RELEARNING,
        base_interval: p.base_interval,
        created_at: p.created_at.map(ymd),
        mastered_at: p.mastered_at.map(ymd),
        history: history
            .into_iter()
            .map(|r| HistoryEntry {
                date: ymd(r.reviewed_at),
                result: r.result,
            })
            .collect(),
    }
}

fn load_dto(conn: &Connection, id: i64) -> Result<ProblemDto, AppError> {
    let problem = db::get_problem(conn, id)?.ok_or(AppError::NotFound)?;
    let history = db::reviews_by_problem(conn, Some(id))?
        .remove(&id)
        .unwrap_or_default();
    Ok(to_dto(problem, history, today()))
}

fn normalize_difficulty(d: Option<&str>) -> Result<Option<String>, AppError> {
    match d.map(|d| d.trim().to_lowercase()) {
        None => Ok(None),
        Some(d) if d.is_empty() => Ok(None),
        Some(d) if DIFFICULTIES.contains(&d.as_str()) => Ok(Some(d)),
        Some(d) => Err(AppError::BadRequest(format!("Unknown difficulty '{d}'"))),
    }
}

// --- Routes ---

async fn list_problems(State(db): State<Db>) -> ApiResult<Vec<ProblemDto>> {
    let conn = db.lock().unwrap();
    let mut reviews = db::reviews_by_problem(&conn, None)?;
    let today = today();
    let problems = db::list_problems(&conn)?
        .into_iter()
        .map(|p| {
            let history = reviews.remove(&p.id).unwrap_or_default();
            to_dto(p, history, today)
        })
        .collect();
    Ok(Json(problems))
}

#[derive(Deserialize)]
struct CreateRequest {
    link: String,
    name: Option<String>,
    difficulty: Option<String>,
    notes: Option<String>,
    reps: Option<i64>,
    /// Days until the first review; later gaps double from this.
    first_interval: Option<i64>,
}

async fn create_problem(
    State(db): State<Db>,
    Json(req): Json<CreateRequest>,
) -> Result<(StatusCode, Json<ProblemDto>), AppError> {
    if req.link.trim().is_empty() {
        return Err(AppError::BadRequest("A link is required".into()));
    }
    let reps = req.reps.unwrap_or(3);
    if !(1..=20).contains(&reps) {
        return Err(AppError::BadRequest("Reviews must be between 1 and 20".into()));
    }
    let first_interval = req.first_interval.unwrap_or(14);
    if !(1..=schedule::MAX_INTERVAL).contains(&first_interval) {
        return Err(AppError::BadRequest(format!(
            "First review must be between 1 and {} days",
            schedule::MAX_INTERVAL
        )));
    }
    let link = schedule::normalize_link(&req.link);
    let name = match req.name.as_deref().map(str::trim) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => schedule::extract_name_from_url(&link),
    };
    let difficulty = normalize_difficulty(req.difficulty.as_deref())?;

    let conn = db.lock().unwrap();
    if db::find_active_by_link(&conn, &link)?.is_some() {
        return Err(AppError::Conflict(format!("'{name}' is already scheduled")));
    }
    let id = db::insert_problem(
        &conn,
        &NewProblem {
            name: &name,
            link: &link,
            difficulty: difficulty.as_deref(),
            notes: req.notes.as_deref().unwrap_or("").trim(),
            reps,
            first_interval,
            now: now(),
        },
    )?;
    Ok((StatusCode::CREATED, Json(load_dto(&conn, id)?)))
}

#[derive(Deserialize)]
struct UpdateRequest {
    name: Option<String>,
    /// Empty string clears it.
    difficulty: Option<String>,
    notes: Option<String>,
}

async fn update_problem(
    State(db): State<Db>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateRequest>,
) -> ApiResult<ProblemDto> {
    let conn = db.lock().unwrap();
    let current = db::get_problem(&conn, id)?.ok_or(AppError::NotFound)?;
    let name = match req.name.as_deref().map(str::trim) {
        Some("") => return Err(AppError::BadRequest("Name can't be empty".into())),
        Some(n) => n.to_string(),
        None => current.name,
    };
    let difficulty = match req.difficulty.as_deref() {
        Some(d) => normalize_difficulty(Some(d))?,
        None => current.difficulty,
    };
    let notes = req.notes.unwrap_or(current.notes);
    db::update_details(&conn, id, &name, difficulty.as_deref(), &notes)?;
    Ok(Json(load_dto(&conn, id)?))
}

#[derive(Deserialize)]
struct ReviewRequest {
    result: String,
}

async fn review_problem(
    State(db): State<Db>,
    Path(id): Path<i64>,
    Json(req): Json<ReviewRequest>,
) -> ApiResult<ProblemDto> {
    let outcome = Outcome::parse(&req.result)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown result '{}'", req.result)))?;
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    let problem = db::get_problem(&tx, id)?.ok_or(AppError::NotFound)?;
    if problem.mastered_at.is_some() {
        return Err(AppError::Conflict("This problem is already mastered".into()));
    }
    let now = now();
    let current = Schedule {
        reps_left: problem.reps_left,
        interval: problem.interval,
        base_interval: problem.base_interval,
    };
    match schedule::next(current, outcome) {
        Next::Mastered => db::mark_mastered(&tx, id, now)?,
        Next::Due {
            reps_left,
            interval,
            in_days,
        } => db::update_schedule(&tx, id, now + TimeDelta::days(in_days), interval, reps_left)?,
    }
    db::insert_review(&tx, &problem, outcome.as_str(), now)?;
    tx.commit()?;
    Ok(Json(load_dto(&conn, id)?))
}

async fn undo_review(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<ProblemDto> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    if db::get_problem(&tx, id)?.is_none() {
        return Err(AppError::NotFound);
    }
    if !db::undo_last_review(&tx, id)? {
        return Err(AppError::Conflict("Nothing to undo".into()));
    }
    tx.commit()?;
    Ok(Json(load_dto(&conn, id)?))
}

#[derive(Deserialize)]
struct SnoozeRequest {
    days: Option<i64>,
}

/// Pushes the due date back without affecting the interval or reps.
async fn snooze_problem(
    State(db): State<Db>,
    Path(id): Path<i64>,
    Json(req): Json<SnoozeRequest>,
) -> ApiResult<ProblemDto> {
    let days = req.days.unwrap_or(1);
    if !(1..=schedule::MAX_INTERVAL).contains(&days) {
        return Err(AppError::BadRequest("Invalid snooze length".into()));
    }
    let conn = db.lock().unwrap();
    let p = db::get_problem(&conn, id)?.ok_or(AppError::NotFound)?;
    if p.mastered_at.is_some() {
        return Err(AppError::Conflict("This problem is already mastered".into()));
    }
    let now = now();
    let from = p.due.max(now);
    db::update_schedule(&conn, id, from + TimeDelta::days(days), p.interval, p.reps_left)?;
    Ok(Json(load_dto(&conn, id)?))
}

/// Brings a mastered problem back for one more review, due today.
async fn reactivate_problem(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<ProblemDto> {
    let conn = db.lock().unwrap();
    let p = db::get_problem(&conn, id)?.ok_or(AppError::NotFound)?;
    if p.mastered_at.is_none() {
        return Err(AppError::Conflict("This problem is still being scheduled".into()));
    }
    db::reactivate(&conn, id, now())?;
    Ok(Json(load_dto(&conn, id)?))
}

async fn delete_problem(
    State(db): State<Db>,
    Path(id): Path<i64>,
) -> ApiResult<serde_json::Value> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    if !db::delete_problem(&tx, id)? {
        return Err(AppError::NotFound);
    }
    tx.commit()?;
    Ok(Json(json!({ "message": "Deleted" })))
}

#[derive(Serialize)]
struct Stats {
    streak: i64,
    reviewed_today: i64,
    total_reviews: i64,
}

async fn stats(State(db): State<Db>) -> ApiResult<Stats> {
    let conn = db.lock().unwrap();
    let today = today();
    let days = db::review_days(&conn)?;
    Ok(Json(Stats {
        streak: schedule::streak(&days, today),
        reviewed_today: db::review_count_since(&conn, today.and_time(NaiveTime::MIN))?,
        total_reviews: db::review_count_since(&conn, NaiveDateTime::MIN)?,
    }))
}
