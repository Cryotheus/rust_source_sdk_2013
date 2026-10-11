//! TF2 player hooks that run before the game's `CTFPlayer::DamageEffect`, and
//! may skip the effects it shows of a hit.
//!
//! `CTFPlayer::OnTakeDamage` calls it for each hit the player took, after the
//! damage (see [`DAMAGE_EFFECT_SLOT`]): it flashes the player's screen red for crushing damage and blue for
//! drowning, spawns blood for slashing damage, and plays the sound of a
//! bullet's impact for bullets. Skipping it skips nothing else of the hit:
//! not the damage, nor the pain sound the game plays, such as the drowning
//! noise, nor the `player_hurt` event or the damage indicator.
//!
//! Install for each distinct player class (for example as each player is put
//! in the server; bots have a vtable of their own). Hooks cover that class,
//! including subsequently connected players of the same class, until removed
//! or the plugin unloads. As with other Metamod hooks, they stop calling
//! handlers while the plugin is paused.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/damage_effect.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::damage::{DAMAGE_EFFECT_SLOT, DamageEffectFn as DamageEffect};
use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::tf2::damage::DamageType;
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

/// A callback-scoped server, the player hit, the damage they took and its
/// types, which decides whether the game shows the hit's effects. A panic is
/// contained by the hook dispatcher.
pub type DamageEffectFn = for<'s> fn(Server<'s>, Entity<'s>, f32, DamageType) -> DamageEffectAction;

/// `DamageEffect` in a TF2 player's primary vtable.
const DAMAGE_EFFECT: VirtualFunction<DamageEffect> = VirtualFunction::new(DAMAGE_EFFECT_SLOT);

static ROUTES: [DamageEffectRoute; 8] = [const { DamageEffectRoute::new() }; 8];

/// What a damage effect hook does with the effects of a hit.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DamageEffectAction {
	/// Lets the game show them.
	#[default]
	Show,

	/// Skips `DamageEffect`, so that the game shows none of them.
	Skip,
}

/// Why a damage effect hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum DamageEffectHookError {
	/// The server does not run TF2, or the entity is not a TF2 player.
	#[error("damage effect hooks require a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means this
	/// player's class is already hooked, which callers installing on every
	/// player can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct DamageEffectRoute {
	state: Cell<Option<RoutedDamageEffect>>,
}

impl DamageEffectRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<DamageEffect> for DamageEffectRoute {
	fn call(&self, call: &HookCall<'_, DamageEffect>) -> HookAction<()> {
		// An earlier hook already skipped the effects.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let Some(player) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let (damage, damage_type) = call.args();
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live player whose method is about
		// to run, which stays in the entity list through the call.
		let player = unsafe { Entity::from_live(server, player) };
		let damage_type = DamageType::from_bits_retain(damage_type as u32);

		match (route.callback)(server, player, damage, damage_type) {
			DamageEffectAction::Show => HookAction::Ignore,
			DamageEffectAction::Skip => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for DamageEffectRoute {}

#[derive(Clone, Copy)]
struct RoutedDamageEffect {
	binding: ServerBinding,
	callback: DamageEffectFn,
	hook: HookId,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CTFPlayer::DamageEffect` of a TF2 player
	/// of this player's class, which shows the effects of a hit they took
	/// unless the callback returns [`DamageEffectAction::Skip`].
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots. Repeating a class returns
	/// [`HookError::AlreadyInstalled`]; removing its ID allows replacement.
	///
	/// The callback is not run once an earlier hook skipped the effects, as
	/// far as the hooking library reports it (see [`crate::hook`]). It runs
	/// within the hit's `OnTakeDamage`, so a plugin that deals the damage
	/// itself can tell its own hits by noting them as it deals them.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next hit
	/// will be intercepted. KHook polls every 5 ms and can retry while a
	/// detour is busy, so hits shortly after installation can be missed.
	pub fn hook_player_damage_effect(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: DamageEffectFn,
	) -> Result<HookId, DamageEffectHookError> {
		if binding.game() != Game::TeamFortress2
			|| !player
				.server_class()
				.is_some_and(|class| class.name() == c"CTFPlayer")
		{
			return Err(DamageEffectHookError::NotTfPlayer);
		}

		// SAFETY: The player is a live TF2 player, whose vtable belongs to the
		// server DLL, which outlives this plugin.
		unsafe {
			self.install_damage_effect(NonNull::new(player.as_ptr()).unwrap(), binding, callback)
		}
	}

	/// Hooks `DamageEffect` on the class of `object`.
	///
	/// # Safety
	///
	/// `object` must be live, and its primary vtable must hold a function of
	/// the signature [`DamageEffect`] at [`DAMAGE_EFFECT_SLOT`], and stay
	/// loaded until Metamod unloads the plugin.
	unsafe fn install_damage_effect(
		self,
		object: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		callback: DamageEffectFn,
	) -> Result<HookId, DamageEffectHookError> {
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
		// `void (float, int)` at the slot, and stays loaded.
		let hook = unsafe {
			self.add_hook(
				DAMAGE_EFFECT,
				HookTarget::class_of(object),
				HookTiming::Pre,
				route,
			)
		}?;
		route.state.set(Some(RoutedDamageEffect {
			binding,
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}
}
