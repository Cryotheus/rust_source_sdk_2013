//! TF2 building hooks, which run as buildings die, finish being built and
//! start upgrading, and before an engineer's wrench hits one, which they may
//! refuse.
//!
//! [`MetamodApi::hook_buildings`] hooks the `CBaseObject` methods
//! [`BuildingCallbacks`] names a callback for, in every building class at
//! once, through the vtables [`building_vtables`] finds, without any
//! building: engineers' sentry guns, dispensers and teleporters, spies'
//! sappers, and the dispensers of payload carts, Player Destruction and Robot
//! Destruction. Each callback is told the building and its class.
//!
//! # What runs them
//!
//! - [`BuildingCallbacks::killed`] runs before `CBaseObject::Killed`, which
//!   destroys a building as damage kills it, as it is detonated, and as a
//!   Red-Tape Recorder finishes taking its levels away. The building is
//!   intact, its sapper still on it, and the callback gets a copy of the
//!   damage. A detonated building, such as by its builder's PDA or a
//!   `func_nobuild` brush, is its own inflictor. Buildings removed without
//!   dying, as their builder changes class or leaves, or as the game cleans
//!   up, do not reach it.
//! - [`BuildingCallbacks::finished`] runs after `CBaseObject::FinishedBuilding`,
//!   once a building is built, a sapper placed, or a carried building
//!   redeployed, which [`Building::is_redeploying`] tells apart.
//! - [`BuildingCallbacks::upgrading`] runs after
//!   `CBaseObject::StartUpgrading`, once a building's level has gone up by
//!   one. A redeployed building goes through it for each level it gets back,
//!   and a map's building for each level the map gives it.
//! - [`BuildingCallbacks::wrench_hit`] runs before
//!   `CBaseObject::InputWrenchHit`, as an engineer's wrench hits a building of
//!   their team, and can refuse the hit. A wrench hitting the other team's
//!   building damages it instead.
//!
//! # When to install
//!
//! Install once, such as while loading: the hooks last until removed or the
//! plugin unloads, through level changes, and installing again returns
//! [`HookError::AlreadyInstalled`] without searching the module again.
//!
//! # What gets through
//!
//! As with other Metamod hooks, the callbacks do not run while the plugin is
//! paused or after it unloads. With Metamod 2.0, KHook keeps a vtable entry it
//! detoured pointing to its own code for the rest of the process, and adds a
//! hook to such an entry from its worker thread, so the calls just after a
//! reload can pass unseen. Another plugin's hook can skip a method before the
//! callbacks that run before it, which KHook does not report. The callbacks
//! that run after a method cannot tell, and run even when it was skipped.
//!
//! [`Building::is_redeploying`]: source_sdk_2013::tf2::buildings::Building::is_redeploying

#[cfg(test)]
#[path = "tests/building_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, Signature,
	VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::math::Vector;

use source_sdk_2013::raw::tf2::buildings::{
	FINISHED_BUILDING_SLOT, FinishedBuildingFn as FinishedBuilding, INPUT_WRENCH_HIT_SLOT,
	InputWrenchHitFn as InputWrenchHit, KILLED_SLOT, KilledFn as Killed, START_UPGRADING_SLOT,
	StartUpgradingFn as StartUpgrading,
};

use source_sdk_2013::tf2::buildings::{BuildingClass, BuildingVtableError, building_vtables};
use source_sdk_2013::tf2::damage::DamageInfo;
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// A callback-scoped server, and a building that finished being built or
/// started upgrading. A panic is contained by the hook dispatcher.
pub type BuildingFn = for<'s> fn(Server<'s>, BuildingEvent<'s>);

/// A callback-scoped server, a building about to die, and an owned copy of
/// the damage killing it. A panic is contained by the hook dispatcher.
pub type BuildingKilledFn = for<'s> fn(Server<'s>, BuildingEvent<'s>, &DamageInfo);

