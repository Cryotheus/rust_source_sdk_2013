//! Tests of `crate::hooks::tf2::duel`: pre hooks of `BYieldingRunJobFromMsg` on mock
//! duel job classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::InterfaceFactory;
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`run`], in order, with
	/// whether the game rules claimed to wait for players as each ran.
	static CALLS: RefCell<Vec<(&'static str, bool)>> = const { RefCell::new(Vec::new()) };

	/// What the callback decides.
	static ACTION: Cell<DuelAction> = const { Cell::new(DuelAction::Allow) };

	/// Whether the fake game rules claim to wait for players.
	static WAITING: Cell<bool> = const { Cell::new(false) };

	/// Whether the fake game rules can be found.
	static RULES: Cell<bool> = const { Cell::new(true) };
}

/// The message each job runs for.
const PACKET: usize = 0x5ac;

/// A job of a C++ class, as hooks know it.
#[repr(C)]
struct Job {
	vtable: *mut *mut c_void,
}

impl Job {
	/// The vtable of a new class, which holds `run_job` at
	/// [`RUN_JOB_FROM_MSG_SLOT`] and `run_gc_job` at [`RUN_GC_JOB_SLOT`].
	fn new_class(run_job: RunJobFromMsg, run_gc_job: RunJobFromMsg) -> NonNull<*mut c_void> {
		let mut slots = vec![unexpected as RunJobFromMsg as *mut c_void; RUN_GC_JOB_SLOT + 1];

		slots[RUN_JOB_FROM_MSG_SLOT] = run_job as *mut c_void;
		slots[RUN_GC_JOB_SLOT] = run_gc_job as *mut c_void;

		NonNull::new(Vec::leak(slots).as_mut_ptr()).unwrap()
	}

	/// A job of the class whose vtable is `vtable`, as the GC client creates
	/// one for each message.
	fn of_class(vtable: NonNull<*mut c_void>) -> Box<Self> {
		Box::new(Self {
			vtable: vtable.as_ptr(),
		})
	}

	fn ptr(&mut self) -> *mut c_void {
		(&raw mut *self).cast()
	}
}

/// Installs the hooks on `vtables`, with [`on_duel_job`] and
/// [`while_waiting`].
fn install(
	harness: &Harness,
	vtables: [NonNull<*mut c_void>; 2],
) -> Result<DuelHooks, DuelHookError> {
	// SAFETY: The mock classes have `bool (IMsgNetPacket *)` at both slots, and
	// are leaked.
	unsafe {
		harness.api().install_duels(
			vtables,
			tf2_binding(no_interfaces),
			on_duel_job,
			while_waiting,
		)
	}
}

#[test]
fn jobs_an_earlier_hook_skipped_run_no_callback() {
	on_both(|harness| {
		let vtables = new_classes();
		let mut response = Job::of_class(vtables[1]);

		fn skip(_call: &HookCall<'_, RunJobFromMsg>) -> HookAction<bool> {
			HookAction::Supersede(true)
		}

		// SAFETY: As for `install`.
		unsafe {
			harness.api().add_hook(
				RUN_JOB_FROM_MSG,
				HookTarget::vtable(vtables[1]),
				HookTiming::Pre,
				&skip,
			)
		}
		.unwrap();

		ACTION.set(DuelAction::Refuse);
		let _hooks = install(harness, vtables).unwrap();
		assert_eq!(run(harness, &mut response), (true, vec![]));
	});
}

/// The vtables of a new request class and a new response class, as the game's
/// duel jobs lay them out.
fn new_classes() -> [NonNull<*mut c_void>; 2] {
	[
		Job::new_class(run_job, run_request),
		Job::new_class(run_job, run_response),
	]
}

/// The callback, which notes that it ran, and decides as [`ACTION`] says.
fn on_duel_job(_server: Server<'_>, job: DuelJob) -> DuelAction {
	let name = match job {
		DuelJob::Request => "callback for the request",
		DuelJob::Response => "callback for the response",
	};

	CALLS.with_borrow_mut(|calls| calls.push((name, WAITING.get())));
	ACTION.get()
}

/// Another job class's `BYieldingRunJobFromMsg`.
unsafe extern "C" fn other_run_job(_this: *mut c_void, _packet: *mut c_void) -> bool {
	CALLS.with_borrow_mut(|calls| calls.push(("other", WAITING.get())));
	true
}

