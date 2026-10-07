//! TF2 airblast hooks, which run before a weapon's airblast acts on a player
//! it reached, and may spare the player.
//!
//! The airblast of the Pyro's flame throwers and of the Dragon's Fury goes
//! through the players in front of the weapon's owner, and has the weapon act
//! on each through `CTFWeaponBase::DeflectPlayer` (see
//! [`source_sdk_2013::tf2::airblast`]): it extinguishes a burning teammate,
//! and pushes a player of the other team. The hooks run before it, and their
//! callback decides whether it runs. A spared player is neither pushed nor
//! slowed, nor extinguished, and the airblast counts them as not deflected,
//! which only changes its sound and whether its owner speaks. Projectiles are
//! reflected as usual, as the airblast reflects them through another
//! function.
//!
//! # When to install
//!
//! [`MetamodApi::hook_airblasts`] finds both weapons' classes in the game
//! server module through [`airblast_vtables`], without any weapon, and hooks
//! `DeflectPlayer` in the vtable of each. Install once, such as while
//! loading: the hooks last until removed or the plugin unloads, and
//! installing again returns [`HookError::AlreadyInstalled`] without searching
//! the module again.
//!
//! # What gets through
//!
//! As with other Metamod hooks, the callback does not run while the plugin is
//! paused or after it unloads, so airblasts act on every player as usual then.
//! With Metamod 2.0, KHook keeps a vtable entry it detoured pointing to its
//! own code for the rest of the process, and adds a hook to such an entry from
//! its worker thread, so the airblasts just after a reload can pass unseen.
//! Another plugin's hook on the same function can skip it before the callback
//! runs, which KHook does not report.

#[cfg(test)]
#[path = "tests/airblast_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::airblast::{
	DEFLECT_PLAYER_SLOT, DEFLECT_PROJECTILES_SLOT, DeflectPlayerFn as DeflectPlayer,
	DeflectProjectilesFn as DeflectProjectiles,
};

use source_sdk_2013::tf2::airblast::{AirblastVtableError, AirblastWeapon, airblast_vtables};
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;

/// A callback-scoped server, and a player a weapon's airblast reached, which
/// decides whether the weapon acts on them. A panic is contained by the hook
/// dispatcher, and lets the weapon act.
pub type AirblastFn = for<'s> fn(Server<'s>, AirblastPush<'s>) -> AirblastAction;

/// `DeflectPlayer` in a TF2 weapon's primary vtable.
const DEFLECT_PLAYER: VirtualFunction<DeflectPlayer> = VirtualFunction::new(DEFLECT_PLAYER_SLOT);

/// `DeflectProjectiles` in a TF2 weapon's primary vtable.
const DEFLECT_PROJECTILES: VirtualFunction<DeflectProjectiles> =
	VirtualFunction::new(DEFLECT_PROJECTILES_SLOT);

/// The routes of the weapons, in the order of [`AirblastWeapon::ALL`].
static ROUTES: [AirblastRoute; 2] = [
	AirblastRoute::new(AirblastWeapon::FlameThrower),
	AirblastRoute::new(AirblastWeapon::DragonsFury),
];

/// What an airblast hook does with a player the airblast reached.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AirblastAction {
	/// Lets the weapon act on the player.
	#[default]
	Allow,

	/// Spares the player: the weapon's `DeflectPlayer` is skipped, so the
	/// airblast neither pushes nor extinguishes them, and counts them as not
	/// deflected.
	Refuse,
}

/// Why the airblast hooks could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum AirblastHookError {
	/// The weapon classes' vtables could not be found.
	#[error(transparent)]
	Target(#[from] AirblastVtableError),

	/// The vtables found are not laid out as the weapons' are: before any
	/// hook, [`DEFLECT_PROJECTILES_SLOT`] does not hold one function in the
	/// vtables of both weapons and of the reference, or
	/// [`DEFLECT_PLAYER_SLOT`] does not hold one function in both weapons' and
	/// another in the reference's.
	#[error("the airblast weapons' vtables do not have the expected layout")]
	UnexpectedLayout,

	/// Metamod refused a hook. [`HookError::AlreadyInstalled`] means the
	/// weapons are already hooked.
	#[error(transparent)]
	Hook(#[from] HookError),
}

/// Handles for the airblast hooks installed by one call.
///
/// Metamod disables them while paused and removes them before unloading the
/// plugin. [`Self::remove`] disables them earlier. No weapon is retained, so
/// the hooks last through level changes.
#[must_use = "retain airblast hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct AirblastHooks {
	hooks: Vec<HookId>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl AirblastHooks {
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);

			for route in &ROUTES {
				if route.state.get().is_some_and(|state| state.hook == hook) {
					route.state.set(None);
				}
			}
		}
	}
}

