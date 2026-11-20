//! Carrying a session or fork to another machine or landing, and checking a
//! brought side copy.

mod bring;
mod carry;

use std::path::{Path, PathBuf};

use humata2_revive::Env;
use humata2_sessions_core::{StartError, Started, Starter};
use humata2_store::Take;

pub use bring::{Brought, check_brought};
pub use carry::{
    CARRY_DIR, CHANNELS_ROOT, CHUNKED_FROM, CHUNKS_DIR, CHUNKS_FILE, CLONES_FILE, CarryError, Discard, ForkPack,
    ForkPackError, ForkPacked, Landing, Packed, REUSE_FILE, Unpacked, chunk_store_dir, frame_chunk, held_chunks,
};

pub(crate) struct CarryOps<'a>(pub(crate) &'a Starter);

impl std::ops::Deref for CarryOps<'_> {
    type Target = Starter;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

/// Carry operations on [`Starter`].
pub trait CarryStarter {
    fn pack(&self, operation: &str, session: &str) -> Result<Packed, CarryError>;
    fn pack_for(&self, operation: &str, session: &str, target: Option<&Path>) -> Result<Packed, CarryError>;
    fn shell_holders(&self, session: &str) -> Result<Vec<(PathBuf, u32)>, CarryError>;
    fn unpack(&self, operation: &str, session: &str, channel: &str, take: Take) -> Result<Unpacked, CarryError>;
    fn abort_incoming(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        folder: &str,
        take: Take,
    ) -> Result<(), CarryError>;
    fn discards(&self, session: &str) -> Result<Vec<Discard>, CarryError>;
    fn forget_moved(&self, operation: &str, session: &str, channel: &str, take: Take) -> Result<(), CarryError>;
    fn unstage(&self, operation: &str, channel: &str) -> Result<(), CarryError>;
    fn pack_fork(&self, request: &ForkPack) -> Result<ForkPacked, ForkPackError>;
    fn launch_unlaunched<'a>(
        &'a self,
        session: &'a str,
        cols: u16,
        rows: u16,
        env: Env<'a>,
        progress: &'a mut (dyn FnMut(&str) + Send),
    ) -> impl std::future::Future<Output = Result<Started, StartError>> + Send + 'a;
    fn pack_landing(&self, operation: &str, session: &str, landing: &Landing) -> Result<Packed, CarryError>;
    fn check_landing(&self, session: &str) -> Result<(), CarryError>;
    fn left_behind(&self, operation: &str, channel: &str) -> Result<Vec<Discard>, CarryError>;
    fn land_copy(&self, operation: &str, session: &str, channel: &str, packed: &Packed) -> Result<PathBuf, CarryError>;
    fn resync_landing(&self, operation: &str, session: &str, landing: &Landing) -> Result<(), CarryError>;
    fn unpack_landed(&self, operation: &str, session: &str, channel: &str) -> Result<Unpacked, CarryError>;
    fn take_landing_back(&self, operation: &str, session: &str, channel: &str, unpacked: bool) -> Result<(), CarryError>;
    fn carry_target(&self, channel: &str, rel: &str) -> Result<PathBuf, CarryError>;
}

impl CarryStarter for Starter {
    fn pack(&self, operation: &str, session: &str) -> Result<Packed, CarryError> {
        CarryOps(self).pack(operation, session)
    }

    fn pack_for(&self, operation: &str, session: &str, target: Option<&Path>) -> Result<Packed, CarryError> {
        CarryOps(self).pack_for(operation, session, target)
    }

    fn shell_holders(&self, session: &str) -> Result<Vec<(PathBuf, u32)>, CarryError> {
        CarryOps(self).shell_holders(session)
    }

    fn unpack(&self, operation: &str, session: &str, channel: &str, take: Take) -> Result<Unpacked, CarryError> {
        CarryOps(self).unpack(operation, session, channel, take)
    }

    fn abort_incoming(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        folder: &str,
        take: Take,
    ) -> Result<(), CarryError> {
        CarryOps(self).abort_incoming(operation, session, channel, folder, take)
    }

    fn discards(&self, session: &str) -> Result<Vec<Discard>, CarryError> {
        CarryOps(self).discards(session)
    }

    fn forget_moved(&self, operation: &str, session: &str, channel: &str, take: Take) -> Result<(), CarryError> {
        CarryOps(self).forget_moved(operation, session, channel, take)
    }

    fn unstage(&self, operation: &str, channel: &str) -> Result<(), CarryError> {
        CarryOps(self).unstage(operation, channel)
    }

    fn pack_fork(&self, request: &ForkPack) -> Result<ForkPacked, ForkPackError> {
        CarryOps(self).pack_fork(request)
    }

    fn launch_unlaunched<'a>(
        &'a self,
        session: &'a str,
        cols: u16,
        rows: u16,
        env: Env<'a>,
        progress: &'a mut (dyn FnMut(&str) + Send),
    ) -> impl std::future::Future<Output = Result<Started, StartError>> + Send + 'a {
        async move { CarryOps(self).launch_unlaunched(session, cols, rows, env, progress).await }
    }

    fn pack_landing(&self, operation: &str, session: &str, landing: &Landing) -> Result<Packed, CarryError> {
        CarryOps(self).pack_landing(operation, session, landing)
    }

    fn check_landing(&self, session: &str) -> Result<(), CarryError> {
        CarryOps(self).check_landing(session)
    }

    fn left_behind(&self, operation: &str, channel: &str) -> Result<Vec<Discard>, CarryError> {
        CarryOps(self).left_behind(operation, channel)
    }

    fn land_copy(&self, operation: &str, session: &str, channel: &str, packed: &Packed) -> Result<PathBuf, CarryError> {
        CarryOps(self).land_copy(operation, session, channel, packed)
    }

    fn resync_landing(&self, operation: &str, session: &str, landing: &Landing) -> Result<(), CarryError> {
        CarryOps(self).resync_landing(operation, session, landing)
    }

    fn unpack_landed(&self, operation: &str, session: &str, channel: &str) -> Result<Unpacked, CarryError> {
        CarryOps(self).unpack_landed(operation, session, channel)
    }

    fn take_landing_back(&self, operation: &str, session: &str, channel: &str, unpacked: bool) -> Result<(), CarryError> {
        CarryOps(self).take_landing_back(operation, session, channel, unpacked)
    }

    fn carry_target(&self, channel: &str, rel: &str) -> Result<PathBuf, CarryError> {
        CarryOps(self).carry_target(channel, rel)
    }
}
