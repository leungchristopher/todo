use chrono::{Datelike, Duration, Local, NaiveDate};

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Parses due-date input.
///
/// `Ok(None)` means the user cleared the date; `Err(())` means unparseable.
pub fn parse_due(input: &str) -> Result<Option<NaiveDate>, ()> {
    let s = input.trim().to_lowercase();
    if s.is_empty() || s == "-" || s == "none" || s == "clear" {
        return Ok(None);
    }
    if s == "today" {
        return Ok(Some(today()));
    }
    if s == "tomorrow" || s == "tmr" {
        return Ok(Some(today() + Duration::days(1)));
    }
    if let Some(d) = parse_offset(&s) {
        return Ok(Some(d));
    }
    if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return Ok(Some(d));
    }
    if let Some(d) = parse_month_day(&s) {
        return Ok(Some(d));
    }
    Err(())
}

/// Bare `12-25`. Resolves into the coming year, so a month-day that has
/// already passed means next year rather than a date in the past.
fn parse_month_day(s: &str) -> Option<NaiveDate> {
    let (m, d) = s.split_once('-')?;
    let (m, d) = (m.parse().ok()?, d.parse().ok()?);
    let today = today();
    let this_year = NaiveDate::from_ymd_opt(today.year(), m, d)?;
    if this_year < today {
        NaiveDate::from_ymd_opt(today.year() + 1, m, d)
    } else {
        Some(this_year)
    }
}

/// `+3d`, `3d`, `+2w`, `1m` — offsets from today.
fn parse_offset(s: &str) -> Option<NaiveDate> {
    let s = s.strip_prefix('+').unwrap_or(s);
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    let days = match unit {
        "d" => n,
        "w" => n * 7,
        "m" => n * 30,
        _ => return None,
    };
    Some(today() + Duration::days(days))
}

/// Short human label for a due date, relative where that reads better.
pub fn label(due: NaiveDate) -> String {
    let delta = (due - today()).num_days();
    match delta {
        0 => "today".into(),
        1 => "tomorrow".into(),
        -1 => "yesterday".into(),
        d if d < 0 => format!("{}d late", -d),
        d if d < 7 => format!("in {}d", d),
        _ => due.format("%Y-%m-%d").to_string(),
    }
}

/// Short label for a date in the past, for history entries. The mirror of
/// `label`, which words itself for dates that are still ahead.
pub fn ago(d: NaiveDate) -> String {
    match (today() - d).num_days() {
        0 => "today".into(),
        1 => "yesterday".into(),
        n if (2..7).contains(&n) => format!("{n}d ago"),
        _ => d.format("%Y-%m-%d").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_past_dates_for_history() {
        assert_eq!(ago(today()), "today");
        assert_eq!(ago(today() - Duration::days(1)), "yesterday");
        assert_eq!(ago(today() - Duration::days(3)), "3d ago");
        let old = today() - Duration::days(30);
        assert_eq!(ago(old), old.to_string());
    }

    #[test]
    fn clears_on_empty_and_dashes() {
        for s in ["", "  ", "-", "none", "clear"] {
            assert_eq!(parse_due(s), Ok(None));
        }
    }

    #[test]
    fn parses_keywords_and_offsets() {
        assert_eq!(parse_due("today"), Ok(Some(today())));
        assert_eq!(parse_due("TOMORROW"), Ok(Some(today() + Duration::days(1))));
        assert_eq!(parse_due("+3d"), Ok(Some(today() + Duration::days(3))));
        assert_eq!(parse_due("2w"), Ok(Some(today() + Duration::days(14))));
    }

    #[test]
    fn parses_iso_dates() {
        assert_eq!(
            parse_due("2026-12-25"),
            Ok(NaiveDate::from_ymd_opt(2026, 12, 25))
        );
    }

    #[test]
    fn month_day_rolls_into_next_year_once_past() {
        let past = today() - Duration::days(30);
        let s = past.format("%m-%d").to_string();
        let got = parse_due(&s).unwrap().unwrap();
        assert!(
            got >= today(),
            "{s} resolved to {got}, which is in the past"
        );
    }

    #[test]
    fn rejects_nonsense() {
        for s in ["next thursday", "2026-13-45", "soon", "3x"] {
            assert_eq!(parse_due(s), Err(()), "{s} should not parse");
        }
    }

    #[test]
    fn labels_relative_dates() {
        assert_eq!(label(today()), "today");
        assert_eq!(label(today() + Duration::days(1)), "tomorrow");
        assert_eq!(label(today() - Duration::days(1)), "yesterday");
        assert_eq!(label(today() - Duration::days(5)), "5d late");
        assert_eq!(label(today() + Duration::days(3)), "in 3d");
        assert_eq!(
            label(today() + Duration::days(90)),
            (today() + Duration::days(90)).to_string()
        );
    }
}