#[test]
fn pauses_and_reloads_let_the_jobs_run() {
	on_both(|harness| {
		let api = harness.api();
		let vtables = new_classes();
		let mut response = Job::of_class(vtables[1]);

		RULES.set(true);
		ACTION.set(DuelAction::Refuse);

		let hooks = install(harness, vtables).unwrap();

		harness.set_status(true, true, harness.generation);
		assert_eq!(
			run(harness, &mut response),
			(false, vec![("response", false)])
		);

		// A later load, which has not installed its own hooks yet. Its
		// generation is one no later harness takes, as it installs hooks.
		harness.set_status(true, false, harness.generation | 1 << 63);
		assert_eq!(
			run(harness, &mut response),
			(false, vec![("response", false)])
		);
		assert!(!hooks.hooks.iter().any(|&hook| api.has_hook(hook)));

		let _hooks = install(harness, vtables).unwrap();
		assert_eq!(
			run(harness, &mut response),
			(
				false,
				vec![("callback for the response", false), ("response", true)]
			)
		);
	});
}

#[test]
fn refused_requests_are_skipped_and_refused_responses_run_waiting() {
	on_both(|harness| {
		let vtables = new_classes();
		let mut request = Job::of_class(vtables[0]);
		let mut response = Job::of_class(vtables[1]);

		RULES.set(true);
		let _hooks = install(harness, vtables).unwrap();

		ACTION.set(DuelAction::Allow);
		assert_eq!(
			run(harness, &mut request),
			(
				true,
				vec![("callback for the request", false), ("request", false)]
			)
		);
		assert_eq!(
			run(harness, &mut response),
			(
				false,
				vec![("callback for the response", false), ("response", false)]
			)
		);

		// The request does not run. The response runs once, while the game
		// rules claim to wait for players, and returns its own result.
		ACTION.set(DuelAction::Refuse);
		assert_eq!(
			run(harness, &mut request),
			(true, vec![("callback for the request", false)])
		);
		assert_eq!(
			run(harness, &mut response),
			(
				false,
				vec![("callback for the response", false), ("response", true)]
			)
		);
		assert!(!WAITING.get());

		// What the game rules claimed is put back as it was.
		WAITING.set(true);
		assert_eq!(
			run(harness, &mut response),
			(
				false,
				vec![("callback for the response", true), ("response", true)]
			)
		);
		assert!(WAITING.get());
		WAITING.set(false);

		// Without game rules to claim it, the response does not run either.
		RULES.set(false);
		assert_eq!(
			run(harness, &mut response),
			(true, vec![("callback for the response", false)])
		);
	});
}

