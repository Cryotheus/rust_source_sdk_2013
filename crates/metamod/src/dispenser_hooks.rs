//! Hooks before TF2 dispensers supply a player with ammunition and metal.
//!
//! [`MetamodApi::hook_dispenser_ammo`] targets only the four classes derived
//! from `CObjectDispenser`, including map dispensers. A callback can keep
//! stock supply or replace the complete `DispenseAmmo` call, reporting its
//! own result to the game's existing resupply timing. Healing is separate.
//!
//! Hooks last through level changes until removed or the plugin unloads;
//! they retain no entities. They are inactive while the plugin is paused.
//! KHook may queue activation on its worker for an already detoured slot,
//! so accepted registration does not guarantee interception of the next
//! call. Another plugin can skip supply first; KHook cannot report all such
//! skips. A callback panic is contained by the dispatcher and lets stock
//! supply run.

#[cfg(test)]
#[path = "tests/dispenser_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, Signature,
	VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::dispenser_hooks::{
	DISPENSE_AMMO_SLOT, DispenseAmmoFn as DispenseAmmo,
};

use source_sdk_2013::tf2::buildings::{BuildingClass, BuildingVtableError};
use source_sdk_2013::tf2::class_targets::ClassTargets;
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// A callback-scoped server, dispenser and recipient, with a decision about
/// the complete ammunition and metal supply call.
pub type DispenserAmmoFn = for<'s> fn(Server<'s>, DispenserAmmoEvent<'s>) -> DispenserAmmoAction;

const DISPENSE_AMMO: VirtualFunction<DispenseAmmo> = VirtualFunction::new(DISPENSE_AMMO_SLOT);

/// Only these building classes derive from `CObjectDispenser`. They keep
/// its `DispenseAmmo`; the cart overrides `DispenseMetal` instead.
const DISPENSER_CLASSES: [BuildingClass; 4] = [
	BuildingClass::CartDispenser,
	BuildingClass::Dispenser,
	BuildingClass::PlayerDestructionDispenser,
	BuildingClass::RobotDispenser,
];

static AMMO_ROUTES: [DispenserRoute; 4] = [const { DispenserRoute::new() }; 4];

/// What happens to one dispenser resupply attempt.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DispenserAmmoAction {
	/// Continue through other hooks to TF2's normal ammunition and metal supply.
	#[default]
	Continue,
	/// Skip the complete native supply call and report whether the replacement
	/// supplied ammunition. `true` selects the normal one-second retry; `false`
	/// selects the shorter retry used when stock supply gives nothing.
	Supply(bool),
}

/// The dispenser and recipient of a live resupply attempt.
#[derive(Debug, Clone, Copy)]
pub struct DispenserAmmoEvent<'s> {
	/// The dispenser whose method is about to run.
	pub building: Entity<'s>,
	/// Its dispenser-derived C++ class.
	pub class: BuildingClass,
	/// The player the dispenser is trying to supply.
	pub player: Entity<'s>,
}

/// Why the dispenser ammunition hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum DispenserHookError {
	/// Metamod refused registration, or this plugin already installed the hook.
	#[error(transparent)]
	Hook(#[from] HookError),
	/// A dispenser class could not be found, or the binding is for another game.
	#[error(transparent)]
	Target(#[from] BuildingVtableError),
	/// The four classes do not have distinct vtables sharing the native
	/// `CObjectDispenser::DispenseAmmo` implementation.
	#[error("the dispenser classes' vtables do not have the expected layout")]
	UnexpectedLayout,
}

/// The registered dispenser supply hooks. Explicit removal supports later
/// replacement. Metamod disables them while paused and removes them before
/// unloading the plugin; dropping this handle alone does not remove them.
#[must_use = "retain dispenser hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct DispenserHooks {
	hooks: Vec<HookId>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl DispenserHooks {
	/// Disable these hooks and forget their callback routes.
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);
			clear_route(hook);
		}
	}
}

struct DispenserRoute {
	state: Cell<Option<RoutedDispenser>>,
}

