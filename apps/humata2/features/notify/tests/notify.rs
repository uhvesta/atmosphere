use humata2_clients::model::{ChannelId, MachineId, RequestId, RequestKind, SessionId};
use humata2_clients::NotifyTarget;
use humata2_notify::{
    BurstState, FrontmostContext, NotifyConfig, NotifyEvent, channel_notice_id, clear_request,
    clear_restart, clear_session, evaluate, migrate_notice_id, restart_notice_id,
    session_notice_id,
};

fn enabled_config() -> NotifyConfig {
    NotifyConfig {
        enabled: true,
        ..NotifyConfig::default()
    }
}

fn background() -> FrontmostContext {
    FrontmostContext {
        app_active: false,
        ..FrontmostContext::default()
    }
}

fn inbox_request(req: &str, session: &str, channel: Option<(&str, &str)>) -> NotifyEvent {
    NotifyEvent::InboxRequest {
        request_id: RequestId::new(req),
        session_id: SessionId::new(session),
        session_title: format!("Session {session}"),
        channel_id: channel.map(|(id, _)| ChannelId::new(id)),
        channel_name: channel.map(|(_, name)| name.to_owned()),
        kind: RequestKind::Command,
        operation: "shell.run".to_owned(),
        answered: false,
        title: format!("claude is stopped: Run {req}"),
        body: "Open the Agent inbox to answer.".to_owned(),
    }
}

