//! TF2 voice chat hooks, which run after TF2 decides whether a player hears
//! another's voice chat, and may change the decision.
//!
//! The game's voice manager asks TF2's voice chat helper,
//! `CVoiceGameMgrHelper::CanPlayerHearPlayer`, which players hear whom, every
//! [`VOICE_MASK_UPDATE_INTERVAL`](source_sdk_2013::raw::tf2::voice_chat::VOICE_MASK_UPDATE_INTERVAL)
//! seconds, for each listener and each talker,
//! and tells the engine, which relays each talker's voice to those who hear
//! them. TF2 lets players hear their team, and dead players only their dead
//! teammates while a round is on, unless `tf_gravetalk` is set, and every
//! team hear each other in Mann vs. Machine.
//!
//! The manager asks only for listeners whose client has voice enabled, which
//! bots never do, and does not ask at all while `sv_alltalk` is on, when
//! everyone hears everyone. A changed decision applies at the next update.
//! The players' own mutes still apply.
//!
//! [`MetamodApi::hook_voice_chat`] hooks the helper's class, found by its
//! run-time type information, as the [`ClassTargets`] the plugin loaded find
//! it. Under SourceHook, the hooks take one hook manager, which every handle
//! shares.

#[cfg(test)]
#[path = "tests/voice_chat_hooks.rs"]
mod tests;

use crate::MetamodApi;
use crate::hook::VirtualFunction;
use crate::hook::{Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming};
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::voice_chat::{
	CAN_PLAYER_HEAR_PLAYER_SLOT, CanPlayerHearPlayerFn as CanPlayerHearPlayer,
	VOICE_GAME_MGR_HELPER_CLASS,
};

use source_sdk_2013::tf2::class_targets::{ClassTargetError, ClassTargets};
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::c_void;
use std::fmt::{self, Debug, Formatter};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// A callback-scoped server, the listener, the talker, and whether the game,
/// or an earlier hook, lets the listener hear the talker, deciding whether
/// they do. A panic is contained by the hook dispatcher, and keeps the
/// decision.
pub type VoiceChatFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>, bool) -> VoiceChatAction;

/// `CanPlayerHearPlayer` in the voice chat helper's primary vtable.
const CAN_PLAYER_HEAR_PLAYER: VirtualFunction<CanPlayerHearPlayer> =
	VirtualFunction::new(CAN_PLAYER_HEAR_PLAYER_SLOT);

/// Why the voice chat hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum VoiceChatHookError {
	/// Metamod refused the hook.
	#[error(transparent)]
	Hook(#[from] HookError),

	/// The voice chat helper's class has no unique primary vtable in the game
	/// module.
	#[error(transparent)]
	Target(#[from] ClassTargetError),
}

/// Whether a player hears another's voice chat.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum VoiceChatAction {
	/// Keeps the decision as the game, or an earlier hook, made it.
	#[default]
	Continue,

	/// Lets the listener hear the talker.
	Hear,

	/// Keeps the listener from hearing the talker.
	Mute,
}

/// A voice chat hook, which [`Self::remove`] removes.
///
/// Metamod disables the hook while paused and removes it before unloading the
/// plugin. Dropping the handle keeps it.
#[must_use = "retain voice chat hooks to support explicitly removing them"]
pub struct VoiceChatHooks {
	hook: HookId,
	route: &'static VoiceRoute,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl VoiceChatHooks {
	/// Removes the hook, after which the callback no longer runs.
	pub fn remove(self, api: MetamodApi<'_>) {
		self.route.removed.set(true);
		api.remove_hook(self.hook);
	}
}

impl Debug for VoiceChatHooks {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("VoiceChatHooks")
			.field("hook", &self.hook)
			.finish_non_exhaustive()
	}
}

