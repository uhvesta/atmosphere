//! Settings › Machines › Providers (PROVIDERS), headless: Commands opens a
//! row in place, Remove is refused while machines use the provider, and a
//! pasted provider file is reviewed before it is imported. The frames are
//! A at rest, B a row open with a variable, C an import review, D a file
//! with problems and E a blocked Remove (new goldens, for Fable to approve).
use super::*;
use humata2_clients::machines::{
    ProviderCommand, ProviderEntry, ProviderImportPlan, ProviderParam, ProviderProblem, ProviderVariable,
};
use humata2_settings::{ProvidersAction as P, Section, SettingsAction};

fn lima() -> ProviderEntry {
    ProviderEntry {
        id: "lima".to_owned(),
        label: "Lima VM".to_owned(),
        built_in: true,
        actions: ["create", "start", "stop", "delete"].map(str::to_owned).to_vec(),
        commands: vec![ProviderCommand {
            action: "create".to_owned(),
            command: "limactl create --name {name} --cpus {cpus} template://default".to_owned(),
        }],
        params: vec![ProviderParam {
            name: "cpus".to_owned(),
            label: "CPUs".to_owned(),
            default: "2".to_owned(),
            required: false,
        }],
        variables: vec![],
        machines: vec![],
    }
}

fn hcloud(machines: &[&str]) -> ProviderEntry {
    ProviderEntry {
        id: "hcloud".to_owned(),
        label: "Hetzner Cloud".to_owned(),
        built_in: false,
        actions: ["create", "connect", "delete"].map(str::to_owned).to_vec(),
        commands: vec![
            ProviderCommand {
                action: "create".to_owned(),
                command: "hcloud server create --name {name} --type {type} --image ubuntu-24.04".to_owned(),
            },
            ProviderCommand {
                action: "connect".to_owned(),
                command: "ssh root@$(hcloud server ip {name}) {cmd}".to_owned(),
            },
            ProviderCommand { action: "delete".to_owned(), command: "hcloud server delete {name}".to_owned() },
        ],
        params: vec![ProviderParam {
            name: "type".to_owned(),
            label: "Server type".to_owned(),
            default: "cx22".to_owned(),
            required: false,
        }],
        variables: vec![
            ProviderVariable { name: "HCLOUD_TOKEN".to_owned(), set: false },
            ProviderVariable { name: "SSH_KEY_NAME".to_owned(), set: true },
        ],
        machines: machines.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn open(providers: Vec<ProviderEntry>) -> App {
    let mut app = App::new(fixture::design());
    app.fakes.machines.set_providers(providers);
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(Section::Machines)));
    app
}

/// Scrolls Settings to its end, where the Providers block and its
/// expanded rows sit below the fold in the test window.
fn to_end(app: &mut App) {
    app.draw();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.scroll("settings-content", gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))), cx);
        })
        .expect("window stays open");
    app.settle();
    app.draw();
}

fn providers(app: &App) -> humata2_settings::ProvidersState {
    app.ui.read(|state| state.settings.providers.clone())
}

#[test]
fn providers_rest_above_scripts_with_commands_on_every_row() {
    let mut app = open(vec![lima(), hcloud(&[])]);
    assert!(app.contains("settings-provider-commands lima"));
    assert!(app.contains("settings-provider-commands hcloud"));
    // Only an imported provider can be removed.
    assert!(!app.contains("settings-provider-remove lima"));
    assert!(app.contains("settings-provider-remove hcloud"));
    app.snapshot("providers-a-at-rest");
}

#[test]
fn commands_open_a_row_in_place_with_its_variables() {
    let mut app = open(vec![lima(), hcloud(&[])]);
    to_end(&mut app);
    app.click("settings-provider-commands hcloud");
    to_end(&mut app);
    assert_eq!(providers(&app).open.as_deref(), Some("hcloud"));
    assert!(app.contains("settings-provider-variable HCLOUD_TOKEN"));
    // Clear shows only for a variable that has a value.
    assert!(app.contains("settings-provider-clear SSH_KEY_NAME"));
    assert!(!app.contains("settings-provider-clear HCLOUD_TOKEN"));
    app.snapshot("providers-b-row-open");
    app.click("settings-provider-commands lima");
    to_end(&mut app);
    assert_eq!(providers(&app).open.as_deref(), Some("lima"));
    assert!(!app.contains("settings-provider-variable HCLOUD_TOKEN"));
}

#[test]
fn a_pasted_file_is_reviewed_then_imported() {
    let mut app = open(vec![lima()]);
    to_end(&mut app);
    app.fakes.machines.set_import_plan(Ok(ProviderImportPlan {
        providers: vec![hcloud(&[])],
        replaces: vec![],
        problems: vec![],
        token: Some("pl_1".to_owned()),
    }));
    app.click("settings-provider-import-open");
    to_end(&mut app);
    app.send(AppAction::Settings(SettingsAction::Providers(P::ImportText(
        "[providers.hcloud]\nlabel = \"Hetzner Cloud\"\n# reads HCLOUD_TOKEN and SSH_KEY_NAME\ncreate = 'hcloud server create --name {name} --type {type} --image ubuntu-24.04'\ndelete = 'hcloud server delete {name}'\nconnect = 'ssh root@$(hcloud server ip {name}) {cmd}'\nparams = [{ name = \"type\", prompt = \"Server type\", default = \"cx22\" }]\n".to_owned(),
    ))));
    to_end(&mut app);
    app.click("settings-provider-import-review");
    to_end(&mut app);
    assert!(app.contains("settings-provider-import-confirm"));
    app.snapshot("providers-c-import-review");
    app.click("settings-provider-import-confirm");
    to_end(&mut app);
    let after = providers(&app);
    assert!(after.import.is_none());
    assert!(after.provider("hcloud").is_some());
}

#[test]
fn a_file_with_problems_says_why_under_the_field() {
    let mut app = open(vec![lima()]);
    to_end(&mut app);
    app.fakes.machines.set_import_plan(Ok(ProviderImportPlan {
        problems: vec![ProviderProblem {
            provider: "hcloud".to_owned(),
            field: String::new(),
            line: 0,
            message: "hcloud has no create command, so Humata can't make its machines. Add `create = '<command>'` and press Review again.".to_owned(),
        }],
        ..ProviderImportPlan::default()
    }));
    app.click("settings-provider-import-open");
    to_end(&mut app);
    app.send(AppAction::Settings(SettingsAction::Providers(P::ImportText(
        "[providers.hcloud]\nlabel = \"Hetzner Cloud\"\n".to_owned(),
    ))));
    to_end(&mut app);
    app.click("settings-provider-import-review");
    to_end(&mut app);
    assert!(app.contains("settings-provider-import-problem"));
    assert!(!app.contains("settings-provider-import-confirm"));
    app.snapshot("providers-d-parse-error");
}

#[test]
fn remove_is_blocked_while_machines_use_the_provider() {
    let mut app = open(vec![lima(), hcloud(&["box-1", "box-2"])]);
    app.click("settings-provider-remove hcloud");
    to_end(&mut app);
    assert!(app.contains("settings-provider-remove-blocked"));
    assert!(!app.contains("settings-provider-remove-confirm"));
    app.snapshot("providers-e-remove-blocked");
}
