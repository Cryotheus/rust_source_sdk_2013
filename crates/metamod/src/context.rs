//! Callback-scoped context identity and thread-local cache.

use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ops::Deref;
use std::panic;
use std::ptr::NonNull;
use std::rc::Rc;
use std::thread;

thread_local! {
	static STATE: Cell<CacheState> = const { Cell::new(CacheState::released()) };
}

/// Identifies one loaded generation of a plugin.
///
/// The generation prevents a cached context from an old load from resolving
/// after a plugin object is reused at the same address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextKey {
	plugin: NonNull<c_void>,
	generation: u64,
}

impl ContextKey {
	pub const fn new(plugin: NonNull<c_void>, generation: u64) -> Self {
		Self { plugin, generation }
	}

	pub const fn plugin(self) -> NonNull<c_void> {
		self.plugin
	}
}

/// Returns the active callback's key, if one is cached on this thread.
pub fn cached_context_key() -> Option<ContextKey> {
	STATE.with(|state| state.get().key)
}

/// A context cached on the current thread for one callback's lifetime.
#[derive(Debug)]
pub struct CachedContext<T> {
	context: T,
	key: ContextKey,
	_not_send_or_sync: PhantomData<Rc<()>>,
}

impl<T> CachedContext<T> {
	#[track_caller]
	pub fn new(context: T, key: ContextKey) -> Self {
		CacheState::acquire(key);
		Self {
			context,
			key,
			_not_send_or_sync: PhantomData,
		}
	}

	pub fn context(&self) -> &T {
		&self.context
	}
}

impl<T: Clone> Clone for CachedContext<T> {
	fn clone(&self) -> Self {
		Self::new(self.context.clone(), self.key)
	}
}

impl<T> Deref for CachedContext<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		self.context()
	}
}

impl<T> Drop for CachedContext<T> {
	fn drop(&mut self) {
		if thread::panicking() {
			panic::catch_unwind(CacheState::release).ok();
		} else {
			CacheState::release();
		}
	}
}

#[derive(Debug, Clone, Copy)]
struct CacheState {
	key: Option<ContextKey>,
	depth: usize,
}

impl CacheState {
	const fn released() -> Self {
		Self {
			key: None,
			depth: 0,
		}
	}

	#[track_caller]
	fn acquire(key: ContextKey) {
		STATE.with(|state| {
			let mut current = state.get();
			match current.key {
				Some(cached) => assert_eq!(
					cached, key,
					"cannot cache contexts from different plugin generations"
				),
				None => current.key = Some(key),
			}
			current.depth = current
				.depth
				.checked_add(1)
				.expect("plugin context cache depth overflowed");
			state.set(current);
		});
	}

	#[track_caller]
	fn release() {
		STATE.with(|state| {
			let mut current = state.get();
			assert_ne!(
				current.depth, 0,
				"context cache release without matching acquire"
			);
			current.depth -= 1;
			if current.depth == 0 {
				current.key = None;
			}
			state.set(current);
		});
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn nested_caches_release_the_key_at_zero_depth() {
		let plugin = NonNull::<c_void>::dangling();
		let key = ContextKey::new(plugin, 1);
		let outer = CachedContext::new(12, key);
		assert_eq!(cached_context_key(), Some(key));
		let inner = outer.clone();
		assert_eq!(*inner, 12);
		drop(inner);
		assert_eq!(cached_context_key(), Some(key));
		drop(outer);
		assert_eq!(cached_context_key(), None);
	}
}
