//! Hand-written ABI of the engine's game event listener and visitor objects.
//!
//! [`GameEventListenerObject`] is an `IGameEventListener2` implemented in Rust,
//! which the game event manager calls with the events it fires, and
//! [`for_event_data`] walks an event's data with an `IGameEventVisitor2`
//! implemented in Rust. [`VERSION`] is the version string of the game event
//! manager, `IGameEventManager2`.

use crate::abi::{CppDestructors, VTABLE_SLOT_SIZE, WChar};
use crate::util::cstr::{borrow_cstr, borrow_wide_cstr};
use crate::{vcall, vtable_slot};
use std::ffi::{CStr, c_char, c_float, c_int, c_void};
use std::fmt::{Debug, Formatter};
use std::mem::offset_of;
use std::ptr::NonNull;

/// `void IGameEventListener2::FireGameEvent(IGameEvent *event)`.
type FireGameEventFn =
	unsafe extern "C" fn(this: *mut sys::IGameEventListener2, event: *mut sys::IGameEvent);

// The listener's hand-written vtable lines up with the generated one under the
// target's ABI, whose destructor slots `CppDestructors` selects.
const _: () = {
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

// `IGameEventVisitor2` declares no destructor, so its seven methods fill the
// vtable from its start under both ABIs, and `VISITOR_VTABLE` is built as the
// generated vtable itself.
const _: () = {
	assert!(
		offset_of!(
			sys::IGameEventVisitor2__bindgen_vtable,
			IGameEventVisitor2_VisitLocal
		) == 0
	);
	assert!(size_of::<sys::IGameEventVisitor2__bindgen_vtable>() == VTABLE_SLOT_SIZE * 7);
};

// `IGameEvent` and `IGameEventManager2` declare a virtual destructor first,
// which occupies one slot under MSVC and two under the Itanium ABI. The slots
// of the methods after it are checked against the generated vtables, so a
// regenerated binding cannot turn a call into a destructive dispatch.
const _: () = {
	let destructor = CppDestructors::VTABLE_SLOTS;

	assert!(vtable_slot!(sys::IGameEvent__bindgen_vtable, IGameEvent_GetName) == destructor);
	assert!(vtable_slot!(sys::IGameEvent__bindgen_vtable, IGameEvent_IsReliable) == destructor + 1);
	assert!(vtable_slot!(sys::IGameEvent__bindgen_vtable, IGameEvent_IsLocal) == destructor + 2);
	assert!(
		vtable_slot!(
			sys::IGameEventManager2__bindgen_vtable,
			IGameEventManager2_LoadEventsFromFile
		) == destructor
	);
	assert!(
		vtable_slot!(
			sys::IGameEventManager2__bindgen_vtable,
			IGameEventManager2_AddListener
		) == destructor + 2
	);
	assert!(
		vtable_slot!(
			sys::IGameEventManager2__bindgen_vtable,
			IGameEventManager2_RemoveListener
		) == destructor + 4
	);
};

// The generated binding has this signature.
const _: fn(&sys::IGameEventListener2__bindgen_vtable) -> FireGameEventFn =
	|vtable| vtable.IGameEventListener2_FireGameEvent;

/// The most bytes the engine serializes of one event.
///
/// This is `MAX_EVENT_BYTES` from `public/igameevents.h`.
pub const MAX_EVENT_BYTES: usize = 1024;

/// The version string `IGameEventManager2` is exported and requested under.
///
/// This is `INTERFACEVERSION_GAMEEVENTSMANAGER2` from `public/igameevents.h`.
#[doc(alias("INTERFACEVERSION_GAMEEVENTSMANAGER2"))]
pub const VERSION: &CStr = c"GAMEEVENTSMANAGER002";

/// The vtable of every [`EventVisitor`], whose methods pass what the engine
/// visits to the visitor's callback.
static VISITOR_VTABLE: sys::IGameEventVisitor2__bindgen_vtable =
	sys::IGameEventVisitor2__bindgen_vtable {
		IGameEventVisitor2_VisitLocal: visit_local,
		IGameEventVisitor2_VisitString: visit_string,
		IGameEventVisitor2_VisitFloat: visit_float,
		IGameEventVisitor2_VisitInt: visit_int,
		IGameEventVisitor2_VisitUint64: visit_uint64,
		IGameEventVisitor2_VisitWString: visit_wstring,
		IGameEventVisitor2_VisitBool: visit_bool,
	};

/// A value of an event's data, as an `IGameEventVisitor2` method delivered
/// it.
///
/// Strings borrow the event's data, so they are only valid during the visit
/// that delivered them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventValue<'a> {
	/// From `VisitLocal`: an opaque pointer, or `None` for null.
	///
	/// The pointee's type and lifetime are not part of the visitor's
	/// contract.
	Local(Option<NonNull<c_void>>),

	/// From `VisitString`, or `None` for null.
	String(Option<&'a CStr>),

	/// From `VisitFloat`.
	Float(c_float),

	/// From `VisitInt`.
	Int(c_int),

	/// From `VisitUint64`.
	UInt64(u64),

	/// From `VisitWString`, without its terminator, or `None` for null.
	WString(Option<&'a [WChar]>),

	/// From `VisitBool`.
	Bool(bool),
}

/// The `IGameEventVisitor2` [`for_event_data`] lends the engine.
#[repr(C)]
struct EventVisitor<'a> {
	interface: sys::IGameEventVisitor2,
	visit: &'a mut dyn FnMut(&CStr, EventValue<'_>) -> bool,
}

