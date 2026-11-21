//! Offline provider authoring helpers (PROVIDERCLI, D906): guide, schema
//! generated from the real parser types, worked examples, offline validation,
//! exact command rendering without execution, and output parser validation.
//!
//! Shared between the CLI (`humata provider {guide,schema,example,check,render,parse}`)
//! and MCP read-only twins so both stay identical and never drift from
//! [`Provider::parse`], [`parse_identity`], [`parse_status_report`], and
//! [`parse_list`].

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::error::{ConfigError, IdentityError, NameError, ProblemKind, ValuesError};
use crate::identity::{
    parse_identity, Identity, SshAccess, SshTarget, IDENTITY_KEYS, KEY_ALGORITHMS, SSH_KEYS,
};
use crate::name::{MachineName, BASELINE};
use crate::provider::{
    parse_list, parse_status_report, Action, DiscoveredInstance, MachineStatus, Provider,
    StatusReport, LIMA_TOML, RAW_PARAM_KEYS, RAW_PROVIDER_KEYS, STATUS_WORD_GROUPS,
};
use crate::runner::{EnvValue, Purpose};

/// Plain-text guide to authoring a Humata machine provider (MP-1..MP-23).
pub fn provider_guide() -> &'static str {
    GUIDE_TEXT
}

const GUIDE_TEXT: &str = r#"# Humata Machine Providers Guide

## 1. What a provider is (MP-1, MP-2)
A provider is a `[providers.<id>]` TOML table that teaches Humata how to create,
reach, inspect, start, stop, wake, refresh, and delete machines of a given kind
(a local VM, a cloud VM, a Terraform workspace, or an existing SSH fleet).

Provider ids must match `[a-z][a-z0-9-]*` (at most 63 characters).
Humata shows only the actions a provider defines.

## 2. Actions and when Humata calls them (MP-21)
- `create`   : Provision a new machine (called by `humata provider create` / Add machine).
- `connect`  : Run `{cmd}` on the machine (used for setup, peer link, and bridge).
- `identity` : Print the machine's SSH details and platform as JSON.
- `list`     : Discover existing instances as a JSON array (called `discover` in `parse`).
- `status`   : Report a machine's current state (plain-text word or JSON `{ state, instance_id?, ssh? }`).
- `start`    : Start a stopped machine.
- `stop`     : Stop a running machine.
- `wake`     : Wake a sleeping/suspended machine (falls back to `start` when `wake_on_demand = true`).
- `refresh`  : Refresh credentials or session tokens (often paired with `interactive = true`).
- `ssh`      : Optional custom SSH command override.
- `delete`   : Destroy what `create` made (only after the user confirms a removal plan).

A provider must declare at least one of `connect`, `identity`, or `list`.
`wake_on_demand` defaults to `true` when `wake` or `start` is declared, else `false`.

## 3. Three ways to reach a machine (MP-7 to MP-10)
1. `connect` (MP-7):
   A command template containing `{cmd}`, which Humata replaces with a POSIX
   `sh -c '...'` command, already single-quoted. `{cmd}` is allowed ONLY in
   `connect`. Example: `exec limactl shell --workdir / humata2-{name} sh -c {cmd}`.
2. `identity` (MP-8, MP-9):
   A command that prints one JSON object to stdout:
     { "name": "<m>", "ssh": { ... }, "os": "linux", "arch": "aarch64" }
   Optional top-level fields: `state`, `instance_id`.
   `ssh` has `host` plus EITHER `ssh_config` (path to an ssh_config file) OR
   direct fields (`user`, optional `port` defaulting to 22, optional `identity_file`).
   Optional `ssh.host_key` (`ssh-ed25519 AAAA...`) pins the host key in Humata's
   own `known_hosts` with no fingerprint prompt. NEVER print private key material:
   output containing `PRIVATE KEY`, `-----BEGIN`, or `PuTTY-User-Key-File` is refused.
   A provider may declare BOTH `connect` and `identity`: `connect` is the primary
   transport, and `identity` refreshes SSH metadata/state.
3. Plain SSH (`humata host add <ssh-host>`, MP-10):
   For standalone SSH hosts without a provider, or an SSH fleet using `list` + `identity`.

## 4. Templates `{name}`, `{cmd}`, and environment variables (MP-3, MP-4)
- `{name}` is replaced in any command with the validated machine name
  (`[a-z0-9-]`, starts/ends with alphanumeric, max 63 chars — always one safe shell word).
- `{cmd}` is replaced ONLY in `connect`, quoted for `/bin/sh -c`.
- Other braces (`${HOME}`, `{{.Status}}`, `{state: .status}`) are left untouched.
- Every per-machine command receives:
  - `HUMATA_NAME`: the machine's name.
  - `HUMATA_<PARAM>`: each declared param's value, with the param name uppercased
    (e.g. param `cpus` -> `$HUMATA_CPUS`). Quote them in shell scripts.
  - Baseline host vars: `HOME`, `PATH`, `USER`, `LOGNAME`, `SHELL`, `LANG`,
    `LC_ALL`, `TMPDIR`, `SSH_AUTH_SOCK`.
  - Any host variables listed in `env = ["MY_TOKEN"]`, read from Settings > This Mac > Variables.
    Never put secrets in the TOML file; list their variable names in `env`.
- Provider-level `list` commands receive baseline vars and `env` (no `HUMATA_NAME` or params).

## 5. Where commands run (MP-5)
By default, every provider command runs on the host Mac via `/bin/sh -c <script>`.
Set `run_on = "<machine>"` to run the provider's commands on another linked machine instead.

## 6. Interactive logins and takeover (MP-12 to MP-18)
Set `interactive = true` when the tool needs a human to log in (SSO, 2FA, browser URL,
hardware key). Humata opens a real PTY (`humata provider login` or the app UI):
- If `refresh` (or `wake` without `connect`) is declared, Humata runs it in the PTY
  until it exits so the user can complete browser/device auth.
