//! TF2 chat hooks, which run after the game's `CBasePlayer::CheckChatText`
//! with the text of each message a player says in chat.
//!
//! The game's `Host_Say` (`game/server/client.cpp:173-379`) runs the `say` and
//! `say_team` commands of humans and bots alike, such as those a plugin runs
//! for a bot through `IServerPluginHelpers::ClientCommand`. Once the player
//! may speak (`CanSpeak`), it calls the player's `CheckChatText` with the
//! message's text, its surrounding quotes trimmed, and cut at the first line
//! break or escape and at 127 bytes. It then checks the text again, and sends
//! the message to each player who can read it, unless the text is empty by
//! then. The text is the message alone, without the name and location TF2's
//! chat format adds. What the server's console says has no player, and
//! reaches no hook.
//!
//! Install for each distinct player class (for example as each player is put
//! in the server; bots have a vtable of their own). Hooks cover that class,
//! including subsequently connected players of the same class, until removed
//! or the plugin unloads. As with other Metamod hooks, they stop calling
//! handlers while the plugin is paused.

#[cfg(test)]
#[path = "tests/chat_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::chat::{CHECK_CHAT_TEXT_SLOT, CheckChatTextFn as CheckChatText};
use source_sdk_2013::raw::util::cstr::borrow_cstr;
use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{CStr, c_void};
use std::ptr::NonNull;

/// A callback-scoped server, the player saying a message in chat, and a copy
/// of its text. A panic is contained by the hook dispatcher.
pub type ChatFn = for<'s> fn(Server<'s>, Entity<'s>, &CStr);

/// `CheckChatText` in a TF2 player's primary vtable.
const CHECK_CHAT_TEXT: VirtualFunction<CheckChatText> = VirtualFunction::new(CHECK_CHAT_TEXT_SLOT);

static ROUTES: [ChatRoute; 8] = [const { ChatRoute::new() }; 8];

/// Why a chat hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum ChatHookError {
	/// The server does not run TF2, or the entity is not a TF2 player.
	#[error("chat hooks require a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means this
	/// player's class is already hooked, which callers installing on every
	/// player can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct ChatRoute {
	state: Cell<Option<RoutedChat>>,
}

impl ChatRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<CheckChatText> for ChatRoute {
	fn call(&self, call: &HookCall<'_, CheckChatText>) -> HookAction<()> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (text, _size) = call.args();
		let Some(player) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		// SAFETY: The game passes the message's terminated text, which outlives
		// the call, and is only copied here.
		let Some(text) = (unsafe { borrow_cstr(text.cast_const()) }).map(CStr::to_owned) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live player whose method ran, which
		// stays in the entity list through the call.
		let player = unsafe { Entity::from_live(server, player) };

		(route.callback)(server, player, text.as_c_str());
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for ChatRoute {}

#[derive(Clone, Copy)]
struct RoutedChat {
	binding: ServerBinding,
	callback: ChatFn,
	hook: HookId,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Runs `callback` after each `CBasePlayer::CheckChatText` of a TF2 player
	/// of this player's class, with the text of the message they are saying in
	/// chat, which the game sends next unless the text is empty by then; see
	/// the [module documentation](crate::chat_hooks).
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots. Repeating a class returns
	/// [`HookError::AlreadyInstalled`]; removing its ID allows replacement.
	///
	/// The hook runs after the call even if another plugin's hook superseded
	/// it, which does not keep the game from sending the message, and sees the
	/// text as the method and the hooks before it left it.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next
	/// message will be intercepted. KHook polls every 5 ms and can retry while
	/// a detour is busy, so messages shortly after installation can be missed.
	pub fn hook_player_chat(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: ChatFn,
	) -> Result<HookId, ChatHookError> {
		if binding.game() != Game::TeamFortress2
			|| !player
				.server_class()
				.is_some_and(|class| class.name() == c"CTFPlayer")
		{
			return Err(ChatHookError::NotTfPlayer);
		}

		// SAFETY: The player is a live TF2 player, whose vtable belongs to the
		// server DLL, which outlives this plugin.
		unsafe { self.install_chat(NonNull::new(player.as_ptr()).unwrap(), binding, callback) }
	}

	/// Hooks `CheckChatText` on the class of `object`.
	///
	/// # Safety
	///
	/// `object` must be live, and its primary vtable must hold a function of
	/// the signature [`CheckChatText`] at [`CHECK_CHAT_TEXT_SLOT`], and stay
	/// loaded until Metamod unloads the plugin.
	unsafe fn install_chat(
		self,
		object: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		callback: ChatFn,
	) -> Result<HookId, ChatHookError> {
		// SAFETY: The object is live, and starts with its primary vtable
		// pointer, of which only the address is used.
		let vtable = unsafe { vtable_pointer::<c_void>(object.as_ptr()) }.addr();
		if ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| state.vtable == vtable && self.has_hook(state.hook))
		}) {
			return Err(HookError::AlreadyInstalled.into());
		}
		let route = ROUTES
			.iter()
			.find(|route| {
				route
					.state
					.get()
					.is_none_or(|state| !self.has_hook(state.hook))
			})
			.ok_or(HookError::TooManyFunctions)?;
		// SAFETY: As the caller promises, the object is live, and its vtable has
		// `void (char *, int)` at the slot, and stays loaded.
		let hook = unsafe {
			self.add_hook(
				CHECK_CHAT_TEXT,
				HookTarget::class_of(object),
				HookTiming::Post,
				route,
			)
		}?;
		route.state.set(Some(RoutedChat {
			binding,
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}
}
