//! TF2 vote creation gates, through Metamod's managed virtual hooks.

use crate::MetamodApi;
use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};
use source_sdk_2013::voting::{
	REQUEST_CALL_VOTE_SLOT, VoteDecision, VoteHookTargetError, VoteIssue, VoteRequest,
	VoteStartHandler, vote_issue_vtables,
};
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int};
use std::marker::PhantomData;
use std::rc::Rc;

type RequestCallVote = unsafe extern "C" fn(
	*mut sys::CBaseIssue,
	c_int,
	*const c_char,
	*mut sys::vote_create_failed_t,
	*mut c_int,
) -> bool;

const REQUEST: VirtualFunction<RequestCallVote> = VirtualFunction::new(REQUEST_CALL_VOTE_SLOT);

static ROUTES: [VoteRoute; 11] = [
	VoteRoute::new(VoteIssue::RestartGame),
	VoteRoute::new(VoteIssue::Kick),
	VoteRoute::new(VoteIssue::ChangeLevel),
	VoteRoute::new(VoteIssue::NextLevel),
	VoteRoute::new(VoteIssue::ExtendLevel),
	VoteRoute::new(VoteIssue::ScrambleTeams),
	VoteRoute::new(VoteIssue::ChangeMission),
	VoteRoute::new(VoteIssue::Eternaween),
	VoteRoute::new(VoteIssue::TeamAutoBalance),
	VoteRoute::new(VoteIssue::ClassLimits),
	VoteRoute::new(VoteIssue::PauseGame),
];

struct VoteRoute {
	issue: VoteIssue,
	state: Cell<Option<(HookId, ServerBinding, &'static dyn VoteStartHandler)>>,
}

impl VoteRoute {
	const fn new(issue: VoteIssue) -> Self {
		Self {
			issue,
			state: Cell::new(None),
		}
	}
}

// SAFETY: Metamod invokes handlers only on its main thread. Installation and
// removal require MetamodApi, which is confined to that same thread.
unsafe impl Sync for VoteRoute {}

impl Handler<RequestCallVote> for VoteRoute {
	fn call(&self, call: &HookCall<'_, RequestCallVote>) -> HookAction<bool> {
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}
		let Some((_, binding, handler)) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (caller_entity_index, details, failure, time) = call.args();
		if call.this().is_null() || details.is_null() || failure.is_null() || time.is_null() {
			return HookAction::Ignore;
		}
		let scope = ();
		// SAFETY: This is TF2's RequestCallVote call on the server's main
		// thread; the installed binding belongs to that server. The detail
		// string and both output references remain live through this callback.
		let server = unsafe { binding.server(&scope) };
		let details = unsafe { CStr::from_ptr(details) };
		let request = VoteRequest {
			issue: self.issue,
			caller_entity_index,
			details,
		};
		// SAFETY: The nonnull output references are writable scalars owned
		// by the active CreateVote stack frame, for this callback's duration.
		unsafe { dispatch(server, request, handler, failure, time) }
	}
}