- Otherwise Humata opens an interactive shell over the link and takes over once the
  user finishes logging in. Menu-only tools cannot carry a takeover (MP-18): give them
  an `identity` command or a wrapper that ends in a POSIX shell.

## 7. SSH steps after `identity` (MP-19)
After `create` and `identity`:
1. Pin `ssh.host_key` (if given) or scan and ask the owner to confirm the fingerprint
   into Humata's own `known_hosts` (never `~/.ssh/known_hosts`).
2. Probe `uname -sm` over the link and verify Linux (`x86_64` or `aarch64`) matches `os`/`arch`.
3. Check `git`, free disk space, and service manager (`systemd --user` or `setsid` + `crontab`).
4. Stream `humatad2`, its holder, and `bundle.json` into `~/.humata2/bin`.
5. Write the peer enrollment key into `~/.humata2/state`.
6. Start the service and open the peer link (`humatad2 serve --stdio`).

## 8. What a failure looks like (MP-20)
Failed commands report the step name, exit code/signal/timeout, and the tail of
stdout/stderr (with host variable values scrubbed), paired with concrete remedies:
Try again, Fix provider, Open interactively, Confirm host key, Run as admin,
Start without systemd, or Delete what was made.

## 9. Offline workflow and handing the file to the owner (MP-23)
1. Inspect the exact schema and examples:
     humata provider schema [--json]
     humata provider example [lima|cloud|terraform|ssh]
2. Check your TOML offline with the real parser:
     humata provider check <file.toml>
3. Preview the exact argv, env, and target machine without running anything:
     humata provider render <file.toml> <action> --name <machine> [--param k=v]...
4. Validate sample stdout from your own command:
     humata provider parse <identity|status|discover> <file|-> [--name <machine>]
5. Hand the file to the owner for approval (never modifies providers directly):
     humata provider request-import <file.toml> --reason "<why the task needs it>"
   This creates a host Inbox card showing the full file and every command; only
   the owner can approve or deny it.
"#;

/// A single documented field in a TOML table or JSON payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchemaField {
    pub name: &'static str,
    #[serde(rename = "type")]
    pub field_type: &'static str,
    pub required: bool,
    pub default: Option<&'static str>,
    pub description: &'static str,
}

/// A status category and all recognized words that map to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusGroup {
    pub canonical: &'static str,
    pub words: Vec<&'static str>,
}

/// Complete machine provider schema derived from the real Rust parser types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderSchema {
    pub toml_table: &'static str,
    pub provider_id_pattern: &'static str,
    pub provider_fields: Vec<SchemaField>,
    pub param_fields: Vec<SchemaField>,
    pub placeholders: Vec<SchemaField>,
    pub baseline_env: Vec<&'static str>,
    pub identity_fields: Vec<SchemaField>,
    pub ssh_fields: Vec<SchemaField>,
    pub valid_os: Vec<&'static str>,
    pub valid_arch: Vec<&'static str>,
    pub host_key_algorithms: Vec<&'static str>,
    pub status_fields: Vec<SchemaField>,
    pub status_words: Vec<StatusGroup>,
    pub discover_fields: Vec<SchemaField>,
}

fn provider_field_meta(name: &'static str) -> SchemaField {
    match name {
        "label" => SchemaField {
            name,
            field_type: "string",
            required: false,
            default: Some("provider id"),
            description: "Human-readable name shown in Settings and Add machine.",
        },
        "create" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Creates a new machine. Uses {name}, $HUMATA_NAME, $HUMATA_<PARAM>.",
        },
        "connect" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Runs {cmd} on the machine (must contain {cmd}). At least one of connect, identity, or list is required.",
        },
        "identity" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Prints machine SSH details and platform as JSON (see identity_fields).",
        },
        "list" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Provider-level discovery command printing a JSON array of existing instances (see discover_fields).",
        },
        "start" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Starts a stopped machine.",
        },
        "stop" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Stops a running machine.",
        },
        "wake" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Wakes a sleeping machine. Falls back to `start` when wake_on_demand is true.",
        },
        "refresh" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Refreshes credentials or session auth (run in a PTY when interactive = true).",
        },
        "ssh" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Optional custom SSH command template.",
        },
        "delete" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Deletes what `create` made.",
        },
        "status" => SchemaField {
            name,
            field_type: "string (shell template)",
            required: false,
            default: None,
            description: "Prints a plain-text status word or JSON `{ state, instance_id?, ssh? }`.",
        },
        "wake_on_demand" => SchemaField {
            name,
            field_type: "bool",
            required: false,
            default: Some("true if `wake` or `start` is declared, else false"),
            description: "Whether Humata may automatically wake a sleeping or stopped machine on demand.",
        },
        "params" => SchemaField {
            name,
            field_type: "array of tables",
            required: false,
            default: Some("[]"),
            description: "Parameters prompted on create and exported as $HUMATA_<PARAM>.",
        },
        "env" => SchemaField {
            name,
            field_type: "array of strings",
            required: false,
            default: Some("[]"),
            description: "Host variable names (`[A-Z_][A-Z0-9_]*`, not `HUMATA_*` or baseline vars) forwarded from Settings > This Mac > Variables.",
        },
        "interactive" => SchemaField {
            name,
            field_type: "bool",
            required: false,
            default: Some("false"),
            description: "True when reaching or refreshing the machine requires an interactive terminal login.",
        },
        "run_on" => SchemaField {
            name,
            field_type: "string",
            required: false,
            default: Some("host Mac"),
            description: "Linked machine name where provider commands run instead of the host Mac (MP-5).",
        },
        other => panic!("missing schema metadata for RawProvider field `{other}`"),
    }
}

fn param_field_meta(name: &'static str) -> SchemaField {
    match name {
        "name" => SchemaField {
            name,
            field_type: "string",
            required: true,
            default: None,
            description: "Param id (`[a-z][a-z0-9_]*`, max 32 chars, not `name`). Exported as $HUMATA_<NAME>.",
        },
        "prompt" => SchemaField {
            name,
            field_type: "string",
            required: false,
            default: Some("param name"),
            description: "Human-readable label shown in Add machine.",
        },
        "default" => SchemaField {
            name,
            field_type: "string",
            required: false,
            default: None,
            description: "Default value. If omitted, the caller must supply `--param <name>=<value>`.",
        },
        other => panic!("missing schema metadata for RawParam field `{other}`"),
    }
}

