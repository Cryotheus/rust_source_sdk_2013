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
//! The hooks run before both functions on the engine's interface object. When
//! the callback's choice differs from the creator's, they call the engine's
//! `CreateFakeClientEx` with the callback's choice in place of the creator's
//! call, so a `CreateFakeClient` the callback keeps from being reported
//! becomes a `CreateFakeClientEx`. Calling the engine's own function skips
//! every plugin's hooks on it, as with [`DamageAction::Apply`].
//!
//! The engine is not public, so what it leaves out for an unreported client is
//! not documented: Steam's player list and count of bots, which the server
//! browser shows, are what Mann vs. Machine keeps its robots out of.
//!
//! [`DamageAction::Apply`]: crate::damage_hooks::DamageAction::Apply

#[cfg(test)]
#[path = "tests/fake_client_hooks.rs"]
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

/// The two hooks [`MetamodApi::hook_fake_client_reports`] installs.
///
/// Metamod disables them while the plugin is paused, and removes them before
/// it unloads. [`Self::remove`] stops them earlier.
#[must_use = "retain the hooks to remove them"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FakeClientHooks {
	create: HookId,
	create_ex: HookId,
}

impl FakeClientHooks {
	/// Stops both hooks, so every fake client is reported as its creator asks.
	pub fn remove(self, api: MetamodApi<'_>) {
		api.remove_hook(self.create);
		api.remove_hook(self.create_ex);
		ROUTE.state.set(None);
	}
}

struct FakeClientRoute {
	state: Cell<Option<RoutedFakeClients>>,
}

impl FakeClientRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}

	/// The callback's choice for a client named `name`, which its creator
	/// asked to be reported or not, if the choice differs: whether to report
	/// it, and the engine's own `CreateFakeClientEx` to create it with.
	///
	/// # Safety
	///
	/// `name` must be null or the name a call of either hooked function is
	/// about to create a client with.
	unsafe fn overrule(
		&self,
		name: *const c_char,
		requested: bool,
		superseded: Option<bool>,
	) -> Option<(CreateFakeClientEx, bool)> {
		// An earlier hook already created the client, or refused to.
		if superseded == Some(true) || name.is_null() {
			return None;
		}

		let route = self.state.get()?;

		// SAFETY: The caller passes the creator's name, a string that lives
		// through the call.
		let report = (route.callback)(unsafe { CStr::from_ptr(name) }, requested);

		(report != requested).then_some((route.original_ex, report))
	}
}

impl Handler<CreateFakeClient> for FakeClientRoute {
	fn call(&self, call: &HookCall<'_, CreateFakeClient>) -> HookAction<*mut sys::edict_t> {
		let (name,) = call.args();

		// SAFETY: The hook runs before the engine's `CreateFakeClient`, which
		// reports the client, with its arguments.
		unsafe {
			create(
				call.this(),
				name,
				self.overrule(name, true, call.superseded()),
			)
		}
	}
}

impl Handler<CreateFakeClientEx> for FakeClientRoute {
	fn call(&self, call: &HookCall<'_, CreateFakeClientEx>) -> HookAction<*mut sys::edict_t> {
		let (name, requested) = call.args();

		// SAFETY: The hook runs before the engine's `CreateFakeClientEx`, with
		// its arguments.
		unsafe {
			create(
				call.this(),
				name,
				self.overrule(name, requested, call.superseded()),
			)
		}
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
	/// Runs `callback` before the engine creates each fake client through
	/// `engine`, which decides whether the engine reports it to Steam; see the
	/// [module documentation](crate::fake_client_hooks).
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

	/// Hooks `CreateFakeClient` and `CreateFakeClientEx` on `engine`.
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
			.is_some_and(|state| self.has_hook(state.hooks.create_ex))
		{
			return Err(HookError::AlreadyInstalled);
		}

		let target = HookTarget::instance(engine);

		// SAFETY: As the caller promises.
		let original_ex = unsafe { self.original_function(CREATE_EX, target) }?;

		ROUTE.state.set(None);

		// SAFETY: As the caller promises.
		let create = unsafe { self.add_hook(CREATE, target, HookTiming::Pre, &ROUTE) }?;

		// SAFETY: As the caller promises.
		let create_ex = match unsafe { self.add_hook(CREATE_EX, target, HookTiming::Pre, &ROUTE) } {
			Ok(hook) => hook,

			Err(error) => {
				self.remove_hook(create);
				return Err(error);
			}
		};

		let hooks = FakeClientHooks { create, create_ex };

		ROUTE.state.set(Some(RoutedFakeClients {
			callback,
			original_ex,
			hooks,
		}));

		Ok(hooks)
	}
}

/// Lets the creator's call run, or with the callback's choice, creates the
/// client in its place through the engine's own `CreateFakeClientEx`.
///
/// # Safety
///
/// `this` and `name` must be the arguments of a call of either hooked function,
/// about to run, and `overrule` its [`FakeClientRoute::overrule`].
unsafe fn create(
	this: *mut sys::IVEngineServer,
	name: *const c_char,
	overrule: Option<(CreateFakeClientEx, bool)>,
) -> HookAction<*mut sys::edict_t> {
	match overrule {
		None => HookAction::Ignore,

		// SAFETY: The engine's unhooked function, called on its own object with
		// the creator's name, in place of the creator's call.
		Some((original_ex, report)) => {
			HookAction::Supersede(unsafe { original_ex(this, name, report) })
		}
	}
}
