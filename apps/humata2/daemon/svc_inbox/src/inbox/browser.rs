//! BROWSER: approval cards for actions in the user's own browser (read a
//! tab, list tabs, click, type, navigate), for claude, codex, copilot and
//! agy only. Reads may be allowed for 30 minutes per session; writes are
//! asked every time.

use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::helpers as status;
use pb::invalid_field::Problem;
use tonic::{Request, Response, Status};

use super::Inbox;

impl Inbox {
    /// Raises a browser read or write card, or answers a read at once under
    /// an active 30-minute grant of the caller's session.
    pub(super) async fn browser_approval(
        &self,
        _request: Request<pb::RequestBrowserApprovalRequest>,
    ) -> Result<Response<pb::WaitResponse>, Status> {
        Err(Status::unimplemented(
            "asking for browser access isn't built into this humatad2 yet; update Humata to a release that has Settings › Browser",
        ))
    }
}

/// `Some(true)` for a browser read card, `Some(false)` for a browser write
/// card, `None` for any other request.
pub(super) fn is_read(request: &pb::InboxRequest) -> Option<bool> {
    use pb::request_preview::Preview as P;
    match request.preview.as_ref()?.preview.as_ref()? {
        P::BrowserRead(_) => Some(true),
        P::BrowserWrite(_) => Some(false),
        _ => None,
    }
}

/// What answering browser card `id` with `answer` does: Allow once and Deny
/// on every browser card, Allow 30 min only on a read. Always allow, a
/// reply and to-do answers never fit.
pub(super) fn decide(
    id: &str,
    read: bool,
    answer: pb::request_answer::Answer,
) -> Result<(pb::RequestState, pb::request_answer::Answer), Status> {
    use pb::request_answer::Answer as A;
    match answer {
        answer @ A::Deny(_) => Ok((pb::RequestState::Denied, answer)),
        answer @ A::Approve(_) => Ok((pb::RequestState::Approved, answer)),
        answer @ A::AllowFor30Min(_) if read => Ok((pb::RequestState::Approved, answer)),
        A::AllowFor30Min(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!(
                "request {id} asks to click, type or open a page in your browser: that is asked every time and can't be allowed for 30 minutes"
            ),
            "allow it once, or deny it",
        )),
        A::Always(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is a browser request: browser access is never allowed always"),
            if read { "allow it once, allow it for 30 minutes, or deny it" } else { "allow it once, or deny it" },
        )),
        A::Reply(_) | A::TodoAction(_) | A::TodoReview(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is a browser request: it takes allow or deny"),
            if read { "allow it once, allow it for 30 minutes, or deny it" } else { "allow it once, or deny it" },
        )),
    }
}
