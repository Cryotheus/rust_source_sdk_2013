//! Hooks of which entities the engine sends each client: an entity class's
//! `ShouldTransmit`, which decides for each client whether the class's
//! entities with a [full check] are sent, and `SetTransmit`, which marks an
//! entity as sent to a client.
//!
//! [`source_sdk_2013::raw::transmit`] describes when the engine calls them.
//! Hooks cover the class of the entity they were installed from, but not the
//! classes deriving from it, which have vtables of their own, until removed or
//! the plugin unloads. As with other Metamod hooks, they stop calling handlers
//! while the plugin is paused.
//!
//! The engine calls them on the main thread, for each client and each entity
//! it checks at every snapshot, so callbacks should be cheap. They are not
//! called for the client's own player, which every client must be sent.
//!
//! [full check]: source_sdk_2013::edicts::TransmitState::FullCheck

#[cfg(test)]
#[path = "tests/transmit_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::edicts::TransmitCheck;
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::edicts::{FL_EDICT_ALWAYS, FL_EDICT_DONTSEND, FL_EDICT_PVSCHECK};

use source_sdk_2013::raw::transmit::{
	SET_TRANSMIT_SLOT, SHOULD_TRANSMIT_SLOT, SetTransmitFn as SetTransmit,
	ShouldTransmitFn as ShouldTransmit,
};

use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{c_int, c_void};
use std::ptr::{self, NonNull};

/// A callback-scoped server, an entity of the hooked class about to be marked
/// as sent, the record of the client it is sent to, and whether it is sent
/// wherever it is, rather than because the client's potentially visible set
/// holds it. A panic is contained by the hook dispatcher, and lets the game
/// mark it.
pub type SetTransmitFn =
	for<'s> fn(Server<'s>, Entity<'s>, TransmitCheck<'s>, bool) -> TransmitAction;

/// A callback-scoped server, an entity of the hooked class, whose edict has a
/// full check, and the record of the client it is checked for. A panic is
/// contained by the hook dispatcher, and lets the game decide.
pub type ShouldTransmitFn =
	for<'s> fn(Server<'s>, Entity<'s>, TransmitCheck<'s>) -> TransmitDecision;

/// `SetTransmit` in an entity's primary vtable.
const SET_TRANSMIT: VirtualFunction<SetTransmit> = VirtualFunction::new(SET_TRANSMIT_SLOT);

/// `ShouldTransmit` in an entity's primary vtable.
const SHOULD_TRANSMIT: VirtualFunction<ShouldTransmit> = VirtualFunction::new(SHOULD_TRANSMIT_SLOT);

static SET_ROUTES: [TransmitRoute<SetTransmitFn>; 32] = [const { TransmitRoute::new() }; 32];
static SHOULD_ROUTES: [TransmitRoute<ShouldTransmitFn>; 32] = [const { TransmitRoute::new() }; 32];

#[derive(Clone, Copy)]
struct RoutedTransmit<F> {
	binding: ServerBinding,
	callback: F,
	hook: HookId,
	vtable: usize,
}

/// Callbacks compared by address.
trait SameCallback {
	fn same(self, other: Self) -> bool;
}

impl SameCallback for SetTransmitFn {
	fn same(self, other: Self) -> bool {
		ptr::fn_addr_eq(self, other)
	}
}

impl SameCallback for ShouldTransmitFn {
	fn same(self, other: Self) -> bool {
		ptr::fn_addr_eq(self, other)
	}
}

/// What a `SetTransmit` hook does with an entity about to be marked as sent
/// to a client.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransmitAction {
	/// Lets the game's `SetTransmit` mark it.
	#[default]
	Allow,

	/// Skips the game's `SetTransmit`, and the hooks of other plugins that
	/// would run after this one, so that the entity is not marked, nor the
	/// entities the game's would mark with it, such as its move parent and a
	/// character's weapons. It is still sent if another entity's
	/// `SetTransmit` marks it, as one moving with it does through its own.
	Block,
}

