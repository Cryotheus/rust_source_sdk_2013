//! Hand-written glue for replacing the send proxies of the game's networked
//! variables: trampolines that the engine calls in place of a property's
//! proxy (`m_ProxyFn`), which call the property's own proxy and then a handler
//! that may change what it sent, and the table of the properties they
//! replaced the proxies of.
//!
//! The engine calls a property's proxy each time it encodes the variable for a
//! snapshot: on the main thread, or, while `sv_parallel_packentities` is on,
//! on the worker threads it packs entities on while the main thread waits for
//! them. Handlers must therefore be [`Send`] and [`Sync`], and the table is
//! read with atomics. Proxies are installed and restored on the main thread,
//! which never runs while the engine packs entities.
//!
//! Each loaded copy of this crate has its own table, of
//! [`MAX_SEND_PROXY_OVERRIDES`] slots, and its own trampolines: another
//! plugin replacing the same property's proxy sees one of this crate's
//! trampolines as the property's proxy, as this crate sees another's.

#[cfg(test)]
#[path = "tests/send_proxies.rs"]
mod tests;

use std::any::Any;
use std::ffi::{c_int, c_void};
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

/// An array of `function::<0>` up to `function::<63>`, one per slot.
#[rustfmt::skip]
macro_rules! trampolines {
	($function:ident) => {
		[
			$function::<0>, $function::<1>, $function::<2>, $function::<3>,
			$function::<4>, $function::<5>, $function::<6>, $function::<7>,
			$function::<8>, $function::<9>, $function::<10>, $function::<11>,
			$function::<12>, $function::<13>, $function::<14>, $function::<15>,
			$function::<16>, $function::<17>, $function::<18>, $function::<19>,
			$function::<20>, $function::<21>, $function::<22>, $function::<23>,
			$function::<24>, $function::<25>, $function::<26>, $function::<27>,
			$function::<28>, $function::<29>, $function::<30>, $function::<31>,
			$function::<32>, $function::<33>, $function::<34>, $function::<35>,
			$function::<36>, $function::<37>, $function::<38>, $function::<39>,
			$function::<40>, $function::<41>, $function::<42>, $function::<43>,
			$function::<44>, $function::<45>, $function::<46>, $function::<47>,
			$function::<48>, $function::<49>, $function::<50>, $function::<51>,
			$function::<52>, $function::<53>, $function::<54>, $function::<55>,
			$function::<56>, $function::<57>, $function::<58>, $function::<59>,
			$function::<60>, $function::<61>, $function::<62>, $function::<63>,
		]
	};
}

/// A send proxy, as `SendVarProxyFn` holds it, which the trampolines are.
type ProxyFn = unsafe extern "C" fn(
	prop: *const sys::SendProp,
	base: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	element: c_int,
	object_id: c_int,
);

/// What runs after a property's own send proxy, through one of this crate's
/// trampolines, on whichever thread the engine encodes the variable on.
///
/// A panic is caught, and leaves what the property's own proxy sent.
pub type ProxyHandler = dyn Fn(ProxyCall) + Send + Sync;

/// How many properties can have their send proxies replaced at once by one
/// loaded copy of this crate.
pub const MAX_SEND_PROXY_OVERRIDES: usize = 64;

/// The slots of the table, one per trampoline.
static SLOTS: [Slot; MAX_SEND_PROXY_OVERRIDES] = [const { Slot::new() }; MAX_SEND_PROXY_OVERRIDES];

/// `trampoline::<0>` up to `trampoline::<63>`, the proxy of each slot.
static TRAMPOLINES: [ProxyFn; MAX_SEND_PROXY_OVERRIDES] = trampolines!(trampoline);

/// Why a property's send proxy could not be replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum InstallError {
	/// The property has no send proxy to call first.
	#[error("the property has no send proxy")]
	NoProxy,

	/// This copy of the crate already replaced the property's proxy.
	#[error("the property's send proxy is already overridden")]
	AlreadyOverridden,

	/// Every slot of the table is taken.
	#[error("{MAX_SEND_PROXY_OVERRIDES} send proxies are already overridden")]
	Full,
}

/// One call of a replaced send proxy, after the property's own proxy wrote
/// what it sends to [`out`](Self::out).
#[derive(Debug, Clone, Copy)]
pub struct ProxyCall {
	/// The property whose variable is encoded.
	pub prop: *const sys::SendProp,

	/// The value the engine encodes, which the property's own proxy wrote, as
	/// the member of the `DVariant` that matches the property's type.
	pub out: *mut sys::DVariant,

	/// The variable's index in its array, or 0.
	pub element: c_int,

	/// The edict index of the entity whose variable is encoded.
	pub object_id: c_int,
}

/// What [`restore`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Restored {
	/// The property has its own proxy back.
	Restored,

	/// Another proxy has replaced the slot's trampoline since, and may call it
	/// still, so the trampoline stays in place, calling the property's own
	/// proxy and no handler, and the slot is never reused.
	Retired,

	/// The slot replaced no proxy.
	Free,
}

