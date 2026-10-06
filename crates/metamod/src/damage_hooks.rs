//! TF2 damage hooks with owned, editable damage arguments: players', and
//! any other entity's.
//!
//! Install for each distinct player class (for example on player activation;
//! bots can have a different vtable), or entity class. Hooks cover that
//! class, including subsequently connected players or created entities of
//! the same class, until removed or the plugin unloads. As with other
//! Metamod hooks, they stop calling handlers while the plugin is paused.

#[cfg(test)]
#[path = "tests/damage_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::damage::{
	ON_TAKE_DAMAGE_ALIVE_SLOT, ON_TAKE_DAMAGE_SLOT, TakeDamageFn as TakeDamage,
};

use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::tf2::damage::DamageEvent;
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

/// A callback-scoped server and victim with an independently owned record.
/// A panic is contained by the hook dispatcher and lets the game continue
/// with the original damage arguments.
pub type DamageFn = for<'s> fn(Server<'s>, DamageStage, &mut DamageEvent<'s>) -> DamageAction;

static ROUTES: [DamageRoute; 32] = [const { DamageRoute::new() }; 32];

/// What to do with the original call after inspecting or editing damage.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DamageAction {
	/// Continue normally. Any edits to the owned copy are discarded.
	#[default]
	Continue,
	/// Call the original virtual method with the edited copy and supersede
	/// this invocation. This bypasses other plugins' subsequent hooks on
	/// this method; nested calls (including the Alive stage) run normally.
	/// Other hooks observing the original invocation still see its original
	/// arguments. No plugin's const argument storage is overwritten.
	Apply,
	/// Skip damage and return zero without invoking the original method.
	Block,
}

#[derive(Debug, thiserror::Error)]
pub enum DamageHookError {
	#[error("damage hooks require a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,
	#[error("entity damage hooks require a TF2 server")]
	NotTf2,
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct DamageRoute {
	state: Cell<Option<RoutedDamage>>,
}

impl DamageRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<TakeDamage> for DamageRoute {
	fn call(&self, call: &HookCall<'_, TakeDamage>) -> HookAction<c_int> {
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (info,) = call.args();
		let (Some(victim), Some(info)) = (NonNull::new(call.this()), NonNull::new(info.cast_mut()))
		else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies a live entity, a const damage
		// reference and the matching original virtual method.
		unsafe {
			dispatch(
				server,
				route.stage,
				route.original,
				route.callback,
				victim,
				info,
			)
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for DamageRoute {}

/// Where in TF2's damage processing a hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageStage {
	/// `CTFPlayer::OnTakeDamage`, before TF2 applies damage rules. Changes to
	/// amount, type and incoming critical classification flow through those
	/// rules, and can affect the rest of the damage and death processing.
	Incoming,
	/// `CTFPlayer::OnTakeDamage_Alive`, after full and mini critical bonuses
	/// were calculated, before health loss and resistance processing. Use
	/// `DamageInfo::apply_critical_policy` here. Earlier visuals and assist
	/// statistics, and the caller's later death processing, are not rewritten.
	Alive,
}

impl DamageStage {
	/// The stage's method in a TF2 player's primary vtable.
	const fn function(self) -> VirtualFunction<TakeDamage> {
		VirtualFunction::new(match self {
			Self::Incoming => ON_TAKE_DAMAGE_SLOT,
			Self::Alive => ON_TAKE_DAMAGE_ALIVE_SLOT,
		})
	}
}

#[derive(Clone, Copy)]
struct RoutedDamage {
	binding: ServerBinding,
	callback: DamageFn,
	original: TakeDamage,
	hook: HookId,
	stage: DamageStage,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Hooks incoming damage on this TF2 entity's class, before the class's
	/// `OnTakeDamage` handles it, as [`DamageStage::Incoming`]. For an entity
	/// other than a player, that is the damage that pushes a `prop_ragdoll` or
	/// breaks a prop; the game skips it for entities that take no damage
	/// (`DAMAGE_NO`), and radius damage also for those out of its sight.
	///
	/// `entity` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class to watch. A player's class is the same hook as
	/// [`Self::hook_player_damage`]'s incoming stage: repeating a class
	/// returns `HookError::AlreadyInstalled`; removing its ID allows
	/// replacement.
	///
	/// Installation is as for [`Self::hook_player_damage`], and so is what
	/// [`DamageAction::Apply`] does.
	pub fn hook_entity_damage(
		self,
		entity: Entity<'_>,
		binding: ServerBinding,
		callback: DamageFn,
	) -> Result<HookId, DamageHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(DamageHookError::NotTf2);
		}

		// SAFETY: Every TF2 entity's primary vtable has `OnTakeDamage` at the
		// slot, which belongs to `CBaseEntity`, and the entity is live. Its vtable
		// belongs to the server DLL, which outlives this plugin.
		unsafe {
			self.install_damage(
				NonNull::new(entity.as_ptr()).unwrap(),
				binding,
				DamageStage::Incoming,
				callback,
			)
		}
	}

