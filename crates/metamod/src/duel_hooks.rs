//! TF2 duel hooks, which run before the game server's jobs for the Steam Game
//! Coordinator's (GC) messages about duels, and may refuse a duel.
//!
//! The GC tells the server of each challenge of a Dueling Mini-Game, and of
//! the challenged player's answer, and the server handles each with a job (see
//! [`source_sdk_2013::tf2::duels`]). The hooks run before the job's
//! `BYieldingRunJobFromMsg`, and their callback decides whether the game runs
//! it as usual. A refused request's job is skipped, so the challenger does not
//! speak. A refused response's job runs while the game rules claim to wait for
//! players, which it checks before starting a duel: it tells the GC that an
//! accepted duel is cancelled, and does nothing else. The claim is put back as
//! soon as the job returns. It is written without recording a change for
//! clients, and the job does not yield, so neither clients nor the rest of the
//! game see it.
//!
//! # When to install
//!
//! [`MetamodApi::hook_duels`] finds both jobs' classes in the game server
//! module through [`duel_job_vtables`], without any job, and hooks
//! `BYieldingRunJobFromMsg` in the vtable of each. Install once, such as while
//! loading: the hooks last until removed or the plugin unloads, and installing
//! again returns [`HookError::AlreadyInstalled`] without searching the module
//! again.
//!
//! # What gets through
//!
//! The GC shows the players the challenge and the answer itself, whatever the
//! callback decides, then the cancellation of a refused duel the challenged
//! player accepted. As with other Metamod hooks, the callback does not run
//! while the plugin is paused or after it unloads, so duels start as usual
//! then. A refused response whose game rules cannot be found, as on a game
//! whose networked tables changed, is skipped, so its duel is neither started
//! nor cancelled with the GC. With Metamod 2.0, KHook keeps a vtable entry it
//! detoured pointing to its own code for the rest of the process, and adds a
//! hook to such an entry from its worker thread, so the jobs run just after a
//! reload can pass unseen. Another plugin's hook on the same function can skip
//! a job before the callback runs, which KHook does not report.

#[cfg(test)]
#[path = "tests/duel_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::raw::tf2::duels::{
	RUN_GC_JOB_SLOT, RUN_JOB_FROM_MSG_SLOT, RunJobFromMsgFn as RunJobFromMsg,
};

use source_sdk_2013::tf2::duels::{DuelJob, DuelJobVtable, DuelJobVtableError, duel_job_vtables};
use source_sdk_2013::tf2::game_rules::GameRules;
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::{CStr, c_void};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;

/// A callback-scoped server and the duel job about to run, which decides
/// whether the game runs it as usual. A panic is contained by the hook
/// dispatcher, and lets the job run.
pub type DuelJobFn = for<'s> fn(Server<'s>, DuelJob) -> DuelAction;

/// Runs a job while the game rules claim to wait for players, then puts back
/// what they claimed. Returns whether it ran the job.
type WaitingFn = for<'s> fn(Server<'s>, &mut dyn FnMut()) -> bool;

/// The packet's `BYieldingRunGCJob` in a GC client job's primary vtable, of
/// the same signature.
const RUN_GC_JOB: VirtualFunction<RunJobFromMsg> = VirtualFunction::new(RUN_GC_JOB_SLOT);

/// `BYieldingRunJobFromMsg` in a job's primary vtable.
const RUN_JOB_FROM_MSG: VirtualFunction<RunJobFromMsg> =
	VirtualFunction::new(RUN_JOB_FROM_MSG_SLOT);

/// The game rules' flag of waiting for players, which
/// `CTFGameRules::CanInitiateDuels` checks first.
const WAITING_FOR_PLAYERS: &CStr = c"m_bInWaitingForPlayers";

/// The routes of the jobs, in the order of [`DuelJob::ALL`].
static ROUTES: [DuelRoute; 2] = [
	DuelRoute::new(DuelJob::Request),
	DuelRoute::new(DuelJob::Response),
];

/// What a duel hook does with a job.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DuelAction {
	/// Lets the game run the job.
	#[default]
	Allow,

	/// Refuses the duel. A request's job is skipped, so the challenger does not
	/// speak. A response's runs as if the game were waiting for players, so it
	/// tells the GC that an accepted duel is cancelled, and does nothing else:
	/// no one speaks, and no duel starts.
	Refuse,
}

