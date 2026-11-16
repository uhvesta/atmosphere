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
