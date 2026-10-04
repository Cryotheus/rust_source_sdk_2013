//! A cache of what a resolver found in a module, kept for the rest of the
//! process and keyed by the module's identity, for resolvers that inspect a
//! whole module and would be too slow to run in every callback.

use super::{Error, Module};
use std::sync::{Mutex, PoisonError};

/// The last resolution a resolver kept, with the module it was resolved in.
///
/// A [`get_or_resolve`](Self::get_or_resolve) for the same [`ModuleKey`]
/// returns it again without inspecting the module. One for another key
/// resolves anew and replaces it. Failures are not kept, so the next call
/// inspects the module again instead of repeating an error that may have been
/// transient, such as a failed read.
///
/// The cache cannot tell a module from a different image later loaded at the
/// same base with its `CreateInterface` at the same address, and its users
/// assume that this does not happen. Source never unloads the game server
/// module while plugins are loaded: Metamod:Source and the engine unload
/// plugins first, and a cache, a static of the plugin that links this crate,
/// is unloaded with it.
#[derive(Debug)]
pub struct ModuleCache<T: Copy>(Mutex<Option<(ModuleKey, T)>>);

impl<T: Copy> ModuleCache<T> {
	/// An empty cache.
	pub const fn new() -> Self {
		Self(Mutex::new(None))
	}

	/// The value the cache holds for `key`, or else the one `resolve` finds,
	/// which then replaces its entry. A failure leaves the entry as it was.
	///
	/// `resolve` runs without the cache's lock: concurrent misses each
	/// resolve, and the last to finish keeps its entry.
	pub fn get_or_resolve<E>(
		&self,
		key: ModuleKey,
		resolve: impl FnOnce() -> Result<T, E>,
	) -> Result<T, E> {
		// Nothing that can panic runs while the lock is held, and the entry is
		// `Copy` and written in one assignment, so even a poisoned lock would hold a
		// complete entry.
		let cached = *self.0.lock().unwrap_or_else(PoisonError::into_inner);

		if let Some((cached_key, value)) = cached
			&& cached_key == key
		{
			return Ok(value);
		}

		let value = resolve()?;

		*self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((key, value));

		Ok(value)
	}
}

impl<T: Copy> Default for ModuleCache<T> {
	fn default() -> Self {
		Self::new()
	}
}

/// A module a [`ModuleCache`] resolved: the base address it is mapped at, and
/// the address of its `CreateInterface` export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleKey {
	/// The module's base address.
	pub base: usize,

	/// The address of the module's `CreateInterface` export.
	pub factory: usize,
}

impl ModuleKey {
	/// The key of the module whose `CreateInterface` export is at `factory`,
	/// which the loader finds.
	///
	/// Fails if no loaded module contains `factory`.
	///
	/// # Safety
	///
	/// The module containing `factory` stays loaded for the whole call.
	pub unsafe fn of(factory: usize) -> Result<Self, Error> {
		// SAFETY: The caller keeps the factory's module loaded for this call, so
		// it stays loaded while the loader finds it. On Windows, the loader
		// reference `Module` takes is released when it drops, at the end of this
		// statement.
		let base = unsafe { Module::at(factory) }?.base();

		Ok(Self { base, factory })
	}
}
