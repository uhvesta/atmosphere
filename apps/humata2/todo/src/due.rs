use crate::day::Day;

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