/// A callback-scoped server, and a wrench's hit on a building, which decides
/// whether the hit acts. A panic is contained by the hook dispatcher, and
/// lets the hit act.
pub type WrenchHitFn = for<'s> fn(Server<'s>, WrenchHit<'s>) -> WrenchHitAction;

/// `FinishedBuilding` in a TF2 building's primary vtable.
const FINISHED_BUILDING: VirtualFunction<FinishedBuilding> =
	VirtualFunction::new(FINISHED_BUILDING_SLOT);

/// Which classes, in the order of [`BuildingClass::ALL`], share their
/// `FinishedBuilding`: the sapper and the teleporter override it.
const FINISHED_BUILDING_SHARING: [u8; 7] = [0, 0, 0, 0, 1, 0, 2];

/// `InputWrenchHit` in a TF2 building's primary vtable.
const INPUT_WRENCH_HIT: VirtualFunction<InputWrenchHit> =
	VirtualFunction::new(INPUT_WRENCH_HIT_SLOT);

/// Which classes share their `InputWrenchHit`: the teleporter overrides it.
const INPUT_WRENCH_HIT_SHARING: [u8; 7] = [0, 0, 0, 0, 0, 0, 1];

/// `Killed` in a TF2 building's primary vtable.
const KILLED: VirtualFunction<Killed> = VirtualFunction::new(KILLED_SLOT);

/// Which classes share their `Killed`: the sapper and the sentry gun override
/// it.
const KILLED_SHARING: [u8; 7] = [0, 0, 0, 0, 1, 2, 0];

/// `StartUpgrading` in a TF2 building's primary vtable.
const START_UPGRADING: VirtualFunction<StartUpgrading> = VirtualFunction::new(START_UPGRADING_SLOT);

/// Which classes share their `StartUpgrading`: the dispenser overrides it for
/// every dispenser, and the sentry gun and the teleporter override it too,
/// which leaves the sapper with `CBaseObject`'s.
const START_UPGRADING_SHARING: [u8; 7] = [0, 0, 0, 0, 1, 2, 3];

/// The routes of each hook, in the order of [`BuildingClass::ALL`].
static FINISHED_ROUTES: [BuildingRoute<BuildingFn>; 7] = [const { BuildingRoute::new() }; 7];

static KILLED_ROUTES: [BuildingRoute<BuildingKilledFn>; 7] = [const { BuildingRoute::new() }; 7];
static UPGRADING_ROUTES: [BuildingRoute<BuildingFn>; 7] = [const { BuildingRoute::new() }; 7];
static WRENCH_HIT_ROUTES: [BuildingRoute<WrenchHitFn>; 7] = [const { BuildingRoute::new() }; 7];

/// The callbacks of the building hooks, each of which hooks its method in
/// every building class; see the [module documentation](crate::building_hooks).
#[derive(Debug, Clone, Copy, Default)]
pub struct BuildingCallbacks {
	/// Runs after a building finishes being built, a sapper is placed, or a
	/// carried building is redeployed.
	pub finished: Option<BuildingFn>,

	/// Runs before a building dies.
	pub killed: Option<BuildingKilledFn>,

	/// Runs after a building's level went up by one.
	pub upgrading: Option<BuildingFn>,

	/// Runs before an engineer's wrench hits a building of their team, and
	/// decides whether the hit acts.
	pub wrench_hit: Option<WrenchHitFn>,
}

/// A building a hook's method runs on.
#[derive(Debug, Clone, Copy)]
pub struct BuildingEvent<'s> {
	/// The building.
	pub building: Entity<'s>,

	/// The building's class, whose kind is [`BuildingClass::kind`].
	pub class: BuildingClass,
}