/// A slot of the table: a property whose proxy its trampoline replaced, the
/// property's own proxy, and the handler the trampoline runs after it.
struct Slot {
	/// The property whose proxy the slot's trampoline replaced, or null while
	/// the slot is free.
	prop: AtomicPtr<sys::SendProp>,

	/// The property's own proxy, which the trampoline calls first, or null.
	original: AtomicPtr<c_void>,

	/// The handler the trampoline runs after the property's own proxy, or null
	/// for none. A leaked `Box<Box<ProxyHandler>>`, freed when restored.
	handler: AtomicPtr<Box<ProxyHandler>>,

	/// Whether the slot was [retired](Restored::Retired).
	retired: AtomicBool,

	/// How many times the slot was taken.
	generation: AtomicUsize,
}

impl Slot {
	const fn new() -> Self {
		Self {
			prop: AtomicPtr::new(ptr::null_mut()),
			original: AtomicPtr::new(ptr::null_mut()),
			handler: AtomicPtr::new(ptr::null_mut()),
			retired: AtomicBool::new(false),
			generation: AtomicUsize::new(0),
		}
	}

	/// Whether the slot holds `id`'s override, which is neither restored nor
	/// retired.
	fn holds(&self, id: SlotId) -> bool {
		!self.prop.load(Ordering::Acquire).is_null()
			&& !self.retired.load(Ordering::Acquire)
			&& self.generation.load(Ordering::Acquire) == id.generation
	}

	/// The property's own proxy, which the slot's trampoline calls first.
	fn original(&self) -> Option<ProxyFn> {
		let original = self.original.load(Ordering::Acquire);

		// SAFETY: `install` stored the address of a proxy, which is a function
		// of this type, or nothing.
		(!original.is_null()).then(|| unsafe { mem::transmute::<*mut c_void, ProxyFn>(original) })
	}

	/// Frees the slot's handler, if it has one.
	///
	/// # Safety
	///
	/// No trampoline may run the handler during the call, and it must be made
	/// on the main thread.
	unsafe fn take_handler(&self) {
		let handler = self.handler.swap(ptr::null_mut(), Ordering::AcqRel);

		if !handler.is_null() {
			// SAFETY: `install` leaked the box, and the swap took it out of the
			// slot, so it is dropped once. No trampoline runs it, as the caller
			// promises.
			let handler = unsafe { Box::from_raw(handler) };

			if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(handler))) {
				drop_payload(payload);
			}
		}
	}
}

/// A slot [`install`] took, for [`restore`]. A slot restored and taken again
/// has another ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId {
	index: usize,
	generation: usize,
}

impl SlotId {
	/// The slot's index in the table, below [`MAX_SEND_PROXY_OVERRIDES`].
	pub const fn index(self) -> usize {
		self.index
	}
}

/// Drops a panic's payload, whose own drop may panic too.
fn drop_payload(payload: Box<dyn Any + Send>) {
	if let Err(nested) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
		mem::forget(nested);
	}
}

/// The send proxy the game gave `prop`, looking through the trampolines of
/// this copy of the crate: the property's own proxy where one of them replaced
/// it, or the property's proxy otherwise, which may be another plugin's.
///
/// # Safety
///
/// `prop` must point to a live `SendProp`.
#[doc(alias("m_ProxyFn"))]
pub unsafe fn game_proxy(prop: *const sys::SendProp) -> sys::SendVarProxyFn {
	// SAFETY: The property is live, and its field is read without forming a
	// reference.
	let proxy = unsafe { (&raw const (*prop).m_ProxyFn).read() }?;

	match slot_of_trampoline(proxy) {
		Some(slot) => SLOTS[slot].original(),
		None => Some(proxy),
	}
}

