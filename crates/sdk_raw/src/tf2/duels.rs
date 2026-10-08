//! Hand-written ABI of the jobs through which TF2's game server handles the
//! Steam Game Coordinator's (GC) messages about duels: the vtable slots of
//! `CJob::BYieldingRunJobFromMsg`, which runs a job for a message, and of
//! `CGCClientJob::BYieldingRunGCJob`, which each duel job overrides, their
//! signature, and the search for the jobs' vtables.
//!
//! The GC tells the server of a Dueling Mini-Game's challenge
//! (`k_EMsgGC_Duel_Request`), and of the challenged player's answer
//! (`k_EMsgGC_Duel_Response`). The server's GC client creates a job of the
//! class `tf_duel.cpp` registers for each message, [`DUEL_REQUEST_CLASS`] and
//! [`DUEL_RESPONSE_CLASS`], and its job manager runs the job through
//! `BYieldingRunJobFromMsg`, which `CGCClientJob` implements by calling the
//! job's `BYieldingRunGCJob`. Neither yields: `CGCClientJob` only does without
//! a GC client, which its constructor requires, and the duel jobs wait on
//! nothing.

use crate::abi::CppDestructors;
use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use std::ffi::c_void;
use std::ptr::NonNull;

/// The signature of `CJob::BYieldingRunJobFromMsg`, `bool (IMsgNetPacket *)`,
/// with the job as its receiver and the message that created it as `packet`,
/// which returns whether the job succeeded. The overload of
/// `CGCClientJob::BYieldingRunGCJob` taking a packet has the same signature.
#[doc(alias("BYieldingRunJobFromMsg", "BYieldingRunGCJob"))]
pub type RunJobFromMsgFn = unsafe extern "C" fn(this: *mut c_void, packet: *mut c_void) -> bool;

// `CJob` (`public/gcsdk/job.h`) declares its virtual methods in this order: its
// destructor, `Validate`, `BYieldingRunJob(void *)`,
// `BYieldingRunJobFromMsg(IMsgNetPacket *)` and `CHeartbeatsBeforeTimeout()`.
// `Validate` only exists under `DBGFLAG_VALIDATE`, which `tier0/dbgflag.h`
// defines only in Steam's own debug or `RELEASEASSERTS` builds. `CGCClientJob`
// (`public/gcsdk/gcclientjob.h`) then adds `BYieldingRunGCJob(IMsgNetPacket *)`,
// `BYieldingRunGCJob()` and `GetServerType()`, and the duel jobs add none.
//
// On Linux, the Itanium ABI's two destructor slots put
// `BYieldingRunJobFromMsg` at 3, `CHeartbeatsBeforeTimeout` at 4, and the
// packet's `BYieldingRunGCJob` at 5. On Windows, MSVC's one destructor slot
// puts `BYieldingRunJobFromMsg` at 2 and `CHeartbeatsBeforeTimeout` at 3, and
// MSVC emits a class's new overloads in reverse declaration order, so
// `BYieldingRunGCJob()` takes 4 and the packet's 5. No binary confirmed these,
// so `metamod_source`'s duel hooks check that the first slot holds one function
// in both duel jobs' vtables, and the second a function of each job's own.
const _: () = assert!(RUN_JOB_FROM_MSG_SLOT == CppDestructors::VTABLE_SLOTS + 1);

/// The name of the class of the job handling a challenge to a duel
/// (`k_EMsgGC_Duel_Request`), in the run-time type information.
pub const DUEL_REQUEST_CLASS: &str = "CGC_GameServer_Duel_Request";

/// The name of the class of the job handling the answer to a challenge
/// (`k_EMsgGC_Duel_Response`), in the run-time type information.
pub const DUEL_RESPONSE_CLASS: &str = "CGC_GameServer_Duel_Response";

/// The slot of `CGCClientJob::BYieldingRunGCJob(IMsgNetPacket *)` in a GC
/// client job's primary vtable, the same on both ABIs. Each duel job
/// overrides it with its own.
#[doc(alias("BYieldingRunGCJob"))]
pub const RUN_GC_JOB_SLOT: usize = 5;

/// The slot of `CJob::BYieldingRunJobFromMsg` in a job's primary vtable.
/// Every job of the GC client, the duel jobs included, inherits
/// `CGCClientJob`'s.
#[doc(alias("BYieldingRunJobFromMsg"))]
pub const RUN_JOB_FROM_MSG_SLOT: usize = cfg_select! {
	target_os = "windows" => 2,
	target_os = "linux" => 3,
};

/// Finds the unique primary vtables of [`DUEL_REQUEST_CLASS`] and
/// [`DUEL_RESPONSE_CLASS`], in that order, whose [`RUN_GC_JOB_SLOT`] entries
/// are executable, from the run-time type information of the module whose
/// `CreateInterface` export is `factory`, such as the game server module. Each
/// is `None` if there is no such table, or more than one. The module is
/// snapshot and searched once for both.
///
/// The search does not check that the classes derive from `CGCClientJob`, so
/// that the slots hold its methods: any class with that many virtual methods
/// passes. The addresses are metadata from the snapshot: they do not keep the
/// module loaded, and the tables are the classes' only while it stays loaded.
///
/// # Safety
///
/// `factory` must be the `CreateInterface` export of a module that stays
/// loaded throughout this call.
pub unsafe fn find_duel_job_vtables(
	factory: CreateInterfaceFn,
) -> Result<[Option<NonNull<*mut c_void>>; 2], util::Error> {
	// SAFETY: The factory is an executable address in its module, which the
	// caller keeps loaded while it is inspected.
	let image = unsafe { Image::load(factory as usize) }?;
	let mut tables = image
		.primary_vtables(&[DUEL_REQUEST_CLASS, DUEL_RESPONSE_CLASS], RUN_GC_JOB_SLOT)
		.into_iter()
		.map(|table| NonNull::new(table? as *mut *mut c_void));

	Ok([tables.next().flatten(), tables.next().flatten()])
}