/// A player a weapon's airblast reached, which the weapon is about to act on.
#[derive(Debug, Clone, Copy)]
pub struct AirblastPush<'s> {
	/// The weapon whose airblast it is.
	pub weapon: Entity<'s>,

	/// Which of the weapons with airblast [`Self::weapon`] is.
	pub kind: AirblastWeapon,

	/// The player the airblast reached.
	pub target: Entity<'s>,

	/// The weapon's owner, whose airblast it is. The same player as
	/// [`Self::target`] when the weapon's attributes make its airblast push its
	/// owner instead.
	pub owner: Entity<'s>,
}

struct AirblastRoute {
	weapon: AirblastWeapon,
	state: Cell<Option<RoutedAirblast>>,
}

impl AirblastRoute {
	const fn new(weapon: AirblastWeapon) -> Self {
		Self {
			weapon,
			state: Cell::new(None),
		}
	}
}

impl Handler<DeflectPlayer> for AirblastRoute {
	fn call(&self, call: &HookCall<'_, DeflectPlayer>) -> HookAction<bool> {
		// An earlier hook already skipped the weapon's function.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (target, owner, _forward) = call.args();
		let (Some(weapon), Some(target), Some(owner)) = (
			NonNull::new(call.this()),
			NonNull::new(target),
			NonNull::new(owner),
		) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live weapon whose method runs, and
		// the live players the game passes it, which stay in the entity list
		// through the call: the game only marks entities it removes then for
		// deletion.
		let push = unsafe {
			AirblastPush {
				weapon: Entity::from_live(server, weapon),
				kind: self.weapon,
				target: Entity::from_live(server, target),
				owner: Entity::from_live(server, owner),
			}
		};

		match (route.callback)(server, push) {
			AirblastAction::Allow => HookAction::Ignore,
			AirblastAction::Refuse => HookAction::Supersede(false),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for AirblastRoute {}

#[derive(Clone, Copy)]
struct RoutedAirblast {
	binding: ServerBinding,
	callback: AirblastFn,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Whether the weapons with airblast are hooked, for this load of the
	/// plugin.
	fn airblasts_hooked(self) -> bool {
		ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| self.has_hook(state.hook))
		})
	}

	/// Checks that `weapons` and `reference`, before any hook, are laid out as
	/// the vtables of the weapons with airblast and of the rocket launchers:
	/// [`DEFLECT_PROJECTILES_SLOT`] holds one function in all three, as no
	/// class overrides `DeflectProjectiles`, and [`DEFLECT_PLAYER_SLOT`] holds
	/// another in both weapons', the flame throwers' override, and a third in
	/// the reference's, `CTFWeaponBase`'s own. Fails with
	/// [`AirblastHookError::UnexpectedLayout`] otherwise.
	///
	/// # Safety
	///
	/// As for [`Self::install_airblasts`].
	unsafe fn check_airblast_layout(
		self,
		[flame_thrower, dragons_fury]: [NonNull<*mut c_void>; 2],
		reference: NonNull<*mut c_void>,
	) -> Result<(), AirblastHookError> {
		// SAFETY: As the caller promises, the vtable is live and has `bool ()` at
		// the slot.
		let projectiles = |vtable| unsafe {
			self.original_function(DEFLECT_PROJECTILES, HookTarget::vtable(vtable))
		};
		// SAFETY: As the caller promises, the vtable is live and has
		// `bool (CTFPlayer *, CTFPlayer *, Vector &)` at the slot.
		let player =
			|vtable| unsafe { self.original_function(DEFLECT_PLAYER, HookTarget::vtable(vtable)) };

		let shared = projectiles(reference)?;
		let own = player(reference)?;
		let pushes = player(flame_thrower)?;

		let laid_out = ptr::fn_addr_eq(shared, projectiles(flame_thrower)?)
			&& ptr::fn_addr_eq(shared, projectiles(dragons_fury)?)
			&& ptr::fn_addr_eq(pushes, player(dragons_fury)?)
			&& !ptr::fn_addr_eq(pushes, own)
			&& !ptr::fn_addr_eq(shared, pushes)
			&& !ptr::fn_addr_eq(shared, own);

		match laid_out {
			true => Ok(()),
			false => Err(AirblastHookError::UnexpectedLayout),
		}
	}