	/// Hooks incoming or already-scaled damage on this TF2 player's class.
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots. Repeating a class/stage
	/// returns `HookError::AlreadyInstalled`; removing its ID allows replacement.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that an immediate
	/// synthetic call will be intercepted. Install during player activation
	/// and keep the hook registered; for probes, fire damage from a later
	/// engine callback. KHook polls every 5 ms and can retry while a detour is
	/// busy, so a fixed single-frame delay is not a readiness guarantee.
	///
	/// `Apply` calls the original with a copy, so later hooks on the same
	/// invocation cannot transform that copy. Use `Continue` for observation
	/// and document this ordering when composing multiple plugins.
	pub fn hook_player_damage(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		stage: DamageStage,
		callback: DamageFn,
	) -> Result<HookId, DamageHookError> {
		if binding.game() != Game::TeamFortress2
			|| !player
				.server_class()
				.is_some_and(|class| class.name() == c"CTFPlayer")
		{
			return Err(DamageHookError::NotTfPlayer);
		}

		// SAFETY: CTFPlayer's primary vtable has these TF2 slots, and the player
		// is live. Its vtable belongs to the server DLL, which outlives this
		// plugin.
		unsafe {
			self.install_damage(
				NonNull::new(player.as_ptr()).unwrap(),
				binding,
				stage,
				callback,
			)
		}
	}

	/// Hooks the stage's method on the class of `object`.
	///
	/// # Safety
	///
	/// `object` must be live, and its primary vtable must hold a function of
	/// the signature [`TakeDamage`] at the stage's slot, and stay loaded until
	/// Metamod unloads the plugin.
	unsafe fn install_damage(
		self,
		object: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		stage: DamageStage,
		callback: DamageFn,
	) -> Result<HookId, DamageHookError> {
		// SAFETY: Every live CBaseEntity starts with its primary vtable pointer,
		// of which only the address is used.
		let vtable = unsafe { vtable_pointer::<c_void>(object.as_ptr()) }.addr();
		if ROUTES.iter().any(|route| {
			route.state.get().is_some_and(|state| {
				state.vtable == vtable && state.stage == stage && self.has_hook(state.hook)
			})
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
		let target = HookTarget::class_of(object);
		// SAFETY: As the caller promises, the vtable has the signature
		// int (CTakeDamageInfo const &) at the slot, on both supported ABIs.
		let original = unsafe { self.original_function(stage.function(), target) }?;
		// SAFETY: The same vtable and signature as checked for original above.
		let hook = unsafe { self.add_hook(stage.function(), target, HookTiming::Pre, route) }?;
		route.state.set(Some(RoutedDamage {
			binding,
			callback,
			original,
			hook,
			stage,
			vtable,
		}));
		Ok(hook)
	}
}

/// # Safety
/// The pointers must be the live arguments of the selected native damage
/// method, and `original` must be its unhooked function of this signature.
unsafe fn dispatch(
	server: Server<'_>,
	stage: DamageStage,
	original: TakeDamage,
	callback: DamageFn,
	victim: NonNull<sys::CBaseEntity>,
	info: NonNull<sys::CTakeDamageInfo>,
) -> HookAction<c_int> {
	// SAFETY: The caller supplies live callback-scoped arguments. Only
	// byte-copying reads the const damage object; all modifications are local.
	let mut event = unsafe { DamageEvent::from_raw(server, victim, info) };
	match callback(server, stage, &mut event) {
		DamageAction::Continue => HookAction::Ignore,
		DamageAction::Block => HookAction::Supersede(0),

		DamageAction::Apply => {
			// SAFETY: Original matches the target method's ABI. Victim lifetime
			// is callback-scoped, and the copy lives through the synchronous
			// call. The Server contract guarantees only deferred removal.
			let result = unsafe { original(victim.as_ptr(), event.info.as_ptr()) };
			HookAction::Supersede(result)
		}
	}
}