fn identity_field_meta(name: &'static str) -> SchemaField {
    match name {
        "name" => SchemaField {
            name,
            field_type: "string",
            required: true,
            default: None,
            description: "Must match the machine's name.",
        },
        "ssh" => SchemaField {
            name,
            field_type: "object",
            required: true,
            default: None,
            description: "SSH destination object (see ssh_fields). Paths only, never private keys.",
        },
        "os" => SchemaField {
            name,
            field_type: "string",
            required: true,
            default: None,
            description: "Operating system: `linux` or `darwin` (`macos`).",
        },
        "arch" => SchemaField {
            name,
            field_type: "string",
            required: true,
            default: None,
            description: "CPU architecture: `x86_64` (`amd64`) or `aarch64` (`arm64`).",
        },
        "state" => SchemaField {
            name,
            field_type: "string",
            required: false,
            default: None,
            description: "Optional machine status word (see status_words).",
        },
        "instance_id" => SchemaField {
            name,
            field_type: "string",
            required: false,
            default: None,
            description: "Optional provider-assigned instance identifier.",
        },
        other => panic!("missing schema metadata for identity key `{other}`"),
    }
}

fn ssh_field_meta(name: &'static str) -> SchemaField {
    match name {
        "host" => SchemaField {
            name,
            field_type: "string",
            required: true,
            default: None,
            description: "SSH hostname, IP, or ssh_config alias (no spaces, control chars, or leading `-`).",
        },
        "ssh_config" => SchemaField {
            name,
            field_type: "string (path)",
            required: false,
            default: None,
            description: "Path to an ssh_config file that knows `host`. Mutually exclusive with `port`, `user`, `identity_file`.",
        },
        "port" => SchemaField {
            name,
            field_type: "integer (1..65535) or numeric string",
            required: false,
            default: Some("22"),
            description: "SSH port (direct mode only; conflicts with `ssh_config`).",
        },
        "user" => SchemaField {
            name,
            field_type: "string",
            required: false,
            default: None,
            description: "SSH username. Required when `ssh_config` is omitted; conflicts with `ssh_config`.",
        },
        "identity_file" => SchemaField {
            name,
            field_type: "string (path)",
            required: false,
            default: None,
            description: "Path to a private key file on disk (never key contents). Conflicts with `ssh_config`.",
        },
        "host_key" => SchemaField {
            name,
            field_type: "string",
            required: false,
            default: None,
            description: "Public host key in known_hosts format (`<algorithm> <base64>`), pinned without prompting.",
        },
        other => panic!("missing schema metadata for ssh key `{other}`"),
    }
}

/// Compile-time exhaustiveness witness for `Identity`, `SshTarget`, `SshAccess`,
/// `StatusReport`, `DiscoveredInstance`, `Action`, and `MachineStatus`.
#[allow(dead_code)]
fn _assert_output_types_exhaustive(
    id: Identity,
    status: StatusReport,
    discovered: DiscoveredInstance,
    action: Action,
    machine_status: MachineStatus,
) {
    let Identity {
        ssh,
        platform: _,
        state: _,
        instance_id: _,
    } = id;
    let SshTarget {
        host: _,
        access,
        host_key: _,
    } = ssh;
    match access {
        SshAccess::Config { ssh_config: _ } => {}
        SshAccess::Direct {
            port: _,
            user: _,
            identity_file: _,
        } => {}
    }
    let StatusReport {
        state: _,
        instance_id: _,
        ssh: _,
    } = status;
    let DiscoveredInstance {
        name: _,
        state: _,
        params: _,
        ssh: _,
        instance_id: _,
    } = discovered;
    match action {
        Action::Create
        | Action::Identity
        | Action::List
        | Action::Start
        | Action::Stop
        | Action::Wake
        | Action::Refresh
        | Action::Ssh
        | Action::Delete
        | Action::Status => {}
    }
    match machine_status {
        MachineStatus::Running
        | MachineStatus::Sleeping
        | MachineStatus::Stopped
        | MachineStatus::Starting
        | MachineStatus::Missing
        | MachineStatus::Other(_) => {}
    }
}

/// Returns the complete provider schema derived from the real parser structs.
pub fn provider_schema() -> ProviderSchema {
    ProviderSchema {
        toml_table: "[providers.<id>]",
        provider_id_pattern: "^[a-z][a-z0-9-]{0,62}$",
        provider_fields: RAW_PROVIDER_KEYS
            .iter()
            .map(|&name| provider_field_meta(name))
            .collect(),
        param_fields: RAW_PARAM_KEYS
            .iter()
            .map(|&name| param_field_meta(name))
            .collect(),
        placeholders: vec![
            SchemaField {
                name: "{name}",
                field_type: "placeholder",
                required: false,
                default: None,
                description: "Replaced in any per-machine command with the validated machine name (one safe shell word).",
            },
            SchemaField {
                name: "{cmd}",
                field_type: "placeholder",
                required: false,
                default: None,
                description: "Replaced ONLY in `connect` with the remote command, already quoted for `/bin/sh -c`.",
            },
        ],
        baseline_env: BASELINE.to_vec(),
        identity_fields: IDENTITY_KEYS
            .iter()
            .map(|&name| identity_field_meta(name))
            .collect(),
        ssh_fields: SSH_KEYS
            .iter()
            .map(|&name| ssh_field_meta(name))
            .collect(),
        valid_os: vec!["linux", "darwin", "macos"],
        valid_arch: vec!["x86_64", "amd64", "aarch64", "arm64"],
        host_key_algorithms: KEY_ALGORITHMS.to_vec(),
        status_fields: vec![
            SchemaField {
                name: "state",
                field_type: "string",
                required: true,
                default: None,
                description: "Status word (or plain-text last line when stdout is not JSON).",
            },
            SchemaField {
                name: "instance_id",
                field_type: "string",
                required: false,
                default: None,
                description: "Optional provider-assigned instance identifier.",
            },
            SchemaField {
                name: "ssh",
                field_type: "object",
                required: false,
                default: None,
                description: "Optional updated SSH target (same shape as identity.ssh).",
            },
        ],
        status_words: STATUS_WORD_GROUPS
            .iter()
            .map(|(canonical, words)| StatusGroup {
                canonical,
                words: words.to_vec(),
            })
            .collect(),
        discover_fields: vec![
            SchemaField {
                name: "name",
                field_type: "string",
                required: true,
                default: None,
                description: "Non-empty instance name.",
            },
            SchemaField {
                name: "state",
                field_type: "string",
                required: true,
                default: None,
                description: "Status word (see status_words).",
            },
            SchemaField {
                name: "params",
                field_type: "object (string|number|bool values)",
                required: false,
                default: Some("{}"),
                description: "Optional param values associated with the discovered instance.",
            },
            SchemaField {
                name: "ssh",
                field_type: "object",
                required: false,
                default: None,
                description: "Optional SSH target (same shape as identity.ssh).",
            },
            SchemaField {
                name: "instance_id",
                field_type: "string",
                required: false,
                default: None,
                description: "Optional provider-assigned instance identifier.",
            },
        ],
    }
}