impl DispenserRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<DispenseAmmo> for DispenserRoute {
	fn call(&self, call: &HookCall<'_, DispenseAmmo>) -> HookAction<bool> {
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (player,) = call.args();
		let (Some(building), Some(player)) = (NonNull::new(call.this()), NonNull::new(player))
		else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The dispatcher invokes this on the main thread during one
		// live engine invocation; installation supplied this server's binding.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class method receives a live dispenser and player, which
		// remain in the entity list during the call. Removal must be deferred.
		let event = unsafe {
			DispenserAmmoEvent {
				building: Entity::from_live(server, building),
				class: route.class,
				player: Entity::from_live(server, player),
			}
		};
		match (route.callback)(server, event) {
			DispenserAmmoAction::Continue => HookAction::Ignore,
			DispenserAmmoAction::Supply(supplied) => HookAction::Supersede(supplied),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi, and the dispatcher
// calls handlers only on that thread. Cell state is copied before callbacks.
unsafe impl Sync for DispenserRoute {}

#[derive(Clone, Copy)]
struct RoutedDispenser {
	binding: ServerBinding,
	callback: DispenserAmmoFn,
	class: BuildingClass,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Run `callback` before dispensers supply ammunition and metal, finding
	/// only dispenser-derived classes in a shared game-module snapshot.
	///
	/// Install once, such as while loading. An installed hook returns
	/// [`HookError::AlreadyInstalled`] without searching again. The vtables
	/// are checked before registration; a partial refusal rolls back all
	/// hooks installed by this call. The callback must defer entity deletion
	/// as [`Server::new`] requires. See the module's KHook timing limitations.
	///
	/// # Safety
	///
	/// `targets` and `binding` must describe the same TF2 server. The classes
	/// [`BuildingClass::name`] names for the four dispenser classes must be
	/// TF2's dispenser-derived classes from `tf_obj_dispenser.h`,
	/// `tf_logic_player_destruction.h` and `tf_robot_destruction_robot.h`, with
	/// `CObjectDispenser` as their primary base. Their primary vtables must
	/// hold `bool DispenseAmmo(CTFPlayer *)` at [`DISPENSE_AMMO_SLOT`] and stay
	/// loaded until the plugin unloads. RTTI and shared-function checks do not
	/// prove inheritance or the native signature by themselves.
	pub unsafe fn hook_dispenser_ammo(
		self,
		targets: &ClassTargets<'_>,
		binding: ServerBinding,
		callback: DispenserAmmoFn,
	) -> Result<DispenserHooks, DispenserHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(BuildingVtableError::WrongGame.into());
		}
		if dispenser_hooked(self) {
			return Err(HookError::AlreadyInstalled.into());
		}
		let names = DISPENSER_CLASSES.map(BuildingClass::name);
		let found = targets.find_all(&names, DISPENSE_AMMO_SLOT);
		let mut vtables = [NonNull::dangling(); 4];
		for ((vtable, found), class) in vtables.iter_mut().zip(found).zip(DISPENSER_CLASSES) {
			*vtable = found.ok_or(BuildingVtableError::NotFound(class))?.as_ptr();
		}
		// SAFETY: The caller promises the found classes have the typed method
		// at this slot and their module remains loaded.
		unsafe { self.install_dispenser_ammo(vtables, binding, callback) }
	}

	/// # Safety
	///
	/// Each vtable must stay live and hold a function of `DispenseAmmo` at its
	/// slot, on dispenser objects of the corresponding `DISPENSER_CLASSES`.
	unsafe fn install_dispenser_ammo(
		self,
		vtables: [NonNull<*mut c_void>; 4],
		binding: ServerBinding,
		callback: DispenserAmmoFn,
	) -> Result<DispenserHooks, DispenserHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(BuildingVtableError::WrongGame.into());
		}
		if dispenser_hooked(self) {
			return Err(HookError::AlreadyInstalled.into());
		}
		let mut functions = [NonNull::dangling(); 4];
		for (function, vtable) in functions.iter_mut().zip(vtables) {
			// SAFETY: As the caller promises; native hooks are unwrapped.
			*function =
				unsafe { self.original_function(DISPENSE_AMMO, HookTarget::vtable(vtable)) }?
					.address();
		}
		let distinct =
			(0..4).all(|first| (first + 1..4).all(|second| vtables[first] != vtables[second]));
		if !distinct || functions.iter().any(|&function| function != functions[0]) {
			return Err(DispenserHookError::UnexpectedLayout);
		}
		let mut installed = DispenserHooks {
			hooks: Vec::new(),
			_not_thread_safe: PhantomData,
		};
		for ((route, vtable), class) in AMMO_ROUTES.iter().zip(vtables).zip(DISPENSER_CLASSES) {
			// SAFETY: As the caller promises, this is a live typed class slot.
			let added = unsafe {
				self.add_hook(
					DISPENSE_AMMO,
					HookTarget::vtable(vtable),
					HookTiming::Pre,
					route,
				)
			};
			match added {
				Ok(hook) => {
					route.state.set(Some(RoutedDispenser {
						binding,
						callback,
						class,
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

fn clear_route(hook: HookId) {
	for route in &AMMO_ROUTES {
		if route.state.get().is_some_and(|state| state.hook == hook) {
			route.state.set(None);
		}
	}
}

fn dispenser_hooked(api: MetamodApi<'_>) -> bool {
	AMMO_ROUTES.iter().any(|route| {
		route
			.state
			.get()
			.is_some_and(|state| api.has_hook(state.hook))
	})
}
