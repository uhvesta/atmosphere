# Brief micro-25: Undo and mistakes
Output: `apps/humata2/flows/cases/micro/25-undo-and-mistakes.md`

Scenario: I archive the wrong session and hit Undo on the toast; I mistype a channel name and fix it with ⌘Z in the field; I pick the wrong machine for a new session; I rewind an agent turn I regret; I try ⌘Z in a terminal.

Steps: Archive… offers a working Undo; text fields honour Edit > Undo/Redo; a wrong machine pick can be changed before Start (or the session moved after); ⌘Z in the terminal does nothing on Mac (⌘ keys ignored) — note readline undo is ⌃_.

Branches: Undo after the toast has gone; Undo after the app restarted; rewind (SPEC: search "rewind") — built in Humata 2? (grep features and daemon for rewind; likely Not built); the review chat's undo of a view move (daemon/humatad/tests/review.rs `the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off`).

Pointers: desktop/tests/native/overlays.rs `archiving_offers_a_working_undo`, desktop/src/main.rs Edit menu, features/app/tests/*, daemon/humatad/tests/review.rs (undo after a restart), SPEC.
