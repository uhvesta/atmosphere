//! Queues saved by older builds still load.
use humata2_clients::model::{Comment, CommentContext};
use humata2_session::QueuedMessage;

#[test]
fn a_saved_comment_batch_loads_as_the_text_it_would_have_sent() {
    let comments = vec![Comment {
        path: "src/client.rs".into(),
        line: 43,
        body: "Split this function.".into(),
        sent: false,
        context: Some(CommentContext {
            repo: "api".into(),
            base: "main".into(),
            quoted: None,
        }),
    }];
    let saved = serde_json::json!([
        "plain text",
        { "comments": serde_json::to_value(&comments).unwrap() },
    ]);
    let loaded: Vec<QueuedMessage> = serde_json::from_value(saved).unwrap();
    assert_eq!(
        loaded,
        [
            QueuedMessage::Text("plain text".into()),
            QueuedMessage::Text(Comment::message(&comments)),
        ]
    );
    assert!(loaded[1].text().contains("Split this function."));
}

#[test]
fn a_queue_saves_as_plain_text() {
    let saved = serde_json::to_value([QueuedMessage::Text("later".into())]).unwrap();
    assert_eq!(saved, serde_json::json!(["later"]));
}