/// An `IGameEventListener2` implemented in Rust, which passes the events the
/// engine fires it to an [`OnFireGameEvent`].
///
/// `IGameEventManager2::AddListener` keeps the address it is given, from
/// [`Self::as_raw`], and the manager calls the listener through it until
/// `RemoveListener` removes it. A registered listener must therefore stay
/// live, at the same address, and the module containing its code must stay
/// loaded, until it is removed. The manager only reads the listener, and never
/// deletes it, so its destructor slots do nothing.
#[doc(alias("IGameEventListener2"))]
#[repr(C)]
pub struct GameEventListenerObject<T> {
	vtable: &'static GameEventListenerVtable,
	inner: T,
}

impl<T: OnFireGameEvent> GameEventListenerObject<T> {
	const VTABLE: GameEventListenerVtable = GameEventListenerVtable {
		destructor: CppDestructors::new_noop(),
		fire_game_event: Self::fire_game_event,
	};

	/// A listener that passes the events it is fired to `inner`.
	pub const fn new(inner: T) -> Self {
		Self {
			vtable: &Self::VTABLE,
			inner,
		}
	}

	/// `FireGameEvent`, which passes non-null events to the listener's
	/// [`OnFireGameEvent`].
	unsafe extern "C" fn fire_game_event(
		this: *mut sys::IGameEventListener2,
		event: *mut sys::IGameEvent,
	) {
		let Some(event) = NonNull::new(event) else {
			return;
		};

		// SAFETY: The manager only calls the listeners registered with it,
		// which stay live until removed, as `GameEventListenerObject` requires,
		// and only reads them.
		let listener = unsafe { &*this.cast::<Self>() };

		// SAFETY: The manager fires a live event for the duration of the call,
		// on the server's main thread.
		unsafe { listener.inner.fire_game_event(event) };
	}
}

impl<T> GameEventListenerObject<T> {
	/// The address the manager registers and calls the listener by.
	pub const fn as_raw(&self) -> *mut sys::IGameEventListener2 {
		(&raw const *self).cast_mut().cast()
	}

	/// The value the listener passes events to.
	pub const fn inner(&self) -> &T {
		&self.inner
	}
}

impl<T: Debug> Debug for GameEventListenerObject<T> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("GameEventListenerObject")
			.field("inner", &self.inner)
			.finish_non_exhaustive()
	}
}

/// The `IGameEventListener2` vtable of a [`GameEventListenerObject`].
#[repr(C)]
struct GameEventListenerVtable {
	destructor: CppDestructors,
	fire_game_event: FireGameEventFn,
}

/// Receives the events the engine fires a [`GameEventListenerObject`].
pub trait OnFireGameEvent {
	/// Called for each event the engine fires the listener, as the listener's
	/// `FireGameEvent`.
	///
	/// The call comes from C++, so a panic cannot unwind out of it and aborts
	/// the process.
	///
	/// # Safety
	///
	/// `event` must point to a live `IGameEvent`, which stays live for the
	/// duration of the call, and the call must be made on the thread the
	/// engine fires events on, the server's main thread.
	#[doc(alias("FireGameEvent"))]
	unsafe fn fire_game_event(&self, event: NonNull<sys::IGameEvent>);
}

