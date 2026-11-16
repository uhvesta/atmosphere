//! Finding and reading what sessions said (LD-23–LD-25): `humata sessions
//! search` and `humata session read`.

use humata2_proto::humata::v1 as pb;

use super::pages;
use crate::args::SessionsCommand;
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

pub(super) async fn sessions<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: SessionsCommand,
) -> Result<Done, CliError> {
    match command {
        SessionsCommand::Search {
            words,
            agent,
            since,
            until,
        } => search(ctx, &words, agent, since, until).await,
    }
}

async fn search<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    words: &[String],
    agent: Option<String>,
    since: Option<String>,
    until: Option<String>,
) -> Result<Done, CliError> {
    let query = words
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() {
        return Err(CliError::BadValue {
            field: "search words",
            value: words.join(" "),
            expected: "at least one word to look for",
        });
    }
    let since = since.as_deref().map(|text| time("--since", text)).transpose()?;
    let until = until.as_deref().map(|text| time("--until", text)).transpose()?;
    let channel = ctx.channel_flag().await?;
    let machine = ctx.machine_flag().await?;
    let agent = agent.unwrap_or_default();
    let mut base = pb::SearchSessionsRequest {
        query,
        channel,
        machine,
        agent,
        ..Default::default()
    };
    if let Some((seconds, nanos)) = since {
        let at = base.since.get_or_insert_default();
        (at.seconds, at.nanos) = (seconds, nanos);
    }
    if let Some((seconds, nanos)) = until {
        let at = base.until.get_or_insert_default();
        (at.seconds, at.nanos) = (seconds, nanos);
    }
    pages(ctx, rpc::history::SEARCH_SESSIONS, "hits", |page| {
        pb::SearchSessionsRequest {
            page,
            ..base.clone()
        }
    })
    .await
    .map(Done::reply)
}

pub(super) async fn read<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: &str,
    from: Option<u32>,
    to: Option<u32>,
) -> Result<Done, CliError> {
    let id = ctx.session(session).await?;
    pages(ctx, rpc::history::READ_TURNS, "entries", |page| {
        pb::ReadTurnsRequest {
            session: Some(id.clone()),
            first_turn: from,
            last_turn: to,
            page,
        }
    })
    .await
    .map(Done::reply)
}

/// An RFC 3339 time as seconds and nanos since the Unix epoch.
fn time(flag: &'static str, text: &str) -> Result<(i64, i32), CliError> {
    parse(text).ok_or_else(|| CliError::BadValue {
        field: flag,
        value: text.to_owned(),
        expected: "a time like 2026-11-15T09:30:00Z",
    })
}

fn parse(text: &str) -> Option<(i64, i32)> {
    let (date, rest) = text.split_once(['T', 't', ' '])?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: i64 = date.next()?.parse().ok()?;
    let day: i64 = date.next()?.parse().ok()?;
    if date.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let split = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (clock, zone) = rest.split_at(split);
    let (clock, fraction) = clock.split_once('.').unwrap_or((clock, ""));
    let mut clock = clock.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let nanos = if fraction.is_empty() {
        0
    } else if fraction.len() <= 9 && fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{fraction:0<9}").parse::<i32>().ok()?
    } else {
        return None;
    };
    let offset = match zone {
        "Z" | "z" => 0,
        _ => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let (hours, minutes) = zone.get(1..)?.split_once(':')?;
            let hours: i64 = hours.parse().ok()?;
            let minutes: i64 = minutes.parse().ok()?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            sign * (hours * 3600 + minutes * 60)
        }
    };
    // Days from the civil date (proleptic Gregorian), counted from 1970-01-01.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted = (month + 9) % 12;
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some((days * 86_400 + hour * 3600 + minute * 60 + second - offset, nanos))
}
