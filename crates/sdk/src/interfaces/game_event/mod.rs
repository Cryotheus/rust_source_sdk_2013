//! `IGameEventManager2` and the game events it creates, fires, and delivers.

mod id;

use crate::abi::{CppDestructors, WChar};
use crate::bitbuf::{BitWriter, RawBfWrite};
use crate::ffi::{NotThreadSafe, borrow_cstr, copy_cstr, vcall};
use crate::players::UserId;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_float, c_int, c_void};
use std::fmt::{Display, Formatter};
use std::marker::{PhantomData, PhantomPinned};
use std::mem::{ManuallyDrop, offset_of, size_of};
use std::ops::{ControlFlow, Deref};
use std::pin::Pin;
use std::ptr::NonNull;
use std::slice::from_raw_parts;

pub use id::GameEventId;

/// Name used to request [`sys::IGameEventManager2`] from an engine interface factory.
pub const GAME_EVENT_MANAGER_INTERFACE_VERSION: &CStr = GameEventManager::<'static>::VERSION;

/// A game event, readable for `'e` (`IGameEvent`).
///
/// Events delivered to a [`GameEventHandler`] live for the duration of the
/// call, and events created with [`GameEventManager::create_event`] until they
/// are fired or dropped, so the lifetime keeps a handle from outliving either.
#[doc(alias = "IGameEvent")]
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

	/// Searches the event data for a [value] with a matching [key].
	///
	/// [key]: GameEventDataKey
	/// [value]: GameEventDataValue
	pub fn find(self, key: impl AsRef<str>) -> Option<GameEventDataValue> {
		let mut finder = VisitKeyFinder::new(key.as_ref());

		// SAFETY: The finder lives on the stack for the whole call.
		unsafe { self.visitor_exec((&raw mut finder).cast::<sys::IGameEventVisitor2>()) };
		finder.state.break_value()
	}

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
	#[doc(alias = "GetBool")]
	pub fn get_bool(self, key: &CStr) -> Option<bool> {
		// SAFETY: The event is live for `'e`.
		self.has_key(key)
			.then(|| unsafe { vcall!(self.as_ptr() => IGameEvent_GetBool(key.as_ptr(), false)) })
	}

	/// Reads a float value, or `None` if the event has no such key.
	#[doc(alias = "GetFloat")]
	pub fn get_float(self, key: &CStr) -> Option<f32> {
		// SAFETY: As for `get_bool`.
		self.has_key(key)
			.then(|| unsafe { vcall!(self.as_ptr() => IGameEvent_GetFloat(key.as_ptr(), 0.0)) })
	}

	/// Reads an integer value, or `None` if the event has no such key.
	#[doc(alias = "GetInt")]
	pub fn get_int(self, key: &CStr) -> Option<c_int> {
		// SAFETY: As for `get_bool`.
		self.has_key(key)
			.then(|| unsafe { vcall!(self.as_ptr() => IGameEvent_GetInt(key.as_ptr(), 0)) })
	}

	/// Reads a string value, or `None` if the event has no such key.
	///
	/// Values of other types are formatted as strings.
	#[doc(alias = "GetString")]
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
	#[doc(alias = "GetUint64")]
	pub fn get_uint64(self, key: &CStr) -> Option<u64> {
		// SAFETY: As for `get_bool`.
		self.has_key(key)
			.then(|| unsafe { vcall!(self.as_ptr() => IGameEvent_GetUint64(key.as_ptr(), 0)) })
	}

	/// Whether the event has a value for `key`.
	#[doc(alias = "IsEmpty")]
	pub fn has_key(self, key: &CStr) -> bool {
		// SAFETY: As for `get_bool`.
		!unsafe { vcall!(self.as_ptr() => IGameEvent_IsEmpty(key.as_ptr())) }
	}

	pub fn id(self) -> Option<GameEventId> {
		GameEventId::from_cstr(self.name())
	}

	#[doc(alias = "IsLocal")]
	pub fn is_local(self) -> bool {
		// SAFETY: As for `get_bool`.
		unsafe { vcall!(self.as_ptr() => IGameEvent_IsLocal()) }
	}

	#[doc(alias = "IsReliable")]
	pub fn is_reliable(self) -> bool {
		// SAFETY: As for `get_bool`.
		unsafe { vcall!(self.as_ptr() => IGameEvent_IsReliable()) }
	}

	/// Runs a [`GameEventVisitor`] which collects the event's [data keys] into a [`HashSet`].
	///
	/// [data keys]: GameEventDataKey
	pub fn keys_to_set(self) -> HashSet<GameEventDataKey> {
		#[repr(transparent)]
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
		#[repr(transparent)]
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
	/// This has the same affect as calling [`Self::id`] and [`GameEventId::name_cstr`] together,
	/// but with less overhead.
	#[doc(alias = "GetName")]
	pub fn name(self) -> &'e CStr {
		// SAFETY: As for `get_bool`. The name belongs to the event's descriptor,
		// which the manager keeps while the event exists.
		let name = unsafe { borrow_cstr(vcall!(self.as_ptr() => IGameEvent_GetName())) };

		name.expect("IGameEvent::GetName returned null")
	}

	/// Runs a [`GameEventVisitor`] which collects the event's data [key]-[value] pairs into a [`Vec`].
	///
	/// [key]: GameEventDataKey
	/// [value]: GameEventDataValue
	pub fn pairs_to_vec(self) -> Vec<GameEventDataPair> {
		#[repr(transparent)]
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

	/// Runs a [`GameEventVisitor`] which collects the event's data [key]-[value] pairs into a [`HashMap`].
	///
	/// [key]: GameEventDataKey
	/// [value]: GameEventDataValue
	pub fn pairs_to_set(self) -> HashMap<GameEventDataKey, GameEventDataValue> {
		#[repr(transparent)]
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

	/// Runs a [`GameEventVisitor`] which collects the event's [data values] into a [`Vec`].
	///
	/// [data values]: GameEventDataValue
	pub fn values_to_vec(self) -> Vec<GameEventDataValue> {
		#[repr(transparent)]
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
	pub fn visit<V: GameEventVisitor>(self, visitor: V) -> ControlFlow<V::Break, V> {
		let mut executor = VisitorExecutor::new(visitor);

		// SAFETY: The executor lives on the stack for the whole call.
		unsafe { self.visitor_exec((&raw mut executor).cast::<sys::IGameEventVisitor2>()) };
		executor.state
	}

	/// # Safety
	///
	/// `object` must be a visitor laid out as `IGameEventVisitor2`, live for the call.
	#[inline(always)]
	unsafe fn visitor_exec(self, object: *mut sys::IGameEventVisitor2) -> bool {
		// SAFETY: The event is live for `'e`, and the caller upholds the rest.
		unsafe { vcall!(self.as_ptr() => IGameEvent_ForEventData(object)) }
	}

	#[inline(always)]
	fn visit_nb<V: GameEventVisitor<Break = !>>(self, visitor: V) -> V {
		let ControlFlow::Continue(visitor) = self.visit(visitor);

		visitor
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

	#[doc(alias = "SetBool")]
	pub fn set_bool(&mut self, key: &CStr, value: bool) {
		// SAFETY: The event is live and owned by this plugin.
		unsafe { vcall!(self.raw.as_ptr() => IGameEvent_SetBool(key.as_ptr(), value)) };
	}

	#[doc(alias = "SetFloat")]
	pub fn set_float(&mut self, key: &CStr, value: f32) {
		// SAFETY: As for `set_bool`.
		unsafe { vcall!(self.raw.as_ptr() => IGameEvent_SetFloat(key.as_ptr(), value)) };
	}

	#[doc(alias = "SetInt")]
	pub fn set_int(&mut self, key: &CStr, value: c_int) {
		// SAFETY: As for `set_bool`.
		unsafe { vcall!(self.raw.as_ptr() => IGameEvent_SetInt(key.as_ptr(), value)) };
	}

	/// Stores a copy of `value`.
	#[doc(alias = "SetString")]
	pub fn set_string(&mut self, key: &CStr, value: &CStr) {
		// SAFETY: As for `set_bool`. The event copies the string.
		unsafe { vcall!(self.raw.as_ptr() => IGameEvent_SetString(key.as_ptr(), value.as_ptr())) };
	}

	#[doc(alias = "SetUint64")]
	pub fn set_uint64(&mut self, key: &CStr, value: u64) {
		// SAFETY: As for `set_bool`.
		unsafe { vcall!(self.raw.as_ptr() => IGameEvent_SetUint64(key.as_ptr(), value)) };
	}

	/// Delivers the event to every listener, and to clients if `broadcast` is set.
	///
	/// Listeners run synchronously, so this may run arbitrary game and plugin
	/// code, including this plugin's own listeners. The engine frees the event
	/// either way.
	#[doc(alias = "FireEvent")]
	pub fn fire(self, broadcast: bool) -> Result<(), FireEventError> {
		let this = ManuallyDrop::new(self);
		let name = this.as_event().name().to_owned();

		// SAFETY: The manager takes ownership of the event, which is not used again.
		let fired = unsafe {
			vcall!(this.manager.as_ptr() => IGameEventManager2_FireEvent(this.raw.as_ptr(), !broadcast))
		};

		fired.then_some(()).ok_or(FireEventError { name })
	}
}

impl Drop for OwnedGameEvent<'_> {
	fn drop(&mut self) {
		// SAFETY: The event was created by this manager and never fired.
		unsafe { vcall!(self.manager.as_ptr() => IGameEventManager2_FreeEvent(self.raw.as_ptr())) };
	}
}

/// The manager refused to create an event.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no game event named `{}` is registered", .name.to_string_lossy())]
pub struct CreateEventError {
	name: CString,
}

/// The manager refused to fire an event.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the game event manager did not fire `{}`", .name.to_string_lossy())]
pub struct FireEventError {
	name: CString,
}

/// The manager refused to register a listener.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not listen for `{}`; no such game event is registered", .name.to_string_lossy())]
pub struct AddListenerError {
	name: CString,
}

interface! {
	/// Creates, fires, and delivers game events (`IGameEventManager2`).
	#[doc(alias = "IGameEventManager2")]
	pub struct GameEventManager(sys::IGameEventManager2) = Engine c"GAMEEVENTSMANAGER002";
}

impl<'s> GameEventManager<'s> {
	/// Registers a listener for an event name.
	///
	/// # Safety
	///
	/// The listener must be removed with [`Self::remove_listener`] before it is
	/// dropped or the module containing it is unloaded, since the manager keeps
	/// calling it through its address until then.
	#[doc(alias = "AddListener")]
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

	/// Whether a listener is registered for an event name.
	#[doc(alias = "FindListener")]
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

	/// Removes every registration of a listener. Removing a listener that is
	/// not registered does nothing.
	#[doc(alias = "RemoveListener")]
	pub fn remove_listener<H: GameEventHandler>(self, listener: Pin<&GameEventListener<H>>) {
		// SAFETY: As for `is_listening`.
		unsafe { vcall!(self.as_ptr() => IGameEventManager2_RemoveListener(listener.as_raw())) };
	}

	/// Creates an event to fill in and fire.
	#[doc(alias = "CreateEvent")]
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

	/// Creates a copy of an event to fill in and fire.
	#[doc(alias = "DuplicateEvent")]
	pub fn duplicate_event(self, event: GameEvent<'_>) -> Option<OwnedGameEvent<'s>> {
		// SAFETY: As for `add_listener`, and the event is live.
		let duplicate =
			unsafe { vcall!(self.as_ptr() => IGameEventManager2_DuplicateEvent(event.as_ptr())) };

		NonNull::new(duplicate).map(|raw| OwnedGameEvent { raw, manager: self })
	}

	/// Encodes an event as the engine sends it to clients: its ID, then its
	/// fields in the order its description lists them.
	///
	/// [`net::messages::GameEvent`](crate::net::messages::GameEvent) sends the
	/// result to a single client. Returns `None` if the manager has no
	/// description of the event, or the encoding exceeds 1024 bytes.
	#[doc(alias = "SerializeEvent")]
	pub fn serialize_event(self, event: GameEvent<'_>) -> Option<BitWriter> {
		// `MAX_EVENT_BYTES`, the most the engine sends of an event.
		let mut storage = [0u32; 1024 / 4];
		let mut buffer = RawBfWrite::empty(&mut storage);

		// SAFETY: As for `add_listener`, and the event is live. The engine writes
		// through the buffer, within the bounds it describes, and marks it
		// overflowed rather than exceed them.
		let serialized = unsafe {
			vcall!(self.as_ptr() => IGameEventManager2_SerializeEvent(
				event.as_ptr(),
				(&raw mut buffer).cast::<sys::bf_write>(),
			))
		};

		// SAFETY: The buffer describes `storage`, which the call has finished
		// writing.
		serialized
			.then(|| unsafe { RawBfWrite::read_back(NonNull::from(&mut buffer)) })
			.flatten()
	}

	/// Loads event descriptions from a resource file.
	///
	/// # Safety
	///
	/// Loading a new event schema mutates shared manager state and may invalidate
	/// events or borrowed data obtained from this manager. The caller must ensure
	/// no such values are live or concurrently in use.
	#[doc(alias = "LoadEventsFromFile")]
	pub unsafe fn load_events_from_file(self, filename: &CStr) -> c_int {
		// SAFETY: The caller upholds the contract.
		unsafe { vcall!(self.as_ptr() => IGameEventManager2_LoadEventsFromFile(filename.as_ptr())) }
	}

	/// Removes every event description and listener.
	///
	/// # Safety
	///
	/// Reset removes the manager's event data. The caller must ensure no events
	/// or values borrowed from them remain live or concurrently in use.
	#[doc(alias = "Reset")]
	pub unsafe fn reset(self) {
		// SAFETY: The caller upholds the contract.
		unsafe { vcall!(self.as_ptr() => IGameEventManager2_Reset()) }
	}
}

/// Receives the game events a [`GameEventListener`] is registered for.
pub trait GameEventHandler {
	/// Called by the engine for each event the listener is registered for.
	///
	/// A panic cannot unwind into the engine and aborts the server, so catch
	/// any the handler may raise.
	fn fire_game_event(&self, event: GameEvent<'_>);
}

/// A Rust implementation of the engine's `IGameEventListener2`.
///
/// The manager keeps the address of a registered listener, so registering
/// takes it pinned. The listener is `!Unpin` so it cannot move while pinned,
/// and `!Send`/`!Sync` since the engine calls it on the server's main thread.
#[repr(C)]
pub struct GameEventListener<H> {
	vtable: &'static GameEventListenerVtable,
	handler: H,
	_pinned: PhantomPinned,
	_not_thread_safe: NotThreadSafe,
}

impl<H: GameEventHandler> GameEventListener<H> {
	const VTABLE: GameEventListenerVtable = GameEventListenerVtable {
		destructor: CppDestructors::new_noop(),
		fire_game_event: Self::fire_game_event,
	};

	pub const fn new(handler: H) -> Self {
		Self {
			vtable: &Self::VTABLE,
			handler,
			_pinned: PhantomPinned,
			_not_thread_safe: PhantomData,
		}
	}

	pub const fn handler(&self) -> &H {
		&self.handler
	}

	fn as_raw(self: Pin<&Self>) -> *mut sys::IGameEventListener2 {
		// The engine only reads the vtable pointer, never writing to the listener.
		ptr_from_pin(self).cast()
	}

	unsafe extern "C" fn fire_game_event(
		this: *mut sys::IGameEventListener2,
		event: *mut sys::IGameEvent,
	) {
		let Some(event) = NonNull::new(event) else {
			return;
		};

		// SAFETY: The engine only calls listeners registered by `add_listener`,
		// which are pinned instances of `Self` that outlive their registration.
		let listener = unsafe { &*this.cast::<Self>() };

		// SAFETY: The engine passes a live event for the duration of the call.
		listener
			.handler
			.fire_game_event(unsafe { GameEvent::from_raw(event) });
	}
}

impl<H: std::fmt::Debug> std::fmt::Debug for GameEventListener<H> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("GameEventListener")
			.field("handler", &self.handler)
			.finish_non_exhaustive()
	}
}

fn ptr_from_pin<T>(pinned: Pin<&T>) -> *mut T {
	(&raw const *pinned.get_ref()).cast_mut()
}

/// The `IGameEventListener2` vtable of a [`GameEventListener`].
#[repr(C)]
struct GameEventListenerVtable {
	destructor: CppDestructors,
	fire_game_event:
		unsafe extern "C" fn(this: *mut sys::IGameEventListener2, event: *mut sys::IGameEvent),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct GameEventDataKey(pub(super) CString);

impl GameEventDataKey {
	pub(super) unsafe fn from_ptr(ptr: *const c_char) -> Self {
		Self(unsafe { CStr::from_ptr(ptr) }.to_owned())
	}

	pub fn as_c_str(&self) -> &CStr {
		&self.0
	}

	pub fn as_str(&self) -> &str {
		let Ok(name) = self.0.to_str() else {
			unreachable!()
		};
		name
	}

	pub fn into_string(self) -> String {
		let Ok(string) = self.0.into_string() else {
			unreachable!()
		};
		string
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
#[repr(transparent)]
pub struct GameEventDataLocal(pub(super) NonNull<c_void>);

impl GameEventDataLocal {
	/// Pointer is guaranteed non-null.
	pub const fn as_ptr(self) -> *const c_void {
		self.0.as_ptr()
	}
}

#[derive(Debug, Clone)]
pub struct GameEventDataPair(pub GameEventDataKey, pub GameEventDataValue);

#[derive(Debug, Clone)]
pub enum GameEventDataValue {
	/// See [`GameEventDataLocal`].
	///
	/// `GameEventDataLocal` is never a null-ptr, [`Self::Null`] is used instead.
	Local(GameEventDataLocal),

	String(CString),

	Float(f32),

	Int(c_int),

	UInt64(u64),

	WString(Vec<WChar>),

	Bool(bool),

	/// Source supplied a null pointer for a pointer-valued event field.
	///
	/// The visitor ABI permits this, so null is represented explicitly instead
	/// of manufacturing a reference or panicking inside the callback.
	Null,
}

impl GameEventDataValue {
	/// Copies a string value from the pointer supplied by Source.
	///
	/// # Safety
	///
	/// A non-null `ptr` must point to a readable NUL-terminated string for the
	/// duration of this call.
	pub unsafe fn from_raw_string(ptr: *const c_char) -> Self {
		if ptr.is_null() {
			return Self::Null;
		}

		Self::String(unsafe { CStr::from_ptr(ptr) }.to_owned())
	}

	/// Copies a wide-string value from the pointer supplied by Source.
	///
	/// # Safety
	///
	/// A non-null `ptr` must be properly aligned and point to a readable,
	/// NUL-terminated sequence of [`WChar`] values for the duration of this call.
	/// The sequence length in bytes must not exceed `isize::MAX`.
	pub unsafe fn from_raw_wstring(ptr: *const WChar) -> Self {
		if ptr.is_null() {
			return Self::Null;
		}

		let mut len = 0;
		while unsafe { *ptr.add(len) } != 0 {
			len += 1;
		}

		Self::WString(unsafe { from_raw_parts(ptr, len) }.to_vec())
	}

	#[track_caller]
	pub fn unwrap_bool(self) -> bool {
		let Self::Bool(bool) = self else {
			panic!("Expected `Bool` found {self:?}");
		};

		bool
	}

	#[track_caller]
	pub fn unwrap_float(self) -> f32 {
		let Self::Float(float) = self else {
			panic!("Expected `Float` found {self:?}");
		};

		float
	}

	#[track_caller]
	pub fn unwrap_int(self) -> c_int {
		let Self::Int(int) = self else {
			panic!("Expected `Int` found {self:?}");
		};

		int
	}

	#[track_caller]
	pub fn unwrap_local(self) -> GameEventDataLocal {
		let Self::Local(local) = self else {
			panic!("Expected `Local` found {self:?}");
		};

		local
	}

	#[track_caller]
	pub fn unwrap_optional_local(self) -> Option<GameEventDataLocal> {
		match self {
			Self::Local(local) => Some(local),
			Self::Null => None,
			other => panic!("Expected `Local` found {other:?}"),
		}
	}

	#[track_caller]
	pub fn unwrap_optional_user_id(self) -> Option<UserId> {
		let Self::Int(int) = self else {
			panic!("Expected `Int` found {self:?}");
		};

		match UserId::from_raw(int) {
			Ok(user_id) => Some(user_id),
			Err(error) if error.is_sentinel() => None, //suppressed
			Err(error) => {
				panic!("Game event value was an integer {int}, but not a valid user ID {error}")
			}
		}
	}

	#[track_caller]
	pub fn unwrap_string(self) -> CString {
		let Self::String(string) = self else {
			panic!("Expected `String` found {self:?}");
		};

		string
	}

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

	#[track_caller]
	pub fn unwrap_wstring(self) -> Vec<WChar> {
		let Self::WString(wstring) = self else {
			panic!("Expected `WString` found {self:?}");
		};

		wstring
	}
}

/// Types that can consume an iterator of key-pairs emitted by the Source SDK's Game Event key-value iterator.
pub trait GameEventVisitor {
	type Break;

	fn visit(
		&mut self,
		key: GameEventDataKey,
		value: GameEventDataValue,
	) -> ControlFlow<Self::Break>;
}

#[repr(C)]
#[derive(Debug)]
#[non_exhaustive]
struct GameEventVistorVtable {
	visit_local:
		unsafe extern "C" fn(this: *mut c_void, name: *const c_char, value: *const c_void) -> bool,
	visit_string:
		unsafe extern "C" fn(this: *mut c_void, name: *const c_char, value: *const c_char) -> bool,
	visit_float:
		unsafe extern "C" fn(this: *mut c_void, name: *const c_char, value: c_float) -> bool,
	visit_int: unsafe extern "C" fn(this: *mut c_void, name: *const c_char, value: c_int) -> bool,
	visit_uint64: unsafe extern "C" fn(this: *mut c_void, name: *const c_char, value: u64) -> bool,
	visit_wstring:
		unsafe extern "C" fn(this: *mut c_void, name: *const c_char, value: *const WChar) -> bool,
	visit_bool: unsafe extern "C" fn(this: *mut c_void, name: *const c_char, value: bool) -> bool,
}

impl GameEventVistorVtable {
	/// Layout assertions.
	const _ASSERT_LAYOUT: () = {
		assert!(offset_of!(Self, visit_local) == 0);
		assert!(size_of::<Self>() == size_of::<*const ()>() * 7);
	};
}

macro_rules! visit_methods {
	($Method:ident) => {
		const VTABLE: GameEventVistorVtable = GameEventVistorVtable {
			visit_local: Self::visit_local,
			visit_string: Self::visit_string,
			visit_float: Self::visit_float,
			visit_int: Self::visit_int,
			visit_uint64: Self::visit_uint64,
			visit_wstring: Self::visit_wstring,
			visit_bool: Self::visit_bool,
		};

		unsafe extern "C" fn visit_local(
			this: *mut c_void,
			name: *const c_char,
			value: *const c_void,
		) -> bool {
			let value = NonNull::new(value.cast_mut()).map_or(GameEventDataValue::Null, |value| {
				GameEventDataValue::Local(GameEventDataLocal(value))
			});

			unsafe { Self::$Method(this, name, value) }
		}

		unsafe extern "C" fn visit_string(
			this: *mut c_void,
			name: *const c_char,
			value: *const c_char,
		) -> bool {
			unsafe { Self::$Method(this, name, GameEventDataValue::from_raw_string(value)) }
		}

		unsafe extern "C" fn visit_float(
			this: *mut c_void,
			name: *const c_char,
			value: c_float,
		) -> bool {
			unsafe { Self::$Method(this, name, GameEventDataValue::Float(value)) }
		}

		unsafe extern "C" fn visit_int(
			this: *mut c_void,
			name: *const c_char,
			value: c_int,
		) -> bool {
			unsafe { Self::$Method(this, name, GameEventDataValue::Int(value)) }
		}

		unsafe extern "C" fn visit_uint64(
			this: *mut c_void,
			name: *const c_char,
			value: u64,
		) -> bool {
			unsafe { Self::$Method(this, name, GameEventDataValue::UInt64(value)) }
		}

		unsafe extern "C" fn visit_wstring(
			this: *mut c_void,
			name: *const c_char,
			value: *const WChar,
		) -> bool {
			unsafe { Self::$Method(this, name, GameEventDataValue::from_raw_wstring(value)) }
		}

		unsafe extern "C" fn visit_bool(
			this: *mut c_void,
			name: *const c_char,
			value: bool,
		) -> bool {
			unsafe { Self::$Method(this, name, GameEventDataValue::Bool(value)) }
		}
	};
}

/// Structure required C++ implementation.
#[repr(C)]
struct VisitorExecutor<V: GameEventVisitor> {
	vtable: &'static GameEventVistorVtable,
	state: ControlFlow<V::Break, V>,
}

impl<V: GameEventVisitor> VisitorExecutor<V> {
	fn new(visitor: V) -> Self {
		Self {
			vtable: &Self::VTABLE,
			state: ControlFlow::Continue(visitor),
		}
	}

	unsafe fn visit(this: *mut c_void, name: *const c_char, value: GameEventDataValue) -> bool {
		assert!(!name.is_null());

		let exec = unsafe { this.cast::<Self>().as_mut() }.expect("Visitor object is null");

		let ControlFlow::Continue(visitor) = &mut exec.state else {
			panic!();
		};

		if let ControlFlow::Break(output) =
			visitor.visit(unsafe { GameEventDataKey::from_ptr(name) }, value)
		{
			exec.state = ControlFlow::Break(output);

			false
		} else {
			true
		}
	}

	visit_methods!(visit);
}

#[repr(C)]
struct VisitKeyFinder<'a> {
	vtable: &'static GameEventVistorVtable,
	state: ControlFlow<GameEventDataValue, &'a str>,
}

impl<'a> VisitKeyFinder<'a> {
	fn new(key: &'a str) -> Self {
		Self {
			vtable: &Self::VTABLE,
			state: ControlFlow::Continue(key),
		}
	}

	unsafe fn visit(this: *mut c_void, name: *const c_char, value: GameEventDataValue) -> bool {
		assert!(!name.is_null());

		let finder = unsafe { this.cast::<Self>().as_mut() }.expect("Visitor object is null");

		let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else {
			panic!("Invalid UTF-8 yielded IGameEvent KeyValues visitor {name:?}")
		};

		let ControlFlow::Continue(key) = finder.state else {
			unreachable!()
		};

		if key == name {
			finder.state = ControlFlow::Break(value);

			false
		} else {
			true
		}
	}

	visit_methods!(visit);
}

// IGameEvent declares a virtual destructor before GetName. It occupies one slot
// under Win64 MSVC and two slots under the Itanium ABI used on x86-64 Linux.
// Keep this assertion beside the safe wrapper so regenerating an incorrect sys
// binding cannot turn a GetName call into a destructive dispatch.
const _: () = {
	let slot_size = size_of::<*const ()>();
	let destructor_slots = if cfg!(target_os = "linux") { 2 } else { 1 };

	assert!(
		offset_of!(sys::IGameEvent__bindgen_vtable, IGameEvent_GetName)
			== slot_size * destructor_slots
	);
	assert!(
		offset_of!(sys::IGameEvent__bindgen_vtable, IGameEvent_IsReliable)
			== slot_size * (destructor_slots + 1)
	);
	assert!(
		offset_of!(sys::IGameEvent__bindgen_vtable, IGameEvent_IsLocal)
			== slot_size * (destructor_slots + 2)
	);
	assert!(
		offset_of!(
			sys::IGameEventManager2__bindgen_vtable,
			IGameEventManager2_LoadEventsFromFile
		) == slot_size * destructor_slots
	);
	assert!(
		offset_of!(
			sys::IGameEventManager2__bindgen_vtable,
			IGameEventManager2_AddListener
		) == slot_size * (destructor_slots + 2)
	);
	assert!(
		offset_of!(
			sys::IGameEventManager2__bindgen_vtable,
			IGameEventManager2_RemoveListener
		) == slot_size * (destructor_slots + 4)
	);

	// The Rust listener's vtable must line up with the engine's declaration.
	assert!(
		offset_of!(GameEventListenerVtable, fire_game_event)
			== offset_of!(
				sys::IGameEventListener2__bindgen_vtable,
				IGameEventListener2_FireGameEvent
			)
	);
	assert!(
		size_of::<GameEventListenerVtable>()
			== size_of::<sys::IGameEventListener2__bindgen_vtable>()
	);
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use std::cell::{Cell, RefCell};
	use std::ptr::null_mut;

	thread_local! {
		static RECEIVED_MANAGER: Cell<*mut sys::IGameEventManager2> = const { Cell::new(null_mut()) };
		static RECEIVED_LISTENER: Cell<*mut sys::IGameEventListener2> = const { Cell::new(null_mut()) };
		static RECEIVED_NAME: Cell<*const c_char> = const { Cell::new(std::ptr::null()) };
		static RECEIVED_SERVER_SIDE: Cell<bool> = const { Cell::new(false) };
		static REMOVE_CALLED: Cell<bool> = const { Cell::new(false) };
		static FREED: Cell<usize> = const { Cell::new(0) };
		static FIRED: Cell<usize> = const { Cell::new(0) };
		static DELIVERED: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
	}

	unsafe extern "C" fn record_add_listener(
		manager: *mut sys::IGameEventManager2,
		listener: *mut sys::IGameEventListener2,
		name: *const c_char,
		server_side: bool,
	) -> bool {
		RECEIVED_MANAGER.set(manager);
		RECEIVED_LISTENER.set(listener);
		RECEIVED_NAME.set(name);
		RECEIVED_SERVER_SIDE.set(server_side);
		let name = unsafe { CStr::from_ptr(name) };

		name == c"player_regenerate"
	}

	unsafe extern "C" fn record_remove_listener(
		manager: *mut sys::IGameEventManager2,
		listener: *mut sys::IGameEventListener2,
	) {
		RECEIVED_MANAGER.set(manager);
		RECEIVED_LISTENER.set(listener);
		REMOVE_CALLED.set(true);
	}

	unsafe extern "C" fn event_name(_: *const sys::IGameEvent) -> *const c_char {
		c"player_regenerate".as_ptr()
	}

	unsafe extern "C" fn event_is_empty(_: *mut sys::IGameEvent, key: *const c_char) -> bool {
		let key = unsafe { CStr::from_ptr(key) };

		key != c"userid"
	}

	unsafe extern "C" fn event_get_int(
		_: *const sys::IGameEvent,
		_: *const c_char,
		_: c_int,
	) -> c_int {
		7
	}

	unsafe extern "C" fn create_event(
		_: *mut sys::IGameEventManager2,
		name: *const c_char,
		force: bool,
	) -> *mut sys::IGameEvent {
		assert!(!force);
		if unsafe { CStr::from_ptr(name) } == c"player_regenerate" {
			EVENT.with(|event| event.as_ptr())
		} else {
			null_mut()
		}
	}

	unsafe extern "C" fn free_event(_: *mut sys::IGameEventManager2, _: *mut sys::IGameEvent) {
		FREED.set(FREED.get() + 1);
	}

	unsafe extern "C" fn fire_event(
		_: *mut sys::IGameEventManager2,
		event: *mut sys::IGameEvent,
		dont_broadcast: bool,
	) -> bool {
		assert!(dont_broadcast);
		FIRED.set(FIRED.get() + 1);
		// The engine delivers the event synchronously, then frees it.
		unsafe { deliver(event) };
		true
	}

	thread_local! {
		static EVENT_VTABLE: Box<sys::IGameEvent__bindgen_vtable> = unsafe {
			mock_vtable::<sys::IGameEvent__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).IGameEvent_GetName).write(event_name);
				(&raw mut (*vtable).IGameEvent_IsEmpty).write(event_is_empty);
				(&raw mut (*vtable).IGameEvent_GetInt).write(event_get_int);
			})
		};
		static EVENT: Cell<sys::IGameEvent> = Cell::new(sys::IGameEvent { vtable_: EVENT_VTABLE.with(|vtable| &raw const **vtable) });
		static LISTENER: Cell<*mut sys::IGameEventListener2> = const { Cell::new(null_mut()) };
	}

	unsafe fn deliver(event: *mut sys::IGameEvent) {
		let listener = LISTENER.get();
		let fire = unsafe { (*(*listener).vtable_).IGameEventListener2_FireGameEvent };

		unsafe { fire(listener, event) };
	}

	#[derive(Debug)]
	struct RecordingHandler;

	impl GameEventHandler for RecordingHandler {
		fn fire_game_event(&self, event: GameEvent<'_>) {
			assert_eq!(event.get_int(c"userid"), Some(7));
			assert_eq!(event.get_int(c"attacker"), None);
			DELIVERED.with_borrow_mut(|delivered| delivered.push(event.name().to_owned()));
		}
	}

	fn mock_manager() -> Box<sys::IGameEventManager2__bindgen_vtable> {
		unsafe {
			mock_vtable::<sys::IGameEventManager2__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IGameEventManager2_AddListener).write(record_add_listener);
					(&raw mut (*vtable).IGameEventManager2_RemoveListener)
						.write(record_remove_listener);
					(&raw mut (*vtable).IGameEventManager2_CreateEvent).write(create_event);
					(&raw mut (*vtable).IGameEventManager2_FreeEvent).write(free_event);
					(&raw mut (*vtable).IGameEventManager2_FireEvent).write(fire_event);
				},
			)
		}
	}

	#[test]
	fn listeners_register_by_address_and_receive_fired_events() {
		let vtable = mock_manager();
		let mut interface = sys::IGameEventManager2 {
			vtable_: &raw const *vtable,
		};
		let interface_pointer = &raw mut interface;
		let manager =
			unsafe { GameEventManager::from_raw(NonNull::new(interface_pointer).unwrap()) };
		let listener = std::pin::pin!(GameEventListener::new(RecordingHandler));
		let listener = listener.into_ref();
		let listener_pointer = listener.as_raw();

		LISTENER.set(listener_pointer);

		assert!(unsafe { manager.add_listener(listener, c"player_regenerate", true) }.is_ok());
		assert_eq!(RECEIVED_MANAGER.get(), interface_pointer);
		assert_eq!(RECEIVED_LISTENER.get(), listener_pointer);
		assert!(RECEIVED_SERVER_SIDE.get());
		assert_eq!(
			unsafe { manager.add_listener(listener, c"not_an_event", true) }
				.unwrap_err()
				.to_string(),
			"could not listen for `not_an_event`; no such game event is registered"
		);

		// Dropping an unfired event frees it; firing hands it to the engine.
		drop(manager.create_event(c"player_regenerate").unwrap());
		assert_eq!(FREED.get(), 1);
		assert!(manager.create_event(c"not_an_event").is_err());

		let event = manager.create_event(c"player_regenerate").unwrap();
		assert_eq!(event.as_event().name(), c"player_regenerate");
		event.fire(false).unwrap();

		assert_eq!((FIRED.get(), FREED.get()), (1, 1));
		assert_eq!(DELIVERED.take(), [c"player_regenerate".to_owned()]);

		manager.remove_listener(listener);
		assert!(REMOVE_CALLED.get());
		assert_eq!(RECEIVED_LISTENER.get(), listener_pointer);
	}

	#[test]
	fn int_values_unwrap_as_user_ids() {
		assert_eq!(
			GameEventDataValue::Int(7).unwrap_user_id(),
			UserId::new(7).unwrap()
		);
		assert_eq!(
			GameEventDataValue::Int(7).unwrap_optional_user_id(),
			UserId::new(7)
		);
		assert_eq!(GameEventDataValue::Int(0).unwrap_optional_user_id(), None);
		assert_eq!(GameEventDataValue::Int(-1).unwrap_optional_user_id(), None);
	}

	#[test]
	#[should_panic(expected = "Invalid UserId representation: 0")]
	fn unwrapping_the_no_player_zero_as_a_user_id_panics() {
		GameEventDataValue::Int(0).unwrap_user_id();
	}
}
