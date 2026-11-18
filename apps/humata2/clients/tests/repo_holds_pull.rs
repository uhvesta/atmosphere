//! Which repository's lines can carry a pull request draft (D732).

use humata2_clients::comments::repo_holds_pull;

#[test]
fn a_draft_fits_the_same_repository_or_its_bare_name_only() {
    assert!(repo_holds_pull("acme/api", "acme/api"));
    assert!(repo_holds_pull("api", "acme/api"));
    assert!(!repo_holds_pull("other/api", "acme/api"));
    assert!(!repo_holds_pull("web", "acme/api"));
    assert!(!repo_holds_pull("acme", "acme/api"));
    assert!(!repo_holds_pull("api", "api2"));
}
