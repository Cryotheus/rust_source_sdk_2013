//! TF2 building placement hooks, which run instead of the game's
//! `CBaseObject::IsPlacementPosValid`, and may run it themselves.
//!
//! While an Engineer holds a blueprint, the game checks each frame whether
//! the building may be built where the blueprint is, through the building's
//! `IsPlacementPosValid`. The check refuses, among other places, respawn
//! rooms, `func_nobuild` brushes, places that hurt, and blueprints across a
//! respawn room visualizer, and the teleporter's adds its own. A callback can
//! decide alone, or run the game's check with [`Placement::check`], in a
//! world it changed for the check's duration.
//!
//! Every building class an Engineer places from a blueprint is hooked at once,
//! through the vtables [`object_vtables`] finds, without any building. As
//! with other Metamod hooks, they stop calling the callback while the plugin
//! is paused.

#[cfg(test)]
#[path = "tests/placement_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::objects::{
	IS_PLACEMENT_POS_VALID_SLOT, IsPlacementPosValidFn as IsPlacementPosValid,
};

use source_sdk_2013::tf2::objects::{ObjectHookTargetError, ObjectKind, object_vtables};
use source_sdk_2013::{Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// A callback-scoped server and the placement being checked, which decides
/// whether the building may be built there. A panic is contained by the hook
/// dispatcher, and leaves the decision to the game.
pub type PlacementFn = for<'s> fn(Server<'s>, Placement<'s>) -> PlacementAction;

/// `IsPlacementPosValid` in a TF2 building's primary vtable.
const IS_PLACEMENT_POS_VALID: VirtualFunction<IsPlacementPosValid> =
	VirtualFunction::new(IS_PLACEMENT_POS_VALID_SLOT);

static ROUTES: [PlacementRoute; 3] = [
	PlacementRoute::new(ObjectKind::Dispenser),
	PlacementRoute::new(ObjectKind::Sentry),
	PlacementRoute::new(ObjectKind::Teleporter),
];

/// The check of where a building's blueprint is, as a placement hook sees it.
#[derive(Clone, Copy)]
pub struct Placement<'s> {
	building: Entity<'s>,
	kind: ObjectKind,
	original: IsPlacementPosValid,
}

impl<'s> Placement<'s> {
	/// The building whose blueprint is placed.
	pub fn building(&self) -> Entity<'s> {
		self.building
	}

	/// Runs the game's check of the blueprint's position, and returns whether
	/// it allows building there. Each call runs it again. The other plugins'
	/// hooks on the check do not run.
	///
	/// The check traces against the world as it is during the call, and sets
	/// what the building notes of its position, such as the origin it would be
	/// built at.
	pub fn check(&self) -> bool {
		// SAFETY: The original is the unhooked check of this building's class,
		// called on the main thread with the live building, as the game calls
		// it.
		unsafe { (self.original)(self.building.as_ptr()) }
	}

	/// What the building is.
	pub fn kind(&self) -> ObjectKind {
		self.kind
	}
}

/// What a placement hook decides of a blueprint's position.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlacementAction {
	/// Lets the game check the position, through the other plugins' hooks.
	#[default]
	Continue,

	/// Allows building there, without the game's check, besides any the
	/// callback ran itself.
	Allow,

	/// Refuses building there, as for [`Self::Allow`].
	Refuse,
}

