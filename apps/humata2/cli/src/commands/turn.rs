//! `humata turn working|idle|needs-input` (SE-21, D169): an agent's hook
//! says what turn it is in. Prints nothing and always exits 0, so a hook
//! adds nothing to the agent's context and can't hold up or fail it; the
//! daemon's next report, or its output fallback, sets right a lost one.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_proto::humata::v1 as pb;

use crate::args::TurnArg;
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

/// The longest a hook waits for the daemon.
const WAIT: Duration = Duration::from_secs(2);

pub(super) async fn turn<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    state: TurnArg,
    started: SystemTime,
) -> Result<Done, CliError> {
    let state = match state {
        TurnArg::Working => pb::TurnState::Working,
        TurnArg::Idle => pb::TurnState::Idle,
        TurnArg::NeedsInput => pb::TurnState::NeedsInput,
    };
    // Async hooks can overtake each other; the daemon keeps the newest
    // report by the time this process started.
    let reported_at = started
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_nanos()).ok())
        .unwrap_or(0);
    let request = pb::ReportTurnRequest {
        state: state as i32,
        reported_at_unix_ns: reported_at,
    };
    let _ = tokio::time::timeout(WAIT, ctx.raw(rpc::session::REPORT_TURN, &request)).await;
    Ok(Done::printed())
}