/// The output pointers must be writable for this callback, including after
/// the user's handler returns. Neither may be retained by the handler.
unsafe fn dispatch(
	server: Server<'_>,
	request: VoteRequest<'_>,
	handler: &dyn VoteStartHandler,
	failure: *mut sys::vote_create_failed_t,
	time: *mut c_int,
) -> HookAction<bool> {
	match handler.vote_start(server, request) {
		VoteDecision::Allow => HookAction::Ignore,
		VoteDecision::Block => {
			// SAFETY: The caller supplies live C++ scalar output references.
			// A generic failure has no countdown in SendVoteCreationFailedMessage.
			unsafe {
				failure.write(sys::vote_create_failed_t_VOTE_FAILED_GENERIC);
				time.write(-1);
			}
			HookAction::Supersede(false)
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub enum VoteHookError {
	#[error(transparent)]
	Target(#[from] VoteHookTargetError),
	#[error(transparent)]
	Hook(#[from] HookError),
}

/// Handles for all TF2 vote gates installed by one call.
///
/// Metamod automatically disables these while paused and removes them before
/// library unload, including failed loads. [`Self::remove`] disables them
/// earlier. No entity or per-map issue pointers are retained, so the hooks
/// survive level changes without touching destroyed issue objects.
#[must_use = "retain vote hooks to support explicitly removing them"]
pub struct VoteHooks {
	hooks: Vec<HookId>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl VoteHooks {
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);
			for route in &ROUTES {
				if route.state.get().is_some_and(|state| state.0 == hook) {
					route.state.set(None);
				}
			}
		}
	}
}

impl MetamodApi<'_> {
	/// Consults `handler` before every built-in TF2 issue evaluates a vote
	/// request. A blocked request never creates a vote and receives TF2's
	/// generic failure response. This covers client commands, automatic
	/// server votes, and game-coordinator requests, including concurrent team
	/// votes. `Allow` preserves all normal game eligibility checks.
	///
	/// Install during load. Every known built-in issue must be found before
	/// any hooks are installed; a refused installation rolls back its earlier
	/// hooks. Panics are contained by the common Metamod hook dispatcher and
	/// leave the request to TF2. Other plugins can still influence a vote.
	/// Metamod 2.0 can complete installation asynchronously, particularly when
	/// another plugin already hooks the function. Do not install and immediately
	/// trigger a vote in the same callback to test whether the gate is active;
	/// run the probe from a later server callback after installation completes.
	///
	/// Observe actual starts and accepted ballots through
	/// [`source_sdk_2013::voting::VoteEvent`] and a game-event listener.
	pub fn hook_vote_starts(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		handler: &'static dyn VoteStartHandler,
	) -> Result<VoteHooks, VoteHookError> {
		if ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| self.has_hook(state.0))
		}) {
			return Err(HookError::AlreadyInstalled.into());
		}
		let targets = vote_issue_vtables(server)?;
		let mut installed = VoteHooks {
			hooks: Vec::with_capacity(targets.len()),
			_not_thread_safe: PhantomData,
		};
		for target in targets {
			let route = ROUTES
				.iter()
				.find(|route| route.issue == target.issue)
				.expect("every built-in vote issue has a route");
			// SAFETY: The SDK matched the concrete issue's primary RTTI
			// vtable and verified an executable RequestCallVote slot. Its ABI
			// comes from the generated CBaseIssue declaration. The game module
			// stays loaded until Metamod unloads this plugin; target holds no
			// per-map object and applies to all instances of the issue class.
			match unsafe {
				self.add_hook(
					REQUEST,
					HookTarget::vtable(target.as_ptr()),
					HookTiming::Pre,
					route,
				)
			} {
				Ok(hook) => {
					route.state.set(Some((hook, binding, handler)));
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
}

#[cfg(test)]
mod tests {
	use super::*;
	use source_sdk_2013::{Game, InterfaceFactory};
	use std::ffi::c_void;

	unsafe extern "C" fn factory(_: *const c_char, _: *mut c_int) -> *mut c_void {
		std::ptr::null_mut()
	}

	struct Policy;
	impl VoteStartHandler for Policy {
		fn vote_start(&self, _: Server<'_>, request: VoteRequest<'_>) -> VoteDecision {
			assert_eq!(request.caller_entity_index, 99);
			assert!(request.is_server_request());
			assert_eq!(request.details, c"test");
			if request.issue == VoteIssue::RestartGame {
				VoteDecision::Block
			} else {
				VoteDecision::Allow
			}
		}
	}

	#[test]
	fn veto_supersedes_with_failure_and_allow_preserves_game_outputs() {
		let scope = ();
		// SAFETY: The callback only inspects owned request data. No engine
		// interfaces or entity operations are reachable through this fixture.
		let server = unsafe {
			Server::new(
				InterfaceFactory::new(factory),
				InterfaceFactory::new(factory),
				Game::TeamFortress2,
				&scope,
			)
		};
		for (issue, expected, expected_outputs) in [
			(
				VoteIssue::RestartGame,
				HookAction::Supersede(false),
				(0, -1),
			),
			(VoteIssue::NextLevel, HookAction::Ignore, (25, 42)),
		] {
			let (mut failure, mut time) = (25, 42);
			let request = VoteRequest {
				issue,
				caller_entity_index: 99,
				details: c"test",
			};
			// SAFETY: These two stack outputs remain allocated for the call.
			assert_eq!(
				unsafe { dispatch(server, request, &Policy, &mut failure, &mut time) },
				expected
			);
			assert_eq!((failure, time), expected_outputs);
		}
	}

	#[test]
	fn every_builtin_issue_has_exactly_one_gate() {
		for issue in VoteIssue::ALL {
			assert_eq!(
				ROUTES.iter().filter(|route| route.issue == issue).count(),
				1
			);
		}
	}
}
