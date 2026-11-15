//! Reading what an identity command prints (MP-8, MP-11).

use humata2_machines::{
    parse_identity, Arch, FieldProblem, IdentityError, MachineName, Os, SshAccess,
};

fn machine() -> MachineName {
    MachineName::new("box-1").unwrap()
}

fn field_error(json: &str) -> (String, FieldProblem) {
    match parse_identity(json.as_bytes(), &machine()) {
        Err(IdentityError::Field { field, problem }) => (field, problem),
        other => panic!("expected a field error, got {other:?}"),
    }
}

#[test]
fn direct_details_are_read_with_port_22_by_default() {
    let identity = parse_identity(
        br#"{"name":"box-1","ssh":{"host":"10.0.0.5","user":"dev","identity_file":"/keys/box 1","host_key":"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB comment"},"os":"linux","arch":"arm64"}"#,
        &machine(),
    )
    .unwrap();

    assert_eq!(identity.ssh.host, "10.0.0.5");
    assert_eq!(
        identity.ssh.access,
        SshAccess::Direct {
            port: 22,
            user: "dev".to_owned(),
            identity_file: Some("/keys/box 1".to_owned()),
        }
    );
    assert_eq!(identity.ssh.host_key.unwrap().algorithm, "ssh-ed25519");
    assert_eq!(identity.platform.os, Os::Linux);
    assert_eq!(identity.platform.arch, Arch::Aarch64);
}

#[test]
fn an_ssh_config_file_can_stand_in_for_the_details() {
    let identity = parse_identity(
        br#"{"name":"box-1","ssh":{"host":"box-1","ssh_config":"/tmp/box-1.ssh_config"},"os":"linux","arch":"x86_64"}"#,
        &machine(),
    )
    .unwrap();

    assert!(matches!(identity.ssh.access, SshAccess::Config { .. }));
}

#[test]
fn a_config_file_with_details_beside_it_is_refused() {
    let (field, problem) = field_error(
        r#"{"name":"box-1","ssh":{"host":"box-1","ssh_config":"/c","user":"dev"},"os":"linux","arch":"x86_64"}"#,
    );

    assert_eq!(field, "ssh.user");
    assert_eq!(problem, FieldProblem::ConflictsWithConfig);
}

#[test]
fn a_missing_host_is_named_by_its_path() {
    let (field, problem) =
        field_error(r#"{"name":"box-1","ssh":{"user":"dev"},"os":"linux","arch":"x86_64"}"#);

    assert_eq!(field, "ssh.host");
    assert_eq!(problem, FieldProblem::Missing);
}

#[test]
fn the_identity_must_be_for_this_machine() {
    let (field, problem) = field_error(
        r#"{"name":"box-2","ssh":{"host":"h","user":"dev"},"os":"linux","arch":"x86_64"}"#,
    );

    assert_eq!(field, "name");
    assert!(matches!(problem, FieldProblem::WrongName { .. }));
}

#[test]
fn unusable_values_are_refused() {
    let cases = [
        (r#""port":70000"#, "ssh.port", FieldProblem::NotPort),
        (r#""port":0"#, "ssh.port", FieldProblem::NotPort),
        (r#""host":"-oProxyCommand=evil""#, "ssh.host", FieldProblem::BadText),
        (r#""user":"dev ops""#, "ssh.user", FieldProblem::BadText),
        (r#""keyfile":"/k""#, "ssh.keyfile", FieldProblem::Unknown),
    ];
    for (extra, wanted_field, wanted) in cases {
        // A later duplicate key wins in serde_json, so `extra` overrides.
        let json = format!(
            r#"{{"name":"box-1","ssh":{{"host":"h","user":"dev",{extra}}},"os":"linux","arch":"x86_64"}}"#
        );

        let (field, problem) = field_error(&json);

        assert_eq!((field.as_str(), problem), (wanted_field, wanted), "{extra}");
    }
}

#[test]
fn an_unknown_platform_is_refused() {
    let (field, problem) =
        field_error(r#"{"name":"box-1","ssh":{"host":"h","user":"dev"},"os":"plan9","arch":"x86_64"}"#);

    assert_eq!(field, "os");
    assert!(matches!(problem, FieldProblem::UnknownOs { .. }));
}

#[test]
fn output_with_a_private_key_is_refused_without_repeating_it() {
    let output = br#"{"name":"box-1","ssh":{"host":"h","user":"dev","key":"-----BEGIN OPENSSH PRIVATE KEY-----\nAAAAsecretsecret\n-----END OPENSSH PRIVATE KEY-----"},"os":"linux","arch":"x86_64"}"#;

    let error = parse_identity(output, &machine()).unwrap_err();

    assert!(matches!(error, IdentityError::KeyMaterial));
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("AAAAsecretsecret"));
}

#[test]
fn output_that_isnt_json_is_refused() {
    let error = parse_identity(b"Host box-1\n  User dev\n", &machine()).unwrap_err();

    assert!(matches!(error, IdentityError::NotJson { .. }));
}
