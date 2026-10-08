//! `IGameEventManager2` and the game events it creates, fires, and delivers.

use crate::NotThreadSafe;
use crate::bitbuf::BitWriter;
use crate::players::UserId;
use sdk_raw::abi::WChar;
use sdk_raw::bitbuf::{BfRead, BfWrite};

use sdk_raw::interfaces::game_event::{
	EventValue, GameEventListenerObject, MAX_EVENT_BITS, MAX_EVENT_BYTES, OnFireGameEvent,
	for_event_data,
};

use sdk_raw::util::cstr::{borrow_cstr, copy_cstr};
use sdk_raw::vcall;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_int, c_void};
use std::fmt::{Display, Formatter};
use std::marker::{PhantomData, PhantomPinned};
use std::mem::{ManuallyDrop, size_of};
use std::ops::{ControlFlow, Deref};
use std::pin::Pin;
use std::ptr::NonNull;

/// Name used to request [`sys::IGameEventManager2`] from an engine interface factory.
#[doc(alias("INTERFACEVERSION_GAMEEVENTSMANAGER2"))]
pub const GAME_EVENT_MANAGER_INTERFACE_VERSION: &CStr = GameEventManager::<'static>::VERSION;

/// The manager refused to register a listener, as
/// [`GameEventManager::add_listener`] reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not listen for `{}`; no such game event is registered", .name.to_string_lossy())]
pub struct AddListenerError {
	name: CString,
}

/// The manager refused to create an event, as
/// [`GameEventManager::create_event`] reports.
///
/// The manager creates no event that is unknown or has no registered listener.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no game event named `{}` is registered", .name.to_string_lossy())]
pub struct CreateEventError {
	name: CString,
}

/// The manager refused to fire an event, as [`OwnedGameEvent::fire`] reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the game event manager did not fire `{}`", .name.to_string_lossy())]
pub struct FireEventError {
	name: CString,
}

