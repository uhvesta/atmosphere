//! Thin plugin crate wiring `humata2_svc_catalog::agent_service` into
//! `humatad_core` without pulling `svc_catalog` into `humatad_core`.

use humata2_daemon_runtime::agent_switches::OnChange;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Agents;
use humata2_svc_catalog::agent_service::AgentSvc;
use humatad_core::plugins::{CatalogPlugin as CatalogPluginTrait, RegisterRoutes};

pub struct CatalogPlugin;

impl CatalogPluginTrait for CatalogPlugin {
    fn build(&self, agents: Agents, use_builtin: bool) -> (OnChange, RegisterRoutes) {
        let svc = AgentSvc::new(agents, use_builtin);
        let on_change = svc.on_change();
        let register: RegisterRoutes = Box::new(move |routes| {
            routes.add_service(pb::agent_service_server::AgentServiceServer::new(svc));
        });
        (on_change, register)
    }
}