/// What a `ShouldTransmit` hook decides for an entity with a full check, for
/// one client.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransmitDecision {
	/// Lets the game's `ShouldTransmit` decide.
	#[default]
	Game,

	/// Sends the entity wherever it is: its `SetTransmit` marks it.
	Always,

	/// Sends the entity if the client's potentially visible set or 3D skybox
	/// holds it: its `SetTransmit` marks it then.
	PvsCheck,

	/// Does not send the entity, unless another entity's `SetTransmit` marks
	/// it, as one moving with it does.
	DontSend,
}

impl TransmitDecision {
	/// The transmit flag the decision returns in place of the game's, if any.
	const fn flag(self) -> Option<c_int> {
		match self {
			Self::Game => None,
			Self::Always => Some(FL_EDICT_ALWAYS),
			Self::PvsCheck => Some(FL_EDICT_PVSCHECK),
			Self::DontSend => Some(FL_EDICT_DONTSEND),
		}
	}
}

struct TransmitRoute<F> {
	state: Cell<Option<RoutedTransmit<F>>>,
}

impl<F: Copy> TransmitRoute<F> {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}

	/// The server, entity and client record of a call of the route's hook,
	/// unless the entity is the client's own player, or the route is free.
	///
	/// # Safety
	///
	/// `entity` and `info` must be the live arguments of a hooked transmit
	/// method, and the call must be made by the hook dispatcher, on the main
	/// thread.
	unsafe fn arguments<'s>(
		&self,
		scope: &'s (),
		entity: *mut sys::CBaseEntity,
		info: *const sys::CCheckTransmitInfo,
	) -> Option<(F, Server<'s>, Entity<'s>, TransmitCheck<'s>)> {
		let route = self.state.get()?;
		let entity = NonNull::new(entity)?;
		let info = NonNull::new(info.cast_mut())?;

		// SAFETY: As the caller promises, the record is live.
		let client = unsafe { (&raw const (*info.as_ptr()).m_pClientEnt).read() };

		// An entity's `IServerUnknown` is its own address, as its class derives
		// from it through primary bases only.
		// SAFETY: As the caller promises, the record is live, and points to the
		// client's edict in the engine's table.
		if !client.is_null()
			&& unsafe { (&raw const (*client)._base.m_pUnk).read() }.addr() == entity.addr().get()
		{
			return None;
		}

		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(scope) };

		// SAFETY: The class hook supplies the live entity whose method runs,
		// which stays in the entity list through the call, and the engine's
		// record of the client whose snapshot it builds meanwhile.
		let (entity, check) = unsafe {
			(
				Entity::from_live(server, entity),
				TransmitCheck::from_live(server, info),
			)
		};

		Some((route.callback, server, entity, check))
	}
}

impl Handler<SetTransmit> for TransmitRoute<SetTransmitFn> {
	fn call(&self, call: &HookCall<'_, SetTransmit>) -> HookAction<()> {
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (info, always) = call.args();
		let scope = ();

		// SAFETY: The hook dispatcher calls with the arguments of the hooked
		// `SetTransmit`, on the main thread.
		let Some((callback, server, entity, check)) =
			(unsafe { self.arguments(&scope, call.this(), info) })
		else {
			return HookAction::Ignore;
		};

		match callback(server, entity, check, always) {
			TransmitAction::Allow => HookAction::Ignore,
			TransmitAction::Block => HookAction::Supersede(()),
		}
	}
}

