//! `humata desk keep [<session>]` (DK-2, D182.6): keeps a Desk session's
//! scratch folder past its close. With no session the daemon takes the
//! caller's own, which works only from inside a Desk session.

use humata2_proto::humata::v1 as pb;

use crate::args::DeskCommand;
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

pub(super) async fn desk<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: DeskCommand,
) -> Result<Done, CliError> {
    match command {
        DeskCommand::Keep { session } => {
            let session = match session {
                Some(name) => Some(ctx.session(&name).await?),
                None => None,
            };
            let request = pb::KeepDeskFolderRequest { session };
            ctx.change(rpc::desk::KEEP_DESK_FOLDER, &request).await
        }
    }
}
