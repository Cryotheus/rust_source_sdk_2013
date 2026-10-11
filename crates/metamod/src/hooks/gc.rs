//! A hook on the messages the game server sends the Steam Game Coordinator
//! (GC), which can pass or block each one.
//!
//! The hook patches `ISteamGameCoordinator::SendMessage` in the vtable of the
//! coordinator's class, which Steam's client library holds, so it sees every
//! message the game sends, and every other sender in the process through the
//! same class, such as a listen server's client.
//!
//! # When to install
//!
//! The coordinator exists only while the Steam game server runs, which starts
//! just before the first map: a plugin loaded with the server finds none at
//! load. Install when a level starts, and again on later callbacks while it
//! fails; [`MetamodApi::hook_gc_send`] reports an installed hook as
//! [`HookError::AlreadyInstalled`].
//!
//! Installing pins Steam's client library for the rest of the process, as the
//! engine frees it whenever the Steam game server shuts down, on a `map`
//! command, an `sv_lan` change, and quitting. A library loaded again would have
//! a fresh vtable at the same address, so the hook would be lost while both
//! hooking libraries, which keep their patches by address, still report it.
//! [`MetamodApi::is_gc_send_hooked`] detects such a loss.
//!
//! # What gets through
//!
//! As with other Metamod hooks, the handler runs only for sends on the
//! server's main thread, and not while the plugin is paused or after it
//! unloads, so messages sent then reach the GC. Messages the game batches,
//! such as TF2's batched Strange events, are sent when the batch is flushed:
//! those batched while the plugin was paused are seen when it runs again, and
//! those batched before it unloads are not. With Metamod 2.0, KHook keeps the
//! vtable entry pointing to its own code for the rest of the process, and when
//! the entry is already detoured, adds the hook from its worker thread, so the
//! sends just after an install or a reload can pass unseen. Another plugin's
//! hook on the same function can change the result the game gets.

#[cfg(test)]
#[path = "../tests/hooks/gc.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::raw::steam::{ISteamGameCoordinator, SEND_MESSAGE_SLOT, SendMessageFn};
use source_sdk_2013::steam::{GameCoordinator, GcResult, MessageType, SteamClientError};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