impl ProviderSchema {
    /// Human-readable rendering of the schema.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Provider TOML table: {}", self.toml_table);
        let _ = writeln!(out, "Provider id pattern: {}\n", self.provider_id_pattern);

        write_fields(&mut out, "Provider fields ([providers.<id>])", &self.provider_fields);
        write_fields(&mut out, "Param fields (params = [{ ... }])", &self.param_fields);
        write_fields(&mut out, "Template placeholders", &self.placeholders);

        let _ = writeln!(
            out,
            "Baseline environment variables forwarded from host:\n  {}\n",
            self.baseline_env.join(", ")
        );

        write_fields(&mut out, "Identity JSON output (stdout of `identity`)", &self.identity_fields);
        write_fields(&mut out, "SSH object (`ssh` in identity, status, discover)", &self.ssh_fields);
        let _ = writeln!(out, "Valid os values:   {}", self.valid_os.join(", "));
        let _ = writeln!(out, "Valid arch values: {}", self.valid_arch.join(", "));
        let _ = writeln!(
            out,
            "Host key algorithms: {}\n",
            self.host_key_algorithms.join(", ")
        );

        write_fields(
            &mut out,
            "Status output (plain text word or JSON object)",
            &self.status_fields,
        );
        let _ = writeln!(out, "Recognized status words:");
        for group in &self.status_words {
            let _ = writeln!(out, "  {:10} {}", group.canonical, group.words.join(", "));
        }
        out.push('\n');

        write_fields(
            &mut out,
            "Discover output (JSON array from `list`)",
            &self.discover_fields,
        );
        out.trim_end().to_owned()
    }
}

fn write_fields(out: &mut String, heading: &str, fields: &[SchemaField]) {
    let _ = writeln!(out, "{heading}:");
    for f in fields {
        let req = if f.required { "required" } else { "optional" };
        let def = f
            .default
            .map(|d| format!(", default: {d}"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {:<16} {} ({req}{def}) — {}",
            f.name, f.field_type, f.description
        );
    }
    out.push('\n');
}

/// One complete, valid example provider file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProviderExample {
    pub kind: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub toml: &'static str,
}

const CLOUD_EXAMPLE_TOML: &str = r#"# Generic cloud CLI provider: creates and manages Linux VMs through a cloud CLI.
# Store credentials in Settings > This Mac > Variables and list their names in `env`.

[providers.cloud-vm]
label = "Cloud VM"
params = [
  { name = "region", prompt = "Region", default = "us-east-1" },
  { name = "size", prompt = "Instance size", default = "medium" },
]
env = ["CLOUD_API_TOKEN"]

create = """
cloudctl vm create {name} \
  --region "$HUMATA_REGION" \
  --size "$HUMATA_SIZE" \
  --image ubuntu-24.04 \
  --wait
"""
connect = "exec cloudctl vm ssh {name} -- sh -c {cmd}"
start = "cloudctl vm start {name}"
stop = "cloudctl vm stop {name}"
delete = "cloudctl vm delete --yes {name}"
status = "cloudctl vm show {name} --format json | jq -c '{state: .status, instance_id: .id}'"
list = "cloudctl vm list --format json | jq -c '[.[] | {name: .name, state: .status, instance_id: .id}]'"
"#;

const TERRAFORM_EXAMPLE_TOML: &str = r#"# Terraform workspace provider: provisions a VM with Terraform and hands Humata
# its SSH details via `identity` JSON (no private key contents are ever printed).

[providers.tf-vm]
label = "Terraform VM"
params = [
  { name = "workspace_dir", prompt = "Terraform directory" },
  { name = "instance_type", prompt = "Instance type", default = "t3.large" },
]
env = ["AWS_PROFILE", "AWS_REGION"]

create = """
cd "$HUMATA_WORKSPACE_DIR" && \
terraform workspace select -or-create {name} && \
terraform apply -auto-approve \
  -var="name={name}" \
  -var="instance_type=$HUMATA_INSTANCE_TYPE"
"""
identity = """
cd "$HUMATA_WORKSPACE_DIR" && \
terraform workspace select {name} >/dev/null && \
host=$(terraform output -raw ssh_host) && \
user=$(terraform output -raw ssh_user) && \
key_path=$(terraform output -raw ssh_key_path) && \
jq -n \
  --arg name {name} \
  --arg host "$host" \
  --arg user "$user" \
  --arg key "$key_path" \
  '{name: $name, ssh: {host: $host, port: 22, user: $user, identity_file: $key}, os: "linux", arch: "x86_64"}'
"""
status = """
cd "$HUMATA_WORKSPACE_DIR" && \
terraform workspace select {name} >/dev/null && \
terraform output -raw state
"""
delete = """
cd "$HUMATA_WORKSPACE_DIR" && \
terraform workspace select {name} && \
terraform destroy -auto-approve -var="name={name}"
"""
"#;

