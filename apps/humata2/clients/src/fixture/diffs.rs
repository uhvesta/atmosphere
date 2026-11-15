use crate::model::{Comment, DiffLine, FileDiff, LineKind, RepoDiff, SessionId};

pub(super) fn lines(spec: &[(LineKind, &str)], start: u32) -> Vec<DiffLine> {
    let mut old = start;
    let mut new = start;
    spec.iter()
        .map(|(kind, text)| {
            let (o, n) = match kind {
                LineKind::Hunk => (None, None),
                LineKind::Context => {
                    old += 1;
                    new += 1;
                    (Some(old), Some(new))
                }
                LineKind::Added => {
                    new += 1;
                    (None, Some(new))
                }
                LineKind::Removed => {
                    old += 1;
                    (Some(old), None)
                }
            };
            DiffLine {
                kind: *kind,
                old: o,
                new: n,
                text: (*text).to_owned(),
            }
        })
        .collect()
}

pub(super) fn diffs() -> Vec<RepoDiff> {
    use LineKind::{Added, Context, Hunk, Removed};
    let client = FileDiff {
        path: "src/client.rs".to_owned(),
        added: 4,
        removed: 2,
        lines: lines(
            &[
                (Hunk, "@@ -40,7 +40,9 @@ impl Client {"),
                (
                    Context,
                    "    pub async fn send(&self, request: Request) -> Result<Response> {",
                ),
                (
                    Removed,
                    "        let response = self.http.send(request.clone()).await?;",
                ),
                (
                    Removed,
                    "        if response.status() == 401 { return self.retry(request).await; }",
                ),
                (
                    Added,
                    "        let response = self.http.send(request.clone()).await?;",
                ),
                (Added, "        if response.status() == 401 {"),
                (Added, "            self.tokens.refresh().await?;"),
                (Added, "            return self.http.send(request).await;"),
                (Context, "        }"),
                (Context, "        Ok(response)"),
            ],
            39,
        ),
    };
    let retry = FileDiff {
        path: "src/retry.rs".to_owned(),
        added: 2,
        removed: 0,
        lines: lines(
            &[
                (Hunk, "@@ -1,3 +1,5 @@"),
                (Added, "//! One retry, after refreshing the token."),
                (Added, ""),
                (Context, "use crate::client::Client;"),
            ],
            0,
        ),
    };
    let form = FileDiff {
        path: "src/auth/SignIn.tsx".to_owned(),
        added: 3,
        removed: 1,
        lines: lines(
            &[
                (Hunk, "@@ -10,6 +10,8 @@ export function SignIn() {"),
                (Context, "  return ("),
                (Context, "    <Form>"),
                (Removed, "      <PasswordField />"),
                (Added, "      <PasskeyButton onSuccess={finish} />"),
                (Added, "      <Divider>or</Divider>"),
                (Added, "      <EmailCodeLink />"),
                (Context, "    </Form>"),
            ],
            9,
        ),
    };
    let tokens = FileDiff {
        path: "tokens/color.json".to_owned(),
        added: 1,
        removed: 1,
        lines: lines(
            &[
                (Hunk, "@@ -3,3 +3,3 @@"),
                (Removed, "  \"surface\": \"#ffffff\","),
                (
                    Added,
                    "  \"surface\": { \"light\": \"#ffffff\", \"dark\": \"#1a1d21\" },",
                ),
                (Context, "  \"text\": \"#1a1d21\""),
            ],
            2,
        ),
    };
    vec![
        RepoDiff {
            session: SessionId::new("s-auth"),
            repo: "acme/api".to_owned(),
            base: "main".to_owned(),
            files: vec![client, retry],
            comments: vec![Comment {
                path: "src/client.rs".to_owned(),
                line: 42,
                body: "Refresh once only; a second 401 should surface.".to_owned(),
                sent: true,
                context: None,
            }],
        },
        RepoDiff {
            session: SessionId::new("s-passkey"),
            repo: "acme/web".to_owned(),
            base: "main".to_owned(),
            files: vec![form.clone()],
            comments: Vec::new(),
        },
        RepoDiff {
            session: SessionId::new("s-login"),
            repo: "acme/web".to_owned(),
            base: "main".to_owned(),
            files: vec![form],
            comments: Vec::new(),
        },
        RepoDiff {
            session: SessionId::new("s-theme"),
            repo: "acme/design-tokens".to_owned(),
            base: "main".to_owned(),
            files: vec![tokens],
            comments: Vec::new(),
        },
    ]
}