/// Decides what happens to a message the game sends the GC. A panic is
/// contained by the hook dispatcher, and lets the message through.
///
/// The handler gets no [`Server`](source_sdk_2013::Server): deciding needs
/// only the message, and nothing touches the engine from inside Steam's
/// calls.
pub type GcSendFn = fn(&GcSend<'_>) -> GcSendAction;

/// `ISteamGameCoordinator::SendMessage`.
const SEND_MESSAGE: VirtualFunction<SendMessageFn> = VirtualFunction::new(SEND_MESSAGE_SLOT);

static ROUTE: GcRoute = GcRoute::new();

/// Why the GC hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum GcHookError {
	/// Steam's client library could not be pinned, or the coordinator is not
	/// in it.
	#[error(transparent)]
	SteamClient(#[from] SteamClientError),

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means the
	/// coordinator's class is already hooked, which callers installing on
	/// every level start can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct GcRoute {
	state: Cell<Option<RoutedGc>>,
}

impl GcRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<SendMessageFn> for GcRoute {
	fn call(&self, call: &HookCall<'_, SendMessageFn>) -> HookAction<i32> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (wire, data, len) = call.args();
		let (message_type, protobuf) = MessageType::from_wire(wire);

		let data = match data.is_null() || len == 0 {
			true => &[][..],

			// SAFETY: The game passes `len` readable bytes at `data`, which
			// `SendMessage` takes as const and which outlive the call.
			false => unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) },
		};

		let send = GcSend {
			message_type,
			protobuf,
			data,
		};

		match (route.callback)(&send) {
			GcSendAction::Pass => HookAction::Ignore,
			GcSendAction::Block(result) => HookAction::Supersede(result.as_raw()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for GcRoute {}

/// A message the game is sending the GC, as a [`GcSendFn`] sees it.
#[derive(Debug, Clone, Copy)]
pub struct GcSend<'a> {
	/// The message's type.
	pub message_type: MessageType,

	/// Whether the message is a protobuf message.
	pub protobuf: bool,

	/// The message after its type: its header, then its body. Valid only for
	/// the call.
	pub data: &'a [u8],
}

/// What to do with a message the game sends the GC.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum GcSendAction {
	/// Send it.
	#[default]
	Pass,

	/// Do not send it, and return this result to the game instead.
	Block(GcResult),
}

#[derive(Clone, Copy)]
struct RoutedGc {
	callback: GcSendFn,
	hook: HookId,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each message the game sends the GC through
	/// `coordinator`'s class, which decides whether the message is sent; see
	/// the [module documentation](crate::hooks::gc).
	///
	/// Pins Steam's client library for the rest of the process first, and
	/// fails if the coordinator is not in it. Returns a removable hook ID.
	/// Installing again while the class is hooked returns
	/// [`HookError::AlreadyInstalled`]; removing the ID allows replacement.
	/// One class can be hooked at a time: while another class's hook is
	/// installed, this returns [`HookError::TooManyFunctions`].
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload, so
	/// a returned ID means registration was accepted, not that the next send
	/// will be intercepted.
	pub fn hook_gc_send(
		self,
		coordinator: GameCoordinator<'_>,
		callback: GcSendFn,
	) -> Result<HookId, GcHookError> {
		let object = NonNull::new(coordinator.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: The coordinator is live for its scope, and starts with its
		// vtable pointer, of which only the address is used.
		if self.routed(unsafe { vtable_of(object) }).is_some() {
			return Err(HookError::AlreadyInstalled.into());
		}

		coordinator.pin_steam_client()?;

		// SAFETY: The coordinator is live, its vtable has `SendMessage` at the
		// slot, and Steam's client library, which holds the vtable, was just
		// pinned for the rest of the process.
		Ok(unsafe { self.install_send(object, callback) }?)
	}

	/// Hooks `SendMessage` on the class of `object`.
	///
	/// # Safety
	///
	/// `object` must be live, and its vtable must hold a function of the
	/// signature [`SendMessageFn`] at [`SEND_MESSAGE_SLOT`], and stay loaded
	/// until Metamod unloads the plugin.
	unsafe fn install_send(
		self,
		object: NonNull<ISteamGameCoordinator>,
		callback: GcSendFn,
	) -> Result<HookId, HookError> {
		// SAFETY: As the caller promises, the object is live.
		let vtable = unsafe { vtable_of(object) };

		match ROUTE.state.get() {
			Some(state) if self.has_hook(state.hook) => {
				return Err(match state.vtable == vtable {
					true => HookError::AlreadyInstalled,
					false => HookError::TooManyFunctions,
				});
			}

			_ => {}
		}

		// SAFETY: As the caller promises, the object is live, and its vtable has
		// `EGCResults (uint32, const void *, uint32)` at the slot, and stays
		// loaded.
		let hook = unsafe {
			self.add_hook(
				SEND_MESSAGE,
				HookTarget::class_of(object),
				HookTiming::Pre,
				&ROUTE,
			)
		}?;

		ROUTE.state.set(Some(RoutedGc {
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}

	/// Whether the hook [`Self::hook_gc_send`] installed is still in place
	/// for `coordinator`'s class: installed for this load of the plugin, and
	/// its vtable slot still patched by the hooking library.
	///
	/// The patch is lost if Steam's client library was freed and loaded again
	/// before [`Self::hook_gc_send`] pinned it. With Metamod 2.0, KHook keeps
	/// the slot detoured once it has hooked it, so a hook its worker thread is
	/// still adding, such as just after a reload, already reads as in place:
	/// the result says the slot is patched, not that the handler runs yet.
	pub fn is_gc_send_hooked(self, coordinator: GameCoordinator<'_>) -> bool {
		let Some(object) = NonNull::new(coordinator.as_ptr()) else {
			return false;
		};

		// SAFETY: The coordinator is live for its scope, and starts with its
		// vtable pointer.
		let vtable = unsafe { vtable_of(object) };

		if self.routed(vtable).is_none() {
			return false;
		}

		// SAFETY: The coordinator is live, and its vtable has `SendMessage` at
		// the slot. The hook pinned the library that holds it.
		let Ok(original) =
			(unsafe { self.original_function(SEND_MESSAGE, HookTarget::class_of(object)) })
		else {
			return false;
		};

		// SAFETY: The vtable is live and has the slot, which is read as an
		// address only.
		let current = unsafe {
			(vtable as *const *const c_void)
				.add(SEND_MESSAGE_SLOT)
				.read()
		};

		current != original as *const c_void
	}

	/// The installed hook of the class whose vtable is at `vtable`, if any.
	fn routed(self, vtable: usize) -> Option<HookId> {
		ROUTE
			.state
			.get()
			.filter(|state| state.vtable == vtable && self.has_hook(state.hook))
			.map(|state| state.hook)
	}
}

/// The address of `object`'s vtable.
///
/// # Safety
///
/// `object` must be live.
unsafe fn vtable_of(object: NonNull<ISteamGameCoordinator>) -> usize {
	// SAFETY: As the caller promises; a polymorphic object starts with its
	// vtable pointer, of which only the address is read.
	unsafe { (&raw const (*object.as_ptr()).vtable_).read() }.addr()
}
