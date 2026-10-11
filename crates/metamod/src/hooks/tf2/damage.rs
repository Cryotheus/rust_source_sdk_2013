//! TF2 damage hooks with owned, editable damage arguments: players', and
//! any other entity's, before the game handles the damage, and after.
//!
//! Install for each distinct player class (bots can have a different vtable),
//! or entity class: on the class of a live entity, for example on player
//! activation, or before any entity of a class exists, on a
//! [`ClassTarget`] that
//! [`ClassTargets`](source_sdk_2013::tf2::class_targets::ClassTargets) finds
//! as the plugin loads. Hooks cover that class, including subsequently
//! connected players or created entities of the same class, until removed or
//! the plugin unloads. As with other Metamod hooks, they stop calling
//! handlers while the plugin is paused.
//!
//! Under SourceHook, each stage's method takes a hook manager, which hooks
//! before and after it share.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/damage.rs"]
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
use source_sdk_2013::tf2::class_targets::{BaseEntity, ClassTarget, DerivesFrom, TfPlayer};
use source_sdk_2013::tf2::damage::DamageEvent;
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{c_int, c_void};
use std::ptr::{self, NonNull};

/// A callback-scoped server and victim with an independently owned record.
/// A panic is contained by the hook dispatcher and lets the game continue
/// with the original damage arguments.
pub type DamageFn = for<'s> fn(Server<'s>, DamageStage, &mut DamageEvent<'s>) -> DamageAction;

/// A callback-scoped server, the stage the damage was taken at, a copy of
/// the damage the stage's method was called with, and what the call returned.
/// A panic is contained by the hook dispatcher.
pub type DamageTakenFn = for<'s> fn(Server<'s>, DamageStage, &DamageEvent<'s>, c_int);

static TAKEN_ROUTES: [DamageTakenRoute; 32] = [const { DamageTakenRoute::new() }; 32];

struct DamageTakenRoute {
	state: Cell<Option<RoutedDamageTaken>>,
}

impl DamageTakenRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<TakeDamage> for DamageTakenRoute {
	fn call(&self, call: &HookCall<'_, TakeDamage>) -> HookAction<c_int> {
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
		// SAFETY: The class hook supplies the live entity that took the damage,
		// which the game only marks for deletion if it removes it, and the const
		// damage reference the method was called with, which is only copied.
		let event = unsafe { DamageEvent::from_raw(server, victim, info) };

		(route.callback)(
			server,
			route.stage,
			&event,
			call.return_value().unwrap_or_default(),
		);

		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for DamageTakenRoute {}

#[derive(Clone, Copy)]
struct RoutedDamageTaken {
	binding: ServerBinding,
	callback: DamageTakenFn,
	hook: HookId,
	stage: DamageStage,
	vtable: usize,
}

/// The primary vtable of `entity`'s class.
fn vtable_of(entity: Entity<'_>) -> Result<NonNull<*mut c_void>, HookError> {
	// SAFETY: Every live CBaseEntity starts with its primary vtable pointer,
	// of which only the address is used.
	let vtable = unsafe { vtable_pointer::<*mut c_void>(entity.as_ptr()) };

	NonNull::new(vtable.cast_mut()).ok_or(HookError::InvalidArgument)
}

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
	/// distinct class to watch, or before any entity of a class exists with
	/// [`Self::hook_entity_class_damage`], which this is the same hook as. A
	/// player's class is the same hook as [`Self::hook_player_damage`]'s
	/// incoming stage: repeating a class returns
	/// `HookError::AlreadyInstalled`; removing its ID allows replacement.
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
		unsafe { self.install_damage(vtable_of(entity)?, binding, DamageStage::Incoming, callback) }
	}

