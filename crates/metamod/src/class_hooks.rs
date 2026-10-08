//! Hooks of one virtual method on whole classes of TF2's entities, which a
//! [`ClassHooks`] handle extends class by class.
//!
//! The game calls an entity's virtual methods through the primary vtable of
//! its class. A hook on that table covers every entity of the class, those
//! created later included, but not those of the classes deriving from it,
//! which have tables of their own. The modules hooking methods so return a
//! [`ClassHooks`] that covers no class yet: cover each class to hook with
//! [`ClassHooks::cover`], from a [`ClassTarget`] that
//! [`ClassTargets`](source_sdk_2013::tf2::class_targets::ClassTargets) found
//! as the plugin loads, such as the players' classes, or with
//! [`ClassHooks::cover_entity`] as the first entity of a class appears, for
//! kinds of too many classes to name ahead, such as TF2's weapons.
//!
//! Hooks hold no entity, so they last through level changes, until removed or
//! the plugin unloads. As with other Metamod hooks, they stop calling handlers
//! while the plugin is paused.
//!
//! # Costs
//!
//! Under SourceHook, a plugin has [64 hook managers](crate::hook), one for each
//! method it hooks by signature and slot, which all the classes it covers
//! share: each module documents how many of them its hooks take. Each handle
//! leaks a small allocation, its hooks' handler, which Metamod keeps until it
//! unloads the plugin, as removing a hook only stops it.
//!
//! With Metamod 2.0, KHook can queue a hook's activation on its worker when
//! the vtable slot already has a detour, such as from the hook after the
//! method for the one before it. A covered class means its hooks were
//! accepted, not that the next call reaches them. KHook polls every 5 ms and
//! can retry while a detour is busy, so calls shortly after a class is covered
//! can be missed, or reach only the hook after the method, which installs
//! first.

#[cfg(test)]
#[path = "tests/class_hooks.rs"]
mod tests;

use crate::MetamodApi;
use crate::hook::{Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming};
use crate::hook::{Signature, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::tf2::class_targets::{ClassKind, ClassTarget, DerivesFrom};
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::fmt::{self, Debug, Formatter};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// How a hook's module calls its plugin's callback `C` for one call of the
/// hooked method on a live entity.
pub(crate) type Dispatch<S, C> =
	for<'s> fn(Server<'s>, C, Entity<'s>, &HookCall<'_, S>) -> HookAction<<S as Signature>::Output>;

/// One callback hooked on one virtual method of the classes of the kind `K`
/// that it covers, and on no other class: a [`HookTarget::vtable`] hook of
/// each class, before or after the method, or both, as the module that
/// returned it hooks.
///
/// Metamod disables the hooks while paused and removes them before unloading
/// the plugin. [`Self::remove`] disables them earlier. Dropping the handle
/// keeps them, and the classes they cover.
///
/// Each handle calls its callback once per call of the method on an entity of
/// a class it covers. Two handles covering a class with the same callback call
/// it twice.
#[must_use = "retain class hooks to cover classes and support explicitly removing them"]
pub struct ClassHooks<K: ClassKind> {
	classes: &'static dyn Classes,
	_kind: PhantomData<fn() -> K>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl<K: ClassKind> ClassHooks<K> {
	/// Hooks `function` on no class yet. `dispatch` calls `callback` for each
	/// call on an entity of a class [`Self::cover`] covers.
	///
	/// Every class of the kind `K` must hold a method of the signature `S` at
	/// `function`'s slot of its primary vtable, which `S` describes with an
	/// entity as `this`.
	pub(crate) fn new<S, C>(
		binding: ServerBinding,
		callback: C,
		function: VirtualFunction<S>,
		timings: &'static [HookTiming],
		dispatch: Dispatch<S, C>,
	) -> Self
	where
		S: Signature<This = sys::CBaseEntity>,
		C: Copy + 'static,
	{
		let route = Box::leak(Box::new(Route {
			binding,
			callback,
			classes: RefCell::new(Vec::new()),
			dispatch,
			function,
			removed: Cell::new(false),
			timings,
		}));

		Self {
			classes: route,
			_kind: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Hooks `target`'s class too, unless the hooks already cover it. Returns
	/// whether they did not.
	///
	/// `target` must come from the server the hooks' binding describes. If a
	/// hook of the class cannot be installed, those installed are removed, and
	/// the class is left uncovered.
	pub fn cover<C: DerivesFrom<K>>(
		&self,
		api: MetamodApi<'_>,
		target: ClassTarget<'_, C>,
	) -> Result<bool, HookError> {
		// SAFETY: The target is the primary vtable of a class of the kind `K` in
		// TF2's game module, which holds the method at its slot, and the module
		// stays loaded until Metamod unloads the plugin.
		unsafe { self.classes.cover(api, target.as_ptr()) }
	}

	/// Hooks the class of `entity` too, if it is of the kind `K`, unless the
	/// hooks already cover it. Returns whether they did not, and
	/// [`HookError::InvalidArgument`] for an entity not of the kind.
	///
	/// A covered class is found by its vtable alone, and a new one through its
	/// data description maps, as [`ClassTarget::of`] finds it. `server` and
	/// `entity` must come from the server the hooks' binding describes.
	pub fn cover_entity<'s>(
		&self,
		api: MetamodApi<'_>,
		server: Server<'s>,
		entity: Entity<'s>,
	) -> Result<bool, HookError> {
		if self.covers(entity) {
			return Ok(false);
		}

		let target = ClassTarget::<K>::of(server, entity).ok_or(HookError::InvalidArgument)?;

		self.cover(api, target)
	}

	/// Whether the hooks cover the class of `entity`.
	pub fn covers(&self, entity: Entity<'_>) -> bool {
		// SAFETY: A live entity starts with the pointer to its primary vtable,
		// of which only the address is used.
		let vtable = unsafe { vtable_pointer::<c_void>(entity.as_ptr()) };

		self.classes.covers(vtable.addr())
	}

	/// Whether the hooks cover no class.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// How many classes the hooks cover.
	pub fn len(&self) -> usize {
		self.classes.count()
	}

	/// Removes the hooks of every class covered, after which the callback no
	/// longer runs.
	pub fn remove(self, api: MetamodApi<'_>) {
		self.classes.remove(api);
	}
}

impl<K: ClassKind> Debug for ClassHooks<K> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("ClassHooks")
			.field("kind", &K::NAME)
			.field("classes", &self.len())
			.finish()
	}
}