/// A game event, readable for `'e` (`IGameEvent`).
///
/// Events delivered to a [`GameEventHandler`] live for the duration of the
/// call, and events created with [`GameEventManager::create_event`] until they
/// are fired or dropped, so the lifetime keeps a handle from outliving either.
#[doc(alias("IGameEvent"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameEvent<'e> {
	raw: NonNull<sys::IGameEvent>,
	_lifetime: PhantomData<&'e ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'e> GameEvent<'e> {
	/// # Safety
	///
	/// `raw` must be a live event that stays allocated for `'e`, used only on
	/// the server's main thread.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::IGameEvent>) -> Self {
		Self {
			raw,
			_lifetime: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the event pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::IGameEvent {
		self.raw.as_ptr()
	}

	/// Searches the event data for a [value] with a matching [key], or returns
	/// `None` if no key matches.
	///
	/// [key]: GameEventDataKey
	/// [value]: GameEventDataValue
	pub fn find(self, key: impl AsRef<str>) -> Option<GameEventDataValue> {
		let mut state = ControlFlow::Continue(key.as_ref());

		// SAFETY: The event is live for `'e`, on the main thread, and the
		// callback runs no code that could change it.
		unsafe {
			for_event_data(self.raw, &mut |name, value| {
				let ControlFlow::Continue(key) = state else {
					unreachable!("the engine visited a pair after the finder stopped")
				};

				// Comparing bytes skips names that are not UTF-8 instead of
				// panicking.
				if key.as_bytes() == name.to_bytes() {
					state = ControlFlow::Break(GameEventDataValue::from_raw(value));

					false
				} else {
					true
				}
			})
		};

		state.break_value()
	}

	/// Returns the first [key] of the event data, or `None` if it has none.
	///
	/// [key]: GameEventDataKey
	pub fn first_key(self) -> Option<GameEventDataKey> {
		struct VisitFirstKey;

		impl GameEventVisitor for VisitFirstKey {
			type Break = GameEventDataKey;

			fn visit(
				&mut self,
				key: GameEventDataKey,
				_value: GameEventDataValue,
			) -> ControlFlow<Self::Break> {
				ControlFlow::Break(key)
			}
		}

		self.visit(VisitFirstKey).break_value()
	}

	/// Returns the first [key]-[value] pair of the event data, or `None` if it
	/// has none.
	///
	/// [key]: GameEventDataKey
	/// [value]: GameEventDataValue
	pub fn first_pair(self) -> Option<GameEventDataPair> {
		struct VisitFirstPair;

		impl GameEventVisitor for VisitFirstPair {
			type Break = GameEventDataPair;

			fn visit(
				&mut self,
				key: GameEventDataKey,
				value: GameEventDataValue,
			) -> ControlFlow<Self::Break> {
				ControlFlow::Break(GameEventDataPair(key, value))
			}
		}

		self.visit(VisitFirstPair).break_value()
	}

	/// Returns the first [value] of the event data, or `None` if it has none.
	///
	/// [value]: GameEventDataValue
	pub fn first_value(self) -> Option<GameEventDataValue> {
		struct VisitFirstValue;

		impl GameEventVisitor for VisitFirstValue {
			type Break = GameEventDataValue;

			fn visit(
				&mut self,
				_key: GameEventDataKey,
				value: GameEventDataValue,
			) -> ControlFlow<Self::Break> {
				ControlFlow::Break(value)
			}
		}

		self.visit(VisitFirstValue).break_value()
	}

	/// Reads a boolean value, or `None` if the event has no such key.
	#[doc(alias("GetBool"))]
	pub fn get_bool(self, key: &CStr) -> Option<bool> {
		// SAFETY: The event is live for `'e`.
		self.has_key(key)
			.then(|| unsafe { vcall!(self.as_ptr() => IGameEvent_GetBool(key.as_ptr(), false)) })
	}

	/// Reads a float value, or `None` if the event has no such key.
	#[doc(alias("GetFloat"))]
	pub fn get_float(self, key: &CStr) -> Option<f32> {
		// SAFETY: As for `get_bool`.
		self.has_key(key)
			.then(|| unsafe { vcall!(self.as_ptr() => IGameEvent_GetFloat(key.as_ptr(), 0.0)) })
	}

	/// Reads an integer value, or `None` if the event has no such key.
	#[doc(alias("GetInt"))]
	pub fn get_int(self, key: &CStr) -> Option<c_int> {
		// SAFETY: As for `get_bool`.
		self.has_key(key)
			.then(|| unsafe { vcall!(self.as_ptr() => IGameEvent_GetInt(key.as_ptr(), 0)) })
	}

	/// Reads a string value, or `None` if the event has no such key.
	///
	/// Values of other types are formatted as strings.
	#[doc(alias("GetString"))]
	pub fn get_string(self, key: &CStr) -> Option<CString> {
		if !self.has_key(key) {
			return None;
		}

		// SAFETY: As for `get_bool`. The string lives in the event's key
		// values, which a later access may rewrite, so it is copied.
		unsafe {
			copy_cstr(vcall!(self.as_ptr() => IGameEvent_GetString(key.as_ptr(), c"".as_ptr())))
		}
	}

	/// Reads a 64-bit value, or `None` if the event has no such key.
	#[doc(alias("GetUint64"))]
	pub fn get_uint64(self, key: &CStr) -> Option<u64> {
		// SAFETY: As for `get_bool`.
		self.has_key(key)
			.then(|| unsafe { vcall!(self.as_ptr() => IGameEvent_GetUint64(key.as_ptr(), 0)) })
	}

	/// Whether the event has a value for `key`.
	#[doc(alias("IsEmpty"))]
	pub fn has_key(self, key: &CStr) -> bool {
		// SAFETY: As for `get_bool`.
		!unsafe { vcall!(self.as_ptr() => IGameEvent_IsEmpty(key.as_ptr())) }
	}

	/// Whether the event is never networked to clients.
	#[doc(alias("IsLocal"))]
	pub fn is_local(self) -> bool {
		// SAFETY: As for `get_bool`.
		unsafe { vcall!(self.as_ptr() => IGameEvent_IsLocal()) }
	}

	/// Whether the event is networked reliably.
	#[doc(alias("IsReliable"))]
	pub fn is_reliable(self) -> bool {
		// SAFETY: As for `get_bool`.
		unsafe { vcall!(self.as_ptr() => IGameEvent_IsReliable()) }
	}

	/// Runs a [`GameEventVisitor`] which collects the event's [data keys] into a [`HashSet`].
	///
	/// # Panics
	///
	/// Panics if a key repeats. The panic cannot unwind into the engine, so it
	/// aborts the process.
	///
	/// [data keys]: GameEventDataKey
	pub fn keys_to_set(self) -> HashSet<GameEventDataKey> {
		struct VisitorHashSetKeys(HashSet<GameEventDataKey>);

		impl GameEventVisitor for VisitorHashSetKeys {
			type Break = !;

			fn visit(
				&mut self,
				key: GameEventDataKey,
				_value: GameEventDataValue,
			) -> ControlFlow<Self::Break> {
				assert!(self.0.insert(key));
				ControlFlow::Continue(())
			}
		}

		self.visit_nb(VisitorHashSetKeys(HashSet::new())).0
	}

	/// Runs a [`GameEventVisitor`] which collects the event's [data keys] into a [`Vec`].
	///
	/// [data keys]: GameEventDataKey
	pub fn keys_to_vec(self) -> Vec<GameEventDataKey> {
		struct VisitorVecKeys(Vec<GameEventDataKey>);

		impl GameEventVisitor for VisitorVecKeys {
			type Break = !;

			fn visit(
				&mut self,
				key: GameEventDataKey,
				_value: GameEventDataValue,
			) -> ControlFlow<Self::Break> {
				self.0.push(key);
				ControlFlow::Continue(())
			}
		}

		self.visit_nb(VisitorVecKeys(Vec::new())).0
	}

	/// The internal name of the [`GameEvent`].
	///
	/// With the `tf2` feature, this has the same effect as calling `id` and
	/// `GameEventId::name_cstr` together, but with less overhead, and also names
	/// events `GameEventId` does not list.
	///
	/// # Panics
	///
	/// Panics if the engine returns a null name.
	#[doc(alias("GetName"))]
	pub fn name(self) -> &'e CStr {
		// SAFETY: As for `get_bool`. The name belongs to the event's descriptor,
		// which the manager keeps while the event exists.
		let name = unsafe { borrow_cstr(vcall!(self.as_ptr() => IGameEvent_GetName())) };

		name.expect("IGameEvent::GetName returned null")
	}

	/// Runs a [`GameEventVisitor`] which collects the event's data [key]-[value] pairs into a [`HashMap`].
	///
	/// # Panics
	///
	/// Panics if a key repeats. The panic cannot unwind into the engine, so it
	/// aborts the process.
	///
	/// [key]: GameEventDataKey
	/// [value]: GameEventDataValue
	pub fn pairs_to_set(self) -> HashMap<GameEventDataKey, GameEventDataValue> {
		struct VisitorHashMapPairs(HashMap<GameEventDataKey, GameEventDataValue>);

		impl GameEventVisitor for VisitorHashMapPairs {
			type Break = !;

			fn visit(
				&mut self,
				key: GameEventDataKey,
				value: GameEventDataValue,
			) -> ControlFlow<Self::Break> {
				assert!(self.0.insert(key, value).is_none());
				ControlFlow::Continue(())
			}
		}

		self.visit_nb(VisitorHashMapPairs(HashMap::new())).0
	}

	/// Runs a [`GameEventVisitor`] which collects the event's data [key]-[value] pairs into a [`Vec`].
	///
	/// [key]: GameEventDataKey
	/// [value]: GameEventDataValue
	pub fn pairs_to_vec(self) -> Vec<GameEventDataPair> {
		struct VisitorVecPairs(Vec<GameEventDataPair>);

		impl GameEventVisitor for VisitorVecPairs {
			type Break = !;

			fn visit(
				&mut self,
				key: GameEventDataKey,
				value: GameEventDataValue,
			) -> ControlFlow<Self::Break> {
				self.0.push(GameEventDataPair(key, value));
				ControlFlow::Continue(())
			}
		}

		self.visit_nb(VisitorVecPairs(Vec::new())).0
	}

	/// Runs a [`GameEventVisitor`] which collects the event's [data values] into a [`Vec`].
	///
	/// [data values]: GameEventDataValue
	pub fn values_to_vec(self) -> Vec<GameEventDataValue> {
		struct VisitorVecValues(Vec<GameEventDataValue>);

		impl GameEventVisitor for VisitorVecValues {
			type Break = !;

			fn visit(
				&mut self,
				_key: GameEventDataKey,
				value: GameEventDataValue,
			) -> ControlFlow<Self::Break> {
				self.0.push(value);
				ControlFlow::Continue(())
			}
		}

		self.visit_nb(VisitorVecValues(Vec::new())).0
	}

	/// Runs a [`GameEventVisitor`] over the event's data key-value pairs.
	///
	/// Returns [`ControlFlow::Break`] with the value the visitor stopped with,
	/// or [`ControlFlow::Continue`] with the visitor after it visited every pair.
	#[doc(alias("ForEventData"))]
	pub fn visit<V: GameEventVisitor>(self, visitor: V) -> ControlFlow<V::Break, V> {
		let mut state = ControlFlow::Continue(visitor);

		// SAFETY: The event is live for `'e`, on the main thread. The callback
		// copies the key and value before the visitor runs, so nothing borrows
		// the event while the visitor's code runs.
		unsafe {
			for_event_data(self.raw, &mut |name, value| {
				let ControlFlow::Continue(visitor) = &mut state else {
					unreachable!("the engine visited a pair after the visitor broke")
				};

				let key = GameEventDataKey(name.to_owned());
				let value = GameEventDataValue::from_raw(value);

				if let ControlFlow::Break(output) = visitor.visit(key, value) {
					state = ControlFlow::Break(output);

					false
				} else {
					true
				}
			})
		};

		state
	}

	/// Runs a visitor that never breaks, returning it after every pair.
	#[inline(always)]
	fn visit_nb<V: GameEventVisitor<Break = !>>(self, visitor: V) -> V {
		let ControlFlow::Continue(visitor) = self.visit(visitor);

		visitor
	}
}

