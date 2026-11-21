//! `humata2_svc_machines`: MachineService (host + box storage/root/forget/scripts)
//! and the machine scripts run engine.

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_events::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_machines::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
}

pub mod machine_scripts;
pub mod machine_service;
pub mod ports;

pub use machine_scripts::{
    Context, LogError, MachineScripts, Outcome, Parts, Report, RunNowError, RunRecord,
    SessionContext, ShellOutput, StepError, StepRun, Steps, build, leaving,
};
pub use machine_service::{
    BoxMachineSvc, Clock, Forgetting, MachineSvc, Work, agents_changed, finish_forgets,
    follow_let_go, machine_id, start_layout,
};