/// Runs `job`'s hooked `BYieldingRunJobFromMsg`, and returns its result and
/// what ran.
fn run(harness: &Harness, job: &mut Job) -> (bool, Vec<(&'static str, bool)>) {
	CALLS.take();

	let packet = ptr::without_provenance_mut(PACKET);
	let succeeded = harness.call::<RunJobFromMsg>(job.ptr(), RUN_JOB_FROM_MSG_SLOT, (packet,));

	(succeeded, CALLS.take())
}

/// `CGCClientJob::BYieldingRunJobFromMsg`, which runs the job's own
/// `BYieldingRunGCJob`.
unsafe extern "C" fn run_job(this: *mut c_void, packet: *mut c_void) -> bool {
	// SAFETY: Every job of the tests starts with its vtable, whose slot holds the
	// job's own function.
	let run_gc_job = unsafe {
		this.cast::<*const RunJobFromMsg>()
			.read()
			.add(RUN_GC_JOB_SLOT)
			.read()
	};

	// SAFETY: As the hooked function was called.
	unsafe { run_gc_job(this, packet) }
}

/// The request's `BYieldingRunGCJob`, which notes that it ran.
unsafe extern "C" fn run_request(_this: *mut c_void, packet: *mut c_void) -> bool {
	expect(
		packet.addr() == PACKET,
		"the request ran for another message",
	);
	CALLS.with_borrow_mut(|calls| calls.push(("request", WAITING.get())));
	true
}

/// The response's `BYieldingRunGCJob`, which notes that it ran, and fails, so
/// that its result tells from the one the hooks would make up.
unsafe extern "C" fn run_response(_this: *mut c_void, packet: *mut c_void) -> bool {
	expect(
		packet.addr() == PACKET,
		"the response ran for another message",
	);
	CALLS.with_borrow_mut(|calls| calls.push(("response", WAITING.get())));
	false
}

#[test]
fn the_jobs_are_hooked_once_until_removed() {
	on_both(|harness| {
		let api = harness.api();
		let vtables = new_classes();
		let mut request = Job::of_class(vtables[0]);
		let hooks = install(harness, vtables).unwrap();

		assert!(matches!(
			install(harness, new_classes()),
			Err(DuelHookError::Hook(HookError::AlreadyInstalled))
		));

		// Removed hooks leave the jobs to the game, and can be installed again.
		ACTION.set(DuelAction::Refuse);
		hooks.remove(api);
		assert_eq!(run(harness, &mut request), (true, vec![("request", false)]));

		install(harness, vtables).unwrap().remove(api);
	});
}

#[test]
fn the_jobs_are_searched_for_unless_already_hooked() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();

		// SAFETY: As for `tf2_binding`, but for another game, whose servers reach
		// no interface before the game is checked.
		let other = unsafe {
			ServerBinding::new(
				InterfaceFactory::new(no_interfaces),
				InterfaceFactory::new(no_interfaces),
				Game::SourceSdk2013,
			)
		};
		// SAFETY: As above.
		let other_server = unsafe { other.server(&scope) };

		assert!(matches!(
			// SAFETY: The search fails before any class is hooked.
			unsafe { api.hook_duels(other_server, other, on_duel_job) },
			Err(DuelHookError::Target(DuelJobVtableError::WrongGame))
		));

		let binding = tf2_binding(no_interfaces);
		// SAFETY: The server's game module is this test's executable, which
		// `no_interfaces` is in, and which has no duel job classes.
		let server = unsafe { binding.server(&scope) };

		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.hook_duels(server, binding, on_duel_job) },
			Err(DuelHookError::Target(DuelJobVtableError::NotFound(
				DuelJob::Request
			)))
		));

		let _hooks = install(harness, new_classes()).unwrap();

		// Installed hooks are reported without searching the module again.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.hook_duels(server, binding, on_duel_job) },
			Err(DuelHookError::Hook(HookError::AlreadyInstalled))
		));
	});
}

#[test]
fn the_layout_is_checked_on_the_functions_before_any_hook() {
	on_both(|harness| {
		let vtables = new_classes();
		let mut request = Job::of_class(vtables[0]);

		// A hook of the request's class alone, as another plugin's could be,
		// which SourceHook patches into the class's vtable.
		fn ignore(_call: &HookCall<'_, RunJobFromMsg>) -> HookAction<bool> {
			HookAction::Ignore
		}

		// SAFETY: As for `install`.
		unsafe {
			harness.api().add_hook(
				RUN_JOB_FROM_MSG,
				HookTarget::vtable(vtables[0]),
				HookTiming::Pre,
				&ignore,
			)
		}
		.unwrap();

		let _hooks = install(harness, vtables).unwrap();
		ACTION.set(DuelAction::Refuse);
		assert_eq!(
			run(harness, &mut request),
			(true, vec![("callback for the request", false)])
		);
	});
}

/// A slot of a mock vtable that no test calls.
unsafe extern "C" fn unexpected(_this: *mut c_void, _packet: *mut c_void) -> bool {
	expect(false, "an unhooked slot was called");
	false
}

#[test]
fn vtables_not_laid_out_as_the_jobs_are_not_hooked() {
	on_both(|harness| {
		let [request, response] = new_classes();
		let other = Job::new_class(other_run_job, run_response);
		let same = Job::new_class(run_job, run_request);
		let shared = Job::new_class(run_job, run_job);

		// Each of the jobs runs the same function, which runs a function of each
		// one's own.
		for vtables in [[request, other], [request, same], [shared, response]] {
			assert!(matches!(
				install(harness, vtables),
				Err(DuelHookError::UnexpectedLayout)
			));
		}

		ACTION.set(DuelAction::Refuse);
		assert_eq!(
			run(harness, &mut Job::of_class(request)),
			(true, vec![("request", false)])
		);
	});
}

/// Runs `job` while the fake game rules claim to wait for players, then puts
/// back what they claimed, unless [`RULES`] says they cannot be found.
fn while_waiting(_server: Server<'_>, job: &mut dyn FnMut()) -> bool {
	if !RULES.get() {
		return false;
	}

	let waiting = WAITING.replace(true);

	job();
	WAITING.set(waiting);
	true
}