/// The hooks of one callback on one method, over the classes they cover.
trait Classes {
	/// How many classes are covered.
	fn count(&self) -> usize;

	/// Hooks the method on the class of `vtable`, unless it already is.
	/// Returns whether it was not.
	///
	/// # Safety
	///
	/// `vtable` must be the live primary vtable of a class whose table holds
	/// the method at its slot, and stay loaded until Metamod unloads the
	/// plugin.
	unsafe fn cover(
		&'static self,
		api: MetamodApi<'_>,
		vtable: NonNull<*mut c_void>,
	) -> Result<bool, HookError>;

	/// Whether the class whose vtable is at `vtable` is covered.
	fn covers(&self, vtable: usize) -> bool;

	/// Removes every hook, and stops the callback.
	fn remove(&self, api: MetamodApi<'_>);
}

/// A class [`Route`] covers.
struct Covered {
	/// The address of the class's primary vtable.
	vtable: usize,

	/// The class's hooks, in [`Route::timings`]' order.
	hooks: Vec<HookId>,
}

/// The handler of one [`ClassHooks`], for every class it covers.
struct Route<S: Signature, C> {
	binding: ServerBinding,
	callback: C,
	classes: RefCell<Vec<Covered>>,
	dispatch: Dispatch<S, C>,
	function: VirtualFunction<S>,

	/// Whether [`ClassHooks::remove`] removed the hooks.
	removed: Cell<bool>,

	/// When the hooks run, in the order each class's install.
	timings: &'static [HookTiming],
}

impl<S, C> Classes for Route<S, C>
where
	S: Signature<This = sys::CBaseEntity>,
	C: Copy + 'static,
{
	fn count(&self) -> usize {
		self.classes.borrow().len()
	}

	unsafe fn cover(
		&'static self,
		api: MetamodApi<'_>,
		vtable: NonNull<*mut c_void>,
	) -> Result<bool, HookError> {
		let address = vtable.addr().get();

		if self.covers(address) {
			return Ok(false);
		}

		let target = HookTarget::vtable(vtable);
		let mut hooks = Vec::with_capacity(self.timings.len());

		for &timing in self.timings {
			// SAFETY: As the caller promises, the vtable is live, holds the method
			// at its slot, and stays loaded.
			match unsafe { api.add_hook(self.function, target, timing, self) } {
				Ok(hook) => hooks.push(hook),

				Err(error) => {
					for hook in hooks {
						api.remove_hook(hook);
					}

					return Err(error);
				}
			}
		}

		self.classes.borrow_mut().push(Covered {
			vtable: address,
			hooks,
		});

		Ok(true)
	}

	fn covers(&self, vtable: usize) -> bool {
		self.classes
			.borrow()
			.iter()
			.any(|covered| covered.vtable == vtable)
	}

	fn remove(&self, api: MetamodApi<'_>) {
		self.removed.set(true);

		for covered in self.classes.take() {
			for hook in covered.hooks {
				api.remove_hook(hook);
			}
		}
	}
}

impl<S, C> Handler<S> for Route<S, C>
where
	S: Signature<This = sys::CBaseEntity>,
	C: Copy + 'static,
{
	fn call(&self, call: &HookCall<'_, S>) -> HookAction<S::Output> {
		if self.removed.get() {
			return HookAction::Ignore;
		}

		let Some(entity) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { self.binding.server(&scope) };
		// SAFETY: The class hook supplies the live entity whose method runs,
		// which stays in the entity list through the call: the game only marks
		// entities it removes then for deletion.
		let entity = unsafe { Entity::from_live(server, entity) };

		(self.dispatch)(server, self.callback, entity, call)
	}
}
