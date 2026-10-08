//! Hooks on the game's entity factories, which run before each entity of a
//! class name is created and may refuse to create it.
//!
//! The game creates every entity it creates by name through the
//! [factory](source_sdk_2013::entities::factory) of its class name, calling the
//! factory's `IEntityFactory::Create` through its vtable: map entities, the
//! game's own `CreateEntityByName` and `CBaseEntity::Create`, `ent_create`,
//! scripts' `SpawnEntityFromTable` and `CreateByClassname`, and the precaches
//! of the classes the game registers for precaching (`PRECACHE_REGISTER`) as
//! each level loads. The hook is installed for one factory object, so it sees
//! the entities created by its class name alone, even when other names link
//! to the same entity class.
//!
//! A refused creation constructs nothing: `Create` returns null, so the game's
//! `CreateEntityByName` returns null, as for a class name it does not know.
//! Callers that check for that skip what they would have done with the entity,
//! as map loading does, or warn, as a class's precache does (`NULL Ent in
//! UTIL_PrecacheOther`). Callers that do not check crash, which is why
//! installing the hook is unsafe.
//!
//! Hooks last until removed or the plugin unloads. As with other Metamod
//! hooks, they stop calling callbacks while the plugin is paused, so the game
//! creates the class's entities as usual then.

#[cfg(test)]
#[path = "tests/entity_factory_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::factory::EntityFactory;
use source_sdk_2013::raw::entities::factory::{CREATE_SLOT, CreateFn};
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::CStr;
use std::ptr::{self, NonNull};

/// A callback-scoped server and the class name an entity is about to be
/// created by, which decides whether it is. A panic is contained by the hook
/// dispatcher, and lets the entity be created.
pub type EntityCreateFn = for<'s> fn(Server<'s>, &CStr) -> CreateAction;

/// `IEntityFactory::Create`.
const CREATE: VirtualFunction<CreateFn> = VirtualFunction::new(CREATE_SLOT);

/// One route per hooked factory.
static ROUTES: [FactoryRoute; 16] = [const { FactoryRoute::new() }; 16];

/// What an entity factory hook does with an entity about to be created.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CreateAction {
	/// Lets the factory create the entity.
	#[default]
	Allow,

	/// Skips the factory's `Create`, which returns null: no entity is
	/// constructed, and the caller gets none.
	Refuse,
}

struct FactoryRoute {
	state: Cell<Option<RoutedFactory>>,
}

impl FactoryRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<CreateFn> for FactoryRoute {
	fn call(&self, call: &HookCall<'_, CreateFn>) -> HookAction<*mut sys::IServerNetworkable> {
		// An earlier hook already decided what the factory returns.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (class_name,) = call.args();

		if class_name.is_null() {
			return HookAction::Ignore;
		}

		// SAFETY: The game passes the NUL-terminated class name the entity is
		// being created by, which outlives the call.
		let class_name = unsafe { CStr::from_ptr(class_name) };
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };

		match (route.callback)(server, class_name) {
			CreateAction::Allow => HookAction::Ignore,
			CreateAction::Refuse => HookAction::Supersede(ptr::null_mut()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for FactoryRoute {}

#[derive(Clone, Copy)]
struct RoutedFactory {
	binding: ServerBinding,
	callback: EntityCreateFn,
	factory: usize,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Runs `callback` before `factory` creates each entity, which the factory
	/// creates unless the callback returns [`CreateAction::Refuse`]; see the
	/// [module documentation](crate::entity_factory_hooks).
	///
	/// `factory` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Installing again on a
	/// hooked factory returns [`HookError::AlreadyInstalled`]; removing its ID
	/// allows replacement. Up to 16 factories can be hooked at a time, past
	/// which this returns [`HookError::TooManyFunctions`].
	///
	/// The callback is not run once an earlier hook superseded the creation,
	/// as far as the hooking library reports it (see [`crate::hook`]). It runs
	/// inside the game's creation of the entity, which can happen while a level
	/// loads, during map I/O and scripts, and during the game's thinks: it must
	/// not delete entities immediately, as [`Server::new`] requires, nor create
	/// an entity through the same factory, which would run it again.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next
	/// creation will be intercepted. KHook polls every 5 ms and can retry while
	/// a detour is busy, so creations shortly after installation can be missed.
	///
	/// # Safety
	///
	/// Every caller that creates entities through `factory` while the hook may
	/// refuse them, in the game DLL, its scripts, and other plugins, must cope
	/// with getting no entity, as from `CreateEntityByName` with an unknown
	/// class name, rather than dereference it.
	pub unsafe fn hook_entity_factory(
		self,
		factory: EntityFactory<'_>,
		binding: ServerBinding,
		callback: EntityCreateFn,
	) -> Result<HookId, HookError> {
		let object = NonNull::new(factory.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: The factory is a static of the game DLL, whose vtable has
		// `Create` at the slot, and which stays loaded until Metamod unloads
		// the plugin. The caller vouches for the factory's callers.
		unsafe { self.install_factory(object, binding, callback) }
	}

	/// Hooks `Create` on `object` alone.
	///
	/// # Safety
	///
	/// `object` must be live, and its vtable must hold a function of the
	/// signature [`CreateFn`] at [`CREATE_SLOT`], and stay loaded until Metamod
	/// unloads the plugin. Every caller of that function on `object` must cope
	/// with it returning null.
	unsafe fn install_factory(
		self,
		object: NonNull<sys::IEntityFactory>,
		binding: ServerBinding,
		callback: EntityCreateFn,
	) -> Result<HookId, HookError> {
		let factory = object.addr().get();

		if ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| state.factory == factory && self.has_hook(state.hook))
		}) {
			return Err(HookError::AlreadyInstalled);
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
		// `IServerNetworkable *(const char *)` at the slot, and stays loaded.
		let hook =
			unsafe { self.add_hook(CREATE, HookTarget::instance(object), HookTiming::Pre, route) }?;

		route.state.set(Some(RoutedFactory {
			binding,
			callback,
			factory,
			hook,
		}));

		Ok(hook)
	}
}
