# question-scheduler

LeetCode spaced-repetition scheduler. The backend is written in Rust (axum + SQLite).

## Running

```sh
cargo run --release
```

Then open http://127.0.0.1:5000. Run it from the repository root so it can find `templates/` and `static/`.

Environment variables:

- `PORT`: port to listen on (default `5000`)
- `DATABASE_PATH`: SQLite file (default `instance/database.db`, created if missing)

## How scheduling works

When you add a problem you choose when the first review happens (e.g. 3 days) and how
many successful reviews it takes to master it. Each review is graded:

- **Solved**: counts a review and doubles the gap (3 → 6 → 12 days…, capped at a year).
  On the last review the problem moves to *Mastered*.
- **Hard** (needed hints or struggled): repeats the same gap; no review is counted.
- **Forgot**: due again tomorrow, then the schedule restarts from the first-review gap.

Every review is logged, so the most recent one can be undone.

## API

| Method | Path                            | Body                                                                            |
|--------|---------------------------------|---------------------------------------------------------------------------------|
| GET    | `/api/problems`                 |                                                                                 |
| POST   | `/api/problems`                 | `{"link", "name"?, "difficulty"?, "notes"?, "reps"?, "first_interval"?}`        |
| PATCH  | `/api/problems/{id}`            | `{"name"?, "difficulty"?, "notes"?}`                                            |
| DELETE | `/api/problems/{id}`            |                                                                                 |
| POST   | `/api/problems/{id}/review`     | `{"result": "solved" \| "hard" \| "forgot"}`                                  |
| POST   | `/api/problems/{id}/undo`       | Reverts the latest review                                                       |
| POST   | `/api/problems/{id}/snooze`     | `{"days"?: 1}`                                                                  |
| POST   | `/api/problems/{id}/reactivate` | Brings a mastered problem back for one more review                              |
| GET    | `/api/stats`                    | Review counts                                                                   |