	/// Hooks incoming or already-scaled damage on this TF2 player's class.
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots, or as the plugin loads with
	/// [`Self::hook_player_class_damage`], which this is the same hook as.
	/// Repeating a class/stage returns `HookError::AlreadyInstalled`; removing
	/// its ID allows replacement.
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
		unsafe { self.install_damage(vtable_of(player)?, binding, stage, callback) }
	}

	/// Hooks incoming damage on `target`'s class, as
	/// [`Self::hook_entity_damage`] does on the class of a live entity: before
	/// any entity of the class exists, such as the buildings' classes that
	/// [`ClassTargets::objects`](source_sdk_2013::tf2::class_targets::ClassTargets::objects)
	/// finds.
	///
	/// `target` must come from this server, and `binding` must describe the
	/// same running server. It is the same hook as `hook_entity_damage`'s on
	/// an entity of the class, and on a player's class as
	/// [`Self::hook_player_class_damage`]'s incoming stage: repeating a class
	/// returns `HookError::AlreadyInstalled`; removing its ID allows
	/// replacement.
	pub fn hook_entity_class_damage<C: DerivesFrom<BaseEntity>>(
		self,
		target: ClassTarget<'_, C>,
		binding: ServerBinding,
		callback: DamageFn,
	) -> Result<HookId, DamageHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(DamageHookError::NotTf2);
		}

		// SAFETY: The target is the primary vtable of an entity class in TF2's
		// game module, which holds `OnTakeDamage` at the slot `CBaseEntity`
		// declares it at, and stays loaded until Metamod unloads this plugin.
		unsafe { self.install_damage(target.as_ptr(), binding, DamageStage::Incoming, callback) }
	}

	/// Hooks incoming or already-scaled damage on `target`'s class of TF2
	/// players, as [`Self::hook_player_damage`] does on the class of a live
	/// player: as the plugin loads, on each of the classes
	/// [`ClassTargets::players`](source_sdk_2013::tf2::class_targets::ClassTargets::players)
	/// finds.
	///
	/// `target` must come from this server, and `binding` must describe the
	/// same running server. It is the same hook as `hook_player_damage`'s on
	/// a player of the class: repeating a class/stage returns
	/// `HookError::AlreadyInstalled`; removing its ID allows replacement.
	pub fn hook_player_class_damage(
		self,
		target: ClassTarget<'_, TfPlayer>,
		stage: DamageStage,
		binding: ServerBinding,
		callback: DamageFn,
	) -> Result<HookId, DamageHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(DamageHookError::NotTfPlayer);
		}

		// SAFETY: The target is the primary vtable of a TF2 player class, which
		// holds both stages' methods at their slots, in TF2's game module, which
		// stays loaded until Metamod unloads this plugin.
		unsafe { self.install_damage(target.as_ptr(), binding, stage, callback) }
	}

	/// Runs `callback` after the incoming damage of the entities of
	/// `target`'s class: once their class's `OnTakeDamage` handled it, and
	/// the death or break it caused, if any.
	///
	/// `target` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Hooking a class with a
	/// callback it is already hooked with returns
	/// `HookError::AlreadyInstalled`; other callbacks can hook it too. At
	/// most 32 classes and callbacks can be hooked at once, those of
	/// [`Self::hook_player_class_damage_taken`] included, past which
	/// installation returns `HookError::TooManyFunctions`.
	///
	/// The callback runs after damage a hook blocked too, and gets what the
	/// call returned: the game's method's value, or that of the hook that
	/// overrode or superseded it, such as 0 for [`DamageAction::Block`]. The
	/// event holds the damage the method was called with. A hook before it
	/// that applied an edited copy, as [`DamageAction::Apply`] does, ran the
	/// method with the copy, which the callback does not see. Installation is
	/// otherwise as for [`Self::hook_player_damage`].
	pub fn hook_entity_class_damage_taken<C: DerivesFrom<BaseEntity>>(
		self,
		target: ClassTarget<'_, C>,
		binding: ServerBinding,
		callback: DamageTakenFn,
	) -> Result<HookId, DamageHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(DamageHookError::NotTf2);
		}

		// SAFETY: As for `hook_entity_class_damage`.
		unsafe {
			self.install_damage_taken(target.as_ptr(), binding, DamageStage::Incoming, callback)
		}
	}

	/// Runs `callback` after the incoming or already-scaled damage of the
	/// players of `target`'s class of TF2 players: after `OnTakeDamage`, once
	/// it handled the death the damage caused, if any, or after
	/// `OnTakeDamage_Alive`, which the game calls in it, before handling the
	/// death.
	///
	/// Installation, what the callback gets, and the limit of hooks are as
	/// for [`Self::hook_entity_class_damage_taken`].
	pub fn hook_player_class_damage_taken(
		self,
		target: ClassTarget<'_, TfPlayer>,
		stage: DamageStage,
		binding: ServerBinding,
		callback: DamageTakenFn,
	) -> Result<HookId, DamageHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(DamageHookError::NotTfPlayer);
		}

		// SAFETY: As for `hook_player_class_damage`.
		unsafe { self.install_damage_taken(target.as_ptr(), binding, stage, callback) }
	}

	/// Hooks the stage's method on the class `vtable` belongs to.
	///
	/// # Safety
	///
	/// `vtable` must be a live entity vtable with a function of the signature
	/// [`TakeDamage`] at the stage's slot, and stay loaded until Metamod
	/// unloads the plugin.
	unsafe fn install_damage(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		stage: DamageStage,
		callback: DamageFn,
	) -> Result<HookId, DamageHookError> {
		let address = vtable.addr().get();
		if ROUTES.iter().any(|route| {
			route.state.get().is_some_and(|state| {
				state.vtable == address && state.stage == stage && self.has_hook(state.hook)
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
		let target = HookTarget::vtable(vtable);
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
			vtable: address,
		}));
		Ok(hook)
	}

	/// Hooks the stage's method after the game's, on the class `vtable`
	/// belongs to.
	///
	/// # Safety
	///
	/// As for [`Self::install_damage`].
	unsafe fn install_damage_taken(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		stage: DamageStage,
		callback: DamageTakenFn,
	) -> Result<HookId, DamageHookError> {
		let address = vtable.addr().get();

		if TAKEN_ROUTES.iter().any(|route| {
			route.state.get().is_some_and(|state| {
				state.vtable == address
					&& state.stage == stage
					&& ptr::fn_addr_eq(state.callback, callback)
					&& self.has_hook(state.hook)
			})
		}) {
			return Err(HookError::AlreadyInstalled.into());
		}

		let route = TAKEN_ROUTES
			.iter()
			.find(|route| {
				route
					.state
					.get()
					.is_none_or(|state| !self.has_hook(state.hook))
			})
			.ok_or(HookError::TooManyFunctions)?;
		// SAFETY: As the caller promises, the vtable has the signature
		// int (CTakeDamageInfo const &) at the slot, and stays loaded.
		let hook = unsafe {
			self.add_hook(
				stage.function(),
				HookTarget::vtable(vtable),
				HookTiming::Post,
				route,
			)
		}?;

		route.state.set(Some(RoutedDamageTaken {
			binding,
			callback,
			hook,
			stage,
			vtable: address,
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