/// A game event, readable and writable for `'e` (`IGameEvent`), which its
/// owner frees.
///
/// Hooks on the manager's `FireEvent` get one, to edit an event before its
/// listeners and clients get it. Like [`GameEvent`], it is a handle, and frees
/// nothing; an event this plugin creates is an [`OwnedGameEvent`] instead.
#[doc(alias("IGameEvent"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameEventMut<'e>(GameEvent<'e>);

impl<'e> GameEventMut<'e> {
	/// # Safety
	///
	/// `raw` must be a live event that stays allocated for `'e`, which its owner
	/// lets this plugin edit, used only on the server's main thread.
	pub const unsafe fn from_raw(raw: NonNull<sys::IGameEvent>) -> Self {
		// SAFETY: As the caller promises.
		Self(unsafe { GameEvent::from_raw(raw) })
	}

	/// Reads the event.
	pub const fn as_event(self) -> GameEvent<'e> {
		self.0
	}

	/// Returns the event pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::IGameEvent {
		self.0.as_ptr()
	}

	/// Sets a boolean value for `key`.
	#[doc(alias("SetBool"))]
	pub fn set_bool(self, key: &CStr, value: bool) {
		// SAFETY: The event is live, and its owner lets this plugin edit it.
		unsafe { vcall!(self.as_ptr() => IGameEvent_SetBool(key.as_ptr(), value)) };
	}

	/// Sets a float value for `key`.
	#[doc(alias("SetFloat"))]
	pub fn set_float(self, key: &CStr, value: f32) {
		// SAFETY: As for `set_bool`.
		unsafe { vcall!(self.as_ptr() => IGameEvent_SetFloat(key.as_ptr(), value)) };
	}

	/// Sets an integer value for `key`.
	#[doc(alias("SetInt"))]
	pub fn set_int(self, key: &CStr, value: c_int) {
		// SAFETY: As for `set_bool`.
		unsafe { vcall!(self.as_ptr() => IGameEvent_SetInt(key.as_ptr(), value)) };
	}

	/// Sets a string value for `key`. The event stores a copy of `value`.
	#[doc(alias("SetString"))]
	pub fn set_string(self, key: &CStr, value: &CStr) {
		// SAFETY: As for `set_bool`. The event copies the string.
		unsafe { vcall!(self.as_ptr() => IGameEvent_SetString(key.as_ptr(), value.as_ptr())) };
	}

	/// Sets a 64-bit value for `key`.
	#[doc(alias("SetUint64"))]
	pub fn set_uint64(self, key: &CStr, value: u64) {
		// SAFETY: As for `set_bool`.
		unsafe { vcall!(self.as_ptr() => IGameEvent_SetUint64(key.as_ptr(), value)) };
	}
}

