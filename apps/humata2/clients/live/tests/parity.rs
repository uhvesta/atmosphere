//! The same cases against the fake daemon client and the real one, so the
//! fake cannot drift from what the daemon does.

mod support {
    pub mod cases;
    pub mod fake;
    pub mod real;
}

use support::{cases, fake, real};

macro_rules! parity {
    ($($case:ident => $fake:ident, $real:ident;)*) => {$(
        #[tokio::test(flavor = "multi_thread")]
        async fn $fake() {
            let (world, daemon, terminals) = fake::start();
            cases::$case(&world, daemon, terminals).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn $real() {
            let (world, daemon, terminals) = real::start().await;
            cases::$case(&world, daemon, terminals).await;
        }
    )*};
}

parity! {
    new_channel_is_listed => new_channel_is_listed_on_fake, new_channel_is_listed_on_real;
    channel_needs_a_name => channel_needs_a_name_on_fake, channel_needs_a_name_on_real;
    new_session_is_listed => new_session_is_listed_on_fake, new_session_is_listed_on_real;
    session_in_missing_channel => session_in_missing_channel_on_fake, session_in_missing_channel_on_real;
    typed_text_reaches_the_agent => typed_text_reaches_the_agent_on_fake, typed_text_reaches_the_agent_on_real;
    resume_missing_session => resume_missing_session_on_fake, resume_missing_session_on_real;
    session_change_is_pushed => session_change_is_pushed_on_fake, session_change_is_pushed_on_real;
}

macro_rules! parity_comments {
    ($($case:ident => $fake:ident, $real:ident;)*) => {$(
        #[tokio::test(flavor = "multi_thread")]
        async fn $fake() {
            let (world, daemon, comments) = fake::start_comments();
            cases::$case(&world, daemon, comments).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn $real() {
            let (world, daemon, comments) = real::start_comments().await;
            cases::$case(&world, daemon, comments).await;
        }
    )*};
}

// Sending needs a session whose agent terminal is up and typed into; the
// support has no wait for that yet, so it is not covered here.
parity_comments! {
    comment_is_listed_as_a_draft => comment_is_listed_as_a_draft_on_fake, comment_is_listed_as_a_draft_on_real;
    edit_with_a_stale_revision_is_refused => edit_with_a_stale_revision_is_refused_on_fake, edit_with_a_stale_revision_is_refused_on_real;
    archived_comment_moves_to_the_archive => archived_comment_moves_to_the_archive_on_fake, archived_comment_moves_to_the_archive_on_real;
    watch_follows_a_new_comment => watch_follows_a_new_comment_on_fake, watch_follows_a_new_comment_on_real;
}
