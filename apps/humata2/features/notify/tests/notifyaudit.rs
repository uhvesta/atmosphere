use humata2_clients::model::{ChannelId, ItemId, MachineId, RequestId, RequestKind, SessionId};
use humata2_clients::{NoticeSound, NotifyTarget};
use humata2_notify::{
    BurstState, DECK_TOAST_DURATION_SECS, DeckToast, DeckToastKind, DeckToastQueue,
    FrontmostContext, GateOutcome, NotifyConfig, NotifyEvent, PermissionStatus, build_test_notice,
    dock_badge_count, evaluate_decision, trace_gate,
};

fn full_config() -> NotifyConfig {
    NotifyConfig {
        enabled: true,
        play_sound: true,
        ..NotifyConfig::default()
    }
}

fn background() -> FrontmostContext {
    FrontmostContext {
        app_active: false,
        ..FrontmostContext::default()
    }
}

fn frontmost_other_page() -> FrontmostContext {
    FrontmostContext {
        app_active: true,
        viewing_home: true,
        ..FrontmostContext::default()
    }
}

fn inbox_request(
    req: &str,
    session: &str,
    channel: Option<(&str, &str)>,
    kind: RequestKind,
) -> NotifyEvent {
    NotifyEvent::InboxRequest {
        request_id: RequestId::new(req),
        session_id: SessionId::new(session),
        session_title: format!("Session {session}"),
        channel_id: channel.map(|(id, _)| ChannelId::new(id)),
        channel_name: channel.map(|(_, name)| name.to_owned()),
        kind,
        operation: "tool.exec".to_owned(),
        answered: false,
        title: format!("claude needs you: {req}"),
        body: "Open the Agent inbox to answer.".to_owned(),
    }
}

#[test]
fn all_request_sources_post_when_background_and_authorized() {
    let config = full_config();
    let sources: Vec<(&str, NotifyEvent, NoticeSound)> = vec![
        (
            "Command",
            inbox_request("r-cmd", "s-cmd", Some(("c1", "api")), RequestKind::Command),
            NoticeSound::Inbox,
        ),
        (
            "FileEdit",
            inbox_request("r-edit", "s-edit", Some(("c2", "web")), RequestKind::FileEdit),
            NoticeSound::Inbox,
        ),
        (
            "Network",
            inbox_request("r-net", "s-net", Some(("c3", "net")), RequestKind::Network),
            NoticeSound::Inbox,
        ),
        (
            "Merge",
            inbox_request("r-merge", "s-merge", Some(("c4", "git")), RequestKind::Merge),
            NoticeSound::Inbox,
        ),
        (
            "Destroy",
            inbox_request("r-destroy", "s-destroy", Some(("c5", "ops")), RequestKind::Destroy),
            NoticeSound::Inbox,
        ),
        (
            "Question/Desk",
            inbox_request("r-q", "s-q", None, RequestKind::Question),
            NoticeSound::Inbox,
        ),
        (
            "TerminalNeedsYou",
            NotifyEvent::TerminalNeedsYou {
                session_id: SessionId::new("s-term"),
                session_title: "Terminal".to_owned(),
                channel_id: None,
                channel_name: None,
                agent_name: "Claude".to_owned(),
                has_pending_inbox_request: false,
            },
            NoticeSound::Inbox,
        ),
        (
            "ResumeFailed",
            NotifyEvent::ResumeFailed {
                session_id: SessionId::new("s-res"),
                session_title: "Resume".to_owned(),
                channel_id: None,
                channel_name: None,
                reason: "Missing directory".to_owned(),
            },
            NoticeSound::Inbox,
        ),
        (
            "AbnormalExit",
            NotifyEvent::AbnormalExit {
                session_id: SessionId::new("s-exit"),
                session_title: "Exit".to_owned(),
                channel_id: None,
                channel_name: None,
                code: 137,
                already_resume_failed: false,
            },
            NoticeSound::Inbox,
        ),
        (
            "RestartFailed",
            NotifyEvent::RestartFailed {
                machine_id: MachineId::new("m-restart"),
                machine_name: "devbox".to_owned(),
                sessions: vec![SessionId::new("s-r1"), SessionId::new("s-r2")],
                not_brought_back: 2,
            },
            NoticeSound::Inbox,
        ),
        (
            "TodoAsk",
            NotifyEvent::TodoAsk {
                request_id: RequestId::new("r-todo"),
                session_id: SessionId::new("s-todo"),
                channel_id: None,
                item: Some(ItemId::new("i-1")),
                answered: false,
                title: "Add todo?".to_owned(),
                body: "Open To-do.".to_owned(),
            },
            NoticeSound::Todo,
        ),
    ];

    for (idx, (label, event, expected_sound)) in sources.into_iter().enumerate() {
        let mut burst = BurstState::default();
        let now = (idx as u64 + 1) * 100;
        let decision = evaluate_decision(
            &config,
            &background(),
            PermissionStatus::Authorized,
            &mut burst,
            event,
            now,
        );
        assert_eq!(decision.trace.outcome, GateOutcome::Posted, "{label} should post");
        assert!(decision.os_notice.is_some(), "{label} should produce os_notice");
        assert!(decision.deck_toast.is_none(), "{label} should not produce deck_toast");
        assert_eq!(decision.in_app_sound, Some(expected_sound), "{label} sound");
        assert_eq!(
            decision.os_notice.as_ref().and_then(|n| n.sound),
            Some(expected_sound),
            "{label} os_notice sound"
        );
    }
}