/// A game event this plugin created, which is freed when dropped unless fired.
#[derive(Debug)]
pub struct OwnedGameEvent<'s> {
	raw: NonNull<sys::IGameEvent>,
	manager: GameEventManager<'s>,
}

impl<'s> OwnedGameEvent<'s> {
	/// Reads the event. The handle cannot outlive this owner.
	pub fn as_event(&self) -> GameEvent<'_> {
		// SAFETY: The event stays allocated until `self` fires or frees it.
		unsafe { GameEvent::from_raw(self.raw) }
	}

	/// Edits the event through a handle, which cannot outlive this owner.
	pub fn as_event_mut(&mut self) -> GameEventMut<'_> {
		// SAFETY: The event stays allocated until `self` fires or frees it, and
		// this plugin owns it.
		unsafe { GameEventMut::from_raw(self.raw) }
	}

	/// Delivers the event to every listener, and to clients if `broadcast` is set.
	///
	/// Listeners run synchronously, so this may run arbitrary game and plugin
	/// code, including this plugin's own listeners. Fails if the manager reports
	/// that it did not fire the event, which the engine frees either way.
	#[doc(alias("FireEvent"))]
	pub fn fire(self, broadcast: bool) -> Result<(), FireEventError> {
		let this = ManuallyDrop::new(self);
		let name = this.as_event().name().to_owned();

		// SAFETY: The manager takes ownership of the event, which is not used again.
		let fired = unsafe {
			vcall!(this.manager.as_ptr() => IGameEventManager2_FireEvent(this.raw.as_ptr(), !broadcast))
		};

		fired.then_some(()).ok_or(FireEventError { name })
	}

	/// Sets a boolean value for `key`.
	#[doc(alias("SetBool"))]
	pub fn set_bool(&mut self, key: &CStr, value: bool) {
		self.as_event_mut().set_bool(key, value);
	}

	/// Sets a float value for `key`.
	#[doc(alias("SetFloat"))]
	pub fn set_float(&mut self, key: &CStr, value: f32) {
		self.as_event_mut().set_float(key, value);
	}

	/// Sets an integer value for `key`.
	#[doc(alias("SetInt"))]
	pub fn set_int(&mut self, key: &CStr, value: c_int) {
		self.as_event_mut().set_int(key, value);
	}

	/// Sets a string value for `key`. The event stores a copy of `value`.
	#[doc(alias("SetString"))]
	pub fn set_string(&mut self, key: &CStr, value: &CStr) {
		self.as_event_mut().set_string(key, value);
	}

	/// Sets a 64-bit value for `key`.
	#[doc(alias("SetUint64"))]
	pub fn set_uint64(&mut self, key: &CStr, value: u64) {
		self.as_event_mut().set_uint64(key, value);
	}
}