const SSH_FLEET_EXAMPLE_TOML: &str = r#"# SSH-only fleet provider: discovers and connects to an existing inventory of
# Linux machines over SSH using `list` and `identity` (no `create` needed).

[providers.ssh-fleet]
label = "SSH Fleet"
params = [
  { name = "inventory", prompt = "Inventory JSON path", default = "~/.config/fleet/hosts.json" },
]

list = """
jq -c '[.[] | {name: .name, state: (.state // "running"), ssh: {host: .host, port: (.port // 22), user: .user, identity_file: .identity_file}}]' "$HOME/.config/fleet/hosts.json"
"""
identity = """
jq -ce --arg name {name} '
  map(select(.name == $name)) | first // error("host not found") |
  {
    name: .name,
    ssh: {host: .host, port: (.port // 22), user: .user, identity_file: .identity_file},
    os: (.os // "linux"),
    arch: (.arch // "x86_64")
  }
' "$HUMATA_INVENTORY"
"""
status = "echo running"
"#;

const EXAMPLES: [ProviderExample; 4] = [
    ProviderExample {
        kind: "lima",
        aliases: &[],
        summary: "Built-in Lima local Linux VM provider using `connect` (MP-22).",
        toml: LIMA_TOML,
    },
    ProviderExample {
        kind: "cloud",
        aliases: &["cloud-cli", "cloud_cli"],
        summary: "Generic cloud CLI provider with `params`, `env`, `connect`, `status`, and `list`.",
        toml: CLOUD_EXAMPLE_TOML,
    },
    ProviderExample {
        kind: "terraform",
        aliases: &["tf"],
        summary: "Terraform workspace provider using `identity` JSON output.",
        toml: TERRAFORM_EXAMPLE_TOML,
    },
    ProviderExample {
        kind: "ssh",
        aliases: &["ssh-fleet", "ssh_fleet", "fleet"],
        summary: "SSH-only fleet provider using `list` + `identity` for existing hosts.",
        toml: SSH_FLEET_EXAMPLE_TOML,
    },
];

/// Every built-in example provider.
pub fn list_examples() -> &'static [ProviderExample] {
    &EXAMPLES
}

/// Error when an unknown example kind is requested.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown provider example {kind:?}; available examples: {}", available())]
pub struct ExampleError {
    pub kind: String,
}

fn available() -> String {
    EXAMPLES
        .iter()
        .map(|e| e.kind)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Looks up one example provider by kind or alias.
pub fn provider_example(kind: &str) -> Result<&'static ProviderExample, ExampleError> {
    let needle = kind.trim().to_ascii_lowercase();
    EXAMPLES
        .iter()
        .find(|e| e.kind == needle || e.aliases.contains(&needle.as_str()))
        .ok_or_else(|| ExampleError {
            kind: kind.to_owned(),
        })
}

/// Summary of one validated provider from `check_providers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckedProvider {
    pub id: String,
    pub label: String,
    pub reach: &'static str,
    pub actions: Vec<&'static str>,
    pub params: Vec<String>,
    pub env: Vec<String>,
    pub interactive: bool,
    pub wake_on_demand: bool,
    pub run_on: Option<String>,
}

/// One actionable validation problem from `check_providers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckProblem {
    pub provider: Option<String>,
    pub field: String,
    pub line: Option<usize>,
    pub message: String,
    pub fix: String,
}

/// Result of offline provider validation (`humata provider check`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub ok: bool,
    pub providers: Vec<CheckedProvider>,
    pub problems: Vec<CheckProblem>,
}

impl CheckReport {
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        if self.ok {
            let _ = writeln!(out, "OK: {} provider(s) valid.", self.providers.len());
            for p in &self.providers {
                let run_on = p.run_on.as_deref().unwrap_or("host Mac");
                let _ = writeln!(
                    out,
                    "  - {} ({}): reach={}, run_on={}, actions=[{}]",
                    p.id,
                    p.label,
                    p.reach,
                    run_on,
                    p.actions.join(", ")
                );
            }
        } else {
            let _ = writeln!(
                out,
                "Invalid provider file ({} problem(s)):",
                self.problems.len()
            );
            for prob in &self.problems {
                let loc = match (&prob.provider, prob.line) {
                    (Some(p), Some(l)) if !prob.field.is_empty() => {
                        format!("providers.{p}.{} (line {l})", prob.field)
                    }
                    (Some(p), None) if !prob.field.is_empty() => format!("providers.{p}.{}", prob.field),
                    (Some(p), _) => format!("providers.{p}"),
                    (None, Some(l)) => format!("{} (line {l})", prob.field),
                    (None, None) => prob.field.clone(),
                };
                let _ = writeln!(out, "  - {loc}: {}", prob.message);
                if !prob.fix.is_empty() {
                    let _ = writeln!(out, "    fix: {}", prob.fix);
                }
            }
        }
        out.trim_end().to_owned()
    }
}

fn fix_for_problem_kind(kind: &ProblemKind) -> &'static str {
    match kind {
        ProblemKind::BadProviderName => {
            "Rename the `[providers.<id>]` table to lowercase letters, digits, and `-`, starting with a letter."
        }
        ProblemKind::MissingLink => {
            "Add `connect` (with `{cmd}`), `identity` (prints SSH JSON), or `list` (prints instance JSON array)."
        }
        ProblemKind::ConnectWithoutCmd => {
            "Include `{cmd}` in `connect`, where Humata inserts the command it runs on the machine."
        }
        ProblemKind::CmdOutsideConnect => "Remove `{cmd}` from this field; only `connect` may use `{cmd}`.",
        ProblemKind::EmptyCommand => "Write a non-empty shell command, or remove the field.",
        ProblemKind::Nul => "Remove the NUL byte from the command string.",
        ProblemKind::BadParamName { .. } => {
            "Use lowercase letters, digits, and `_`, starting with a letter (max 32 chars)."
        }
        ProblemKind::DuplicateParam { .. } => "Remove the duplicate entry from `params`.",
        ProblemKind::ReservedParam => {
            "Rename this param; `name` is reserved for `{name}` and `$HUMATA_NAME`."
        }
        ProblemKind::BadEnvName { .. } => {
            "Use uppercase letters, digits, and `_`, not starting with a digit."
        }
        ProblemKind::ReservedEnvName { .. } => {
            "Remove this variable from `env`; Humata sets `HUMATA_*` and baseline variables itself."
        }
        ProblemKind::DuplicateEnv { .. } => "Remove the duplicate variable name from `env`.",
    }
}