/// Why the building placement hooks could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum PlacementHookError {
	/// The buildings' vtables could not be found.
	#[error(transparent)]
	Target(#[from] ObjectHookTargetError),

	/// Metamod refused a hook. [`HookError::AlreadyInstalled`] means the
	/// buildings are already hooked.
	#[error(transparent)]
	Hook(#[from] HookError),
}

/// Handles for the building placement hooks installed by one call.
///
/// Metamod automatically disables these while paused and removes them before
/// library unload, including failed loads. [`Self::remove`] disables them
/// earlier. No building is retained, so the hooks survive level changes.
#[must_use = "retain placement hooks to support explicitly removing them"]
pub struct PlacementHooks {
	hooks: Vec<HookId>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl PlacementHooks {
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

struct PlacementRoute {
	kind: ObjectKind,
	state: Cell<Option<RoutedPlacement>>,
}

impl PlacementRoute {
	const fn new(kind: ObjectKind) -> Self {
		Self {
			kind,
			state: Cell::new(None),
		}
	}
}

impl Handler<IsPlacementPosValid> for PlacementRoute {
	fn call(&self, call: &HookCall<'_, IsPlacementPosValid>) -> HookAction<bool> {
		// An earlier hook already decided the placement.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let Some(building) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live building whose method is
		// about to run, which stays in the entity list through the call.
		let building = unsafe { Entity::from_live(server, building) };
		let placement = Placement {
			building,
			kind: self.kind,
			original: route.original,
		};

		match (route.callback)(server, placement) {
			PlacementAction::Continue => HookAction::Ignore,
			PlacementAction::Allow => HookAction::Supersede(true),
			PlacementAction::Refuse => HookAction::Supersede(false),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for PlacementRoute {}

#[derive(Clone, Copy)]
struct RoutedPlacement {
	binding: ServerBinding,
	callback: PlacementFn,
	hook: HookId,
	original: IsPlacementPosValid,
}

impl MetamodApi<'_> {
	/// Runs `callback` instead of each `CBaseObject::IsPlacementPosValid` of
	/// a TF2 dispenser, sentry gun or teleporter, which decides whether the
	/// building's blueprint may be built where it is.
	///
	/// `binding` must describe the same running server. Install once, such as
	/// during load: it reads the whole game module to find every building
	/// class before any hook is installed, and a refused installation rolls
	/// back its earlier hooks. Installing again returns
	/// [`HookError::AlreadyInstalled`], without reading the module, until the
	/// hooks are removed.
	///
	/// The callback is not run once an earlier hook superseded the check, as
	/// far as the hooking library reports it (see [`crate::hook`]). It runs
	/// every frame for each blueprint an Engineer holds, so it should be
	/// cheap.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned handle means registration was accepted, not that the next
	/// check will be intercepted.
	pub fn hook_object_placement(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: PlacementFn,
	) -> Result<PlacementHooks, PlacementHookError> {
		if self.placement_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		let targets = object_vtables(server)?
			.into_iter()
			.map(|target| (target.kind, target.as_ptr()))
			.collect::<Vec<_>>();

		// SAFETY: The SDK matched each building class's primary RTTI vtable
		// and verified an executable IsPlacementPosValid slot, whose ABI comes
		// from the generated CBaseObject declaration. The game module stays
		// loaded until Metamod unloads this plugin.
		unsafe { self.install_placement(&targets, binding, callback) }
	}

	/// Hooks `IsPlacementPosValid` through each of `targets`' vtables, for
	/// the building it names.
	///
	/// # Safety
	///
	/// Each vtable must be live, hold a function of the signature
	/// [`IsPlacementPosValid`] at [`IS_PLACEMENT_POS_VALID_SLOT`], and stay
	/// loaded until Metamod unloads the plugin.
	unsafe fn install_placement(
		self,
		targets: &[(ObjectKind, NonNull<*mut c_void>)],
		binding: ServerBinding,
		callback: PlacementFn,
	) -> Result<PlacementHooks, PlacementHookError> {
		if self.placement_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		let mut installed = PlacementHooks {
			hooks: Vec::with_capacity(targets.len()),
			_not_thread_safe: PhantomData,
		};

		for &(kind, vtable) in targets {
			let route = ROUTES
				.iter()
				.find(|route| route.kind == kind)
				.expect("every building kind has a route");
			let target = HookTarget::vtable(vtable);
			// SAFETY: As the caller promises, the vtable is live, has
			// `bool ()` at the slot, and stays loaded.
			let hooked = unsafe { self.original_function(IS_PLACEMENT_POS_VALID, target) }
				.and_then(|original| {
					// SAFETY: As above.
					let hook = unsafe {
						self.add_hook(IS_PLACEMENT_POS_VALID, target, HookTiming::Pre, route)
					}?;

					Ok(RoutedPlacement {
						binding,
						callback,
						hook,
						original,
					})
				});

			match hooked {
				Ok(state) => {
					route.state.set(Some(state));
					installed.hooks.push(state.hook);
				}

				Err(error) => {
					installed.remove(self);
					return Err(error.into());
				}
			}
		}

		Ok(installed)
	}

	/// Whether the buildings' placement checks are hooked, for this load of
	/// the plugin.
	fn placement_hooked(self) -> bool {
		ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| self.has_hook(state.hook))
		})
	}
}