/// Why the duel hooks could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum DuelHookError {
	/// The duel jobs' vtables could not be found.
	#[error(transparent)]
	Target(#[from] DuelJobVtableError),

	/// The vtables found are not laid out as the duel jobs' are: before any
	/// hook, [`RUN_JOB_FROM_MSG_SLOT`] does not hold the same function in both,
	/// or [`RUN_GC_JOB_SLOT`] does not hold two other functions, one in each.
	#[error("the duel jobs' vtables do not have the expected layout")]
	UnexpectedLayout,

	/// Metamod refused a hook. [`HookError::AlreadyInstalled`] means the duel
	/// jobs are already hooked.
	#[error(transparent)]
	Hook(#[from] HookError),
}

/// Handles for the duel hooks installed by one call.
///
/// Metamod disables them while paused and removes them before unloading the
/// plugin. [`Self::remove`] disables them earlier. No job is retained, so the
/// hooks last through level changes.
#[must_use = "retain duel hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct DuelHooks {
	hooks: Vec<HookId>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl DuelHooks {
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);

			for route in &ROUTES {
				if route.state.get().is_some_and(|state| state.hook == hook) {
					route.state.set(None);
				}
			}
		}
	}
}

struct DuelRoute {
	job: DuelJob,
	state: Cell<Option<RoutedDuel>>,
}

impl DuelRoute {
	const fn new(job: DuelJob) -> Self {
		Self {
			job,
			state: Cell::new(None),
		}
	}
}