#[test]
fn frontmost_on_another_page_routes_to_deck_toast_and_in_app_sound() {
    let config = full_config();
    let mut burst = BurstState::default();
    let decision = evaluate_decision(
        &config,
        &frontmost_other_page(),
        PermissionStatus::Authorized,
        &mut burst,
        inbox_request("r1", "s1", Some(("c1", "api")), RequestKind::Command),
        100,
    );

    assert_eq!(decision.trace.outcome, GateOutcome::FrontmostDeckToast);
    assert!(decision.os_notice.is_none());
    let toast = decision.deck_toast.expect("deck toast expected");
    assert_eq!(toast.kind, DeckToastKind::Notice);
    assert_eq!(toast.duration_secs, DECK_TOAST_DURATION_SECS);
    assert!(toast.pause_on_hover);
    assert_eq!(decision.in_app_sound, Some(NoticeSound::Inbox));
}

#[test]
fn exact_request_open_suppresses_os_notice_toast_and_chime() {
    let config = full_config();
    let mut burst = BurstState::default();
    let viewing_exact = FrontmostContext {
        app_active: true,
        viewing_inbox: true,
        viewing_request: Some(RequestId::new("r1")),
        ..FrontmostContext::default()
    };

    let decision = evaluate_decision(
        &config,
        &viewing_exact,
        PermissionStatus::Authorized,
        &mut burst,
        inbox_request("r1", "s1", Some(("c1", "api")), RequestKind::Command),
        100,
    );

    assert_eq!(decision.trace.outcome, GateOutcome::SuppressedExactItem);
    assert!(decision.os_notice.is_none());
    assert!(decision.deck_toast.is_none());
    assert!(decision.in_app_sound.is_none());
    assert_eq!(decision.trace.sound_suppressed, Some("exact_item"));
}

#[test]
fn viewing_different_request_in_inbox_is_not_suppressed() {
    let config = full_config();
    let mut burst = BurstState::default();
    let viewing_other = FrontmostContext {
        app_active: true,
        viewing_inbox: true,
        viewing_request: Some(RequestId::new("r-other")),
        ..FrontmostContext::default()
    };

    let decision = evaluate_decision(
        &config,
        &viewing_other,
        PermissionStatus::Authorized,
        &mut burst,
        inbox_request("r1", "s1", Some(("c1", "api")), RequestKind::Command),
        100,
    );

    assert_eq!(decision.trace.outcome, GateOutcome::FrontmostDeckToast);
    assert!(decision.os_notice.is_none());
    assert!(decision.deck_toast.is_some());
    assert_eq!(decision.in_app_sound, Some(NoticeSound::Inbox));
}

#[test]
fn permission_denied_and_unbundled_fall_back_to_permission_warning_toast() {
    for status in [PermissionStatus::Denied, PermissionStatus::Unbundled] {
        assert!(!status.allows_os_delivery());
        assert!(status.needs_warning());
        assert!(!status.label().is_empty());
        assert!(!status.detail().is_empty());

        let config = full_config();
        let mut burst = BurstState::default();
        let decision = evaluate_decision(
            &config,
            &background(),
            status,
            &mut burst,
            inbox_request("r1", "s1", None, RequestKind::Command),
            100,
        );

        assert_eq!(decision.trace.outcome, GateOutcome::PermissionFallbackToast);
        assert!(decision.os_notice.is_none());
        let toast = decision.deck_toast.expect("permission fallback toast expected");
        assert_eq!(toast.kind, DeckToastKind::PermissionWarning);
        assert_eq!(toast.duration_secs, DECK_TOAST_DURATION_SECS);
        assert_eq!(decision.in_app_sound, Some(NoticeSound::Inbox));
    }

    assert!(PermissionStatus::Authorized.allows_os_delivery());
    assert!(PermissionStatus::Provisional.allows_os_delivery());
    assert!(PermissionStatus::Ephemeral.allows_os_delivery());
    assert!(!PermissionStatus::Unknown.allows_os_delivery());
    assert!(!PermissionStatus::NotDetermined.allows_os_delivery());
    assert!(!PermissionStatus::Authorized.needs_warning());
}