/// Validates a providers TOML document offline with the real [`Provider::parse`].
pub fn check_providers(toml: &str) -> CheckReport {
    match Provider::parse(toml) {
        Ok(providers) if providers.is_empty() => CheckReport {
            ok: false,
            providers: Vec::new(),
            problems: vec![CheckProblem {
                provider: None,
                field: "providers".to_owned(),
                line: None,
                message: "the file defines no providers".to_owned(),
                fix: "Add at least one `[providers.<id>]` table. Run `humata provider example lima` for a working example."
                    .to_owned(),
            }],
        },
        Ok(providers) => {
            let checked = providers
                .into_iter()
                .map(|p| {
                    let reach = if p.has_connect() {
                        "connect"
                    } else if p.has_identity() {
                        "identity"
                    } else {
                        "list"
                    };
                    let actions = p.commands().into_iter().map(|(a, _)| a).collect();
                    let params = p.params().iter().map(|param| param.name.clone()).collect();
                    CheckedProvider {
                        id: p.id().to_owned(),
                        label: p.label().to_owned(),
                        reach,
                        actions,
                        params,
                        env: p.env().to_vec(),
                        interactive: p.interactive(),
                        wake_on_demand: p.wake_on_demand(),
                        run_on: p.run_on().map(str::to_owned),
                    }
                })
                .collect();
            CheckReport {
                ok: true,
                providers: checked,
                problems: Vec::new(),
            }
        }
        Err(ConfigError::Syntax { line, message, .. }) => CheckReport {
            ok: false,
            providers: Vec::new(),
            problems: vec![CheckProblem {
                provider: None,
                field: "toml".to_owned(),
                line: Some(line),
                message: format!("invalid TOML: {message}"),
                fix: "Fix the TOML syntax or unknown key at that line. Run `humata provider schema` for valid keys."
                    .to_owned(),
            }],
        },
        Err(ConfigError::Problems(problems)) => CheckReport {
            ok: false,
            providers: Vec::new(),
            problems: problems
                .0
                .into_iter()
                .map(|p| {
                    let fix = fix_for_problem_kind(&p.kind).to_owned();
                    CheckProblem {
                        provider: Some(p.provider),
                        field: p.field,
                        line: p.line,
                        message: p.kind.to_string(),
                        fix,
                    }
                })
                .collect(),
        },
    }
}

/// Which provider action to render offline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderAction {
    Create,
    Connect,
    Identity,
    List,
    Start,
    Stop,
    Wake,
    Refresh,
    Ssh,
    Delete,
    Status,
}

impl RenderAction {
    pub const ALL: [Self; 11] = [
        Self::Create,
        Self::Connect,
        Self::Identity,
        Self::List,
        Self::Start,
        Self::Stop,
        Self::Wake,
        Self::Refresh,
        Self::Ssh,
        Self::Delete,
        Self::Status,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "create" => Some(Self::Create),
            "connect" => Some(Self::Connect),
            "identity" => Some(Self::Identity),
            "list" | "discover" => Some(Self::List),
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "wake" => Some(Self::Wake),
            "refresh" => Some(Self::Refresh),
            "ssh" => Some(Self::Ssh),
            "delete" => Some(Self::Delete),
            "status" => Some(Self::Status),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Connect => "connect",
            Self::Identity => "identity",
            Self::List => "list",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Wake => "wake",
            Self::Refresh => "refresh",
            Self::Ssh => "ssh",
            Self::Delete => "delete",
            Self::Status => "status",
        }
    }

    fn to_action(self) -> Option<Action> {
        match self {
            Self::Create => Some(Action::Create),
            Self::Connect => None,
            Self::Identity => Some(Action::Identity),
            Self::List => Some(Action::List),
            Self::Start => Some(Action::Start),
            Self::Stop => Some(Action::Stop),
            Self::Wake => Some(Action::Wake),
            Self::Refresh => Some(Action::Refresh),
            Self::Ssh => Some(Action::Ssh),
            Self::Delete => Some(Action::Delete),
            Self::Status => Some(Action::Status),
        }
    }
}

/// One environment variable Humata would pass to the rendered command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderedEnv {
    pub name: String,
    pub value: Option<String>,
    pub source: &'static str,
}

/// Exact invocation Humata would execute for a provider action, without running it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderedAction {
    pub provider: String,
    pub action: &'static str,
    pub machine: String,
    pub run_on: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<RenderedEnv>,
    pub baseline_env: Vec<&'static str>,
    pub timeout_seconds: Option<u64>,
}

impl RenderedAction {
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "provider: {}", self.provider);
        let _ = writeln!(out, "action:   {}", self.action);
        let _ = writeln!(out, "machine:  {}", self.machine);
        let _ = writeln!(out, "run_on:   {}", self.run_on);
        if let Some(secs) = self.timeout_seconds {
            let _ = writeln!(out, "timeout:  {secs}s");
        }
        let _ = writeln!(out, "program:  {}", self.program);
        let _ = writeln!(out, "args:");
        for (i, arg) in self.args.iter().enumerate() {
            let _ = writeln!(out, "  [{i}] {arg}");
        }
        let _ = writeln!(out, "env:");
        if self.env.is_empty() {
            let _ = writeln!(out, "  (none beyond baseline)");
        } else {
            for var in &self.env {
                match &var.value {
                    Some(v) => {
                        let _ = writeln!(out, "  {}={v:?}", var.name);
                    }
                    None => {
                        let _ = writeln!(out, "  {}=<host variable>", var.name);
                    }
                }
            }
        }
        let _ = writeln!(out, "baseline_env: {}", self.baseline_env.join(", "));
        out.trim_end().to_owned()
    }
}