#[test]
fn config_defaults_and_serde_backward_compatibility() {
    let default = NotifyConfig::default();
    assert!(!default.enabled);
    assert!(default.inbox_requests);
    assert!(default.agent_questions);
    assert!(default.failed_resumes);
    assert!(!default.finished_turns);
    assert!(!default.migrate_items);
    assert!(default.quiet_when_frontmost);
    assert_eq!(default.cooldown_secs, 30);

    let from_empty: NotifyConfig = serde_json::from_str("{}").expect("empty JSON parses");
    assert_eq!(from_empty, default);

    let partial: NotifyConfig =
        serde_json::from_str(r#"{"enabled": true}"#).expect("partial JSON parses");
    assert!(partial.enabled);
    assert!(partial.inbox_requests);
    assert_eq!(partial.cooldown_secs, 30);
}

#[test]
fn master_switch_off_suppresses_every_event() {
    let config = NotifyConfig::default();
    let mut burst = BurstState::default();

    assert!(evaluate(&config, &background(), &mut burst, inbox_request("r1", "s1", None), 10).is_none());
    assert!(
        evaluate(
            &config,
            &background(),
            &mut burst,
            NotifyEvent::TerminalNeedsYou {
                session_id: SessionId::new("s1"),
                session_title: "Auth".to_owned(),
                channel_id: None,
                channel_name: None,
                agent_name: "Claude".to_owned(),
                has_pending_inbox_request: false,
            },
            10,
        )
        .is_none()
    );
    assert!(
        evaluate(
            &config,
            &background(),
            &mut burst,
            NotifyEvent::ResumeFailed {
                session_id: SessionId::new("s1"),
                session_title: "Auth".to_owned(),
                channel_id: None,
                channel_name: None,
                reason: "Conversation missing".to_owned(),
            },
            10,
        )
        .is_none()
    );
}

#[test]
fn category_filters_and_hard_exclusions() {
    let config = enabled_config();
    let mut burst = BurstState::default();

    // Answered request never notifies.
    let mut answered = inbox_request("r-ans", "s1", None);
    if let NotifyEvent::InboxRequest { answered: flag, .. } = &mut answered {
        *flag = true;
    }
    assert!(evaluate(&config, &background(), &mut burst, answered, 10).is_none());

    // board.* to-do asks (D846) never notify.
    let mut board_ask = inbox_request("r-board", "s1", None);
    if let NotifyEvent::InboxRequest { operation, .. } = &mut board_ask {
        *operation = "board.item.move".to_owned();
    }
    assert!(evaluate(&config, &background(), &mut burst, board_ask, 10).is_none());

    // Migrate requests are off by default.
    let mut migrate_req = inbox_request("r-mig", "s1", None);
    if let NotifyEvent::InboxRequest { kind, .. } = &mut migrate_req {
        *kind = RequestKind::Migrate;
    }
    assert!(evaluate(&config, &background(), &mut burst, migrate_req, 10).is_none());

    // Finished turns are off by default.
    let finished = NotifyEvent::FinishedTurn {
        session_id: SessionId::new("s1"),
        session_title: "Auth".to_owned(),
        channel_id: None,
        channel_name: None,
        agent_name: "Claude".to_owned(),
    };
    assert!(evaluate(&config, &background(), &mut burst, finished.clone(), 10).is_none());

    // Enabling finished_turns allows it.
    let mut with_turns = enabled_config();
    with_turns.finished_turns = true;
    let notice = evaluate(&with_turns, &background(), &mut burst, finished, 10)
        .expect("finished turn notifies when enabled");
    assert_eq!(notice.id, session_notice_id(&SessionId::new("s1")));
    assert_eq!(notice.target, NotifyTarget::Session(SessionId::new("s1")));

    // MigrateItem is off by default, notifies when migrate_items is enabled.
    let migrate_item = NotifyEvent::MigrateItem {
        request_id: RequestId::new("mig-1"),
        machine_name: "devbox-1".to_owned(),
        title: "Migrate devbox-1".to_owned(),
        body: "Daemon update ready.".to_owned(),
        answered: false,
    };
    assert!(evaluate(&config, &background(), &mut burst, migrate_item.clone(), 10).is_none());
    let mut with_migrate = enabled_config();
    with_migrate.migrate_items = true;
    let notice = evaluate(&with_migrate, &background(), &mut burst, migrate_item, 10)
        .expect("migrate item notifies when enabled");
    assert_eq!(notice.id, migrate_notice_id(&RequestId::new("mig-1")));
    assert_eq!(notice.target, NotifyTarget::Inbox);
}

#[test]
fn terminal_needs_you_and_abnormal_exit_guards() {
    let config = enabled_config();
    let mut burst = BurstState::default();

    // TerminalNeedsYou is suppressed if the session already has a pending Agent-inbox request.
    assert!(
        evaluate(
            &config,
            &background(),
            &mut burst,
            NotifyEvent::TerminalNeedsYou {
                session_id: SessionId::new("s1"),
                session_title: "Auth".to_owned(),
                channel_id: None,
                channel_name: None,
                agent_name: "Claude".to_owned(),
                has_pending_inbox_request: true,
            },
            10,
        )
        .is_none()
    );

    // Without a pending inbox request, TerminalNeedsYou notifies and targets the session.
    let notice = evaluate(
        &config,
        &background(),
        &mut burst,
        NotifyEvent::TerminalNeedsYou {
            session_id: SessionId::new("s1"),
            session_title: "Auth".to_owned(),
            channel_id: None,
            channel_name: None,
            agent_name: "Claude".to_owned(),
            has_pending_inbox_request: false,
        },
        10,
    )
    .expect("terminal prompt notifies");
    assert_eq!(notice.target, NotifyTarget::Session(SessionId::new("s1")));

    // Exit code 0 never notifies.
    assert!(
        evaluate(
            &config,
            &background(),
            &mut burst,
            NotifyEvent::AbnormalExit {
                session_id: SessionId::new("s2"),
                session_title: "Build".to_owned(),
                channel_id: None,
                channel_name: None,
                code: 0,
                already_resume_failed: false,
            },
            10,
        )
        .is_none()
    );

    // AbnormalExit covered by resume_failed does not notify.
    assert!(
        evaluate(
            &config,
            &background(),
            &mut burst,
            NotifyEvent::AbnormalExit {
                session_id: SessionId::new("s2"),
                session_title: "Build".to_owned(),
                channel_id: None,
                channel_name: None,
                code: 1,
                already_resume_failed: true,
            },
            10,
        )
        .is_none()
    );

    // Non-zero exit without resume_failed notifies.
    let notice = evaluate(
        &config,
        &background(),
        &mut burst,
        NotifyEvent::AbnormalExit {
            session_id: SessionId::new("s2"),
            session_title: "Build".to_owned(),
            channel_id: None,
            channel_name: None,
            code: 137,
            already_resume_failed: false,
        },
        50,
    )
    .expect("abnormal exit notifies");
    assert_eq!(notice.id, session_notice_id(&SessionId::new("s2")));
    assert_eq!(notice.target, NotifyTarget::Session(SessionId::new("s2")));
}

#[test]
fn frontmost_on_item_suppression_and_background_override() {
    let config = enabled_config();
    let mut burst = BurstState::default();

    // Frontmost on global Inbox suppresses InboxRequest.
    let on_inbox = FrontmostContext {
        app_active: true,
        viewing_inbox: true,
        ..FrontmostContext::default()
    };
    assert!(
        evaluate(&config, &on_inbox, &mut burst, inbox_request("r1", "s1", Some(("c1", "api"))), 10)
            .is_none()
    );

    // Frontmost on that session's Inbox panel suppresses InboxRequest.
    let on_session_inbox = FrontmostContext {
        app_active: true,
        viewing_session: Some(SessionId::new("s1")),
        viewing_session_inbox: true,
        ..FrontmostContext::default()
    };
    assert!(
        evaluate(
            &config,
            &on_session_inbox,
            &mut burst,
            inbox_request("r1", "s1", Some(("c1", "api"))),
            10,
        )
        .is_none()
    );

    // Frontmost on that channel's Inbox tab suppresses InboxRequest.
    let on_channel_inbox = FrontmostContext {
        app_active: true,
        viewing_channel_inbox: Some(ChannelId::new("c1")),
        ..FrontmostContext::default()
    };
    assert!(
        evaluate(
            &config,
            &on_channel_inbox,
            &mut burst,
            inbox_request("r1", "s1", Some(("c1", "api"))),
            10,
        )
        .is_none()
    );

    // Frontmost on that session's terminal (NOT inbox panel) still notifies for InboxRequest!
    let on_session_terminal = FrontmostContext {
        app_active: true,
        viewing_session: Some(SessionId::new("s1")),
        viewing_session_inbox: false,
        ..FrontmostContext::default()
    };
    let notice = evaluate(
        &config,
        &on_session_terminal,
        &mut burst,
        inbox_request("r1", "s1", Some(("c1", "api"))),
        10,
    )
    .expect("session terminal view does not suppress inbox request");
    assert_eq!(
        notice.target,
        NotifyTarget::Request {
            session: SessionId::new("s1"),
            request: RequestId::new("r1"),
        }
    );

    // Frontmost on that session suppresses TerminalNeedsYou and ResumeFailed.
    assert!(
        evaluate(
            &config,
            &on_session_terminal,
            &mut burst,
            NotifyEvent::ResumeFailed {
                session_id: SessionId::new("s1"),
                session_title: "Session s1".to_owned(),
                channel_id: Some(ChannelId::new("c1")),
                channel_name: Some("api".to_owned()),
                reason: "Missing".to_owned(),
            },
            15,
        )
        .is_none()
    );

    // Backgrounded app ALWAYS notifies even when open on Inbox.
    let bg_inbox = FrontmostContext {
        app_active: false,
        viewing_inbox: true,
        ..FrontmostContext::default()
    };
    assert!(
        evaluate(
            &config,
            &bg_inbox,
            &mut burst,
            inbox_request("r2", "s2", Some(("c2", "web"))),
            20,
        )
        .is_some()
    );
}

#[test]
fn deduplication_by_request_id_and_session_episode() {
    let config = enabled_config();
    let mut burst = BurstState::default();

    let req = inbox_request("r1", "s1", None);
    assert!(evaluate(&config, &background(), &mut burst, req.clone(), 100).is_some());
    assert!(evaluate(&config, &background(), &mut burst, req, 200).is_none());

    let fail = NotifyEvent::ResumeFailed {
        session_id: SessionId::new("s2"),
        session_title: "Worker".to_owned(),
        channel_id: None,
        channel_name: None,
        reason: "Folder missing".to_owned(),
    };
    assert!(evaluate(&config, &background(), &mut burst, fail.clone(), 100).is_some());
    assert!(evaluate(&config, &background(), &mut burst, fail, 200).is_none());

    // AbnormalExit on the same session while ResumeFailed is active is also deduplicated.
    assert!(
        evaluate(
            &config,
            &background(),
            &mut burst,
            NotifyEvent::AbnormalExit {
                session_id: SessionId::new("s2"),
                session_title: "Worker".to_owned(),
                channel_id: None,
                channel_name: None,
                code: 1,
                already_resume_failed: false,
            },
            205,
        )
        .is_none()
    );
}

#[test]
fn same_session_burst_collapses_within_cooldown_and_resets_after() {
    let config = enabled_config();
    let mut burst = BurstState::default();

    // First event posts immediately with its own title/target.
    let first = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r1", "s1", Some(("c1", "backend"))),
        100,
    )
    .expect("first posts");
    assert_eq!(first.id, "humata.session.s1");
    assert_eq!(first.title, "claude is stopped: Run r1");
    assert_eq!(
        first.target,
        NotifyTarget::Request {
            session: SessionId::new("s1"),
            request: RequestId::new("r1"),
        }
    );

    // Repeat for same session within 30 s replaces in place.
    let second = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r2", "s1", Some(("c1", "backend"))),
        112,
    )
    .expect("second replaces in place");
    assert_eq!(second.id, "humata.session.s1");
    assert_eq!(second.title, "2 items need you in Session s1");
    assert_eq!(second.target, NotifyTarget::Session(SessionId::new("s1")));

    // Third within 30 s increments count.
    let third = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r3", "s1", Some(("c1", "backend"))),
        125,
    )
    .expect("third replaces in place");
    assert_eq!(third.id, "humata.session.s1");
    assert_eq!(third.title, "3 items need you in Session s1");

    // After 30 s cooldown expires, posts fresh single-item notice.
    let fourth = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r4", "s1", Some(("c1", "backend"))),
        160,
    )
    .expect("fourth posts fresh after cooldown");
    assert_eq!(fourth.id, "humata.session.s1");
    assert_eq!(fourth.title, "claude is stopped: Run r4");
    assert_eq!(
        fourth.target,
        NotifyTarget::Request {
            session: SessionId::new("s1"),
            request: RequestId::new("r4"),
        }
    );
}