#[test]
fn decoupled_sound_fires_with_enabled_false_when_frontmost_toast_posts_and_respects_cooldown() {
    let config = NotifyConfig {
        enabled: false,
        in_app_toasts: true,
        play_sound: true,
        cooldown_secs: 30,
        ..NotifyConfig::default()
    };
    let mut burst = BurstState::default();

    let first = evaluate_decision(
        &config,
        &frontmost_other_page(),
        PermissionStatus::Authorized,
        &mut burst,
        inbox_request("r1", "s1", None, RequestKind::Command),
        100,
    );
    assert_eq!(first.trace.outcome, GateOutcome::FrontmostDeckToast);
    assert!(first.os_notice.is_none());
    assert!(first.deck_toast.is_some());
    assert_eq!(first.in_app_sound, Some(NoticeSound::Inbox));

    // Within cooldown: toast still posts, sound is suppressed.
    let second = evaluate_decision(
        &config,
        &frontmost_other_page(),
        PermissionStatus::Authorized,
        &mut burst,
        inbox_request("r2", "s2", None, RequestKind::Command),
        110,
    );
    assert_eq!(second.trace.outcome, GateOutcome::FrontmostDeckToast);
    assert!(second.deck_toast.is_some());
    assert_eq!(second.in_app_sound, None);
    assert_eq!(second.trace.sound_suppressed, Some("sound_cooldown"));

    // After cooldown: sound fires again.
    let third = evaluate_decision(
        &config,
        &frontmost_other_page(),
        PermissionStatus::Authorized,
        &mut burst,
        inbox_request("r3", "s3", None, RequestKind::Command),
        140,
    );
    assert_eq!(third.trace.outcome, GateOutcome::FrontmostDeckToast);
    assert!(third.deck_toast.is_some());
    assert_eq!(third.in_app_sound, Some(NoticeSound::Inbox));
}

#[test]
fn deck_toast_queue_replacement_expiry_hover_pause_and_clear_id() {
    assert!(!DeckToastQueue::can_evict_undo());
    let mut queue = DeckToastQueue::default();
    assert!(queue.current().is_none());

    let t1 = DeckToast {
        id: "humata.session.s1".to_owned(),
        title: "First".to_owned(),
        body: "Body 1".to_owned(),
        target: NotifyTarget::Inbox,
        duration_secs: DECK_TOAST_DURATION_SECS,
        pause_on_hover: true,
        kind: DeckToastKind::Notice,
    };
    let t2 = DeckToast {
        id: "humata.session.s2".to_owned(),
        title: "Second".to_owned(),
        body: "Body 2".to_owned(),
        target: NotifyTarget::Inbox,
        duration_secs: DECK_TOAST_DURATION_SECS,
        pause_on_hover: true,
        kind: DeckToastKind::Notice,
    };

    // 1-slot replacement.
    queue.push(t1, 100);
    assert_eq!(queue.current().map(|t| t.id.as_str()), Some("humata.session.s1"));
    queue.push(t2, 102);
    assert_eq!(queue.current().map(|t| t.id.as_str()), Some("humata.session.s2"));

    // Hover at t=105 (3s elapsed, 5s remaining).
    queue.set_hovered(true, 105);
    // While hovered, does not expire even 100s later.
    assert!(!queue.expire(205));
    assert!(queue.current().is_some());

    // Unhover at t=205: 5s remaining -> expires at t=210, not t=209.
    queue.set_hovered(false, 205);
    assert!(!queue.expire(209));
    assert!(queue.current().is_some());
    assert!(queue.expire(210));
    assert!(queue.current().is_none());

    // clear_id only clears matching id.
    queue.push(
        DeckToast {
            id: "humata.session.s3".to_owned(),
            title: "Third".to_owned(),
            body: "Body 3".to_owned(),
            target: NotifyTarget::Inbox,
            duration_secs: DECK_TOAST_DURATION_SECS,
            pause_on_hover: true,
            kind: DeckToastKind::Notice,
        },
        300,
    );
    assert!(!queue.clear_id("humata.session.other"));
    assert!(queue.current().is_some());
    assert!(queue.clear_id("humata.session.s3"));
    assert!(queue.current().is_none());
}

#[test]
fn test_notice_and_dock_badge_helpers() {
    let silent = build_test_notice(false);
    assert_eq!(silent.id, "humata.test");
    assert_eq!(silent.title, "Humata test notification");
    assert_eq!(silent.body, "Notifications are working.");
    assert_eq!(silent.target, NotifyTarget::Inbox);
    assert_eq!(silent.sound, None);

    let audible = build_test_notice(true);
    assert_eq!(audible.sound, Some(NoticeSound::Inbox));

    assert_eq!(dock_badge_count(0), None);
    assert_eq!(dock_badge_count(1), Some(1));
    assert_eq!(dock_badge_count(7), Some(7));

    // Smoke-test trace_gate callable without panicking.
    trace_gate("test", "ok", "smoke");
}

#[test]
fn notify_config_in_app_toasts_serde_backward_compatibility() {
    assert!(NotifyConfig::default().in_app_toasts);
    let legacy: NotifyConfig =
        serde_json::from_str(r#"{"enabled": true, "play_sound": true}"#).expect("legacy JSON");
    assert!(legacy.in_app_toasts);

    let explicit_off: NotifyConfig =
        serde_json::from_str(r#"{"in_app_toasts": false}"#).expect("explicit false JSON");
    assert!(!explicit_off.in_app_toasts);
}