/// Why rendering a provider action failed.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("the provider file has problems:\n{}", .report.render_text())]
    InvalidConfig { report: CheckReport },
    #[error("provider {id:?} was not found in this file; available: {}", .available.join(", "))]
    UnknownProvider { id: String, available: Vec<String> },
    #[error("the file defines multiple providers ({}); pass --provider <id> to pick one", .available.join(", "))]
    AmbiguousProvider { available: Vec<String> },
    #[error(transparent)]
    BadMachineName(#[from] NameError),
    #[error(transparent)]
    BadValues(#[from] ValuesError),
    #[error("provider `{provider}` does not define `{action}`")]
    ActionUndefined { provider: String, action: &'static str },
    #[error("command cannot be quoted for POSIX sh: {0}")]
    Quote(#[from] crate::error::QuoteError),
}

/// Renders the exact invocation Humata would run for `action` without executing anything.
pub fn render_provider_action(
    toml: &str,
    provider_id: Option<&str>,
    action: RenderAction,
    machine: &str,
    params: &BTreeMap<String, String>,
    cmd: Option<&str>,
) -> Result<RenderedAction, RenderError> {
    let report = check_providers(toml);
    if !report.ok {
        return Err(RenderError::InvalidConfig { report });
    }
    let providers = Provider::parse(toml).expect("already validated");
    let provider = match provider_id {
        Some(id) => providers
            .iter()
            .find(|p| p.id() == id)
            .ok_or_else(|| RenderError::UnknownProvider {
                id: id.to_owned(),
                available: providers.iter().map(|p| p.id().to_owned()).collect(),
            })?,
        None if providers.len() == 1 => &providers[0],
        None => {
            return Err(RenderError::AmbiguousProvider {
                available: providers.iter().map(|p| p.id().to_owned()).collect(),
            });
        }
    };

    let machine_name = MachineName::new(machine)?;
    let invocation = match action {
        RenderAction::List => provider
            .list_invocation()
            .ok_or_else(|| RenderError::ActionUndefined {
                provider: provider.id().to_owned(),
                action: action.as_str(),
            })?,
        RenderAction::Connect => {
            let link = provider
                .link(&machine_name, params)?
                .ok_or_else(|| RenderError::ActionUndefined {
                    provider: provider.id().to_owned(),
                    action: action.as_str(),
                })?;
            let remote = cmd.unwrap_or("uname -sm");
            link.script(remote, Purpose::Probe, None, None)?
        }
        other => {
            let real_action = other.to_action().expect("non-connect action");
            provider
                .invocation(real_action, &machine_name, params)?
                .ok_or_else(|| RenderError::ActionUndefined {
                    provider: provider.id().to_owned(),
                    action: action.as_str(),
                })?
        }
    };

    let env = invocation
        .env
        .into_iter()
        .map(|(name, value)| match value {
            EnvValue::Value(v) => RenderedEnv {
                name,
                value: Some(v),
                source: "humata",
            },
            EnvValue::HostVariable(_) => RenderedEnv {
                name,
                value: None,
                source: "host_variable",
            },
        })
        .collect();

    Ok(RenderedAction {
        provider: provider.id().to_owned(),
        action: action.as_str(),
        machine: machine_name.as_str().to_owned(),
        run_on: provider
            .run_on()
            .map(str::to_owned)
            .unwrap_or_else(|| "host (This Mac)".to_owned()),
        program: invocation.program,
        args: invocation.args,
        env,
        baseline_env: BASELINE.to_vec(),
        timeout_seconds: invocation.timeout.map(|d| d.as_secs()),
    })
}

/// Which provider command output format to validate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParseKind {
    Identity,
    Status,
    Discover,
}

impl ParseKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "identity" => Some(Self::Identity),
            "status" => Some(Self::Status),
            "discover" | "list" => Some(Self::Discover),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Status => "status",
            Self::Discover => "discover",
        }
    }
}

/// Serializable SSH target summary for `parse_provider_output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParsedSsh {
    pub host: String,
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_key_fingerprint: Option<String>,
}

impl ParsedSsh {
    fn from_target(target: &SshTarget) -> Self {
        let (mode, ssh_config, port, user, identity_file) = match &target.access {
            SshAccess::Config { ssh_config } => {
                ("ssh_config", Some(ssh_config.clone()), None, None, None)
            }
            SshAccess::Direct {
                port,
                user,
                identity_file,
            } => (
                "direct",
                None,
                Some(*port),
                Some(user.clone()),
                identity_file.clone(),
            ),
        };
        Self {
            host: target.host.clone(),
            mode,
            ssh_config,
            port,
            user,
            identity_file,
            host_key_fingerprint: target
                .host_key
                .as_ref()
                .map(|k| k.fingerprint().to_owned()),
        }
    }
}

fn status_name(status: &MachineStatus) -> String {
    match status {
        MachineStatus::Running => "running".to_owned(),
        MachineStatus::Sleeping => "sleeping".to_owned(),
        MachineStatus::Stopped => "stopped".to_owned(),
        MachineStatus::Starting => "starting".to_owned(),
        MachineStatus::Missing => "missing".to_owned(),
        MachineStatus::Other(s) => format!("other({s})"),
    }
}

/// Serializable instance from `parse_provider_output(ParseKind::Discover, ...)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParsedInstance {
    pub name: String,
    pub state: String,
    pub params: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh: Option<ParsedSsh>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