/// Why the building hooks could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum BuildingHookError {
	/// Metamod refused a hook. [`HookError::AlreadyInstalled`] means the
	/// buildings are already hooked.
	#[error(transparent)]
	Hook(#[from] HookError),

	/// The building classes' vtables could not be found.
	#[error(transparent)]
	Target(#[from] BuildingVtableError),

	/// The vtables found are not laid out as the building classes' are: before
	/// any hook, the slots of the hooked methods do not hold functions shared
	/// and overridden by the classes as theirs are, or a class holds one
	/// function at two of them.
	#[error("the building classes' vtables do not have the expected layout")]
	UnexpectedLayout,
}

/// Handles for the building hooks installed by one call.
///
/// Metamod disables them while paused and removes them before unloading the
/// plugin. [`Self::remove`] disables them earlier. No building is retained,
/// so the hooks last through level changes.
#[must_use = "retain building hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct BuildingHooks {
	hooks: Vec<HookId>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl BuildingHooks {
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);
			clear_route(&FINISHED_ROUTES, hook);
			clear_route(&KILLED_ROUTES, hook);
			clear_route(&UPGRADING_ROUTES, hook);
			clear_route(&WRENCH_HIT_ROUTES, hook);
		}
	}
}

struct BuildingRoute<C: 'static> {
	state: Cell<Option<RoutedBuilding<C>>>,
}

impl<C: Copy> BuildingRoute<C> {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}

	/// The route's state and the server and building of `call`, unless the
	/// route is not installed or the building is null.
	fn enter<'s, S: Signature<This = source_sdk_2013::sys::CBaseEntity>>(
		&self,
		call: &HookCall<'_, S>,
		scope: &'s (),
	) -> Option<(RoutedBuilding<C>, Server<'s>, BuildingEvent<'s>)> {
		let route = self.state.get()?;
		let building = NonNull::new(call.this())?;
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(scope) };
		// SAFETY: The class hook supplies the live building whose method runs,
		// which stays in the entity list through the call: the game only marks
		// entities it removes then for deletion.
		let building = unsafe { Entity::from_live(server, building) };

		Some((
			route,
			server,
			BuildingEvent {
				building,
				class: route.class,
			},
		))
	}
}

impl Handler<FinishedBuilding> for BuildingRoute<BuildingFn> {
	fn call(&self, call: &HookCall<'_, FinishedBuilding>) -> HookAction<()> {
		let scope = ();

		if let Some((route, server, event)) = self.enter(call, &scope) {
			(route.callback)(server, event);
		}

		HookAction::Ignore
	}
}

impl Handler<InputWrenchHit> for BuildingRoute<WrenchHitFn> {
	fn call(&self, call: &HookCall<'_, InputWrenchHit>) -> HookAction<bool> {
		// An earlier hook already skipped the building's function.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let scope = ();
		let Some((route, server, event)) = self.enter(call, &scope) else {
			return HookAction::Ignore;
		};
		let (player, wrench, position) = call.args();
		let (Some(player), Some(wrench)) = (NonNull::new(player), NonNull::new(wrench)) else {
			return HookAction::Ignore;
		};
		// SAFETY: The game passes the live engineer and their wrench, which stay
		// in the entity list through the call, as the building does.
		let hit = unsafe {
			WrenchHit {
				building: event.building,
				class: event.class,
				player: Entity::from_live(server, player),
				position: position.into(),
				wrench: Entity::from_live(server, wrench),
			}
		};

		match (route.callback)(server, hit) {
			WrenchHitAction::Continue => HookAction::Ignore,
			WrenchHitAction::Refuse => HookAction::Supersede(false),
		}
	}
}

