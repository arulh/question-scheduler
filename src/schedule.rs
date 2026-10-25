//! Pure scheduling rules, independent of the database and HTTP layers.

use chrono::{NaiveDate, TimeDelta};

/// Interval value stored while a forgotten problem is being relearned.
/// The next review is tomorrow, after which the schedule restarts from the
/// problem's base (first-review) interval.
pub const RELEARNING: i64 = 0;
pub const MAX_INTERVAL: i64 = 365;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Couldn't solve it: review again tomorrow and restart the schedule.
    Forgot,
    /// Solved, but with hints or a struggle: repeat the same gap, no rep counted.
    Hard,
    /// Solved cleanly: one rep done, gap doubles.
    Solved,
}

impl Outcome {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "forgot" | "fail" => Some(Self::Forgot),
            "hard" => Some(Self::Hard),
            "solved" | "success" => Some(Self::Solved),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forgot => "forgot",
            Self::Hard => "hard",
            Self::Solved => "solved",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub reps_left: i64,
    pub interval: i64,
    pub base_interval: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Mastered,
    Due {
        reps_left: i64,
        interval: i64,
        in_days: i64,
    },
}

pub fn next(s: Schedule, outcome: Outcome) -> Next {
    let relearning = s.interval == RELEARNING;
    match outcome {
        Outcome::Forgot => Next::Due {
            reps_left: s.reps_left,
            interval: RELEARNING,
            in_days: 1,
        },
        Outcome::Hard => {
            let interval = if relearning {
                (s.base_interval / 2).max(1)
            } else {
                s.interval
            };
            Next::Due {
                reps_left: s.reps_left,
                interval,
                in_days: interval,
            }
        }
        Outcome::Solved => {
            let reps_left = s.reps_left - 1;
            if reps_left <= 0 {
                return Next::Mastered;
            }
            let interval = if relearning {
                s.base_interval
            } else {
                s.interval * 2
            }
            .clamp(1, MAX_INTERVAL);
            Next::Due {
                reps_left,
                interval,
                in_days: interval,
            }
        }
    }
}

/// Consecutive days with at least one review, ending today or yesterday
/// (a streak isn't broken until a full day passes without reviewing).
/// `days` must be sorted newest first, without duplicates.
pub fn streak(days: &[NaiveDate], today: NaiveDate) -> i64 {
    let Some(&first) = days.first() else {
        return 0;
    };
    if first != today && first != today - TimeDelta::days(1) {
        return 0;
    }
    let mut count = 1;
    for pair in days.windows(2) {
        if pair[0] - pair[1] != TimeDelta::days(1) {
            break;
        }
        count += 1;
    }
    count
}

/// Adds a scheme if missing and canonicalises LeetCode problem URLs to
/// 'https://leetcode.com/problems/<slug>/' so duplicates can be detected.
pub fn normalize_link(input: &str) -> String {
    let trimmed = input.trim();
    let link = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let parts = url_parts(&link);
    let is_leetcode = matches!(parts.get(2), Some(&("leetcode.com" | "www.leetcode.com")));
    match problem_slug(&parts) {
        Some(slug) if is_leetcode => format!("https://leetcode.com/problems/{slug}/"),
        _ => link,
    }
}

/// Parses 'https://leetcode.com/problems/two-sum/...' into 'Two Sum'
pub fn extract_name_from_url(url: &str) -> String {
    problem_slug(&url_parts(url))
        .map(|slug| title_case(&slug.replace('-', " ")))
        .unwrap_or_else(|| "Unknown Problem".to_string())
}

fn url_parts(url: &str) -> Vec<&str> {
    let without_query = url.split(['?', '#']).next().unwrap_or_default();
    without_query.trim_end_matches('/').split('/').collect()
}

/// The segment after 'problems', if any.
fn problem_slug<'a>(parts: &[&'a str]) -> Option<&'a str> {
    let index = parts.iter().position(|&p| p == "problems")?;
    parts.get(index + 1).copied().filter(|s| !s.is_empty())
}

