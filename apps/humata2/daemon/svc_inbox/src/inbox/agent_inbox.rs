//! What belongs in the Agent inbox (D846): every card but a to-do ask. A
//! to-do ask (a board item's change or a new item an agent wants) leaves its
//! agent working and shows on its item in To-do instead.

use crate::pb;

/// Whether `request` is listed and counted in the Agent inbox.
pub(super) fn in_agent_inbox(request: &pb::InboxRequest) -> bool {
    use pb::request_preview::Preview;
    !matches!(
        request.preview.as_ref().and_then(|preview| preview.preview.as_ref()),
        Some(Preview::BoardChange(_) | Preview::NewBoardItem(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pb::request_preview::Preview;

    fn with(preview: Option<Preview>) -> pb::InboxRequest {
        pb::InboxRequest {
            preview: Some(pb::RequestPreview { preview }),
            ..Default::default()
        }
    }

    #[test]
    fn a_to_do_change_or_new_item_waits_in_to_do_not_the_agent_inbox() {
        assert!(!in_agent_inbox(&with(Some(Preview::BoardChange(Default::default())))));
        assert!(!in_agent_inbox(&with(Some(Preview::NewBoardItem(Default::default())))));
    }

    #[test]
    fn every_other_ask_is_in_the_agent_inbox() {
        assert!(in_agent_inbox(&with(Some(Preview::CrossMachineCopy(Default::default())))));
        assert!(in_agent_inbox(&with(Some(Preview::Move(Default::default())))));
        assert!(in_agent_inbox(&with(Some(Preview::Question(Default::default())))));
        assert!(in_agent_inbox(&with(None)));
        assert!(in_agent_inbox(&pb::InboxRequest::default()));
    }
}