impl Drop for OwnedGameEvent<'_> {
	fn drop(&mut self) {
		// SAFETY: The event was created by this manager and never fired.
		unsafe { vcall!(self.manager.as_ptr() => IGameEventManager2_FreeEvent(self.raw.as_ptr())) };
	}
}

interface! {
	/// Creates, fires, and delivers game events (`IGameEventManager2`).
	#[doc(alias("IGameEventManager2"))]
	pub struct GameEventManager(sys::IGameEventManager2) = Engine sdk_raw::interfaces::game_event::VERSION;
}

/// The name of a field in a game event's data, copied from the engine.
///
/// The string conversions, including [`Deref`] and [`Display`], panic if the
/// name is not UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameEventDataKey(pub(super) CString);

impl GameEventDataKey {
	/// Returns the name as a C string.
	pub fn as_c_str(&self) -> &CStr {
		&self.0
	}

	/// Returns the name as a string slice.
	///
	/// # Panics
	///
	/// Panics if the name is not UTF-8.
	pub fn as_str(&self) -> &str {
		self.0.to_str().expect("game event key should be UTF-8")
	}

	/// Converts the name into a [`String`].
	///
	/// # Panics
	///
	/// Panics if the name is not UTF-8.
	pub fn into_string(self) -> String {
		self.0
			.into_string()
			.expect("game event key should be UTF-8")
	}
}

impl Deref for GameEventDataKey {
	type Target = str;

	fn deref(&self) -> &Self::Target {
		self.as_str()
	}
}

impl Display for GameEventDataKey {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.as_str())
	}
}

/// An opaque, non-null local value supplied by Source.
///
/// Source declares this as `const void *`. The wrapper intentionally exposes
/// only a const raw pointer because the pointee type and lifetime are not part
/// of the game-event visitor contract.
#[derive(Debug, Clone, Copy)]
pub struct GameEventDataLocal(pub(super) NonNull<c_void>);

impl GameEventDataLocal {
	/// Returns the pointer, which is never null.
	pub const fn as_ptr(self) -> *const c_void {
		self.0.as_ptr()
	}
}

/// A key and its value from a game event's data.
#[derive(Debug, Clone)]
pub struct GameEventDataPair(
	/// The field's name.
	pub GameEventDataKey,
	/// The field's value.
	pub GameEventDataValue,
);

/// A value from a game event's data, copied from the engine and typed by the
/// `IGameEventVisitor2` method that delivered it.
#[derive(Debug, Clone)]
pub enum GameEventDataValue {
	/// See [`GameEventDataLocal`].
	///
	/// `GameEventDataLocal` is never a null-ptr, [`Self::Null`] is used instead.
	Local(GameEventDataLocal),

	/// A copied string, from `VisitString`.
	String(CString),

	/// A float, from `VisitFloat`.
	Float(f32),

	/// An integer, from `VisitInt`.
	Int(c_int),

	/// A 64-bit integer, from `VisitUint64`.
	UInt64(u64),

	/// A copied wide string without its NUL terminator, from `VisitWString`.
	WString(Vec<WChar>),

	/// A boolean, from `VisitBool`.
	Bool(bool),

	/// Source supplied a null pointer for a pointer-valued event field.
	///
	/// The visitor ABI permits this, so null is represented explicitly instead
	/// of manufacturing a reference or panicking inside the callback.
	Null,
}

impl GameEventDataValue {
	/// Copies a value an `IGameEventVisitor2` method delivered, with
	/// [`Null`](Self::Null) for a null pointer.
	fn from_raw(value: EventValue<'_>) -> Self {
		match value {
			EventValue::Local(local) => {
				local.map_or(Self::Null, |local| Self::Local(GameEventDataLocal(local)))
			}

			EventValue::String(string) => {
				string.map_or(Self::Null, |string| Self::String(string.to_owned()))
			}

			EventValue::Float(float) => Self::Float(float),
			EventValue::Int(int) => Self::Int(int),
			EventValue::UInt64(uint) => Self::UInt64(uint),

			EventValue::WString(wide) => {
				wide.map_or(Self::Null, |wide| Self::WString(wide.to_vec()))
			}

			EventValue::Bool(bool) => Self::Bool(bool),
		}
	}