	/// Runs `callback` before the weapons whose airblast pushes players act on
	/// each player it reaches, to let them act or spare the player; see the
	/// [module documentation](crate::airblast_hooks).
	///
	/// `binding` must describe the same running server as `server`. Finds the
	/// weapons' classes in the game server module, which needs no level to be
	/// loaded, and snapshots the module to do so: install once, such as while
	/// loading. Before hooking, checks that their vtables are laid out as the
	/// weapons' (see [`AirblastHookError::UnexpectedLayout`]). Installing again
	/// while the hooks are installed returns [`HookError::AlreadyInstalled`]
	/// without searching the module again; removing them allows replacement.
	///
	/// The callback is not run once an earlier hook skipped the weapon's
	/// function, as far as the hooking library reports it (see
	/// [`crate::hook`]). It runs as the game runs the owner's commands, while
	/// lag compensation has moved the other players back to where the owner
	/// saw them, and must not delete entities immediately, as [`Server::new`]
	/// requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload, and
	/// both weapons' vtables hold the same function. A returned handle means
	/// registration was accepted, not that the next airblast will be
	/// intercepted. KHook polls every 5 ms and can retry while a detour is busy,
	/// so airblasts shortly after installation can be missed.
	///
	/// # Safety
	///
	/// The game module's classes named `CTFFlameThrower`, `CTFWeaponFlameBall`
	/// and `CTFRocketLauncher` must be TF2's weapon classes, which derive from
	/// `CTFWeaponBase` through their primary bases as the game server's
	/// `tf_weaponbase.h` declares it, so that their vtables hold
	/// `DeflectProjectiles` at [`DEFLECT_PROJECTILES_SLOT`] and
	/// `DeflectPlayer` at [`DEFLECT_PLAYER_SLOT`], and their objects are
	/// weapons. The search only checks that each vtable holds code at the
	/// latter, and the layout check that the slots hold functions shared and
	/// overridden as theirs are, which other classes' vtables can too.
	pub unsafe fn hook_airblasts(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: AirblastFn,
	) -> Result<AirblastHooks, AirblastHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(AirblastVtableError::WrongGame.into());
		}

		// The classes' vtables are the same for the whole load.
		if self.airblasts_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		let vtables = airblast_vtables(server)?;

		// SAFETY: The caller promises that the classes found are TF2's weapons,
		// whose vtables hold `DeflectProjectiles` and `DeflectPlayer` at the
		// slots. The game module stays loaded until Metamod unloads this plugin.
		unsafe { self.install_airblasts(vtables.weapons(), vtables.reference(), binding, callback) }
	}

	/// Hooks `DeflectPlayer` through each of `weapons`, in the order of
	/// [`AirblastWeapon::ALL`], once they and `reference` are found laid out as
	/// the vtables of the weapons with airblast and of the rocket launchers.
	///
	/// # Safety
	///
	/// Each vtable must be live, hold functions of the signatures
	/// [`DeflectProjectiles`] at [`DEFLECT_PROJECTILES_SLOT`] and
	/// [`DeflectPlayer`] at [`DEFLECT_PLAYER_SLOT`], called on weapon entities,
	/// and stay loaded until Metamod unloads the plugin.
	unsafe fn install_airblasts(
		self,
		weapons: [NonNull<*mut c_void>; 2],
		reference: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: AirblastFn,
	) -> Result<AirblastHooks, AirblastHookError> {
		if self.airblasts_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		// SAFETY: As the caller promises.
		unsafe { self.check_airblast_layout(weapons, reference) }?;

		let mut installed = AirblastHooks {
			hooks: Vec::with_capacity(weapons.len()),
			_not_thread_safe: PhantomData,
		};

		for (route, vtable) in ROUTES.iter().zip(weapons) {
			let target = HookTarget::vtable(vtable);

			// SAFETY: As the caller promises, the vtable is live, has
			// `DeflectPlayer` at the slot, and stays loaded.
			match unsafe { self.add_hook(DEFLECT_PLAYER, target, HookTiming::Pre, route) } {
				Ok(hook) => {
					route.state.set(Some(RoutedAirblast {
						binding,
						callback,
						hook,
					}));
					installed.hooks.push(hook);
				}

				Err(error) => {
					installed.remove(self);
					return Err(error.into());
				}
			}
		}

		Ok(installed)
	}
}
