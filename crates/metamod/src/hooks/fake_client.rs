//! Hooks on the engine's creation of fake clients, such as bots, which decide
//! whether the engine reports each one to Steam.
//!
//! The game and plugins create fake clients through `IVEngineServer`:
//! `CreateFakeClient`, or `CreateFakeClientEx` with the engine's
//! `bReportFakeClient`. TF2's own bots come through `CreateFakeClientEx`, by
//! way of `NextBotCreatePlayerBot`, which asks for no report only for Mann vs.
//! Machine's robots. The engine's own SourceTV and replay clients do not go
//! through the interface.
//!
//! `CreateFakeClientEx` sets the engine's choice, calls `CreateFakeClient`
//! through the interface's vtable, and sets the choice back to reporting, so
//! every client the interface creates passes through `CreateFakeClient`. The
//! callback runs once for each, in a hook before `CreateFakeClient`, with the
//! choice of the `CreateFakeClientEx` call it is part of, which two more hooks
//! note before and after that call. When the callback's choice differs, the
//! hook calls the engine's own `CreateFakeClientEx` with it in place of the
//! creator's call, which skips every plugin's hooks on that function, but not
//! on the `CreateFakeClient` it calls.
//!
//! The engine is not public. TF2's leaves an unreported client out of what it
//! tells Steam, as it does SourceTV: it counts neither a bot nor a slot in the
//! counts the server browser shows, and any session the client has with Steam
//! ends, which leaves it out of the server's players.

#[cfg(test)]
#[path = "../tests/hooks/fake_client.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::interfaces::ValveEngine;

use source_sdk_2013::raw::interfaces::valve_engine::{
	CREATE_FAKE_CLIENT_EX_SLOT, CREATE_FAKE_CLIENT_SLOT,
	CreateFakeClientExFn as CreateFakeClientEx, CreateFakeClientFn as CreateFakeClient,
};

use source_sdk_2013::sys;
use std::cell::Cell;
use std::ffi::{CStr, c_char};
use std::ptr::NonNull;

/// Decides whether the engine reports a new fake client to Steam, from its
/// name and whether its creator asked for it to be reported. A panic is
/// contained by the hook dispatcher, and keeps the creator's choice.
///
/// The callback gets no [`Server`](source_sdk_2013::Server): the engine is
/// creating the client, and nothing else may touch it meanwhile.
pub type ReportFn = fn(name: &CStr, requested: bool) -> bool;

/// `IVEngineServer::CreateFakeClient`.
const CREATE: VirtualFunction<CreateFakeClient> = VirtualFunction::new(CREATE_FAKE_CLIENT_SLOT);

/// `IVEngineServer::CreateFakeClientEx`.
const CREATE_EX: VirtualFunction<CreateFakeClientEx> =
	VirtualFunction::new(CREATE_FAKE_CLIENT_EX_SLOT);

static ROUTE: FakeClientRoute = FakeClientRoute::new();

/// The hooks [`MetamodApi::hook_fake_client_reports`] installs.
///
/// Metamod disables them while the plugin is paused, and removes them before
/// it unloads. [`Self::remove`] stops them earlier.
#[must_use = "retain the hooks to remove them"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FakeClientHooks {
	create: HookId,
	create_ex: HookId,
	create_ex_post: HookId,
}

impl FakeClientHooks {
	/// Stops the hooks, so every fake client is reported as its creator asks.
	pub fn remove(self, api: MetamodApi<'_>) {
		api.remove_hook(self.create);
		api.remove_hook(self.create_ex);
		api.remove_hook(self.create_ex_post);
		ROUTE.state.set(None);
		ROUTE.creating.set(None);
	}
}

/// A client a call of `CreateFakeClientEx` is creating.
#[derive(Clone, Copy)]
struct Creating {
	/// The name the creator passed, which `CreateFakeClientEx` passes on to
	/// `CreateFakeClient`.
	name: *const c_char,

	/// Whether the client is to be reported.
	report: bool,

	/// Whether the callback chose already, so that `CreateFakeClient` runs as
	/// it was called.
	chosen: bool,
}

struct FakeClientRoute {
	state: Cell<Option<RoutedFakeClients>>,

	/// The client the running call of `CreateFakeClientEx` is creating.
	creating: Cell<Option<Creating>>,
}