	/// Returns the value of a [`Bool`](Self::Bool).
	///
	/// # Panics
	///
	/// Panics if the value is another variant.
	#[track_caller]
	pub fn unwrap_bool(self) -> bool {
		let Self::Bool(bool) = self else {
			panic!("Expected `Bool` found {self:?}");
		};

		bool
	}

	/// Returns the value of a [`Float`](Self::Float).
	///
	/// # Panics
	///
	/// Panics if the value is another variant.
	#[track_caller]
	pub fn unwrap_float(self) -> f32 {
		let Self::Float(float) = self else {
			panic!("Expected `Float` found {self:?}");
		};

		float
	}

	/// Returns the value of an [`Int`](Self::Int).
	///
	/// # Panics
	///
	/// Panics if the value is another variant.
	#[track_caller]
	pub fn unwrap_int(self) -> c_int {
		let Self::Int(int) = self else {
			panic!("Expected `Int` found {self:?}");
		};

		int
	}

	/// Returns the pointer of a [`Local`](Self::Local).
	///
	/// # Panics
	///
	/// Panics if the value is another variant, including [`Null`](Self::Null).
	#[track_caller]
	pub fn unwrap_local(self) -> GameEventDataLocal {
		let Self::Local(local) = self else {
			panic!("Expected `Local` found {self:?}");
		};

		local
	}

	/// Returns the pointer of a [`Local`](Self::Local), or `None` for
	/// [`Null`](Self::Null).
	///
	/// # Panics
	///
	/// Panics if the value is another variant.
	#[track_caller]
	pub fn unwrap_optional_local(self) -> Option<GameEventDataLocal> {
		match self {
			Self::Local(local) => Some(local),
			Self::Null => None,
			other => panic!("Expected `Local` found {other:?}"),
		}
	}

	/// Returns the [`UserId`] in an [`Int`](Self::Int), or `None` for a
	/// sentinel meaning no player (see [`InvalidUserId::is_sentinel`]).
	///
	/// # Panics
	///
	/// Panics if the value is another variant, or an integer that is neither a
	/// user ID nor a sentinel.
	///
	/// [`InvalidUserId::is_sentinel`]: crate::players::InvalidUserId::is_sentinel
	#[track_caller]
	pub fn unwrap_optional_user_id(self) -> Option<UserId> {
		let Self::Int(int) = self else {
			panic!("Expected `Int` found {self:?}");
		};

		match UserId::from_raw(int) {
			Ok(user_id) => Some(user_id),
			Err(error) if error.is_sentinel() => None,

			Err(error) => {
				panic!("Game event value was an integer {int}, but not a valid user ID {error}")
			}
		}
	}

	/// Returns the value of a [`String`](Self::String).
	///
	/// # Panics
	///
	/// Panics if the value is another variant, including [`Null`](Self::Null).
	#[track_caller]
	pub fn unwrap_string(self) -> CString {
		let Self::String(string) = self else {
			panic!("Expected `String` found {self:?}");
		};

		string
	}

	/// Returns the value of a [`UInt64`](Self::UInt64).
	///
	/// # Panics
	///
	/// Panics if the value is another variant.
	#[track_caller]
	pub fn unwrap_uint(self) -> u64 {
		let Self::UInt64(uint) = self else {
			panic!("Expected `UInt64` found {self:?}");
		};

		uint
	}

	/// Unwraps the value as an [`Int`], assuming it is a valid [`UserId`].
	/// If the integer can be zero, as to represent an `Option<UserId>`, use [`unwrap_optional_user_id`] instead.
	///
	/// # Panics
	///
	/// Panics if the value is another variant, or an integer that is not a
	/// user ID.
	///
	/// [`Int`]: Self::Int
	/// [`unwrap_optional_user_id`]: Self::unwrap_optional_user_id
	#[track_caller]
	pub fn unwrap_user_id(self) -> UserId {
		let Self::Int(int) = self else {
			panic!("Expected `Int` found {self:?}");
		};

		match UserId::from_raw(int) {
			Ok(user_id) => user_id,
			Err(error) => panic!("{error}"),
		}
	}

	/// Returns the value of a [`WString`](Self::WString).
	///
	/// # Panics
	///
	/// Panics if the value is another variant, including [`Null`](Self::Null).
	#[track_caller]
	pub fn unwrap_wstring(self) -> Vec<WChar> {
		let Self::WString(wstring) = self else {
			panic!("Expected `WString` found {self:?}");
		};

		wstring
	}
}