/// Runs `IGameEvent::ForEventData`, which passes the name and value of each
/// key of the event's data to `visit`, in order, until `visit` returns
/// `false`, and returns the engine's result.
///
/// The name and value only borrow the event for the call of `visit` they are
/// passed to. `visit` is called from C++, so a panic cannot unwind out of it
/// and aborts the process, as does the engine passing a null name.
///
/// # Safety
///
/// `event` must point to a live `IGameEvent`, which stays live with its data
/// unchanged until the call returns, including while `visit` runs, and the
/// call must be made on the server's main thread.
#[doc(alias("ForEventData"))]
pub unsafe fn for_event_data(
	event: NonNull<sys::IGameEvent>,
	visit: &mut dyn FnMut(&CStr, EventValue<'_>) -> bool,
) -> bool {
	let mut visitor = EventVisitor {
		interface: sys::IGameEventVisitor2 {
			vtable_: &raw const VISITOR_VTABLE,
		},
		visit,
	};

	// SAFETY: The caller guarantees a live event, on the main thread. The
	// visitor outlives the call, which is the only time the engine uses it,
	// and nothing else uses it meanwhile.
	unsafe { vcall!(event.as_ptr() => IGameEvent_ForEventData((&raw mut visitor).cast())) }
}

/// Passes a key's name and value to the [`EventVisitor`] at `this`, returning
/// whether the engine should visit the next key.
///
/// # Panics
///
/// If `this` or `name` is null. The engine calls the visitor through C++, so
/// the panic aborts the process.
///
/// # Safety
///
/// `this` must be null or point to an [`EventVisitor`] that nothing else uses
/// during the call, and a non-null `name` must point to a NUL-terminated
/// string that stays unchanged during the call.
unsafe fn visit(
	this: *mut sys::IGameEventVisitor2,
	name: *const c_char,
	value: EventValue<'_>,
) -> bool {
	// SAFETY: The caller guarantees that a non-null name is terminated and
	// unchanged during the call.
	let name = unsafe { borrow_cstr(name) }.expect("IGameEventVisitor2 was passed a null name");

	// SAFETY: The caller guarantees that `this` is null or an unaliased
	// `EventVisitor`.
	let visitor =
		unsafe { this.cast::<EventVisitor<'_>>().as_mut() }.expect("Visitor object is null");

	(visitor.visit)(name, value)
}

/// `VisitBool`.
unsafe extern "C" fn visit_bool(
	this: *mut sys::IGameEventVisitor2,
	name: *const c_char,
	value: bool,
) -> bool {
	// SAFETY: The engine calls the visitor `for_event_data` lent it, during
	// `ForEventData`, with the key's terminated name.
	unsafe { visit(this, name, EventValue::Bool(value)) }
}

/// `VisitFloat`.
unsafe extern "C" fn visit_float(
	this: *mut sys::IGameEventVisitor2,
	name: *const c_char,
	value: c_float,
) -> bool {
	// SAFETY: As for `visit_bool`.
	unsafe { visit(this, name, EventValue::Float(value)) }
}

/// `VisitInt`.
unsafe extern "C" fn visit_int(
	this: *mut sys::IGameEventVisitor2,
	name: *const c_char,
	value: c_int,
) -> bool {
	// SAFETY: As for `visit_bool`.
	unsafe { visit(this, name, EventValue::Int(value)) }
}

/// `VisitLocal`.
unsafe extern "C" fn visit_local(
	this: *mut sys::IGameEventVisitor2,
	name: *const c_char,
	value: *const c_void,
) -> bool {
	// SAFETY: As for `visit_bool`.
	unsafe {
		visit(
			this,
			name,
			EventValue::Local(NonNull::new(value.cast_mut())),
		)
	}
}

/// `VisitString`.
unsafe extern "C" fn visit_string(
	this: *mut sys::IGameEventVisitor2,
	name: *const c_char,
	value: *const c_char,
) -> bool {
	// SAFETY: As for `visit_bool`, and the value is null or a terminated
	// string of the event's, unchanged during the call.
	unsafe { visit(this, name, EventValue::String(borrow_cstr(value))) }
}

/// `VisitUint64`.
unsafe extern "C" fn visit_uint64(
	this: *mut sys::IGameEventVisitor2,
	name: *const c_char,
	value: u64,
) -> bool {
	// SAFETY: As for `visit_bool`.
	unsafe { visit(this, name, EventValue::UInt64(value)) }
}

/// `VisitWString`.
unsafe extern "C" fn visit_wstring(
	this: *mut sys::IGameEventVisitor2,
	name: *const c_char,
	value: *const WChar,
) -> bool {
	// SAFETY: As for `visit_bool`, and the value is null or an aligned,
	// terminated wide string of the event's, unchanged during the call.
	unsafe { visit(this, name, EventValue::WString(borrow_wide_cstr(value))) }
}
