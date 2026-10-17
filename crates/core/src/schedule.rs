//! Turning a program template into dated calendar entries (WB-4).

use chrono::{Duration, NaiveDate};

use crate::model::Program;

/// Calendar date for program week/day (both 1-based) given a start date.
pub fn date_for(start: NaiveDate, week: u32, day: u32) -> NaiveDate {
    start + Duration::days(((week - 1) * 7 + (day - 1)) as i64)
}

/// Last calendar day the program covers.
pub fn end_date(start: NaiveDate, weeks: u32) -> NaiveDate {
    start + Duration::days((weeks * 7) as i64 - 1)
}

/// Every workout in the program with the date it falls on, in date order.
pub fn materialize(program: &Program, start: NaiveDate) -> Vec<(NaiveDate, usize)> {
    let mut out: Vec<(NaiveDate, usize)> = program
        .workouts
        .iter()
        .enumerate()
        .map(|(i, pw)| (date_for(start, pw.week, pw.day), i))
        .collect();
    out.sort_by_key(|(d, i)| (*d, *i));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        assert_eq!(date_for(start, 1, 1), start);
        assert_eq!(date_for(start, 1, 3), NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        assert_eq!(date_for(start, 2, 1), NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
        assert_eq!(end_date(start, 4), NaiveDate::from_ymd_opt(2026, 11, 15).unwrap());
    }
}