#[test]
fn multi_session_same_channel_and_desk_collapse_within_cooldown() {
    let config = enabled_config();
    let mut burst = BurstState::default();

    // Two distinct sessions in #backend within 30 s collapse to humata.channel.c1.
    let n1 = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r1", "s1", Some(("c1", "backend"))),
        100,
    )
    .expect("first session posts");
    assert_eq!(n1.id, "humata.session.s1");

    let n2 = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r2", "s2", Some(("c1", "backend"))),
        110,
    )
    .expect("second session collapses to channel");
    assert_eq!(n2.id, channel_notice_id(Some(&ChannelId::new("c1"))));
    assert_eq!(n2.title, "2 sessions need you in #backend");
    assert_eq!(n2.target, NotifyTarget::Inbox);

    let n3 = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r3", "s3", Some(("c1", "backend"))),
        120,
    )
    .expect("third session updates channel count");
    assert_eq!(n3.id, "humata.channel.c1");
    assert_eq!(n3.title, "3 sessions need you in #backend");

    // Two distinct sessions on Desk within 30 s collapse to humata.channel.desk.
    let d1 = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("rd1", "sd1", None),
        200,
    )
    .expect("first desk session posts");
    assert_eq!(d1.id, "humata.session.sd1");

    let d2 = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("rd2", "sd2", None),
        215,
    )
    .expect("second desk session collapses to Desk");
    assert_eq!(d2.id, "humata.channel.desk");
    assert_eq!(d2.title, "2 sessions need you on Desk");
    assert_eq!(d2.target, NotifyTarget::Inbox);
}

