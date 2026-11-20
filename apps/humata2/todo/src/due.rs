use crate::day::Day;
use crate::model::{FuzzyDue, Todo};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Overdue,
    Soon,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DueChip {
    pub text: String,
    pub tone: Tone,
}

pub fn chip(due: Day, today: Day) -> DueChip {
    let d = today.days_until(due);
    let (text, tone) = match d {
        i32::MIN..=-1 => {
            let n = d.unsigned_abs();
            let unit = if n == 1 { "day" } else { "days" };
            (format!("Overdue · {n} {unit}"), Tone::Overdue)
        }
        0 => ("Due today".to_owned(), Tone::Soon),
        1 => ("Due tomorrow".to_owned(), Tone::Soon),
        _ => (due.short(), Tone::Plain),
    };
    DueChip { text, tone }
}

/// Loose/fuzzy due chip: always plain tone, never red/overdue.
pub fn fuzzy_chip(fuzzy: FuzzyDue) -> DueChip {
    DueChip {
        text: fuzzy.label().to_owned(),
        tone: Tone::Plain,
    }
}

/// Due chip for an item, preferring an exact deadline over a fuzzy due label.
pub fn chip_for(todo: &Todo, today: Day) -> Option<DueChip> {
    if let Some(due) = todo.due {
        Some(chip(due, today))
    } else {
        todo.fuzzy_due.map(fuzzy_chip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_due_chip_is_never_overdue() {
        let today = Day::from_ymd(2026, 11, 19).unwrap();
        let mut t = Todo::new("T-1", "Check logs", today.add_days(-30));
        t.fuzzy_due = Some(FuzzyDue::InAFewDays);
        assert_eq!(
            chip_for(&t, today),
            Some(DueChip {
                text: "In a few days".to_owned(),
                tone: Tone::Plain,
            })
        );
        t.fuzzy_due = Some(FuzzyDue::NextWeek);
        assert_eq!(
            chip_for(&t, today),
            Some(DueChip {
                text: "Next week".to_owned(),
                tone: Tone::Plain,
            })
        );
        t.fuzzy_due = Some(FuzzyDue::NextMonth);
        assert_eq!(
            chip_for(&t, today),
            Some(DueChip {
                text: "Next month".to_owned(),
                tone: Tone::Plain,
            })
        );
    }
}
