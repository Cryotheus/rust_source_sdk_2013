//! TF2's duels: the Dueling Mini-Game's challenges between two players, which
//! the Steam Game Coordinator (GC) relays to the game server, and where to
//! intercept them.
//!
//! A player uses a Dueling Mini-Game on another through the GC, which shows
//! the challenge to the players and tells the game server, then does the same
//! with the challenged player's answer. The server handles each message with a
//! job: the request's has the challenger speak, and the response's starts an
//! accepted duel, or has both players speak of a declined one. First, the
//! response's job checks that the game rules allow duels
//! (`CTFGameRules::CanInitiateDuels`), which they only do in a round's
//! pre-round or while it runs, and not while the game waits for players.
//! Otherwise, it only tells the GC that an accepted duel is cancelled. A
//! started duel has both players speak, and the server counts their kills and
//! assists on each other until it ends. One limited to a class also respawns
//! either player who plays another as that class.
//!
//! The job manager runs each job through `CJob::BYieldingRunJobFromMsg`, which
//! it calls through the job's vtable. [`sdk_raw::tf2::duels`] holds the
//! function's signature and vtable slot, and `metamod_source`'s `duel_hooks`
//! hook it in the classes [`duel_job_vtables`] finds.

use crate::{Game, Server};
use sdk_raw::tf2::duels::find_duel_job_vtables;
use sdk_raw::util;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The jobs through which the game server handles the GC's messages about a
/// duel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DuelJob {
	/// A player challenged another (`k_EMsgGC_Duel_Request`). The job has the
	/// challenger speak.
	#[doc(alias("CGC_GameServer_Duel_Request", "k_EMsgGC_Duel_Request"))]
	Request,

	/// The challenged player accepted or declined (`k_EMsgGC_Duel_Response`).
	/// The job starts an accepted duel, or has both players speak of a declined
	/// one.
	#[doc(alias("CGC_GameServer_Duel_Response", "k_EMsgGC_Duel_Response"))]
	Response,
}

impl DuelJob {
	/// Both jobs, in the order [`duel_job_vtables`] gives their vtables.
	pub const ALL: [Self; 2] = [Self::Request, Self::Response];
}

/// A duel job class's primary vtable in this server's game module. Intended
/// for the Metamod adapter; no job is retained.
#[derive(Debug, Clone, Copy)]
pub struct DuelJobVtable<'s> {
	/// The job whose class owns this vtable.
	pub job: DuelJob,
	vtable: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl DuelJobVtable<'_> {
	/// The vtable's address in the game module.
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

/// Why the duel jobs' vtables could not be found.
#[derive(Debug, thiserror::Error)]
pub enum DuelJobVtableError {
	/// The server does not run Team Fortress 2.
	#[error("duel jobs require Team Fortress 2")]
	WrongGame,

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,

	/// The job's class has no unique primary vtable in the game module.
	#[error("no unique primary vtable found for TF2's duel job {0:?}")]
	NotFound(DuelJob),
}

impl From<util::Error> for DuelJobVtableError {
	fn from(error: util::Error) -> Self {
		match error {
			util::Error::InvalidImage => Self::InvalidImage,
			util::Error::Io(error) => Self::Image(error),
		}
	}
}

/// Finds the vtables of both duel jobs' classes, in the order of
/// [`DuelJob::ALL`], through the game server module's run-time type
/// information, which needs no job. A class without a unique vtable is an
/// error. Snapshots the whole module and searches it once for both, so call it
/// once, such as while loading.
///
/// The search only checks that each vtable reaches
/// [`RUN_GC_JOB_SLOT`](sdk_raw::tf2::duels::RUN_GC_JOB_SLOT) and holds code
/// there, not that the class is the job it is named after.
pub fn duel_job_vtables(server: Server<'_>) -> Result<[DuelJobVtable<'_>; 2], DuelJobVtableError> {
	if server.game() != Game::TeamFortress2 {
		return Err(DuelJobVtableError::WrongGame);
	}

	// SAFETY: The game server factory is the game module's `CreateInterface`,
	// and the Server's callback scope keeps the module loaded while its sections
	// are inspected (`Server::new` condition 1).
	let [request, response] =
		unsafe { find_duel_job_vtables(server.game_server_factory().as_raw()) }?;

	let vtable = |job: DuelJob, vtable: Option<NonNull<*mut c_void>>| {
		vtable
			.map(|vtable| DuelJobVtable {
				job,
				vtable,
				_scope: PhantomData,
			})
			.ok_or(DuelJobVtableError::NotFound(job))
	};

	Ok([
		vtable(DuelJob::Request, request)?,
		vtable(DuelJob::Response, response)?,
	])
}