impl Handler<ShouldTransmit> for TransmitRoute<ShouldTransmitFn> {
	fn call(&self, call: &HookCall<'_, ShouldTransmit>) -> HookAction<c_int> {
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (info,) = call.args();
		let scope = ();

		// SAFETY: As for `SetTransmit`.
		let Some((callback, server, entity, check)) =
			(unsafe { self.arguments(&scope, call.this(), info) })
		else {
			return HookAction::Ignore;
		};

		callback(server, entity, check)
			.flag()
			.map_or(HookAction::Ignore, HookAction::Supersede)
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl<F> Sync for TransmitRoute<F> {}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseEntity::SetTransmit` of the entities
	/// of `entity`'s class: as the game marks one as sent to a client, for
	/// being in the client's potentially visible set or 3D skybox, for
	/// being sent wherever it is by a decision of its `ShouldTransmit`, or with
	/// another entity, such as one moving with it.
	///
	/// Entities whose edicts are sent [always] are marked without calling it.
	/// To hide one from some clients, give it a [full check] instead, which
	/// the game may undo as its state changes, as
	/// [`Edict::set_transmit_state`] says.
	///
	/// `entity` and `binding` must come from the same running server. Returns
	/// the hook, which stays until removed or the plugin unloads. Hooking a
	/// class with a callback it is already hooked with returns
	/// [`HookError::AlreadyInstalled`]; other callbacks can hook it too, and
	/// run in the order they were installed.
	///
	/// [always]: source_sdk_2013::edicts::TransmitState::Always
	/// [full check]: source_sdk_2013::edicts::TransmitState::FullCheck
	/// [`Edict::set_transmit_state`]: source_sdk_2013::edicts::Edict::set_transmit_state
	pub fn hook_set_transmit(
		self,
		entity: Entity<'_>,
		binding: ServerBinding,
		callback: SetTransmitFn,
	) -> Result<HookId, HookError> {
		// SAFETY: Every entity's primary vtable has `SetTransmit` at the slot,
		// which precedes every method declared under a game's defines, and the
		// entity is live. Its vtable belongs to the game module, which outlives
		// this plugin.
		unsafe {
			install(
				self,
				&SET_ROUTES,
				SET_TRANSMIT,
				NonNull::new(entity.as_ptr()).unwrap(),
				binding,
				callback,
			)
		}
	}

	/// Runs `callback` before each `CBaseEntity::ShouldTransmit` of the
	/// entities of `entity`'s class: as the game decides, for a client,
	/// whether to send one whose edict has a [full check], such as a player
	/// or a building. Whatever it decides other than
	/// [`TransmitDecision::Game`] is returned in place of the game's
	/// decision, which is skipped.
	///
	/// Installation is as for [`Self::hook_set_transmit`].
	///
	/// [full check]: source_sdk_2013::edicts::TransmitState::FullCheck
	pub fn hook_should_transmit(
		self,
		entity: Entity<'_>,
		binding: ServerBinding,
		callback: ShouldTransmitFn,
	) -> Result<HookId, HookError> {
		// SAFETY: As for `hook_set_transmit`, for `ShouldTransmit`.
		unsafe {
			install(
				self,
				&SHOULD_ROUTES,
				SHOULD_TRANSMIT,
				NonNull::new(entity.as_ptr()).unwrap(),
				binding,
				callback,
			)
		}
	}
}

/// Hooks `function` before the game's, on the class of `object`, with a free
/// route of `routes`.
///
/// # Safety
///
/// `object` must be live, and its primary vtable must hold a function of the
/// signature `S` at `function`'s slot, and stay loaded until Metamod unloads
/// the plugin.
unsafe fn install<S, F>(
	api: MetamodApi<'_>,
	routes: &'static [TransmitRoute<F>],
	function: VirtualFunction<S>,
	object: NonNull<sys::CBaseEntity>,
	binding: ServerBinding,
	callback: F,
) -> Result<HookId, HookError>
where
	S: crate::hook::Signature,
	F: Copy + SameCallback + 'static,
	TransmitRoute<F>: Handler<S>,
{
	// SAFETY: Every live CBaseEntity starts with its primary vtable pointer,
	// of which only the address is used.
	let vtable = unsafe { vtable_pointer::<c_void>(object.as_ptr()) }.addr();

	if routes.iter().any(|route| {
		route.state.get().is_some_and(|state| {
			state.vtable == vtable && state.callback.same(callback) && api.has_hook(state.hook)
		})
	}) {
		return Err(HookError::AlreadyInstalled);
	}

	let route = routes
		.iter()
		.find(|route| {
			route
				.state
				.get()
				.is_none_or(|state| !api.has_hook(state.hook))
		})
		.ok_or(HookError::TooManyFunctions)?;

	// SAFETY: As the caller promises, the vtable holds a function of the
	// signature at the slot, and stays loaded.
	let hook = unsafe {
		api.add_hook(
			function,
			HookTarget::class_of(object),
			HookTiming::Pre,
			route,
		)
	}?;

	route.state.set(Some(RoutedTransmit {
		binding,
		callback,
		hook,
		vtable,
	}));

	Ok(hook)
}