impl MetamodApi<'_> {
	/// Runs `callback` after each `CVoiceGameMgrHelper::CanPlayerHearPlayer`,
	/// once TF2 decided whether a player hears another's voice chat; see the
	/// [module documentation](crate::voice_chat_hooks).
	///
	/// [`VoiceChatAction::Hear`] and [`VoiceChatAction::Mute`] change the
	/// decision. Finds the helper's class in `targets`, which must come from
	/// the server `binding` describes. Each call installs another hook, which
	/// runs the callback once per decision. The callback runs for each pair of
	/// players at each update, so it should be cheap, and must not delete
	/// entities immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue the hook's activation on its worker,
	/// so a returned handle means registration was accepted, not that the next
	/// update will be intercepted.
	pub fn hook_voice_chat(
		self,
		targets: &ClassTargets<'_>,
		binding: ServerBinding,
		callback: VoiceChatFn,
	) -> Result<VoiceChatHooks, VoiceChatHookError> {
		let helper = targets
			.find(VOICE_GAME_MGR_HELPER_CLASS, CAN_PLAYER_HEAR_PLAYER_SLOT)
			.ok_or(ClassTargetError::NotFound(VOICE_GAME_MGR_HELPER_CLASS))?;

		// SAFETY: The class of TF2's voice chat helper derives from
		// `IVoiceGameMgrHelper` through its primary base, so its vtable holds
		// `CanPlayerHearPlayer` at its slot. The game module stays loaded until
		// Metamod unloads the plugin.
		unsafe { self.install_voice_chat(helper.as_ptr(), binding, callback) }
	}

	/// Hooks `CanPlayerHearPlayer` through `vtable` with `callback`.
	///
	/// # Safety
	///
	/// `vtable` must be live, hold a function of the signature
	/// [`CanPlayerHearPlayer`] at [`CAN_PLAYER_HEAR_PLAYER_SLOT`], called with
	/// live players, and stay loaded until Metamod unloads the plugin.
	unsafe fn install_voice_chat(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: VoiceChatFn,
	) -> Result<VoiceChatHooks, VoiceChatHookError> {
		let route = Box::leak(Box::new(VoiceRoute {
			binding,
			callback,
			removed: Cell::new(false),
		}));

		// SAFETY: As the caller promises.
		let hook = unsafe {
			self.add_hook(
				CAN_PLAYER_HEAR_PLAYER,
				HookTarget::vtable(vtable),
				HookTiming::Post,
				&*route,
			)
		}?;

		Ok(VoiceChatHooks {
			hook,
			route,
			_not_thread_safe: PhantomData,
		})
	}
}

/// The handler of one voice chat hook.
struct VoiceRoute {
	binding: ServerBinding,
	callback: VoiceChatFn,

	/// Whether [`VoiceChatHooks::remove`] removed the hook.
	removed: Cell<bool>,
}

impl Handler<CanPlayerHearPlayer> for VoiceRoute {
	fn call(&self, call: &HookCall<'_, CanPlayerHearPlayer>) -> HookAction<bool> {
		if self.removed.get() {
			return HookAction::Ignore;
		}

		let (listener, talker, _proximity) = call.args();

		let (Some(listener), Some(talker)) = (
			NonNull::new(listener.cast::<sys::CBaseEntity>()),
			NonNull::new(talker.cast::<sys::CBaseEntity>()),
		) else {
			return HookAction::Ignore;
		};

		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { self.binding.server(&scope) };

		// SAFETY: The voice manager asks about the live players it finds by
		// their index, whose entity base is at their start.
		let (listener, talker) = unsafe {
			(
				Entity::from_live(server, listener),
				Entity::from_live(server, talker),
			)
		};

		let hears = call.return_value().unwrap_or_default();

		match (self.callback)(server, listener, talker, hears) {
			VoiceChatAction::Continue => HookAction::Ignore,
			VoiceChatAction::Hear => HookAction::Override(true),
			VoiceChatAction::Mute => HookAction::Override(false),
		}
	}
}
