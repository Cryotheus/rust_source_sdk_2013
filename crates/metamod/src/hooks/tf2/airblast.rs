//! TF2 airblast hooks, which run before a weapon's airblast acts on a player
//! or deflects an entity it reached, and may spare it.
//!
//! The airblast of the Pyro's flame throwers and of the Dragon's Fury goes
//! through the players and entities in front of the weapon's owner (see
//! [`source_sdk_2013::tf2::airblast`]). It has the weapon act on each player
//! through `CTFWeaponBase::DeflectPlayer`, which extinguishes a burning
//! teammate and pushes a player of the other team, and deflect each other
//! entity through `CTFWeaponBase::DeflectEntity`, which reflects a projectile
//! of the other team back where the owner aims and pushes a physics prop.
//!
//! [`MetamodApi::hook_airblasts`] runs a callback before `DeflectPlayer`, and
//! [`MetamodApi::hook_deflections`] one before `DeflectEntity`, which decide
//! whether the function runs. A spared player is neither pushed nor slowed,
//! nor extinguished, and a spared projectile keeps its course, owner and team.
//! The airblast counts what it spared as not deflected, which only changes its
//! sound and whether its owner speaks.
//!
//! # When to install
//!
//! Both find the weapons' classes in the game server module through
//! [`airblast_vtables`], without any weapon, and hook their function in the
//! vtable of each. Install once, such as while loading: the hooks last until
//! removed or the plugin unloads, and installing either again returns
//! [`HookError::AlreadyInstalled`] without searching the module again.
//!
//! # What gets through
//!
//! As with other Metamod hooks, the callbacks do not run while the plugin is
//! paused or after it unloads, so airblasts act on everything as usual then.
//! With Metamod 2.0, KHook keeps a vtable entry it detoured pointing to its
//! own code for the rest of the process, and adds a hook to such an entry from
//! its worker thread, so the airblasts just after a reload can pass unseen.
//! Another plugin's hook on the same function can skip it before the callback
//! runs, which KHook does not report.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/airblast.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, Signature,
	VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::airblast::{
	DEFLECT_ENTITY_SLOT, DEFLECT_PLAYER_SLOT, DEFLECT_PROJECTILES_SLOT,
	DeflectEntityFn as DeflectEntity, DeflectPlayerFn as DeflectPlayer,
	DeflectProjectilesFn as DeflectProjectiles,
};

use source_sdk_2013::sys::CBaseEntity;
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

/// A callback-scoped server, and an entity other than a player a weapon's
/// airblast reached, such as a projectile, which decides whether the weapon
/// deflects it. A panic is contained by the hook dispatcher, and lets the
/// weapon deflect it.
pub type DeflectionFn = for<'s> fn(Server<'s>, AirblastDeflection<'s>) -> AirblastAction;

/// `DeflectEntity` in a TF2 weapon's primary vtable.
const DEFLECT_ENTITY: VirtualFunction<DeflectEntity> = VirtualFunction::new(DEFLECT_ENTITY_SLOT);

/// `DeflectPlayer` in a TF2 weapon's primary vtable.
const DEFLECT_PLAYER: VirtualFunction<DeflectPlayer> = VirtualFunction::new(DEFLECT_PLAYER_SLOT);

/// `DeflectProjectiles` in a TF2 weapon's primary vtable.
const DEFLECT_PROJECTILES: VirtualFunction<DeflectProjectiles> =
	VirtualFunction::new(DEFLECT_PROJECTILES_SLOT);

/// The routes of the weapons' deflections of entities, in the order of
/// [`AirblastWeapon::ALL`].
static DEFLECTION_ROUTES: [AirblastRoute<DeflectionFn>; 2] = [
	AirblastRoute::new(AirblastWeapon::FlameThrower),
	AirblastRoute::new(AirblastWeapon::DragonsFury),
];

/// The routes of the weapons' pushes of players, in the order of
/// [`AirblastWeapon::ALL`].
static ROUTES: [AirblastRoute<AirblastFn>; 2] = [
	AirblastRoute::new(AirblastWeapon::FlameThrower),
	AirblastRoute::new(AirblastWeapon::DragonsFury),
];

