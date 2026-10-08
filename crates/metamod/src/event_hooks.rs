//! A hook on the game events the server fires, which can edit or block each
//! one before the game event manager delivers it.
//!
//! The hook patches `IGameEventManager2::FireEvent` on the engine's manager,
//! through which the game, the engine and plugins fire their events, so the
//! callback sees each event before any listener does, on the server or on a
//! client. Its edits reach every listener, and the clients the event is
//! broadcast to.
//!
//! # When to install
//!
//! The manager lives as long as the engine, so install while loading. The hook
//! stops calling back while the plugin is paused and when it unloads, and
//! Metamod removes it after unloading the plugin.
//!
//! # Blocking
//!
//! `FireEvent` takes ownership of the event, and frees it whether or not it
//! fires it. A blocked event is not fired: the hook supersedes the call,
//! returning false as the manager does for an event it did not fire, and frees
//! the event after the call instead, so that other plugins' hooks before the
//! call still find it whole.
//!
//! SourceMod blocks an event by freeing it in its own hook before the call. If
//! a hook running after this one blocks an event that way, such as SourceMod's
//! for an event a SourceMod plugin blocked, an event this hook blocked too is
//! freed twice. To keep an event from clients without that risk, edit it so
//! that clients ignore it.
//!
//! An event that a hook running before this one blocked reaches no callback,
//! as that hook may have freed it. With Metamod 2.0, KHook reports nothing of
//! other plugins' hooks, so the callback runs anyway, and reads a freed event
//! if such a hook freed it.
//!
//! # What gets through
//!
//! As with other Metamod hooks, the callback runs only for events fired on the
//! server's main thread, and not while the plugin is paused or after it
//! unloads, so events fired then reach listeners untouched.
//! `FireEventClientSide`, which only clients call, is not hooked. With Metamod
//! 2.0, when `FireEvent` is already detoured, by another plugin or by the hook
//! after the call, which is installed first, KHook adds the hook before the
//! call from its worker thread, so the events just after an install can pass
//! unseen.

#[cfg(test)]
#[path = "tests/event_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::interfaces::GameEventManager;
use source_sdk_2013::interfaces::game_event::{GameEvent, GameEventMut};
use source_sdk_2013::raw::interfaces::game_event::{FIRE_EVENT_SLOT, FireEventFn as FireEvent};
use source_sdk_2013::raw::vcall;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

/// Decides what happens to an event the server fires, which it can edit
/// first. A panic is contained by the hook dispatcher, and lets the event
/// through, with the edits made before it.
pub type FireEventFn = for<'s> fn(Server<'s>, FiredEvent<'s>) -> FireEventAction;

/// `IGameEventManager2::FireEvent`.
const FIRE_EVENT: VirtualFunction<FireEvent> = VirtualFunction::new(FIRE_EVENT_SLOT);

static ROUTE: EventRoute = EventRoute::new();

struct EventRoute {
	state: Cell<Option<RoutedEvents>>,

	/// The events the callback blocked, which the post hook frees once each
	/// call ends, innermost last.
	blocked: RefCell<Vec<NonNull<sys::IGameEvent>>>,
}

impl EventRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
			blocked: RefCell::new(Vec::new()),
		}
	}

	/// Whether the route's hooks are installed, for this load of the plugin.
	fn installed(&self, api: MetamodApi<'_>) -> bool {
		self.state
			.get()
			.is_some_and(|state| state.hooks.iter().any(|&hook| api.has_hook(hook)))
	}

	/// Before the call: runs the callback, unless a hook before this one
	/// blocked the event.
	fn before(
		&self,
		call: &HookCall<'_, FireEvent>,
		routed: RoutedEvents,
		event: NonNull<sys::IGameEvent>,
		dont_broadcast: bool,
	) -> HookAction<bool> {
		// An earlier hook, such as SourceMod's, blocked it, and may have freed it.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread, during one call
		// from the engine. The plugin supplied the binding with the hook.
		let server = unsafe { routed.binding.server(&scope) };

		let fired = FiredEvent {
			// SAFETY: The manager is about to take ownership of the live event,
			// which stays allocated until the call ends, and which the game made
			// for whoever handles it to read and edit.
			event: unsafe { GameEventMut::from_raw(event) },
			broadcast: !dont_broadcast,
		};

		match (routed.callback)(server, fired) {
			FireEventAction::Continue => HookAction::Ignore,

			FireEventAction::Block => {
				self.blocked.borrow_mut().push(event);
				HookAction::Supersede(false)
			}
		}
	}

	/// After the call: frees the event if the callback blocked it.
	fn after(
		&self,
		manager: *mut sys::IGameEventManager2,
		event: NonNull<sys::IGameEvent>,
	) -> HookAction<bool> {
		let blocked = {
			let mut blocked = self.blocked.borrow_mut();

			blocked
				.iter()
				.rposition(|&other| other == event)
				.map(|index| blocked.remove(index))
		};

		if let Some(event) = blocked {
			// SAFETY: The callback blocked the call, so the manager, which the call
			// was made on, did not take ownership of the event, which no hook
			// running after this one frees, as the module documentation requires.
			unsafe { vcall!(manager => IGameEventManager2_FreeEvent(event.as_ptr())) };
		}

		HookAction::Ignore
	}
}