/// Receives the game events a [`GameEventListener`] is registered for.
pub trait GameEventHandler {
	/// Called by the engine for each event the listener is registered for.
	///
	/// A panic cannot unwind into the engine and aborts the server, so catch
	/// any the handler may raise.
	#[doc(alias("FireGameEvent"))]
	fn fire_game_event(&self, event: GameEvent<'_>);
}

/// A Rust implementation of the engine's `IGameEventListener2`.
///
/// The manager keeps the address of a registered listener, so registering
/// takes it pinned. The listener is `!Unpin` so it cannot move while pinned,
/// and `!Send`/`!Sync` since the engine calls it on the server's main thread.
#[doc(alias("IGameEventListener2"))]
pub struct GameEventListener<H> {
	raw: GameEventListenerObject<HandlerAdapter<H>>,
	_pinned: PhantomPinned,
	_not_thread_safe: NotThreadSafe,
}

impl<H: GameEventHandler> GameEventListener<H> {
	/// Wraps a handler. Pin the listener before registering it with
	/// [`GameEventManager::add_listener`].
	pub const fn new(handler: H) -> Self {
		Self {
			raw: GameEventListenerObject::new(HandlerAdapter(handler)),
			_pinned: PhantomPinned,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the address the manager registers and calls the listener by,
	/// which stays the same while the listener is pinned.
	fn as_raw(self: Pin<&Self>) -> *mut sys::IGameEventListener2 {
		self.get_ref().raw.as_raw()
	}

	/// Returns the handler the listener passes events to.
	pub const fn handler(&self) -> &H {
		&self.raw.inner().0
	}
}

impl<H: std::fmt::Debug> std::fmt::Debug for GameEventListener<H> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("GameEventListener")
			.field("handler", &self.raw.inner().0)
			.finish_non_exhaustive()
	}
}

/// Types that can consume an iterator of key-pairs emitted by the Source SDK's Game Event key-value iterator.
///
/// [`GameEvent::visit`] runs a visitor through an `IGameEventVisitor2`.
#[doc(alias("IGameEventVisitor2"))]
pub trait GameEventVisitor {
	/// The value the visitor stops with.
	type Break;

	/// Visits one key-value pair, returning [`ControlFlow::Break`] to stop.
	///
	/// The engine calls this through a C++ callback, so a panic cannot unwind
	/// into the engine and aborts the process.
	fn visit(
		&mut self,
		key: GameEventDataKey,
		value: GameEventDataValue,
	) -> ControlFlow<Self::Break>;
}

/// Passes the events a [`GameEventListener`] is fired to its
/// [`GameEventHandler`].
struct HandlerAdapter<H>(H);

impl<H: GameEventHandler> OnFireGameEvent for HandlerAdapter<H> {
	unsafe fn fire_game_event(&self, event: NonNull<sys::IGameEvent>) {
		// SAFETY: The caller passes a live event for the duration of the call,
		// on the server's main thread.
		self.0
			.fire_game_event(unsafe { GameEvent::from_raw(event) });
	}
}

impl<'s> GameEventManager<'s> {
	/// Registers a listener for an event name, or fails if the manager refuses.
	///
	/// # Safety
	///
	/// The listener must be removed with [`Self::remove_listener`] before it is
	/// dropped or the module containing it is unloaded, since the manager keeps
	/// calling it through its address until then.
	#[doc(alias("AddListener"))]
	pub unsafe fn add_listener<H: GameEventHandler>(
		self,
		listener: Pin<&GameEventListener<H>>,
		event: &CStr,
		server_side: bool,
	) -> Result<(), AddListenerError> {
		// SAFETY: `Server::new` guarantees the interface is live, and the caller
		// keeps the pinned listener registered no longer than it lives.
		let added = unsafe {
			vcall!(self.as_ptr() => IGameEventManager2_AddListener(listener.as_raw(), event.as_ptr(), server_side))
		};

		added.then_some(()).ok_or_else(|| AddListenerError {
			name: event.to_owned(),
		})
	}

	/// Creates an event to fill in and fire.
	///
	/// Fails if the event is unknown or no listener is registered for it.
	#[doc(alias("CreateEvent"))]
	pub fn create_event(self, name: &CStr) -> Result<OwnedGameEvent<'s>, CreateEventError> {
		// SAFETY: As for `add_listener`.
		let event = unsafe {
			vcall!(self.as_ptr() => IGameEventManager2_CreateEvent(name.as_ptr(), false))
		};

		NonNull::new(event)
			.map(|raw| OwnedGameEvent { raw, manager: self })
			.ok_or_else(|| CreateEventError {
				name: name.to_owned(),
			})
	}