/// What an airblast hook does with a player or entity the airblast reached.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AirblastAction {
	/// Lets the weapon act on the player, or deflect the entity.
	#[default]
	Allow,

	/// Spares the player or entity: the weapon's `DeflectPlayer` or
	/// `DeflectEntity` is skipped, so the airblast neither pushes nor
	/// extinguishes the player, nor deflects the entity, and counts it as not
	/// deflected.
	Refuse,
}

/// An entity other than a player a weapon's airblast reached, which the weapon
/// is about to deflect.
#[derive(Debug, Clone, Copy)]
pub struct AirblastDeflection<'s> {
	/// The weapon whose airblast it is.
	pub weapon: Entity<'s>,

	/// Which of the weapons with airblast [`Self::weapon`] is.
	pub kind: AirblastWeapon,

	/// What the airblast reached: a projectile, which
	/// [`Projectile::new`](source_sdk_2013::tf2::projectiles::Projectile::new)
	/// wraps, a physics prop, or another entity the game lets airblasts
	/// deflect. The weapon deflects neither its owner's team's projectiles nor,
	/// for a weapon whose attributes stop it, any projectile.
	pub target: Entity<'s>,

	/// The weapon's owner, whose airblast it is.
	pub owner: Entity<'s>,
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
	/// another in the reference's. For deflections, the same goes for
	/// [`DEFLECT_ENTITY_SLOT`], whose functions must differ from those of the
	/// other slots.
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
			clear_route(&ROUTES, hook);
			clear_route(&DEFLECTION_ROUTES, hook);
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

/// Where one weapon's hook of `DeflectPlayer` or `DeflectEntity` sends its
/// calls, of the callback type `C`.
struct AirblastRoute<C> {
	weapon: AirblastWeapon,
	state: Cell<Option<RoutedAirblast<C>>>,
}

impl<C: Copy> AirblastRoute<C> {
	const fn new(weapon: AirblastWeapon) -> Self {
		Self {
			weapon,
			state: Cell::new(None),
		}
	}

	/// The server, weapon, target and owner of a call of the weapon's hooked
	/// function, with the callback, or `None` for a call to leave to the game:
	/// one an earlier hook skipped, while the route is unset, or with a null
	/// entity.
	fn enter<'s, S: Signature<This = CBaseEntity>>(
		&self,
		call: &HookCall<'_, S>,
		target: *mut CBaseEntity,
		owner: *mut CBaseEntity,
		scope: &'s (),
	) -> Option<(Server<'s>, [Entity<'s>; 3], C)> {
		// An earlier hook already skipped the weapon's function.
		if call.superseded() == Some(true) {
			return None;
		}

		let route = self.state.get()?;
		let (Some(weapon), Some(target), Some(owner)) = (
			NonNull::new(call.this()),
			NonNull::new(target),
			NonNull::new(owner),
		) else {
			return None;
		};
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(scope) };
		// SAFETY: The class hook supplies the live weapon whose method runs, and
		// the live entities the game passes it, which stay in the entity list
		// through the call: the game only marks entities it removes then for
		// deletion.
		let entities =
			unsafe { [weapon, target, owner].map(|entity| Entity::from_live(server, entity)) };

		Some((server, entities, route.callback))
	}
}

impl Handler<DeflectEntity> for AirblastRoute<DeflectionFn> {
	fn call(&self, call: &HookCall<'_, DeflectEntity>) -> HookAction<bool> {
		let (target, owner, _forward) = call.args();
		let scope = ();
		let Some((server, [weapon, target, owner], callback)) =
			self.enter(call, target, owner, &scope)
		else {
			return HookAction::Ignore;
		};
		let deflection = AirblastDeflection {
			weapon,
			kind: self.weapon,
			target,
			owner,
		};

		match callback(server, deflection) {
			AirblastAction::Allow => HookAction::Ignore,
			AirblastAction::Refuse => HookAction::Supersede(false),
		}
	}
}