/// Replaces `prop`'s send proxy with a trampoline that calls the property's
/// own proxy, then `handler`, and returns the slot it took, for [`restore`].
///
/// # Safety
///
/// - `prop` must point to a live `SendProp` of the loaded game DLL, whose
///   proxy the engine calls with a value of the property's type, and which
///   stays allocated until the slot is restored, or the trampoline would keep
///   being called.
/// - The call must be made on the server's main thread, which never runs while
///   the engine packs entities on other threads.
/// - `handler` must only write to [`ProxyCall::out`] a value of the
///   property's type, which the engine can encode, such as a string that lives
///   until the encoding is done.
/// - The slot must be restored before this library unloads, as the engine
///   would otherwise call into it, such as with [`restore_all`].
pub unsafe fn install(
	prop: NonNull<sys::SendProp>,
	handler: Box<ProxyHandler>,
) -> Result<SlotId, InstallError> {
	let prop = prop.as_ptr();

	// SAFETY: As the caller promises, the property is live, and its field is
	// read without forming a reference.
	let current = unsafe { (&raw const (*prop).m_ProxyFn).read() }.ok_or(InstallError::NoProxy)?;

	if SLOTS.iter().any(|slot| {
		slot.prop.load(Ordering::Acquire) == prop && !slot.retired.load(Ordering::Acquire)
	}) {
		return Err(InstallError::AlreadyOverridden);
	}

	// A free slot is claimed by swapping its property in, so that two calls
	// never take the same slot, should they race despite the rules above.
	let (index, slot) = SLOTS
		.iter()
		.enumerate()
		.find(|(_, slot)| {
			slot.prop
				.compare_exchange(ptr::null_mut(), prop, Ordering::AcqRel, Ordering::Acquire)
				.is_ok()
		})
		.ok_or(InstallError::Full)?;

	let generation = slot.generation.fetch_add(1, Ordering::AcqRel) + 1;

	slot.original
		.store(current as *mut c_void, Ordering::Release);
	slot.handler
		.store(Box::into_raw(Box::new(handler)), Ordering::Release);

	// SAFETY: As the caller promises, the property is live and the engine
	// packs no entity on another thread meanwhile. The field is written
	// without forming a reference, as the engine reads it through its own
	// pointers.
	unsafe { (&raw mut (*prop).m_ProxyFn).write(Some(TRAMPOLINES[index])) };

	Ok(SlotId { index, generation })
}

/// Whether the slot `id` names replaced a property's proxy and has not been
/// restored or retired since.
pub fn is_installed(id: SlotId) -> bool {
	SLOTS.get(id.index).is_some_and(|slot| slot.holds(id))
}

/// Puts back the proxy [`install`] replaced in the slot `id` names, and frees
/// its handler. Does nothing to a slot restored since, even if taken again.
///
/// When another proxy has replaced the slot's trampoline since, that proxy
/// may call the trampoline still, so the slot is [retired](Restored::Retired)
/// instead.
///
/// # Safety
///
/// The call must be made on the server's main thread, which never runs while
/// the engine packs entities on other threads, and the property whose proxy
/// the slot replaced must still be allocated.
pub unsafe fn restore(id: SlotId) -> Restored {
	let Some(entry) = SLOTS.get(id.index).filter(|entry| entry.holds(id)) else {
		return Restored::Free;
	};

	let prop = entry.prop.load(Ordering::Acquire);

	// SAFETY: As the caller promises, no trampoline runs meanwhile.
	unsafe { entry.take_handler() };

	// SAFETY: As the caller promises, the property is still allocated, and the
	// engine reads its field through its own pointers.
	let proxy = unsafe { &raw mut (*prop).m_ProxyFn };

	// SAFETY: As above.
	let current = unsafe { proxy.read() };

	if current.map(|current| current as usize) != Some(TRAMPOLINES[id.index] as usize) {
		entry.retired.store(true, Ordering::Release);
		return Restored::Retired;
	}

	// SAFETY: As above.
	unsafe { proxy.write(entry.original()) };

	entry.original.store(ptr::null_mut(), Ordering::Release);
	entry.prop.store(ptr::null_mut(), Ordering::Release);

	Restored::Restored
}

/// [Restores](restore) every slot, as a plugin must before its library
/// unloads.
///
/// # Safety
///
/// As for [`restore`], for every property whose proxy is replaced.
pub unsafe fn restore_all() {
	for (index, slot) in SLOTS.iter().enumerate() {
		let id = SlotId {
			index,
			generation: slot.generation.load(Ordering::Acquire),
		};

		// SAFETY: As the caller promises.
		unsafe { restore(id) };
	}
}

/// The slot whose trampoline `proxy` is, if it is one of this crate's.
fn slot_of_trampoline(proxy: ProxyFn) -> Option<usize> {
	TRAMPOLINES
		.iter()
		.position(|&trampoline| trampoline as usize == proxy as usize)
}

/// The send proxy of the property in slot `SLOT`: the property's own proxy,
/// then the slot's handler, if any.
///
/// The engine may call it on its packing threads, so it only reads atomics,
/// and never unwinds.
unsafe extern "C" fn trampoline<const SLOT: usize>(
	prop: *const sys::SendProp,
	base: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	element: c_int,
	object_id: c_int,
) {
	let slot = &SLOTS[SLOT];

	let Some(original) = slot.original() else {
		return;
	};

	// SAFETY: The engine called this trampoline with the arguments for the
	// property's own proxy, which `install` stored.
	unsafe { original(prop, base, data, out, element, object_id) };

	let handler = slot.handler.load(Ordering::Acquire);

	// SAFETY: A handler stays allocated until restored, which only happens on
	// the main thread while no trampoline runs.
	let Some(handler) = (unsafe { handler.as_ref() }) else {
		return;
	};

	let call = ProxyCall {
		prop,
		out,
		element,
		object_id,
	};

	if let Err(payload) = catch_unwind(AssertUnwindSafe(|| handler(call))) {
		drop_payload(payload);
	}
}