#[test]
fn restart_failed_single_and_multi_session() {
    let config = enabled_config();
    let mut burst = BurstState::default();

    // Single-session restart targets that session.
    let single = evaluate(
        &config,
        &background(),
        &mut burst,
        NotifyEvent::RestartFailed {
            machine_id: MachineId::new("m1"),
            machine_name: "devbox-1".to_owned(),
            sessions: vec![SessionId::new("s1")],
            not_brought_back: 1,
        },
        100,
    )
    .expect("single-session restart notifies");
    assert_eq!(single.id, "humata.session.s1");
    assert_eq!(single.target, NotifyTarget::Session(SessionId::new("s1")));

    // Multi-session restart posts summary under humata.restart.<machine_id> targeting Home.
    let multi = evaluate(
        &config,
        &background(),
        &mut burst,
        NotifyEvent::RestartFailed {
            machine_id: MachineId::new("m2"),
            machine_name: "gpu-box".to_owned(),
            sessions: vec![SessionId::new("s2"), SessionId::new("s3")],
            not_brought_back: 3,
        },
        100,
    )
    .expect("multi-session restart notifies");
    assert_eq!(multi.id, restart_notice_id(&MachineId::new("m2")));
    assert_eq!(multi.title, "gpu-box restarted: 3 sessions couldn't resume");
    assert_eq!(multi.target, NotifyTarget::Home);

    // Frontmost on Home suppresses multi-session restart.
    let on_home = FrontmostContext {
        app_active: true,
        viewing_home: true,
        ..FrontmostContext::default()
    };
    assert!(
        evaluate(
            &config,
            &on_home,
            &mut burst,
            NotifyEvent::RestartFailed {
                machine_id: MachineId::new("m3"),
                machine_name: "mini".to_owned(),
                sessions: vec![SessionId::new("s4"), SessionId::new("s5")],
                not_brought_back: 2,
            },
            100,
        )
        .is_none()
    );

    // Clearing restart withdraws its identifier.
    assert_eq!(
        clear_restart(&mut burst, &MachineId::new("m2")),
        vec!["humata.restart.m2".to_owned()]
    );
    assert!(clear_restart(&mut burst, &MachineId::new("m2")).is_empty());
}