impl FakeClientRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
			creating: Cell::new(None),
		}
	}

	/// Lets the creator's call of `CreateFakeClient` run, or if the callback
	/// chooses otherwise than the creator, creates the client in its place
	/// through the engine's own `CreateFakeClientEx`.
	///
	/// # Safety
	///
	/// `this` and `name` must be the arguments of a call of `CreateFakeClient`
	/// about to run.
	unsafe fn create(
		&self,
		this: *mut sys::IVEngineServer,
		name: *const c_char,
		superseded: Option<bool>,
	) -> HookAction<*mut sys::edict_t> {
		// An earlier hook already created the client, or refused to.
		if superseded == Some(true) || name.is_null() {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};

		let creating = self.creating.get();

		let requested = match creating.filter(|creating| creating.name == name) {
			Some(Creating { chosen: true, .. }) => return HookAction::Ignore,
			Some(creating) => creating.report,

			// The engine reports what `CreateFakeClient` creates on its own.
			None => true,
		};

		// SAFETY: The caller passes the creator's name, a string that lives
		// through the call.
		let report = (route.callback)(unsafe { CStr::from_ptr(name) }, requested);

		if report == requested {
			return HookAction::Ignore;
		}

		self.creating.set(Some(Creating {
			name,
			report,
			chosen: true,
		}));

		// SAFETY: The engine's unhooked function, called on its own object with
		// the creator's name, in place of the creator's call. It calls
		// `CreateFakeClient` again through the vtable, which this hook lets run.
		let edict = unsafe { (route.original_ex)(this, name, report) };

		self.creating.set(creating);
		HookAction::Supersede(edict)
	}
}

impl Handler<CreateFakeClient> for FakeClientRoute {
	fn call(&self, call: &HookCall<'_, CreateFakeClient>) -> HookAction<*mut sys::edict_t> {
		let (name,) = call.args();

		// SAFETY: The hook runs before the engine's `CreateFakeClient`, with its
		// arguments.
		unsafe { self.create(call.this(), name, call.superseded()) }
	}
}

impl Handler<CreateFakeClientEx> for FakeClientRoute {
	/// Notes the creator's choice before the call, for the hook on the
	/// `CreateFakeClient` it calls, and forgets it after.
	fn call(&self, call: &HookCall<'_, CreateFakeClientEx>) -> HookAction<*mut sys::edict_t> {
		let (name, report) = call.args();

		match call.timing() {
			// Nothing will be created if an earlier hook superseded the call.
			HookTiming::Pre if call.superseded() == Some(true) => {}

			HookTiming::Pre => self.creating.set(Some(Creating {
				name,
				report,
				chosen: false,
			})),

			// Any client a plugin creates while the engine creates this one comes
			// after the callback chose for this one.
			HookTiming::Post => self.creating.set(None),
		}

		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for FakeClientRoute {}

#[derive(Clone, Copy)]
struct RoutedFakeClients {
	callback: ReportFn,
	hooks: FakeClientHooks,
	original_ex: CreateFakeClientEx,
}

impl MetamodApi<'_> {
	/// Runs `callback` once before the engine creates each fake client through
	/// `engine`, which decides whether the engine reports it to Steam; see the
	/// [module documentation](crate::hooks::fake_client).
	///
	/// Installing again while the hooks are installed returns
	/// [`HookError::AlreadyInstalled`]; removing them allows replacement.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload, so
	/// returned hooks mean registration was accepted, not that the next fake
	/// client will be seen.
	pub fn hook_fake_client_reports(
		self,
		engine: ValveEngine<'_>,
		callback: ReportFn,
	) -> Result<FakeClientHooks, HookError> {
		// SAFETY: The engine's interface object lives as long as the engine,
		// which outlives the plugin, and its vtable holds both functions at
		// their slots, which the generated binding checks.
		unsafe { self.install_fake_clients(NonNull::new(engine.as_ptr()).unwrap(), callback) }
	}

	/// Hooks `CreateFakeClient`, and `CreateFakeClientEx` before and after, on
	/// `engine`.
	///
	/// # Safety
	///
	/// `engine` must be live, its vtable must hold functions of the signatures
	/// [`CreateFakeClient`] and [`CreateFakeClientEx`] at their slots, and both
	/// must stay loaded until Metamod unloads the plugin.
	unsafe fn install_fake_clients(
		self,
		engine: NonNull<sys::IVEngineServer>,
		callback: ReportFn,
	) -> Result<FakeClientHooks, HookError> {
		if ROUTE
			.state
			.get()
			.is_some_and(|state| self.has_hook(state.hooks.create))
		{
			return Err(HookError::AlreadyInstalled);
		}

		let target = HookTarget::instance(engine);

		// SAFETY: As the caller promises.
		let original_ex = unsafe { self.original_function(CREATE_EX, target) }?;

		ROUTE.state.set(None);
		ROUTE.creating.set(None);

		// SAFETY: As the caller promises.
		let create_ex = unsafe { self.add_hook(CREATE_EX, target, HookTiming::Pre, &ROUTE) }?;

		// SAFETY: As the caller promises.
		let create_ex_post = unsafe { self.add_hook(CREATE_EX, target, HookTiming::Post, &ROUTE) }
			.inspect_err(|_| {
				self.remove_hook(create_ex);
			})?;

		// SAFETY: As the caller promises.
		let create = unsafe { self.add_hook(CREATE, target, HookTiming::Pre, &ROUTE) }
			.inspect_err(|_| {
				self.remove_hook(create_ex);
				self.remove_hook(create_ex_post);
			})?;

		let hooks = FakeClientHooks {
			create,
			create_ex,
			create_ex_post,
		};

		ROUTE.state.set(Some(RoutedFakeClients {
			callback,
			original_ex,
			hooks,
		}));

		Ok(hooks)
	}
}