impl Handler<FireEvent> for EventRoute {
	fn call(&self, call: &HookCall<'_, FireEvent>) -> HookAction<bool> {
		let (event, dont_broadcast) = call.args();

		let (Some(routed), Some(event)) = (self.state.get(), NonNull::new(event)) else {
			return HookAction::Ignore;
		};

		match call.timing() {
			HookTiming::Pre => self.before(call, routed, event, dont_broadcast),
			HookTiming::Post => self.after(call.this(), event),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Borrows are never held over calls.
unsafe impl Sync for EventRoute {}

/// What to do with an event the server fires.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FireEventAction {
	/// Fire it, with any edits.
	#[default]
	Continue,

	/// Do not fire it, and free it once the call ends; see the
	/// [module documentation](crate::event_hooks#blocking).
	Block,
}

/// An event the server is firing, as a [`FireEventFn`] sees it.
#[derive(Debug, Clone, Copy)]
pub struct FiredEvent<'e> {
	event: GameEventMut<'e>,
	broadcast: bool,
}

impl<'e> FiredEvent<'e> {
	/// Whether the event goes to clients as well as to the server's listeners:
	/// `FireEvent`'s `bDontBroadcast`, negated.
	pub const fn broadcast(self) -> bool {
		self.broadcast
	}

	/// Reads the event.
	pub const fn event(self) -> GameEvent<'e> {
		self.event.as_event()
	}

	/// Edits the event, before every listener and client gets it.
	pub const fn event_mut(self) -> GameEventMut<'e> {
		self.event
	}
}

#[derive(Clone, Copy)]
struct RoutedEvents {
	binding: ServerBinding,
	callback: FireEventFn,
	hooks: [HookId; 2],
}

impl MetamodApi<'_> {
	/// Runs `callback` before each event the server fires through `manager`,
	/// which can edit the event, and decides whether it is fired; see the
	/// [module documentation](crate::event_hooks).
	///
	/// This hooks `IGameEventManager2::FireEvent` before and after the call.
	/// Installing again while the hooks are installed returns
	/// [`HookError::AlreadyInstalled`].
	pub fn hook_fire_event(
		self,
		manager: GameEventManager<'_>,
		binding: ServerBinding,
		callback: FireEventFn,
	) -> Result<(), HookError> {
		let manager = NonNull::new(manager.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: `manager` is the engine's, which outlives the plugin, and has
		// `FireEvent` at the slot.
		unsafe { self.install_fire_event(manager, binding, callback) }
	}

	/// Hooks `FireEvent` on `manager`.
	///
	/// # Safety
	///
	/// `manager` must be live, and its vtable must hold a function of the
	/// signature [`FireEvent`] at [`FIRE_EVENT_SLOT`], which takes ownership of
	/// the event, and frees it with the vtable's `FreeEvent`, until Metamod
	/// unloads the plugin.
	unsafe fn install_fire_event(
		self,
		manager: NonNull<sys::IGameEventManager2>,
		binding: ServerBinding,
		callback: FireEventFn,
	) -> Result<(), HookError> {
		if ROUTE.installed(self) {
			return Err(HookError::AlreadyInstalled);
		}

		let target = HookTarget::instance(manager);

		// SAFETY: As the caller promises; a `MetamodApi` only exists on the main
		// thread.
		let after = unsafe { self.add_hook(FIRE_EVENT, target, HookTiming::Post, &ROUTE) }?;

		// SAFETY: As above.
		let before = match unsafe { self.add_hook(FIRE_EVENT, target, HookTiming::Pre, &ROUTE) } {
			Ok(before) => before,

			Err(error) => {
				self.remove_hook(after);
				return Err(error);
			}
		};

		ROUTE.blocked.take();
		ROUTE.state.set(Some(RoutedEvents {
			binding,
			callback,
			hooks: [before, after],
		}));
		Ok(())
	}
}
