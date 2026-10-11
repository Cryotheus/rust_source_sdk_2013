//! A hook on the game events the engine sends each client, which can keep
//! each one from that client.
//!
//! The game event manager passes each event it fires to the listeners of its
//! kind, and the engine's clients listen too, for the events broadcast to
//! them: a client's `IGameEventListener2::FireGameEvent` writes the event to
//! the client's channel. The hook patches that method on the class of the
//! engine's clients, so the callback sees each event once for every client it
//! is sent to, with that client. Where [`event_hooks`](crate::hooks::event)
//! edit or block an event before any listener gets it, this keeps an event
//! from some clients, while the others, and the server's own listeners, still
//! get it.
//!
//! # When to install
//!
//! The engine creates its client objects as players first connect, bots
//! included, so installing fails with [`HookTargetError::NotReady`] until one
//! has. Try again, such as each frame, until it succeeds. The clients' class
//! lasts as long as the engine. The hook stops calling back while the plugin
//! is paused and when it unloads, and Metamod removes it after unloading the
//! plugin.
//!
//! # Withholding
//!
//! A withheld event is not written to the client's channel: the hook
//! supersedes the call. The manager still owns the event, and frees it once
//! each listener has had it, so a withheld event leaves nothing to clean up,
//! and other clients, and other plugins' hooks, get it whole.
//!
//! An event that a hook running before this one withheld, such as another
//! plugin's, reaches no callback. With Metamod 2.0, KHook reports nothing of
//! other plugins' hooks, so the callback runs for those too.
//!
//! # What gets through
//!
//! As with other Metamod hooks, the callback runs only for events sent on the
//! server's main thread, and not while the plugin is paused or after it
//! unloads, so events sent then reach their clients. Only the game server's
//! clients are covered: SourceTV's spectators are clients of SourceTV's own
//! server, of another class. With Metamod 2.0, when `FireGameEvent` is
//! already detoured by another plugin, KHook adds the hook from its worker
//! thread, so the events just after an install can pass unseen.

#[cfg(test)]
#[path = "../tests/hooks/delivery.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::interfaces::GameClient;
use source_sdk_2013::interfaces::game_event::GameEvent;
use source_sdk_2013::net::events::{Delivery, hook_target, route_event};
use source_sdk_2013::net::incoming::HookTargetError;

use source_sdk_2013::raw::interfaces::game_event::{
	FIRE_GAME_EVENT_SLOT, FireGameEventFn as FireGameEvent,
};

use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ptr::NonNull;

/// Decides whether the engine sends a client an event. It gets the client,
/// and the event to read, whose [name](GameEvent::name) says which it is. A
/// panic is caught, and delivers the event.
pub type DeliveryFn = for<'s> fn(Server<'s>, GameClient<'s>, GameEvent<'s>) -> Delivery;

/// `IGameEventListener2::FireGameEvent`.
const FIRE_GAME_EVENT: VirtualFunction<FireGameEvent> = VirtualFunction::new(FIRE_GAME_EVENT_SLOT);

static ROUTE: DeliveryRoute = DeliveryRoute(Cell::new(None));

/// Why the hook on the events sent to clients could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryHookError {
	/// The engine's clients could not be found, or are not laid out as the
	/// hook expects. [`HookTargetError::NotReady`] means the engine has no
	/// client object yet; see the
	/// [module documentation](crate::hooks::delivery#when-to-install).
	#[error(transparent)]
	Target(#[from] HookTargetError),

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means the
	/// clients' class is already hooked, which callers installing on every
	/// level start can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

/// The callback the hook runs, with the hook.
struct DeliveryRoute(Cell<Option<RoutedDeliveries>>);

impl DeliveryRoute {
	/// Whether the route's hook is installed, for this load of the plugin.
	fn installed(&self, api: MetamodApi<'_>) -> bool {
		self.0.get().is_some_and(|routed| api.has_hook(routed.hook))
	}
}

impl Handler<FireGameEvent> for DeliveryRoute {
	fn call(&self, call: &HookCall<'_, FireGameEvent>) -> HookAction<()> {
		// An earlier hook, such as another plugin's, withheld it.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (event,) = call.args();

		let (Some(routed), Some(listener), Some(event)) =
			(self.0.get(), NonNull::new(call.this()), NonNull::new(event))
		else {
			return HookAction::Ignore;
		};

		// SAFETY: The hook runs before `FireGameEvent` of the vtable `hook_target`
		// found, the clients', on the main thread, with the engine's client and
		// event. Panics are caught inside.
		match unsafe { route_event(&routed.binding, listener, event, routed.callback) } {
			Delivery::Deliver => HookAction::Ignore,
			Delivery::Withhold => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread.
unsafe impl Sync for DeliveryRoute {}

#[derive(Clone, Copy)]
struct RoutedDeliveries {
	binding: ServerBinding,
	callback: DeliveryFn,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Runs `callback` before the engine sends each event to each of its
	/// clients, which decides whether that client gets it; see the
	/// [module documentation](crate::hooks::delivery).
	///
	/// This hooks `IGameEventListener2::FireGameEvent` before the call, on the
	/// class of the engine's clients, which [`hook_target`] finds and checks.
	/// The engine creates its client objects as players first connect, so this
	/// fails with [`HookTargetError::NotReady`] until someone has. Installing
	/// again while the hook is installed returns
	/// [`HookError::AlreadyInstalled`].
	pub fn hook_event_delivery(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: DeliveryFn,
	) -> Result<(), DeliveryHookError> {
		if ROUTE.installed(self) {
			return Err(HookError::AlreadyInstalled.into());
		}

		let listener = hook_target(server)?;

		// SAFETY: `hook_target` checked that the listener is one of the engine's
		// clients, whose class lasts as long as the engine, and confirmed their
		// layout for `route_event`.
		Ok(unsafe { self.install_event_delivery(listener, binding, callback) }?)
	}

	/// Hooks `FireGameEvent` on the class of `listener`.
	///
	/// # Safety
	///
	/// `listener` must be the `IGameEventListener2` base of a live client of
	/// the engine's, which [`listener_of_client`] returned, and its vtable must
	/// hold a function of the signature [`FireGameEvent`] at
	/// [`FIRE_GAME_EVENT_SLOT`], until Metamod unloads the plugin.
	///
	/// [`listener_of_client`]: source_sdk_2013::raw::net::events::listener_of_client
	unsafe fn install_event_delivery(
		self,
		listener: NonNull<sys::IGameEventListener2>,
		binding: ServerBinding,
		callback: DeliveryFn,
	) -> Result<(), HookError> {
		if ROUTE.installed(self) {
			return Err(HookError::AlreadyInstalled);
		}

		// SAFETY: As the caller promises; a `MetamodApi` only exists on the main
		// thread.
		let hook = unsafe {
			self.add_hook(
				FIRE_GAME_EVENT,
				HookTarget::class_of(listener),
				HookTiming::Pre,
				&ROUTE,
			)
		}?;

		ROUTE.0.set(Some(RoutedDeliveries {
			binding,
			callback,
			hook,
		}));
		Ok(())
	}
}
