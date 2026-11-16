//! The approval passphrase (SEC-13), set on the host. Only its Argon2id hash
//! is kept, as a setting; the words Humata generates are returned once and
//! never stored. Machines get the hash with the policy copy (AW-5).

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_passphrase::{PassphraseError, Verifier};
use humata2_store::{Store, StoreError};
use tonic::Status;

use crate::status;

/// The settings key the hash is kept under. An empty value means none is set.
const VERIFIER_KEY: &str = "approval_passphrase";

/// What the request asks for.
#[derive(Debug, PartialEq, Eq)]
enum Chosen {
    /// The user typed this passphrase.
    Typed(String),
    /// Humata picks the words.
    Generated,
    /// No passphrase: nothing user-only can be done on a machine while the
    /// host is away.
    Cleared,
}

/// Why the passphrase was not set.
#[derive(Debug, thiserror::Error)]
enum SetError {
    #[error(
        "the approval passphrase is set on the Mac that hosts Humata, not on a machine; \
         set it in Settings › Agent access on the Mac and it reaches this machine with the rules"
    )]
    NotOnHost,

    #[error(
        "the request has a passphrase and also asks Humata to generate one; \
         send the passphrase you typed, or ask for a generated one with no passphrase"
    )]
    TypedAndGenerated,

    #[error("{source}")]
    Rejected {
        #[source]
        source: PassphraseError,
    },

    #[error("couldn't keep the approval passphrase: {source}")]
    Store {
        #[source]
        source: StoreError,
    },

    #[error("hashing the approval passphrase stopped before it finished: {source}")]
    Task {
        #[source]
        source: tokio::task::JoinError,
    },
}

/// Sets, generates or clears the approval passphrase on the host, keeping
/// only its hash. `machine` is this daemon's machine, named when the disk
/// is full. Machines get the new hash with the next policy copy, so none is
/// listed as updated or pending until that copy exists.
pub(crate) async fn set_approval_passphrase(
    store: &Arc<Store>,
    on_host: bool,
    machine: &str,
    request: pb::SetApprovalPassphraseRequest,
) -> Result<pb::SetApprovalPassphraseResponse, Status> {
    let store = Arc::clone(store);
    let set = async move {
        if !on_host {
            return Err(SetError::NotOnHost);
        }
        let chosen = choose(request.passphrase, request.generate)?;
        tokio::task::spawn_blocking(move || keep(&store, chosen))
            .await
            .map_err(|source| SetError::Task { source })?
    };
    let generated = set.await.map_err(|error| refusal(error, machine))?;
    Ok(pb::SetApprovalPassphraseResponse {
        generated: generated.unwrap_or_default(),
        updated: Vec::new(),
        pending: Vec::new(),
    })
}

fn choose(passphrase: String, generate: bool) -> Result<Chosen, SetError> {
    match (passphrase.trim().is_empty(), generate) {
        (false, true) => Err(SetError::TypedAndGenerated),
        (true, true) => Ok(Chosen::Generated),
        (true, false) => Ok(Chosen::Cleared),
        (false, false) => Ok(Chosen::Typed(passphrase)),
    }
}

/// Hashes and keeps the chosen passphrase, returning the generated words.
fn keep(store: &Store, chosen: Chosen) -> Result<Option<String>, SetError> {
    let (encoded, generated) = match chosen {
        Chosen::Cleared => (String::new(), None),
        Chosen::Typed(typed) => (hash(&typed)?, None),
        Chosen::Generated => {
            let words = humata2_passphrase::generate();
            (hash(&words)?, Some(words))
        }
    };
    let store_error = |source| SetError::Store { source };
    let revision = store
        .setting(VERIFIER_KEY)
        .map_err(store_error)?
        .map_or(0, |saved| saved.revision);
    store
        .put_setting(VERIFIER_KEY, &encoded, revision + 1)
        .map_err(store_error)?;
    Ok(generated)
}

fn hash(passphrase: &str) -> Result<String, SetError> {
    Verifier::hash(passphrase)
        .map(|verifier| verifier.encoded().to_owned())
        .map_err(|source| SetError::Rejected { source })
}