	/// Creates a copy of an event to fill in and fire, or returns `None` if the
	/// manager returns no copy.
	#[doc(alias("DuplicateEvent"))]
	pub fn duplicate_event(self, event: GameEvent<'_>) -> Option<OwnedGameEvent<'s>> {
		// SAFETY: As for `add_listener`, and the event is live.
		let duplicate =
			unsafe { vcall!(self.as_ptr() => IGameEventManager2_DuplicateEvent(event.as_ptr())) };

		NonNull::new(duplicate).map(|raw| OwnedGameEvent { raw, manager: self })
	}

	/// Whether a listener is registered for an event name.
	#[doc(alias("FindListener"))]
	pub fn is_listening<H: GameEventHandler>(
		self,
		listener: Pin<&GameEventListener<H>>,
		event: &CStr,
	) -> bool {
		// SAFETY: As for `add_listener`. The manager only compares the address.
		unsafe {
			vcall!(self.as_ptr() => IGameEventManager2_FindListener(listener.as_raw(), event.as_ptr()))
		}
	}

	/// Loads event descriptions from a resource file.
	///
	/// # Safety
	///
	/// Loading a new event schema mutates shared manager state and may invalidate
	/// events or borrowed data obtained from this manager. The caller must ensure
	/// no such values are live or concurrently in use.
	#[doc(alias("LoadEventsFromFile"))]
	pub unsafe fn load_events_from_file(self, filename: &CStr) -> c_int {
		// SAFETY: The caller upholds the contract.
		unsafe { vcall!(self.as_ptr() => IGameEventManager2_LoadEventsFromFile(filename.as_ptr())) }
	}

	/// Removes every registration of a listener. Removing a listener that is
	/// not registered does nothing.
	#[doc(alias("RemoveListener"))]
	pub fn remove_listener<H: GameEventHandler>(self, listener: Pin<&GameEventListener<H>>) {
		// SAFETY: As for `is_listening`.
		unsafe { vcall!(self.as_ptr() => IGameEventManager2_RemoveListener(listener.as_raw())) };
	}

	/// Removes every event description and listener.
	///
	/// # Safety
	///
	/// Reset removes the manager's event data. The caller must ensure no events
	/// or values borrowed from them remain live or concurrently in use.
	#[doc(alias("Reset"))]
	pub unsafe fn reset(self) {
		// SAFETY: The caller upholds the contract.
		unsafe { vcall!(self.as_ptr() => IGameEventManager2_Reset()) }
	}

	/// Encodes an event as the engine sends it to clients: its ID, then its
	/// fields in the order its description lists them.
	///
	/// [`net::messages::GameEvent`](crate::net::messages::GameEvent) sends the
	/// result to a single client. Returns `None` if the manager has no
	/// description of the event, or the encoding exceeds 1024 bytes.
	#[doc(alias("SerializeEvent"))]
	pub fn serialize_event(self, event: GameEvent<'_>) -> Option<BitWriter> {
		let mut storage = [0u32; MAX_EVENT_BYTES / size_of::<u32>()];
		let mut buffer = BfWrite::empty(&mut storage);

		// SAFETY: As for `add_listener`, and the event is live. The engine writes
		// through the buffer, within the bounds it describes, and marks it
		// overflowed rather than exceed them.
		let serialized = unsafe {
			vcall!(self.as_ptr() => IGameEventManager2_SerializeEvent(
				event.as_ptr(),
				buffer.as_raw(),
			))
		};

		// SAFETY: The buffer describes `storage`, which the call has finished
		// writing.
		serialized
			.then(|| unsafe { BfWrite::read_back(NonNull::from(&mut buffer)) })
			.flatten()
			.map(BitWriter::from)
	}

	/// Decodes an event as clients do, from its ID and then its fields as
	/// [`Self::serialize_event`] encodes them, such as the payload of a
	/// [`net::messages::GameEvent`](crate::net::messages::GameEvent).
	///
	/// Returns `None` if the manager has no description of the event's ID, or
	/// the bits end before its fields do. The new event is freed when dropped,
	/// unless fired.
	#[doc(alias("UnserializeEvent"))]
	pub fn unserialize_event(self, data: &BitWriter) -> Option<OwnedGameEvent<'s>> {
		if data.len() < MAX_EVENT_BITS as usize {
			return None;
		}

		let mut buffer = BfRead::new(data.as_words(), data.len());

		// SAFETY: As for `add_listener`. The engine reads through the buffer,
		// within the bits it describes, which `data` holds and outlives the call,
		// and marks it overflowed rather than read past them.
		let event = unsafe {
			vcall!(self.as_ptr() => IGameEventManager2_UnserializeEvent(buffer.as_raw()))
		};

		let event = NonNull::new(event).map(|raw| OwnedGameEvent { raw, manager: self })?;

		// If the bits ran out, the fields after them read as zero: the event is
		// dropped, which frees it.
		(buffer.overflow == 0).then_some(event)
	}
}