/// Validated output from `parse_provider_output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParsedOutput {
    Identity {
        name: String,
        os: String,
        arch: String,
        ssh: ParsedSsh,
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        instance_id: Option<String>,
    },
    Status {
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        instance_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ssh: Option<ParsedSsh>,
    },
    Discover {
        instances: Vec<ParsedInstance>,
    },
}

impl ParsedOutput {
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        match self {
            Self::Identity {
                name,
                os,
                arch,
                ssh,
                state,
                instance_id,
            } => {
                let _ = writeln!(out, "OK: valid identity output for `{name}`");
                let _ = writeln!(out, "  platform: {os} {arch}");
                let _ = writeln!(out, "  ssh.host: {} ({})", ssh.host, ssh.mode);
                if let Some(cfg) = &ssh.ssh_config {
                    let _ = writeln!(out, "  ssh.ssh_config: {cfg}");
                }
                if let Some(user) = &ssh.user {
                    let _ = writeln!(
                        out,
                        "  ssh.user: {user} (port {})",
                        ssh.port.unwrap_or(22)
                    );
                }
                if let Some(id_file) = &ssh.identity_file {
                    let _ = writeln!(out, "  ssh.identity_file: {id_file}");
                }
                if let Some(fp) = &ssh.host_key_fingerprint {
                    let _ = writeln!(out, "  ssh.host_key: {fp}");
                }
                if let Some(st) = state {
                    let _ = writeln!(out, "  state: {st}");
                }
                if let Some(id) = instance_id {
                    let _ = writeln!(out, "  instance_id: {id}");
                }
            }
            Self::Status {
                state,
                instance_id,
                ssh,
            } => {
                let _ = writeln!(out, "OK: valid status output");
                let _ = writeln!(out, "  state: {state}");
                if let Some(id) = instance_id {
                    let _ = writeln!(out, "  instance_id: {id}");
                }
                if let Some(ssh) = ssh {
                    let _ = writeln!(out, "  ssh.host: {} ({})", ssh.host, ssh.mode);
                }
            }
            Self::Discover { instances } => {
                let _ = writeln!(
                    out,
                    "OK: valid discover output ({} instance(s))",
                    instances.len()
                );
                for inst in instances {
                    let _ = writeln!(out, "  - {} ({})", inst.name, inst.state);
                }
            }
        }
        out.trim_end().to_owned()
    }
}

/// Actionable error when `parse_provider_output` rejects command output.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ParseOutputError {
    pub kind: &'static str,
    pub field: Option<String>,
    pub message: String,
    pub fix: String,
}

impl ParseOutputError {
    fn from_identity_error(kind: ParseKind, error: IdentityError) -> Self {
        let (field, fix) = match &error {
            IdentityError::KeyMaterial => (
                Some("output".to_owned()),
                "Print file paths (`ssh.identity_file`, `ssh.ssh_config`), never private key contents."
                    .to_owned(),
            ),
            IdentityError::NotJson { .. } => (
                Some("output".to_owned()),
                match kind {
                    ParseKind::Discover => {
                        "Print a single JSON array (`[{ \"name\": ..., \"state\": ... }]`) to stdout and send diagnostics to stderr."
                            .to_owned()
                    }
                    _ => {
                        "Print a single JSON object to stdout (for example with `jq -n`) and send diagnostics to stderr."
                            .to_owned()
                    }
                },
            ),
            IdentityError::Field { field, .. } => (
                Some(field.clone()),
                "Fix that field in your command's stdout. Run `humata provider schema` for the exact format."
                    .to_owned(),
            ),
        };
        Self {
            kind: kind.as_str(),
            field,
            message: error.to_string(),
            fix,
        }
    }
}

/// Validates stdout from an `identity`, `status`, or `list`/`discover` command
/// using Humata's real parsers.
pub fn parse_provider_output(
    kind: ParseKind,
    input: &[u8],
    machine: Option<&str>,
) -> Result<ParsedOutput, ParseOutputError> {
    match kind {
        ParseKind::Identity => {
            let expected = match machine {
                Some(name) => MachineName::new(name).map_err(|e| ParseOutputError {
                    kind: kind.as_str(),
                    field: Some("name".to_owned()),
                    message: e.to_string(),
                    fix: "Pass a valid machine name with `--name <machine>`.".to_owned(),
                })?,
                None => {
                    // When --name is omitted, adopt the JSON's own valid `name` so
                    // parse_identity still validates every field, key-material rule,
                    // ssh object, os, arch, and host_key.
                    serde_json::from_slice::<serde_json::Value>(input)
                        .ok()
                        .and_then(|v| v.get("name")?.as_str().map(str::to_owned))
                        .and_then(|s| MachineName::new(&s).ok())
                        .unwrap_or_else(|| MachineName::new("example").expect("valid"))
                }
            };
            let parsed = parse_identity(input, &expected)
                .map_err(|e| ParseOutputError::from_identity_error(kind, e))?;
            Ok(ParsedOutput::Identity {
                name: expected.as_str().to_owned(),
                os: parsed.platform.os.to_string(),
                arch: parsed.platform.arch.to_string(),
                ssh: ParsedSsh::from_target(&parsed.ssh),
                state: parsed.state.as_ref().map(status_name),
                instance_id: parsed.instance_id,
            })
        }
        ParseKind::Status => {
            let report = parse_status_report(input)
                .map_err(|e| ParseOutputError::from_identity_error(kind, e))?;
            Ok(ParsedOutput::Status {
                state: status_name(&report.state),
                instance_id: report.instance_id,
                ssh: report.ssh.as_ref().map(ParsedSsh::from_target),
            })
        }
        ParseKind::Discover => {
            let instances = parse_list(input)
                .map_err(|e| ParseOutputError::from_identity_error(kind, e))?;
            Ok(ParsedOutput::Discover {
                instances: instances
                    .into_iter()
                    .map(|inst| ParsedInstance {
                        name: inst.name,
                        state: status_name(&inst.state),
                        params: inst.params,
                        ssh: inst.ssh.as_ref().map(ParsedSsh::from_target),
                        instance_id: inst.instance_id,
                    })
                    .collect(),
            })
        }
    }
}
