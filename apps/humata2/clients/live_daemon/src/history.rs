//! [`History`] for the real daemon, over its HistoryService (LD-23 to
//! LD-25): search across every session's conversation, and read one.

use humata2_clients::history::{History, SearchHit, SearchPage, Turn, TurnRole};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::Reply;
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::humata::v1::history_service_client::HistoryServiceClient;

use crate::connection::{LiveConnection, Wire};

/// Searching and reading conversations on the real daemon. Cheap to clone.
#[derive(Clone)]
pub struct LiveHistory {
    conn: LiveConnection,
}

impl LiveHistory {
    /// History over `conn`'s connection.
    #[must_use]
    pub fn new(conn: impl AsRef<LiveConnection>) -> Self {
        Self {
            conn: conn.as_ref().clone(),
        }
    }
}

fn client(conn: &LiveConnection) -> HistoryServiceClient<Wire> {
    HistoryServiceClient::new(conn.wire())
}

/// A timestamp as RFC 3339 in UTC; empty when there is none.
fn rfc3339(at: Option<&crate::humata2_grpc::Timestamp>) -> String {
    let Some(at) = at else {
        return String::new();
    };
    let days = at.seconds.div_euclid(86_400);
    let secs = at.seconds.rem_euclid(86_400);
    // Days since 1970-01-01 as a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let (hour, minute, second) = (secs / 3_600, secs % 3_600 / 60, secs % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn hit(wire: &pb::SearchHit) -> SearchHit {
    SearchHit {
        session: SessionId::new(wire.session.as_ref().map(|id| id.value.clone()).unwrap_or_default()),
        channel: ChannelId::new(wire.channel.as_ref().map(|id| id.value.clone()).unwrap_or_default()),
        channel_name: wire.channel_name.clone(),
        title: wire.session_title.clone(),
        machine: MachineId::new(wire.machine.as_ref().map(|id| id.value.clone()).unwrap_or_default()),
        machine_name: wire.machine_name.clone(),
        agent: wire.agent.clone(),
        turn: wire.turn,
        seq: wire.seq,
        snippet: wire.snippet.clone(),
        as_of: rfc3339(wire.as_of.as_ref()),
    }
}

fn turn(entry: &pb::MirrorEntry) -> Turn {
    use pb::mirror_entry::Role;
    let role = match Role::try_from(entry.role) {
        Ok(Role::User) => TurnRole::User,
        Ok(Role::Tool) => TurnRole::Tool,
        Ok(Role::Agent | Role::Unspecified) | Err(_) => TurnRole::Agent,
    };
    Turn {
        seq: entry.seq,
        turn: entry.turn,
        role,
        text: entry.text.clone(),
    }
}

impl History for LiveHistory {
    fn search(&self, query: &str, page_token: &str) -> Reply<SearchPage> {
        let conn = self.conn.clone();
        let (query, page_token) = (query.to_owned(), page_token.to_owned());
        Box::pin(async move {
            let answer = client(&conn)
                .search_sessions(pb::SearchSessionsRequest {
                    query,
                    page: Some(pb::PageRequest {
                        page_size: 0,
                        page_token,
                    }),
                    ..pb::SearchSessionsRequest::default()
                })
                .await
                .map_err(|status| conn.error(&status, "This search"))?
                .into_inner();
            let next = answer.page.map(|page| page.next_page_token).unwrap_or_default();
            Ok((answer.hits.iter().map(hit).collect(), next))
        })
    }

    fn read(&self, session: &SessionId, first_turn: u32, last_turn: u32) -> Reply<Vec<Turn>> {
        let conn = self.conn.clone();
        let session = pb::SessionId {
            value: session.as_str().to_owned(),
        };
        Box::pin(async move {
            let mut turns = Vec::new();
            let mut token = String::new();
            loop {
                let answer = client(&conn)
                    .read_turns(pb::ReadTurnsRequest {
                        session: Some(session.clone()),
                        first_turn: (first_turn != 0).then_some(first_turn),
                        last_turn: (last_turn != u32::MAX).then_some(last_turn),
                        page: Some(pb::PageRequest {
                            page_size: 0,
                            page_token: token,
                        }),
                    })
                    .await
                    .map_err(|status| conn.error(&status, "This conversation"))?
                    .into_inner();
                turns.extend(answer.entries.iter().map(turn));
                token = answer.page.map(|page| page.next_page_token).unwrap_or_default();
                if token.is_empty() {
                    return Ok(turns);
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> crate::humata2_grpc::Timestamp {
        crate::humata2_grpc::Timestamp { seconds, nanos: 0 }
    }

    #[test]
    fn timestamps_read_as_rfc_3339_in_utc() {
        assert_eq!(rfc3339(Some(&at(0))), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(Some(&at(951_782_400 + 3_661))), "2000-02-29T01:01:01Z");
        assert_eq!(rfc3339(Some(&at(1_794_000_000))), "2026-11-06T21:20:00Z");
        assert_eq!(rfc3339(None), "");
    }

    #[test]
    fn a_hit_takes_the_title_from_the_session_title() {
        let wire = pb::SearchHit {
            session_title: "Fix".to_owned(),
            turn: 2,
            ..pb::SearchHit::default()
        };
        let mapped = hit(&wire);
        assert_eq!((mapped.title.as_str(), mapped.turn), ("Fix", 2));
    }
}