impl Handler<Killed> for BuildingRoute<BuildingKilledFn> {
	fn call(&self, call: &HookCall<'_, Killed>) -> HookAction<()> {
		// An earlier hook already skipped the building's death.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let scope = ();
		let Some((route, server, event)) = self.enter(call, &scope) else {
			return HookAction::Ignore;
		};
		let (info,) = call.args();
		let Some(info) = NonNull::new(info.cast_mut()) else {
			return HookAction::Ignore;
		};
		// SAFETY: The game passes the live damage killing the building, which
		// is only copied.
		let info = unsafe { DamageInfo::copy_from_raw(info) };

		(route.callback)(server, event, &info);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl<C> Sync for BuildingRoute<C> {}

#[derive(Clone, Copy)]
struct RoutedBuilding<C> {
	binding: ServerBinding,
	callback: C,
	class: BuildingClass,
	hook: HookId,
}

/// An engineer's wrench hitting a building of their team, which is about to
/// act on it.
#[derive(Debug, Clone, Copy)]
pub struct WrenchHit<'s> {
	/// The building hit.
	pub building: Entity<'s>,

	/// The building's class, whose kind is [`BuildingClass::kind`].
	pub class: BuildingClass,

	/// The engineer whose wrench it is.
	pub player: Entity<'s>,

	/// Where the hit landed.
	pub position: Vector,

	/// The wrench.
	pub wrench: Entity<'s>,
}

/// What a wrench hit hook does with a hit.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WrenchHitAction {
	/// Lets the hit act, through the other plugins' hooks.
	#[default]
	Continue,

	/// Refuses the hit: `InputWrenchHit` is skipped, so it neither removes a
	/// sapper nor speeds up construction, nor repairs, refills or upgrades the
	/// building, and the wrench counts it as doing nothing.
	Refuse,
}

