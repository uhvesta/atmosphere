//! Document versions over `humata.v1.DeskService` (DV-2 to DV-8): the
//! daemon's answers as the app's [`humata2_clients::docs`] types.

use humata2_clients::DaemonError;
use humata2_clients::docs::{DocVersion, DocVersions, Restored, VersionBy, VersionDiff, VersionText};
use humata2_clients::model::{DiffLine, LineKind, SessionId};
use humata2_grpc::humata::v1 as pb;

use crate::comments::comments;
use crate::connection::{Shared, fresh_key};
use crate::map;

fn session_wire(session: &SessionId) -> Option<pb::SessionId> {
    Some(pb::SessionId {
        value: session.as_str().to_owned(),
    })
}

fn version(wire: &pb::DocVersion) -> DocVersion {
    DocVersion {
        id: wire.id.clone(),
        number: wire.number,
        at_ms: wire
            .at
            .as_ref()
            .map_or(0, |at| map::unix_ms(at.seconds, at.nanos)),
        // Only a commit the daemon says is the person's is shown as theirs.
        by: match pb::DocVersionBy::try_from(wire.by) {
            Ok(pb::DocVersionBy::You) => VersionBy::You,
            Ok(pb::DocVersionBy::Agent | pb::DocVersionBy::Unspecified) | Err(_) => VersionBy::Agent,
        },
        who: wire.who.clone(),
        message: wire.message.clone(),
        added: wire.added,
        removed: wire.removed,
        has_comments: wire.has_comments,
    }
}

fn missing(what: &str, part: &str) -> DaemonError {
    DaemonError::Refused {
        what: what.to_owned(),
        reason: format!("the daemon answered without {part}. Reload and try again"),
    }
}

impl Shared {
    pub(crate) async fn list_doc_versions(
        &self,
        session: &SessionId,
        path: &str,
    ) -> Result<DocVersions, DaemonError> {
        const WHAT: &str = "Listing a document's versions";
        let reply = self
            .desk()
            .list_doc_versions(pb::ListDocVersionsRequest {
                session: session_wire(session),
                path: path.to_owned(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(DocVersions {
            versions: reply.versions.iter().map(version).collect(),
            unsaved: reply.unsaved_changes,
        })
    }

    pub(crate) async fn read_doc_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Result<VersionText, DaemonError> {
        const WHAT: &str = "Reading a version of a document";
        let reply = self
            .desk()
            .read_doc_version(pb::ReadDocVersionRequest {
                session: session_wire(session),
                path: path.to_owned(),
                version: id.to_owned(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let wire = reply
            .version
            .as_ref()
            .ok_or_else(|| missing(WHAT, "the version"))?;
        Ok(VersionText {
            version: version(wire),
            text: reply.text,
            comments: reply.comments_recorded.then(|| comments(&reply.comments)),
        })
    }

    pub(crate) async fn diff_doc_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Result<VersionDiff, DaemonError> {
        const WHAT: &str = "Comparing a version with the document";
        let reply = self
            .desk()
            .diff_doc_version(pb::DiffDocVersionRequest {
                session: session_wire(session),
                path: path.to_owned(),
                version: id.to_owned(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let mut lines = Vec::new();
        for hunk in &reply.hunks {
            lines.push(DiffLine {
                kind: LineKind::Hunk,
                old: None,
                new: None,
                text: hunk.header.clone(),
            });
            for each in &hunk.lines {
                let (kind, old, new) = match pb::diff_line::Kind::try_from(each.kind) {
                    Ok(pb::diff_line::Kind::Added) => (LineKind::Added, None, Some(each.new_line)),
                    Ok(pb::diff_line::Kind::Removed) => (LineKind::Removed, Some(each.old_line), None),
                    _ => (LineKind::Context, Some(each.old_line), Some(each.new_line)),
                };
                lines.push(DiffLine {
                    kind,
                    old,
                    new,
                    text: each.text.clone(),
                });
            }
        }
        Ok(VersionDiff {
            lines,
            added: reply.added,
            removed: reply.removed,
        })
    }

    pub(crate) async fn save_doc_version(
        &self,
        session: &SessionId,
        message: &str,
    ) -> Result<DocVersion, DaemonError> {
        const WHAT: &str = "Saving a version";
        let reply = self
            .desk()
            .save_doc_version(pb::SaveDocVersionRequest {
                idempotency_key: fresh_key(),
                session: session_wire(session),
                message: message.to_owned(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(version(&reply))
    }

    pub(crate) async fn restore_doc_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Result<Restored, DaemonError> {
        const WHAT: &str = "Restoring a version";
        let reply = self
            .desk()
            .restore_doc_version(pb::RestoreDocVersionRequest {
                idempotency_key: fresh_key(),
                session: session_wire(session),
                path: path.to_owned(),
                version: id.to_owned(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let restored = reply
            .restored
            .as_ref()
            .ok_or_else(|| missing(WHAT, "the new version"))?;
        Ok(Restored {
            saved_first: reply.saved_first.as_ref().map(version),
            restored: version(restored),
        })
    }
}