#[test]
fn clear_request_and_clear_session_withdraw_and_reset_cooldown() {
    let config = enabled_config();
    let mut burst = BurstState::default();

    // Two requests in s1 within cooldown.
    let _ = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r1", "s1", Some(("c1", "api"))),
        100,
    );
    let _ = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r2", "s1", Some(("c1", "api"))),
        105,
    );

    // Answering r1 leaves r2 pending, so nothing is withdrawn yet, but cooldown resets.
    assert!(clear_request(&mut burst, &RequestId::new("r1")).is_empty());

    // Because cooldown was reset by answering r1, a new request r3 posts fresh (not collapsed).
    let r3 = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r3", "s1", Some(("c1", "api"))),
        110,
    )
    .expect("posts fresh after answer reset cooldown");
    assert_eq!(r3.title, "claude is stopped: Run r3");

    // Second session in same channel triggers channel collapse.
    let collapsed = evaluate(
        &config,
        &background(),
        &mut burst,
        inbox_request("r4", "s2", Some(("c1", "api"))),
        115,
    )
    .expect("channel collapses");
    assert_eq!(collapsed.id, "humata.channel.c1");

    // Clearing s1 withdraws s1's notice, but not the channel notice while s2 is still active.
    assert_eq!(
        clear_session(&mut burst, &SessionId::new("s1")),
        vec!["humata.session.s1".to_owned()]
    );

    // Answering s2's only request clears s2 and withdraws the channel collapsed notice too.
    assert_eq!(
        clear_request(&mut burst, &RequestId::new("r4")),
        vec!["humata.channel.c1".to_owned()]
    );
}
