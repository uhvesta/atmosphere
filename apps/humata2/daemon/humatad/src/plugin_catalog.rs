//! Thin plugin crate wiring `humata2_svc_catalog::agent_service` into
//! `humatad_core` without pulling `svc_catalog` into `humatad_core`.

use std::sync::Arc;

use humata2_access::{Call, Capability, Target};
use humata2_daemon_ports::{BoardAsk, Caller, Deferred};
use humata2_daemon_runtime::agent_switches::OnChange;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Agents;
use humata2_svc_catalog::agent_service::{AgentSettingsGate, AgentSvc, BoxFuture};
use humatad_core::guard::{Checked, Guard};
use humatad_core::inbox::Inbox;
use humatad_core::plugins::{CatalogPlugin as CatalogPluginTrait, RegisterRoutes};
use tonic::Status;

struct SettingsGate {
    guard: Arc<Guard>,
    inbox: Arc<Inbox>,
}

#[tonic::async_trait]
impl AgentSettingsGate for SettingsGate {
    async fn gate_settings_change(
        &self,
        caller: &Caller,
        key: Option<String>,
        scope: String,
        changes: Vec<(String, String, String)>,
        reason: String,
        type_name: &'static str,
        run: Box<dyn FnOnce() -> BoxFuture<Result<Vec<u8>, Status>> + Send>,
    ) -> Result<Vec<u8>, Status> {
        let call = Call::Use {
            capability: Capability::SettingsChange,
            target: Target::Global,
        };
        match self.guard.check(caller, &call)? {
            Checked::Allowed => run().await,
            Checked::Ask(asked) => {
                let notes = changes
                    .iter()
                    .map(|(field, before, after)| format!("{field}: {before} -> {after}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let card = BoardAsk {
                    preview: pb::request_preview::Preview::Command(pb::CommandPreview {
                        machine: None,
                        command: scope,
                        notes,
                    }),
                    reason,
                };
                let deferred: humata2_daemon_ports::Run = Box::new(move |_| {
                    Box::pin(async move {
                        let result = run().await?;
                        Ok(Deferred {
                            result,
                            result_type: type_name.to_owned(),
                        })
                    })
                });
                Err(self
                    .inbox
                    .defer_board(caller, asked, key, card, deferred)
                    .await)
            }
        }
    }
}

pub struct CatalogPlugin;

impl CatalogPluginTrait for CatalogPlugin {
    fn build(
        &self,
        agents: Agents,
        use_builtin: bool,
        guard: Arc<Guard>,
        inbox: Arc<Inbox>,
    ) -> (OnChange, RegisterRoutes) {
        let svc = AgentSvc::new(agents, use_builtin).with_gate(Arc::new(SettingsGate {
            guard,
            inbox,
        }));
        let on_change = svc.on_change();
        let register: RegisterRoutes = Box::new(move |routes| {
            routes.add_service(pb::agent_service_server::AgentServiceServer::new(svc));
        });
        (on_change, register)
    }
}
