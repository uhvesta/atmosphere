use crate::model::{Comment, DiffLine, FileDiff, LineKind, RepoBases, RepoDiff, SessionId};

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

/// What the dummy data changed in `repo` against `branch`, where that is not
/// main. acme/api against release/2.4 has files of its own (VIEWS C1). `None`
/// for a repository and branch the data doesn't model.
pub(crate) fn against(repo: &str, branch: &str) -> Option<Vec<FileDiff>> {
    use LineKind::{Added, Context, Hunk, Removed};
    match (repo, branch) {
        ("acme/api", "release/2.4") => {
            let token = FileDiff {
                path: "src/token.rs".to_owned(),
                added: 5,
                removed: 0,
                lines: lines(
                    &[
                        (Hunk, "@@ -0,0 +1,5 @@"),
                        (Added, "//! Access tokens and their refresh."),
                        (Added, ""),
                        (Added, "pub struct Token {"),
                        (Added, "    pub value: String,"),
                        (Added, "}"),
                    ],
                    0,
                ),
                ..FileDiff::default()
            };
            let client = FileDiff {
                path: "src/client.rs".to_owned(),
                added: 1,
                removed: 1,
                lines: lines(
                    &[
                        (Hunk, "@@ -44,3 +44,3 @@ impl Client {"),
                        (Context, "    pub fn base_url(&self) -> &str {"),
                        (Removed, "        &self.base"),
                        (Added, "        &self.base_url"),
                        (Context, "    }"),
                    ],
                    43,
                ),
                ..FileDiff::default()
            };
            let docs = FileDiff {
                path: "docs/auth.md".to_owned(),
                added: 3,
                removed: 0,
                lines: lines(
                    &[
                        (Hunk, "@@ -0,0 +1,3 @@"),
                        (Added, "# Auth"),
                        (Added, ""),
                        (Added, "Tokens refresh once per 401."),
                    ],
                    0,
                ),
                ..FileDiff::default()
            };
            Some(vec![token, client, docs])
        }
        _ => None,
    }
}

/// The branches a repository has on its remote as the fake's clone last
/// fetched them, the default branch first (VIEWS C3). acme/api has three more
/// than main; a repository the data doesn't model has main alone.
pub(crate) fn bases(repo: &str) -> RepoBases {
    let others: &[&str] = match repo {
        "acme/api" => &["release/2.4", "release/2.3", "feat/ledger-v2"],
        _ => &[],
    };
    RepoBases {
        branches: std::iter::once("main")
            .chain(others.iter().copied())
            .map(str::to_owned)
            .collect(),
        default_branch: "main".to_owned(),
    }
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
        ..FileDiff::default()
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
        ..FileDiff::default()
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
        ..FileDiff::default()
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
        ..FileDiff::default()
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
