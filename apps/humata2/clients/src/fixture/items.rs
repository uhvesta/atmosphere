use crate::model::{ChannelId, Item, ItemId, ItemKind, ItemStatus, Scope, SessionId};

pub(super) fn items() -> Vec<Item> {
    let item = |id: &str, scope: Scope, kind, title: &str, status, depends: &[&str]| Item {
        id: ItemId::new(id),
        scope,
        kind,
        title: title.to_owned(),
        status,
        depends_on: depends.iter().map(|id| ItemId::new(*id)).collect(),
        session: None,
        parent: None,
    };
    let web = || Scope::Channel(ChannelId::new("web"));
    let api = || Scope::Channel(ChannelId::new("api"));
    let mut items = vec![
        item(
            "W-1",
            web(),
            ItemKind::Milestone,
            "Login v2",
            ItemStatus::InProgress,
            &[],
        ),
        item(
            "W-2",
            web(),
            ItemKind::Task,
            "Passkey prompt",
            ItemStatus::InProgress,
            &[],
        ),
        item(
            "W-3",
            web(),
            ItemKind::Task,
            "Email code fallback",
            ItemStatus::Blocked,
            &["W-2"],
        ),
        item(
            "W-4",
            web(),
            ItemKind::Task,
            "Dark mode tokens",
            ItemStatus::Done,
            &[],
        ),
        item(
            "W-5",
            web(),
            ItemKind::Step,
            "Copy review",
            ItemStatus::Todo,
            &["W-2"],
        ),
        item(
            "A-1",
            api(),
            ItemKind::Task,
            "Token refresh retry",
            ItemStatus::InProgress,
            &[],
        ),
        item(
            "A-2",
            api(),
            ItemKind::Task,
            "Rate limit headers",
            ItemStatus::Todo,
            &["A-1"],
        ),
        item(
            "A-3",
            api(),
            ItemKind::Task,
            "v1 tables cleanup",
            ItemStatus::WontDo,
            &[],
        ),
        item(
            "G-1",
            Scope::Global,
            ItemKind::Milestone,
            "Ship login v2",
            ItemStatus::Todo,
            &["W-1", "A-1"],
        ),
    ];
    let link = |items: &mut Vec<Item>, id: &str, session: &str, parent: Option<&str>| {
        if let Some(item) = items.iter_mut().find(|item| item.id.as_str() == id) {
            item.session = Some(SessionId::new(session));
            item.parent = parent.map(ItemId::new);
        }
    };
    link(&mut items, "W-2", "s-passkey", Some("W-1"));
    link(&mut items, "W-3", "s-email", Some("W-1"));
    link(&mut items, "W-4", "s-theme", None);
    link(&mut items, "W-5", "s-copy", Some("W-2"));
    link(&mut items, "A-1", "s-auth", None);
    link(&mut items, "A-2", "s-rate", None);
    items
}