fn refusal(error: SetError, machine: &str) -> Status {
    let message = error.to_string();
    match error {
        SetError::NotOnHost => status::invalid_field(
            "passphrase",
            pb::invalid_field::Problem::NotAllowedHere,
            message,
            "set the approval passphrase from the app on the Mac",
        ),
        SetError::TypedAndGenerated => status::invalid_field(
            "generate",
            pb::invalid_field::Problem::NotAllowedHere,
            message,
            "leave passphrase empty to generate one",
        ),
        SetError::Rejected {
            source: PassphraseError::TooFewWords { .. },
        } => status::invalid_field(
            "passphrase",
            pb::invalid_field::Problem::OutOfRange,
            message,
            "type at least five words, or let Humata generate a passphrase",
        ),
        SetError::Store { source } => {
            status::store(source, machine, "keep the approval passphrase")
        }
        SetError::Rejected { .. } | SetError::Task { .. } => status::internal(message, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Arc<Store>) {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        (directory, store)
    }

    fn request(passphrase: &str, generate: bool) -> pb::SetApprovalPassphraseRequest {
        pb::SetApprovalPassphraseRequest {
            passphrase: passphrase.to_owned(),
            generate,
            ..Default::default()
        }
    }

    fn kept(store: &Store) -> String {
        store.setting(VERIFIER_KEY).unwrap().map(|saved| saved.value).unwrap_or_default()
    }

    const FIVE: &str = "copper lantern drifts over harbor";

    #[tokio::test]
    async fn a_typed_passphrase_is_kept_only_as_a_hash_that_matches_it() {
        let (_directory, store) = store();
        let answer = set_approval_passphrase(&store, true, "mac", request(FIVE, false)).await.unwrap();
        assert!(answer.generated.is_empty());
        let encoded = kept(&store);
        assert!(!encoded.contains("lantern"));
        let verifier = Verifier::parse(&encoded).unwrap();
        assert!(verifier.matches(FIVE));
        assert!(!verifier.matches("copper lantern drifts over river"));
    }

    #[tokio::test]
    async fn a_generated_passphrase_is_returned_once_and_its_hash_kept() {
        let (_directory, store) = store();
        let answer = set_approval_passphrase(&store, true, "mac", request("", true)).await.unwrap();
        assert!(answer.generated.split_whitespace().count() >= 5);
        assert!(Verifier::parse(&kept(&store)).unwrap().matches(&answer.generated));
    }

    #[tokio::test]
    async fn an_empty_passphrase_clears_the_one_kept() {
        let (_directory, store) = store();
        set_approval_passphrase(&store, true, "mac", request(FIVE, false)).await.unwrap();
        set_approval_passphrase(&store, true, "mac", request("", false)).await.unwrap();
        assert_eq!(kept(&store), "");
    }

    #[tokio::test]
    async fn fewer_than_five_words_are_refused_and_keep_the_old_passphrase() {
        let (_directory, store) = store();
        set_approval_passphrase(&store, true, "mac", request(FIVE, false)).await.unwrap();
        let refused = set_approval_passphrase(&store, true, "mac", request("too short here", false))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), tonic::Code::InvalidArgument);
        assert!(Verifier::parse(&kept(&store)).unwrap().matches(FIVE));
    }

    #[tokio::test]
    async fn a_typed_passphrase_with_generate_is_refused() {
        let (_directory, store) = store();
        let refused = set_approval_passphrase(&store, true, "mac", request(FIVE, true)).await.unwrap_err();
        assert_eq!(refused.code(), tonic::Code::InvalidArgument);
        assert_eq!(kept(&store), "");
    }

    #[tokio::test]
    async fn a_machine_refuses_to_set_it() {
        let (_directory, store) = store();
        let refused = set_approval_passphrase(&store, false, "box-1", request(FIVE, false)).await.unwrap_err();
        assert_eq!(refused.code(), tonic::Code::InvalidArgument);
        assert_eq!(kept(&store), "");
    }
}
