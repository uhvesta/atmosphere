//! Runs every shared daemon case in [`cases`] against `FakeDaemon` and
//! against `LiveDaemon` talking to a real `humatad2`.

use humata2_clients_live_test_support::{cases, fake, real};

macro_rules! parity {
    ($case:ident) => {
        mod $case {
            use super::*;

            #[tokio::test]
            async fn fake() {
                let (world, daemon, terminals) = fake::start();
                cases::$case(&world, daemon, terminals).await;
            }

            #[tokio::test]
            async fn real() {
                let (world, daemon, terminals) = real::start().await;
                cases::$case(&world, daemon, terminals).await;
            }
        }
    };
}

parity!(new_channel_is_listed);
parity!(channel_needs_a_name);
parity!(new_session_is_listed);
parity!(a_started_session_tells_where_it_lives);
parity!(session_in_missing_channel);
parity!(typed_text_reaches_the_agent);
parity!(resume_missing_session);
parity!(a_pasted_image_comes_back_as_its_path);
parity!(session_change_is_pushed);
parity!(unviewed_output_marks_a_session_unread);
parity!(reading_an_unknown_session_is_not_an_error);
parity!(an_archived_session_is_restored);
parity!(restoring_a_session_that_is_not_archived_is_refused);