impl Handler<DeflectPlayer> for AirblastRoute<AirblastFn> {
	fn call(&self, call: &HookCall<'_, DeflectPlayer>) -> HookAction<bool> {
		let (target, owner, _forward) = call.args();
		let scope = ();
		let Some((server, [weapon, target, owner], callback)) =
			self.enter(call, target, owner, &scope)
		else {
			return HookAction::Ignore;
		};
		let push = AirblastPush {
			weapon,
			kind: self.weapon,
			target,
			owner,
		};

		match callback(server, push) {
			AirblastAction::Allow => HookAction::Ignore,
			AirblastAction::Refuse => HookAction::Supersede(false),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl<C> Sync for AirblastRoute<C> {}

#[derive(Clone, Copy)]
struct RoutedAirblast<C> {
	binding: ServerBinding,
	callback: C,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Whether `routes` are hooked, for this load of the plugin.
	fn airblast_routes_hooked<C: Copy>(self, routes: &[AirblastRoute<C>; 2]) -> bool {
		routes.iter().any(|route| {
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
	/// the reference's, `CTFWeaponBase`'s own. With `entities`, also checks
	/// that [`DEFLECT_ENTITY_SLOT`] holds functions shared and overridden as
	/// `DeflectPlayer`'s are, but others. Fails with
	/// [`AirblastHookError::UnexpectedLayout`] otherwise.
	///
	/// # Safety
	///
	/// As for [`Self::install_airblast_routes`].
	unsafe fn check_airblast_layout(
		self,
		[flame_thrower, dragons_fury]: [NonNull<*mut c_void>; 2],
		reference: NonNull<*mut c_void>,
		entities: bool,
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

		let mut laid_out = ptr::fn_addr_eq(shared, projectiles(flame_thrower)?)
			&& ptr::fn_addr_eq(shared, projectiles(dragons_fury)?)
			&& ptr::fn_addr_eq(pushes, player(dragons_fury)?)
			&& !ptr::fn_addr_eq(pushes, own)
			&& !ptr::fn_addr_eq(shared, pushes)
			&& !ptr::fn_addr_eq(shared, own);

		if laid_out && entities {
			// SAFETY: As the caller promises with `entities`, the vtable is live
			// and has `bool (CBaseEntity *, CTFPlayer *, Vector &)` at the slot.
			let entity = |vtable| unsafe {
				self.original_function(DEFLECT_ENTITY, HookTarget::vtable(vtable))
			};

			let own_deflection = entity(reference)?;
			let deflects = entity(flame_thrower)?;
			let addresses = [shared as usize, own as usize, pushes as usize];

			laid_out = ptr::fn_addr_eq(deflects, entity(dragons_fury)?)
				&& !ptr::fn_addr_eq(deflects, own_deflection)
				&& !addresses.contains(&(deflects as usize))
				&& !addresses.contains(&(own_deflection as usize));
		}

		match laid_out {
			true => Ok(()),
			false => Err(AirblastHookError::UnexpectedLayout),
		}
	}

	/// Hooks `function` through the vtables of the weapons with airblast, with
	/// `callback`, unless `routes` are hooked; see [`Self::hook_airblasts`].
	///
	/// # Safety
	///
	/// As for [`Self::hook_airblasts`], and for `DeflectEntity` at
	/// [`DEFLECT_ENTITY_SLOT`] if `function` is it.
	unsafe fn hook_airblast_routes<S, C>(
		self,
		function: VirtualFunction<S>,
		routes: &'static [AirblastRoute<C>; 2],
		server: Server<'_>,
		binding: ServerBinding,
		callback: C,
	) -> Result<AirblastHooks, AirblastHookError>
	where
		S: Signature,
		C: Copy,
		AirblastRoute<C>: Handler<S>,
	{
		if binding.game() != Game::TeamFortress2 {
			return Err(AirblastVtableError::WrongGame.into());
		}

		// The classes' vtables are the same for the whole load.
		if self.airblast_routes_hooked(routes) {
			return Err(HookError::AlreadyInstalled.into());
		}

		let vtables = airblast_vtables(server)?;

		// SAFETY: The caller promises that the classes found are TF2's weapons,
		// whose vtables hold the airblast's functions at their slots. The game
		// module stays loaded until Metamod unloads this plugin.
		unsafe {
			self.install_airblast_routes(
				function,
				routes,
				vtables.weapons(),
				vtables.reference(),
				binding,
				callback,
			)
		}
	}

	/// Runs `callback` before the weapons whose airblast pushes players act on
	/// each player it reaches, to let them act or spare the player; see the
	/// [module documentation](crate::hooks::tf2::airblast).
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
	/// weapons. The search only checks that each vtable holds code at
	/// [`DEFLECT_ENTITY_SLOT`], and the layout check that the slots hold
	/// functions shared and overridden as theirs are, which other classes'
	/// vtables can too.
	pub unsafe fn hook_airblasts(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: AirblastFn,
	) -> Result<AirblastHooks, AirblastHookError> {
		// SAFETY: As the caller promises.
		unsafe { self.hook_airblast_routes(DEFLECT_PLAYER, &ROUTES, server, binding, callback) }
	}

	/// Runs `callback` before the weapons with airblast deflect each entity
	/// other than a player it reaches, such as a projectile, to let them
	/// deflect it or spare it; see the [module
	/// documentation](crate::hooks::tf2::airblast).
	///
	/// Installs as [`Self::hook_airblasts`] does, independently of it, and its
	/// callback runs under the same conditions. Before hooking, also checks
	/// that `DeflectEntity` is laid out as the weapons' (see
	/// [`AirblastHookError::UnexpectedLayout`]).
	///
	/// # Safety
	///
	/// As for [`Self::hook_airblasts`], and the classes' vtables must hold
	/// `DeflectEntity` at [`DEFLECT_ENTITY_SLOT`].
	pub unsafe fn hook_deflections(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: DeflectionFn,
	) -> Result<AirblastHooks, AirblastHookError> {
		// SAFETY: As the caller promises.
		unsafe {
			self.hook_airblast_routes(
				DEFLECT_ENTITY,
				&DEFLECTION_ROUTES,
				server,
				binding,
				callback,
			)
		}
	}

	/// Hooks `function`, `DeflectPlayer` or `DeflectEntity`, through each of
	/// `weapons` with `routes`, in the order of [`AirblastWeapon::ALL`], once
	/// they and `reference` are found laid out as the vtables of the weapons
	/// with airblast and of the rocket launchers.
	///
	/// # Safety
	///
	/// Each vtable must be live, hold functions of the signatures
	/// [`DeflectProjectiles`] at [`DEFLECT_PROJECTILES_SLOT`] and
	/// [`DeflectPlayer`] at [`DEFLECT_PLAYER_SLOT`], and for `DeflectEntity`
	/// [`DeflectEntity`] at [`DEFLECT_ENTITY_SLOT`], called on weapon
	/// entities, and stay loaded until Metamod unloads the plugin.
	unsafe fn install_airblast_routes<S, C>(
		self,
		function: VirtualFunction<S>,
		routes: &'static [AirblastRoute<C>; 2],
		weapons: [NonNull<*mut c_void>; 2],
		reference: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: C,
	) -> Result<AirblastHooks, AirblastHookError>
	where
		S: Signature,
		C: Copy,
		AirblastRoute<C>: Handler<S>,
	{
		if self.airblast_routes_hooked(routes) {
			return Err(HookError::AlreadyInstalled.into());
		}

		let entities = function.index() == DEFLECT_ENTITY_SLOT;

		// SAFETY: As the caller promises.
		unsafe { self.check_airblast_layout(weapons, reference, entities) }?;

		let mut installed = AirblastHooks {
			hooks: Vec::with_capacity(weapons.len()),
			_not_thread_safe: PhantomData,
		};

		for (route, vtable) in routes.iter().zip(weapons) {
			let target = HookTarget::vtable(vtable);

			// SAFETY: As the caller promises, the vtable is live, has the function
			// at the slot, and stays loaded.
			match unsafe { self.add_hook(function, target, HookTiming::Pre, route) } {
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

/// Unsets the route of `routes` that `hook` installed, if any.
fn clear_route<C: Copy>(routes: &[AirblastRoute<C>; 2], hook: HookId) {
	for route in routes {
		if route.state.get().is_some_and(|state| state.hook == hook) {
			route.state.set(None);
		}
	}
}