impl Handler<RunJobFromMsg> for DuelRoute {
	fn call(&self, call: &HookCall<'_, RunJobFromMsg>) -> HookAction<bool> {
		// An earlier hook already skipped the job.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };

		match ((route.callback)(server, self.job), self.job) {
			(DuelAction::Allow, _) => HookAction::Ignore,
			(DuelAction::Refuse, DuelJob::Request) => HookAction::Supersede(true),

			(DuelAction::Refuse, DuelJob::Response) => {
				let (packet,) = call.args();
				// Skipped, unless it can run as if the game waited for players.
				let mut succeeded = true;

				(route.waiting)(server, &mut || {
					// SAFETY: The original is the unhooked `BYieldingRunJobFromMsg`
					// of the job's class, called on the main thread with the live
					// job and its message, as the job manager called it. The job
					// does not yield, so it returns before anything else runs.
					succeeded = unsafe { (route.original)(call.this(), packet) };
				});

				HookAction::Supersede(succeeded)
			}
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for DuelRoute {}

#[derive(Clone, Copy)]
struct RoutedDuel {
	binding: ServerBinding,
	callback: DuelJobFn,
	hook: HookId,
	original: RunJobFromMsg,
	waiting: WaitingFn,
}

impl MetamodApi<'_> {
	/// Whether the duel jobs are hooked, for this load of the plugin.
	fn duels_hooked(self) -> bool {
		ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| self.has_hook(state.hook))
		})
	}

	/// Runs `callback` before each of the jobs through which TF2's game server
	/// handles the GC's messages about a duel, to let the job run as usual or
	/// refuse the duel; see the [module documentation](crate::duel_hooks).
	///
	/// `binding` must describe the same running server as `server`. Finds both
	/// jobs' classes in the game server module, which needs no level to be
	/// loaded, and snapshots the module to do so: install once, such as while
	/// loading. Before hooking, checks that their vtables are laid out as the
	/// jobs' (see [`DuelHookError::UnexpectedLayout`]). Installing again while
	/// the hooks are installed returns [`HookError::AlreadyInstalled`] without
	/// searching the module again; removing them allows replacement.
	///
	/// The callback is not run once an earlier hook skipped the job, as far as
	/// the hooking library reports it (see [`crate::hook`]). It runs as the
	/// server's GC client runs its jobs, after the game's frame while a level
	/// runs, and must not delete entities immediately, as [`Server::new`]
	/// requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned handle means registration was accepted, not that the next
	/// job will be intercepted. KHook polls every 5 ms and can retry while a
	/// detour is busy, so jobs shortly after installation can be missed.
	///
	/// # Safety
	///
	/// The game module's classes named `CGC_GameServer_Duel_Request` and
	/// `CGC_GameServer_Duel_Response` must be TF2's duel jobs, which derive
	/// from `GCSDK::CGCClientJob` as the SDK's `gcclientjob.h` declares it, so
	/// that their vtables hold `BYieldingRunJobFromMsg` at
	/// [`RUN_JOB_FROM_MSG_SLOT`] and the packet's `BYieldingRunGCJob` at
	/// [`RUN_GC_JOB_SLOT`], both `bool (IMsgNetPacket *)`. The search only
	/// checks that each vtable holds code at the latter, and the layout check
	/// that the former holds the same function in both and the latter another
	/// in each, which other classes' vtables can too.
	pub unsafe fn hook_duels(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: DuelJobFn,
	) -> Result<DuelHooks, DuelHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(DuelJobVtableError::WrongGame.into());
		}

		// The classes' vtables are the same for the whole load.
		if self.duels_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		let vtables = duel_job_vtables(server)?.map(DuelJobVtable::as_ptr);

		// SAFETY: The caller promises that the classes found are TF2's duel
		// jobs, whose vtables hold `bool (IMsgNetPacket *)` at both slots. The
		// game module stays loaded until Metamod unloads this plugin.
		unsafe { self.install_duels(vtables, binding, callback, while_waiting_for_players) }
	}

	/// Hooks `BYieldingRunJobFromMsg` through each of `vtables`, in the order
	/// of [`DuelJob::ALL`], once they are found laid out as the duel jobs'.
	/// Refused responses run through `waiting`.
	///
	/// # Safety
	///
	/// Each vtable must be live, hold functions of the signature
	/// [`RunJobFromMsg`] at [`RUN_JOB_FROM_MSG_SLOT`] and [`RUN_GC_JOB_SLOT`],
	/// and stay loaded until Metamod unloads the plugin.
	unsafe fn install_duels(
		self,
		vtables: [NonNull<*mut c_void>; 2],
		binding: ServerBinding,
		callback: DuelJobFn,
		waiting: WaitingFn,
	) -> Result<DuelHooks, DuelHookError> {
		if self.duels_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		// SAFETY: As the caller promises.
		let original = unsafe { self.shared_run_job(vtables) }?;

		let mut installed = DuelHooks {
			hooks: Vec::with_capacity(vtables.len()),
			_not_thread_safe: PhantomData,
		};

		for (route, vtable) in ROUTES.iter().zip(vtables) {
			let target = HookTarget::vtable(vtable);

			// SAFETY: As the caller promises, the vtable is live, has
			// `bool (IMsgNetPacket *)` at the slot, and stays loaded.
			match unsafe { self.add_hook(RUN_JOB_FROM_MSG, target, HookTiming::Pre, route) } {
				Ok(hook) => {
					route.state.set(Some(RoutedDuel {
						binding,
						callback,
						hook,
						original,
						waiting,
					}));
					installed.hooks.push(hook);
				}

				Err(error) => {
					installed.remove(self);
					return Err(error.into());
				}
			}
		}

		Ok(installed)
	}

	/// The function both of `vtables` held at [`RUN_JOB_FROM_MSG_SLOT`] before
	/// any hook, as the duel jobs inherit `CGCClientJob`'s, provided that each
	/// held a function of its own at [`RUN_GC_JOB_SLOT`], as each job overrides
	/// the packet's `BYieldingRunGCJob`. Fails with
	/// [`DuelHookError::UnexpectedLayout`] otherwise.
	///
	/// # Safety
	///
	/// As for [`Self::install_duels`].
	unsafe fn shared_run_job(
		self,
		[request, response]: [NonNull<*mut c_void>; 2],
	) -> Result<RunJobFromMsg, DuelHookError> {
		let original = |function, vtable| {
			// SAFETY: As the caller promises, the vtable is live and has
			// `bool (IMsgNetPacket *)` at the slot.
			unsafe { self.original_function(function, HookTarget::vtable(vtable)) }
		};

		let shared = original(RUN_JOB_FROM_MSG, request)?;
		let response_shared = original(RUN_JOB_FROM_MSG, response)?;
		let request_own = original(RUN_GC_JOB, request)?;
		let response_own = original(RUN_GC_JOB, response)?;

		let laid_out = ptr::fn_addr_eq(shared, response_shared)
			&& !ptr::fn_addr_eq(shared, request_own)
			&& !ptr::fn_addr_eq(shared, response_own)
			&& !ptr::fn_addr_eq(request_own, response_own);

		match laid_out {
			true => Ok(shared),
			false => Err(DuelHookError::UnexpectedLayout),
		}
	}
}

/// Runs `job` while TF2's game rules claim to wait for players, then puts
/// back what they claimed. Returns whether it ran the job, which it does not
/// if the game rules cannot be found.
fn while_waiting_for_players(server: Server<'_>, job: &mut dyn FnMut()) -> bool {
	let Ok(rules) = GameRules::get(server) else {
		return false;
	};
	let Ok(waiting) = rules.is_waiting_for_players() else {
		return false;
	};

	// SAFETY: The game holds the flag set while it waits for players, and
	// nothing else of the game runs before it is put back below: only the duel
	// job, which does not yield, reads it.
	if unsafe { rules.write(WAITING_FOR_PLAYERS, true) }.is_err() {
		return false;
	}

	job();

	// SAFETY: The value the flag held. As the same variable was just written,
	// this cannot fail.
	let restored = unsafe { rules.write(WAITING_FOR_PLAYERS, waiting) };

	debug_assert!(restored.is_ok(), "the waiting flag could not be put back");
	true
}