impl MetamodApi<'_> {
	/// Whether any building hook is installed, for this load of the plugin.
	fn buildings_hooked(self) -> bool {
		routed(self, &FINISHED_ROUTES)
			|| routed(self, &KILLED_ROUTES)
			|| routed(self, &UPGRADING_ROUTES)
			|| routed(self, &WRENCH_HIT_ROUTES)
	}

	/// Checks that `vtables`, in the order of [`BuildingClass::ALL`], are laid
	/// out as the building classes' before any hook: each hooked method's slot
	/// holds one function in the classes that share it, and others in those
	/// that override it, as its `*_SHARING` constant says, and no class holds
	/// one function at two of the slots. Fails with
	/// [`BuildingHookError::UnexpectedLayout`] otherwise.
	///
	/// # Safety
	///
	/// As for [`Self::install_buildings`].
	unsafe fn check_building_layout(
		self,
		vtables: [NonNull<*mut c_void>; 7],
	) -> Result<(), BuildingHookError> {
		// SAFETY: As the caller promises, each vtable is live and has the
		// methods' signatures at their slots.
		let slots = unsafe {
			[
				self.building_functions(FINISHED_BUILDING, vtables)?,
				self.building_functions(INPUT_WRENCH_HIT, vtables)?,
				self.building_functions(KILLED, vtables)?,
				self.building_functions(START_UPGRADING, vtables)?,
			]
		};
		let sharing = [
			FINISHED_BUILDING_SHARING,
			INPUT_WRENCH_HIT_SHARING,
			KILLED_SHARING,
			START_UPGRADING_SHARING,
		];

		let shared = slots.iter().zip(sharing).all(|(functions, groups)| {
			(0..functions.len()).all(|first| {
				(0..functions.len()).all(|second| {
					(functions[first] == functions[second]) == (groups[first] == groups[second])
				})
			})
		});

		let distinct = (0..vtables.len()).all(|class| {
			(0..slots.len()).all(|first| {
				(first + 1..slots.len()).all(|second| slots[first][class] != slots[second][class])
			})
		});

		match shared && distinct {
			true => Ok(()),
			false => Err(BuildingHookError::UnexpectedLayout),
		}
	}

	/// The addresses of the functions at `function`'s slot of each of
	/// `vtables`, before any hook.
	///
	/// # Safety
	///
	/// Each vtable must be live, and hold a function of the signature `S` at
	/// the slot.
	unsafe fn building_functions<S: Signature>(
		self,
		function: VirtualFunction<S>,
		vtables: [NonNull<*mut c_void>; 7],
	) -> Result<[NonNull<c_void>; 7], HookError> {
		let mut functions = [NonNull::dangling(); 7];

		for (address, vtable) in functions.iter_mut().zip(vtables) {
			// SAFETY: As the caller promises.
			*address =
				unsafe { self.original_function(function, HookTarget::vtable(vtable)) }?.address();
		}

		Ok(functions)
	}

	/// Runs `callbacks` as TF2's buildings die, finish being built and start
	/// upgrading, and before engineers' wrenches hit them, hooking only the
	/// methods `callbacks` names a callback for; see the [module
	/// documentation](crate::building_hooks).
	///
	/// `binding` must describe the same running server as `server`. Finds every
	/// building class in the game server module, which needs no level to be
	/// loaded, and snapshots the module to do so: install once, such as while
	/// loading. Before hooking, checks that their vtables are laid out as the
	/// building classes' (see [`BuildingHookError::UnexpectedLayout`]), and a
	/// refused hook rolls back the others. Installing again while any of the
	/// hooks is installed returns [`HookError::AlreadyInstalled`] without
	/// searching the module again; removing them allows replacement. Without
	/// any callback, nothing is searched for or hooked.
	///
	/// The callbacks run as the game runs: [`BuildingCallbacks::wrench_hit`] as
	/// it runs the engineer's commands, while lag compensation has moved the
	/// other players back to where the engineer saw them. They must not delete
	/// entities immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload, and
	/// building classes share functions. A returned handle means registration
	/// was accepted, not that the next call will be intercepted.
	///
	/// # Safety
	///
	/// The game module's classes [`BuildingClass::name`] names must be TF2's
	/// building classes, which derive from `CBaseObject` through their primary
	/// bases as the game server's `tf_obj.h` declares it, so that their vtables
	/// hold `FinishedBuilding`, `InputWrenchHit`, `Killed` and `StartUpgrading`
	/// at the slots `source_sdk_2013::raw::tf2::buildings` gives, and their
	/// objects are buildings. The search only checks that each vtable holds
	/// code at a slot every building class has, and the layout check that the
	/// slots hold functions shared and overridden as theirs are, which other
	/// classes' vtables can too.
	pub unsafe fn hook_buildings(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callbacks: BuildingCallbacks,
	) -> Result<BuildingHooks, BuildingHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(BuildingVtableError::WrongGame.into());
		}

		// The classes' vtables are the same for the whole load.
		if self.buildings_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		let none = callbacks.finished.is_none()
			&& callbacks.killed.is_none()
			&& callbacks.upgrading.is_none()
			&& callbacks.wrench_hit.is_none();

		if none {
			return Ok(BuildingHooks {
				hooks: Vec::new(),
				_not_thread_safe: PhantomData,
			});
		}

		let vtables = building_vtables(server)?;

		// SAFETY: The caller promises that the classes found are TF2's
		// buildings, whose vtables hold the methods at their slots. The game
		// module stays loaded until Metamod unloads this plugin.
		unsafe { self.install_buildings(vtables.all(), binding, callbacks) }
	}

	/// Hooks the methods `callbacks` names a callback for through each of
	/// `vtables`, in the order of [`BuildingClass::ALL`], once they are found
	/// laid out as the building classes' vtables.
	///
	/// # Safety
	///
	/// Each vtable must be live, hold functions of the signatures
	/// [`FinishedBuilding`], [`InputWrenchHit`], [`Killed`] and
	/// [`StartUpgrading`] at [`FINISHED_BUILDING_SLOT`],
	/// [`INPUT_WRENCH_HIT_SLOT`], [`KILLED_SLOT`] and [`START_UPGRADING_SLOT`],
	/// called on buildings of its class, and stay loaded until Metamod unloads
	/// the plugin.
	unsafe fn install_buildings(
		self,
		vtables: [NonNull<*mut c_void>; 7],
		binding: ServerBinding,
		callbacks: BuildingCallbacks,
	) -> Result<BuildingHooks, BuildingHookError> {
		if self.buildings_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		// SAFETY: As the caller promises.
		unsafe { self.check_building_layout(vtables) }?;

		let mut installed = BuildingHooks {
			hooks: Vec::new(),
			_not_thread_safe: PhantomData,
		};

		// SAFETY: As the caller promises.
		match unsafe { self.route_callbacks(vtables, binding, callbacks, &mut installed.hooks) } {
			Ok(()) => Ok(installed),

			Err(error) => {
				installed.remove(self);
				Err(error.into())
			}
		}
	}

	/// Hooks `function` through each of `vtables`, in the order of
	/// [`BuildingClass::ALL`], with the route of the same index of `routes`,
	/// and adds the hooks to `hooks`.
	///
	/// # Safety
	///
	/// Each vtable must be live, hold a function of the signature `S` at the
	/// function's slot, called on buildings of its class, and stay loaded until
	/// Metamod unloads the plugin.
	#[allow(clippy::too_many_arguments)]
	unsafe fn route_buildings<S: Signature, C: Copy>(
		self,
		function: VirtualFunction<S>,
		timing: HookTiming,
		routes: &'static [BuildingRoute<C>; 7],
		vtables: [NonNull<*mut c_void>; 7],
		binding: ServerBinding,
		callback: C,
		hooks: &mut Vec<HookId>,
	) -> Result<(), HookError>
	where
		BuildingRoute<C>: Handler<S>,
	{
		for ((route, vtable), class) in routes.iter().zip(vtables).zip(BuildingClass::ALL) {
			// SAFETY: As the caller promises.
			let hook =
				unsafe { self.add_hook(function, HookTarget::vtable(vtable), timing, route) }?;

			route.state.set(Some(RoutedBuilding {
				binding,
				callback,
				class,
				hook,
			}));
			hooks.push(hook);
		}

		Ok(())
	}

	/// Hooks the methods `callbacks` names a callback for through each of
	/// `vtables`, in the order of [`BuildingClass::ALL`], and adds the hooks to
	/// `hooks`, as far as Metamod accepts them.
	///
	/// # Safety
	///
	/// As for [`Self::install_buildings`].
	unsafe fn route_callbacks(
		self,
		vtables: [NonNull<*mut c_void>; 7],
		binding: ServerBinding,
		callbacks: BuildingCallbacks,
		hooks: &mut Vec<HookId>,
	) -> Result<(), HookError> {
		let BuildingCallbacks {
			finished,
			killed,
			upgrading,
			wrench_hit,
		} = callbacks;

		// SAFETY: As the caller promises, each vtable has each method at its
		// slot, called on buildings of its class, and stays loaded.
		unsafe {
			if let Some(callback) = finished {
				let routes = &FINISHED_ROUTES;
				self.route_buildings(
					FINISHED_BUILDING,
					HookTiming::Post,
					routes,
					vtables,
					binding,
					callback,
					hooks,
				)?;
			}

			if let Some(callback) = killed {
				let routes = &KILLED_ROUTES;
				self.route_buildings(
					KILLED,
					HookTiming::Pre,
					routes,
					vtables,
					binding,
					callback,
					hooks,
				)?;
			}

			if let Some(callback) = upgrading {
				let routes = &UPGRADING_ROUTES;
				self.route_buildings(
					START_UPGRADING,
					HookTiming::Post,
					routes,
					vtables,
					binding,
					callback,
					hooks,
				)?;
			}

			if let Some(callback) = wrench_hit {
				let routes = &WRENCH_HIT_ROUTES;
				self.route_buildings(
					INPUT_WRENCH_HIT,
					HookTiming::Pre,
					routes,
					vtables,
					binding,
					callback,
					hooks,
				)?;
			}
		}

		Ok(())
	}
}

/// Forgets `hook` in the route of `routes` it was installed through, if any.
fn clear_route<C: Copy>(routes: &[BuildingRoute<C>], hook: HookId) {
	for route in routes {
		if route.state.get().is_some_and(|state| state.hook == hook) {
			route.state.set(None);
		}
	}
}

/// Whether any of `routes` has a hook installed, for this load of the plugin.
fn routed<C: Copy>(api: MetamodApi<'_>, routes: &[BuildingRoute<C>]) -> bool {
	routes.iter().any(|route| {
		route
			.state
			.get()
			.is_some_and(|state| api.has_hook(state.hook))
	})
}