/// Like Python's str.title() ('3sum' -> '3Sum'), but keeps roman numerals
/// upper-case ('coin change ii' -> 'Coin Change II').
fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|word| {
            let lower = word.to_lowercase();
            if lower.len() >= 2 && lower.chars().all(|c| matches!(c, 'i' | 'v' | 'x')) {
                return lower.to_uppercase();
            }
            let mut out = String::with_capacity(word.len());
            let mut prev_alpha = false;
            for c in lower.chars() {
                if prev_alpha {
                    out.push(c);
                } else {
                    out.extend(c.to_uppercase());
                }
                prev_alpha = c.is_alphabetic();
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sched(reps_left: i64, interval: i64) -> Schedule {
        Schedule {
            reps_left,
            interval,
            base_interval: 7,
        }
    }

    #[test]
    fn solved_doubles_and_counts_rep() {
        assert_eq!(
            next(sched(3, 7), Outcome::Solved),
            Next::Due {
                reps_left: 2,
                interval: 14,
                in_days: 14
            }
        );
        assert_eq!(next(sched(1, 7), Outcome::Solved), Next::Mastered);
    }

    #[test]
    fn solved_interval_is_capped() {
        assert_eq!(
            next(sched(3, 300), Outcome::Solved),
            Next::Due {
                reps_left: 2,
                interval: MAX_INTERVAL,
                in_days: MAX_INTERVAL
            }
        );
    }

    #[test]
    fn hard_repeats_gap_without_counting() {
        assert_eq!(
            next(sched(3, 28), Outcome::Hard),
            Next::Due {
                reps_left: 3,
                interval: 28,
                in_days: 28
            }
        );
    }

    #[test]
    fn forgot_relearns_then_restarts_from_base() {
        let Next::Due {
            reps_left,
            interval,
            in_days,
        } = next(sched(2, 56), Outcome::Forgot)
        else {
            panic!("expected due");
        };
        assert_eq!((reps_left, interval, in_days), (2, RELEARNING, 1));
        assert_eq!(
            next(sched(2, interval), Outcome::Solved),
            Next::Due {
                reps_left: 1,
                interval: 7,
                in_days: 7
            }
        );
        assert_eq!(
            next(sched(2, interval), Outcome::Hard),
            Next::Due {
                reps_left: 2,
                interval: 3,
                in_days: 3
            }
        );
    }

    #[test]
    fn streak_counts_consecutive_days() {
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let today = d("2026-10-24");
        assert_eq!(streak(&[], today), 0);
        assert_eq!(
            streak(&[d("2026-10-24"), d("2026-10-23"), d("2026-10-21")], today),
            2
        );
        assert_eq!(streak(&[d("2026-10-23"), d("2026-10-22")], today), 2);
        assert_eq!(streak(&[d("2026-10-22")], today), 0);
    }

    #[test]
    fn normalizes_links() {
        assert_eq!(
            normalize_link("leetcode.com/problems/two-sum/description/?envType=x"),
            "https://leetcode.com/problems/two-sum/"
        );
        assert_eq!(
            normalize_link(" https://neetcode.io/problems/two-sum "),
            "https://neetcode.io/problems/two-sum"
        );
    }

    #[test]
    fn extracts_name() {
        assert_eq!(
            extract_name_from_url("https://leetcode.com/problems/two-sum/"),
            "Two Sum"
        );
        assert_eq!(
            extract_name_from_url("https://leetcode.com/problems/3sum/description/"),
            "3Sum"
        );
        assert_eq!(
            extract_name_from_url("https://leetcode.com/problems/coin-change-ii?x=1"),
            "Coin Change II"
        );
        assert_eq!(
            extract_name_from_url("https://leetcode.com/problems"),
            "Unknown Problem"
        );
        assert_eq!(extract_name_from_url("garbage"), "Unknown Problem");
    }
}
