//! Humata 2's repo library on the host: every repo the user has added (RL-1
//! to RL-4), the default repo search scope in `config/repos.toml` (RL-7,
//! RL-8), and the pure merge of one repo search (RL-6, RL-9). It knows
//! nothing about `gh`, gRPC or the network.

mod library;
mod name;
mod scope;
mod search;

pub use library::{Entry, Library, LibraryError, Place, Use};
pub use name::{NameError, RepoName};
pub use scope::{Scope, ScopeError};
pub use search::{Found, GitHubQuery, Hit, github_query, merge};
