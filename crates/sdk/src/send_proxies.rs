//! Overriding what clients receive of networked variables, by replacing the
//! send proxies the engine encodes them through.
//!
//! The engine encodes each networked variable through its property's send
//! proxy, which reads the variable and gives the value clients receive. A
//! [`SendProxyOverride`] replaces a property's proxy with one that calls the
//! game's, then a callback that may send another value in its place, while the
//! server keeps the variable as the game set it. The game's own logic, bots
//! and other plugins go by the variable, and only clients see the override.
//!
//! # Threads
//!
//! The engine calls proxies on its main thread or, while
//! `sv_parallel_packentities` is on, on the worker threads it packs entities
//! on while the main thread waits for them. Callbacks are therefore
//! [`Send`] and [`Sync`], which keeps them from holding a [`Server`] or
//! anything scoped to one: they read what they need from atomics, such as an
//! [`EntityValues`] table, which the plugin writes on the main thread.
//!
//! # Every client receives the same value
//!
//! The engine encodes a changed entity once per snapshot, for every client, so
//! a callback cannot tell which client it sends to. To hide an entity from
//! some clients, hook its `SetTransmit` instead (`metamod_source`'s
//! `transmit_hooks`).
//!
//! # Sending anew
//!
//! The engine encodes an entity only when the game records a change of one of
//! its variables, and keeps sending what it encoded last otherwise. When what
//! a callback sends changes while the variable does not, record a change of
//! the entity, such as with [`EntityValues::set_and_resend`],
//! [`Edict::full_state_changed`], or [`SendProxyOverride::resend_all`].
//!
//! # Restoring
//!
//! Dropping an override, or [removing](SendProxyOverride::remove) it, puts the
//! game's proxy back. The engine would call a proxy left in place after the
//! plugin's library unloads, so a plugin drops its overrides before it
//! unloads, or calls [`restore_all`]. Another plugin may have replaced the
//! override's proxy since, keeping it to call: the override's proxy then stays
//! in place, sending what the game's sends, until that plugin puts it back.
//!
//! While overridden, [`NetProp`](crate::datatables::NetProp) still tells how
//! the variable is stored by the game's proxy, and reads and writes it as
//! stored, as it does under the overrides of other plugins built on these
//! crates, of any version that exports
//! [`TRAMPOLINE_ORIGINAL_SYMBOL`](sdk_raw::send_proxies::TRAMPOLINE_ORIGINAL_SYMBOL).
//! [`NetProp::value`](crate::datatables::NetProp::value) reads what clients
//! receive, overrides included.

#[cfg(test)]
#[path = "tests/send_proxies.rs"]
mod tests;

use crate::NotThreadSafe;
use crate::Server;
use crate::datatables::{PropKind, SendProp, SendTable};
use crate::edicts::{Edict, MAX_EDICTS};
use crate::interfaces::ValveEngine;
use crate::math::Vector;
use glam::Vec2;
use sdk_raw::send_proxies::{self as raw, InstallError, ProxyCall, SlotId};
use std::ffi::{CStr, c_char, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

pub use sdk_raw::send_proxies::MAX_SEND_PROXY_OVERRIDES;

/// A value [`EntityValues`] can hold.
pub trait EntityValue: sealed::Sealed + Copy + Send + Sync + 'static {
	/// The value of `bits` from [`Self::to_bits`].
	#[doc(hidden)]
	fn from_bits(bits: u32) -> Self;

	/// The value's bits.
	#[doc(hidden)]
	fn to_bits(self) -> u32;
}

impl EntityValue for bool {
	fn from_bits(bits: u32) -> Self {
		bits != 0
	}

	fn to_bits(self) -> u32 {
		u32::from(self)
	}
}

impl EntityValue for f32 {
	fn from_bits(bits: u32) -> Self {
		Self::from_bits(bits)
	}

	fn to_bits(self) -> u32 {
		self.to_bits()
	}
}

impl EntityValue for i32 {
	fn from_bits(bits: u32) -> Self {
		bits.cast_signed()
	}

	fn to_bits(self) -> u32 {
		self.cast_unsigned()
	}
}

impl EntityValue for u32 {
	fn from_bits(bits: u32) -> Self {
		bits
	}

	fn to_bits(self) -> u32 {
		self
	}
}

/// A value, or none, for each slot of the edict table, which a
/// [`SendProxyOverride`]'s callback can read on any thread, such as the values
/// it sends in place of the game's for some entities, or which entities it
/// overrides.
///
/// Values are read and written with atomics, so one table can be a `static`
/// that the callback reads and the plugin writes on the main thread. A slot
/// keeps its value when its entity is removed, so clear it then, or as the
/// level ends.
#[derive(Debug)]
pub struct EntityValues<T: EntityValue> {
	/// Each slot's value in the low 32 bits, with [`Self::SET`] while it has
	/// one.
	slots: [AtomicU64; MAX_EDICTS as usize],
	_value: PhantomData<T>,
}

impl<T: EntityValue> EntityValues<T> {
	/// The bit of a slot that tells it has a value.
	const SET: u64 = 1 << 32;

	/// A table with no value in any slot.
	pub const fn new() -> Self {
		Self {
			slots: [const { AtomicU64::new(0) }; MAX_EDICTS as usize],
			_value: PhantomData,
		}
	}

	/// Takes every value out of the table.
	pub fn clear(&self) {
		for slot in &self.slots {
			slot.store(0, Ordering::Relaxed);
		}
	}

	/// The value of the entity at an edict index, or `None` if it has none, or
	/// the index lies past the edict table.
	pub fn get(&self, index: usize) -> Option<T> {
		let slot = self.slots.get(index)?.load(Ordering::Relaxed);

		(slot & Self::SET != 0).then(|| T::from_bits(slot as u32))
	}

	/// Gives the entity at an edict index a value, or takes it away for
	/// `None`, and returns whether that changed it. Does nothing for an index
	/// past the edict table.
	pub fn set(&self, index: usize, value: Option<T>) -> bool {
		let Some(slot) = self.slots.get(index) else {
			return false;
		};

		let bits = value.map_or(0, |value| Self::SET | u64::from(value.to_bits()));

		slot.swap(bits, Ordering::Relaxed) != bits
	}

	/// Gives an edict's entity a value, or takes it away for `None`, as
	/// [`Self::set`] does, and records a change of the entity if that changed
	/// it, so that the engine encodes it anew, through the proxies as they
	/// are.
	pub fn set_and_resend(&self, engine: ValveEngine<'_>, edict: Edict<'_>, value: Option<T>) {
		let changed = usize::try_from(edict.index()).is_ok_and(|index| self.set(index, value));

		if changed {
			edict.full_state_changed(engine);
		}
	}
}

impl<T: EntityValue> Default for EntityValues<T> {
	fn default() -> Self {
		Self::new()
	}
}

/// Why a send proxy could not be overridden.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendProxyError {
	/// The property is not a single value, such as an array or a nested
	/// table, or of a type this crate does not know. The elements of an array
	/// share the property describing them, which
	/// [`NetProp::element`](crate::datatables::NetProp::element) resolves.
	#[error("`{name}` is {kind}, which has no single value to send")]
	NotAValue {
		/// The property's name.
		name: String,

		/// The type the property is networked as.
		kind: PropKind,
	},

	/// The property has no send proxy to call first.
	#[error("`{name}` has no send proxy")]
	NoProxy {
		/// The property's name.
		name: String,
	},

	/// The property's proxy is already overridden by this plugin.
	#[error("`{name}`'s send proxy is already overridden")]
	AlreadyOverridden {
		/// The property's name.
		name: String,
	},

	/// [`MAX_SEND_PROXY_OVERRIDES`] proxies are already overridden by this
	/// plugin.
	#[error("{MAX_SEND_PROXY_OVERRIDES} send proxies are already overridden")]
	Full,
}

/// A property's send proxy, overridden until dropped: what clients receive of
/// the property's variable passes through a callback first.
///
/// The override holds no scope, so it can live across callbacks, such as in a
/// `static` or a plugin's state, but it is dropped on the main thread it was
/// installed on. Drop it, or [remove](Self::remove) it, before the plugin
/// unloads, as the [module](self) describes.
#[must_use = "dropping the override puts the game's send proxy back at once"]
#[derive(Debug)]
pub struct SendProxyOverride {
	id: SlotId,
	prop: NonNull<sys::SendProp>,
	_not_thread_safe: NotThreadSafe,
}

impl SendProxyOverride {
	/// Replaces `prop`'s send proxy with one that calls the game's, then
	/// `callback` with the variable and what the game's sent, which returns
	/// what to send instead, or `None` to send what the game's did. A value of
	/// another type than the property's is not sent, and neither is a value
	/// after the callback panics.
	///
	/// The callback runs every time the engine encodes the variable, on any
	/// thread, as the [module](self) describes, so it should be cheap. The
	/// override covers the property in every class that sends it, as the
	/// classes deriving from the one declaring it do through their base class
	/// table: overriding `CBaseEntity`'s `m_CollisionGroup` covers every
	/// entity.
	///
	/// Entities already sent are not sent anew: call [`Self::resend_all`] for
	/// clients to receive the override at once.
	///
	/// Fails if the property is not a single value or has no proxy, if this
	/// plugin already overrides it, or if it overrides
	/// [`MAX_SEND_PROXY_OVERRIDES`] proxies.
	#[doc(alias("m_ProxyFn", "SendProxy"))]
	pub fn install<F>(prop: SendProp<'_>, callback: F) -> Result<Self, SendProxyError>
	where
		F: for<'a> Fn(Sent, SentValue<'a>) -> Option<SentValue<'a>> + Send + Sync + 'static,
	{
		let name = || prop.name().to_string_lossy().into_owned();
		let kind = prop.kind();

		if !SentValue::sendable(kind) {
			return Err(SendProxyError::NotAValue { name: name(), kind });
		}

		let handler = move |call: ProxyCall| {
			let (Ok(entity_index), Ok(element)) = (
				usize::try_from(call.object_id),
				usize::try_from(call.element),
			) else {
				return;
			};

			// SAFETY: The trampoline passes the value the game's proxy wrote, of
			// the property's type, checked to be one of `SentValue`'s, and the
			// engine encodes it once the trampoline returns.
			unsafe {
				let sent = SentValue::read(kind, call.out);

				if let Some(value) = callback(
					Sent {
						entity_index,
						element,
					},
					sent,
				) {
					value.write(kind, call.out);
				}
			}
		};

		let raw = NonNull::new(prop.as_ptr()).expect("a send property is never null");

		// SAFETY: The property is one of the game DLL's send tables', which are
		// statics, and the engine calls its proxy with a value of the property's
		// type. `SendProp` is only made on the main thread, which never runs
		// while the engine packs entities, and this override, which restores the
		// slot as it drops, cannot leave it. The handler writes only a value of
		// the property's type, and a string that lives until the encoding.
		let id = unsafe { raw::install(raw, Box::new(handler)) }.map_err(|error| match error {
			InstallError::NoProxy => SendProxyError::NoProxy { name: name() },
			InstallError::AlreadyOverridden => SendProxyError::AlreadyOverridden { name: name() },
			InstallError::Full => SendProxyError::Full,
		})?;

		Ok(Self {
			id,
			prop: raw,
			_not_thread_safe: PhantomData,
		})
	}

	/// Whether the proxy is still overridden: not since replaced by another
	/// plugin's and [retired](sdk_raw::send_proxies::Restored::Retired) by
	/// [`restore_all`].
	pub fn is_installed(&self) -> bool {
		raw::is_installed(self.id)
	}

	/// The overridden property, for low-level interop.
	pub const fn prop_ptr(&self) -> *mut sys::SendProp {
		self.prop.as_ptr()
	}

	/// Puts the game's proxy back, and has the engine encode every entity
	/// sending the property anew, so that clients receive the game's values at
	/// once.
	pub fn remove(self, server: Server<'_>) {
		let prop = self.prop;

		drop(self);
		resend_all(server, prop);
	}

	/// Has the engine encode every entity sending the property anew, through
	/// the proxies as they are, so that clients receive what the callback
	/// sends now. Entities sending it are those whose class's send table holds
	/// the property, or a table nested within it.
	pub fn resend_all(&self, server: Server<'_>) {
		resend_all(server, self.prop);
	}
}

impl Drop for SendProxyOverride {
	fn drop(&mut self) {
		// SAFETY: The override was installed on the main thread, which it never
		// leaves, and which never runs while the engine packs entities. The
		// property is a static of the game DLL.
		unsafe { raw::restore(self.id) };
	}
}

/// A networked variable a send proxy is encoding: whose, and which element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sent {
	entity_index: usize,
	element: usize,
}

impl Sent {
	/// The variable's index in its array, or 0 for a variable outside one.
	pub const fn element(self) -> usize {
		self.element
	}

	/// The edict index of the entity whose variable is encoded. The game sends
	/// some objects' variables from an entity standing for them, such as the
	/// game rules' from `tf_gamerules` in TF2.
	pub const fn entity_index(self) -> usize {
		self.entity_index
	}
}

/// A value as a send proxy sends it, of the type of its property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SentValue<'a> {
	/// The value of a [`PropKind::Int`] property.
	Int(c_int),

	/// The value of a [`PropKind::Float`] property.
	Float(f32),

	/// The value of a [`PropKind::Vector`] property.
	Vector(Vector),

	/// The X and Y of a [`PropKind::VectorXY`] property.
	VectorXY(Vec2),

	/// The value of a [`PropKind::String`] property. A string sent in place of
	/// the game's must live until the engine encodes it, which the lifetime
	/// ensures: it is either the game's or `'static`.
	String(&'a CStr),
}

impl SentValue<'_> {
	/// Reads what a proxy of a property of `kind` wrote, an empty string for
	/// a null string.
	///
	/// # Safety
	///
	/// `kind` must be [sendable](Self::sendable), and `out` must point to a
	/// value a proxy of a property of `kind` wrote, of which a string lasts
	/// for `'a`.
	unsafe fn read<'a>(kind: PropKind, out: *const sys::DVariant) -> SentValue<'a> {
		// SAFETY: As the caller promises. The member read is the one the proxy
		// of a property of `kind` writes.
		unsafe {
			let value = &raw const (*out).__bindgen_anon_1;

			match kind {
				PropKind::Int => SentValue::Int((&raw const (*value).m_Int).read()),
				PropKind::Float => SentValue::Float((&raw const (*value).m_Float).read()),

				PropKind::Vector => {
					let [x, y, z] = (&raw const (*value).m_Vector).read();

					SentValue::Vector(Vector::new(x, y, z))
				}

				PropKind::VectorXY => {
					let [x, y, _] = (&raw const (*value).m_Vector).read();

					SentValue::VectorXY(Vec2::new(x, y))
				}

				_ => {
					let string: *const c_char = (&raw const (*value).m_pString).read();

					SentValue::String(if string.is_null() {
						c""
					} else {
						CStr::from_ptr(string)
					})
				}
			}
		}
	}

	/// Whether a property of `kind` has a single value, which a proxy sends.
	const fn sendable(kind: PropKind) -> bool {
		matches!(
			kind,
			PropKind::Int
				| PropKind::Float
				| PropKind::Vector
				| PropKind::VectorXY
				| PropKind::String
		)
	}

	/// The type of property this is a value of.
	pub const fn kind(self) -> PropKind {
		match self {
			Self::Int(_) => PropKind::Int,
			Self::Float(_) => PropKind::Float,
			Self::Vector(_) => PropKind::Vector,
			Self::VectorXY(_) => PropKind::VectorXY,
			Self::String(_) => PropKind::String,
		}
	}

	/// Writes the value in place of what a proxy of a property of `kind`
	/// wrote, unless it is of another type.
	///
	/// # Safety
	///
	/// `out` must point to a writable value for a property of `kind`, which the
	/// engine encodes while a string written lasts.
	unsafe fn write(self, kind: PropKind, out: *mut sys::DVariant) {
		if self.kind() != kind {
			return;
		}

		// SAFETY: As the caller promises. The member written is the one the
		// engine reads for a property of `kind`.
		unsafe {
			let value = &raw mut (*out).__bindgen_anon_1;

			match self {
				Self::Int(int) => (&raw mut (*value).m_Int).write(int),
				Self::Float(float) => (&raw mut (*value).m_Float).write(float),

				Self::Vector(vector) => {
					(&raw mut (*value).m_Vector).write([vector.x, vector.y, vector.z])
				}

				Self::VectorXY(vector) => {
					let z = (&raw const (*value).m_Vector).read()[2];

					(&raw mut (*value).m_Vector).write([vector.x, vector.y, z]);
				}

				Self::String(string) => (&raw mut (*value).m_pString).write(string.as_ptr()),
			}
		}
	}
}

/// Whether `table`, or a table nested within it, holds `prop`.
fn holds_prop(table: SendTable<'_>, prop: NonNull<sys::SendProp>, depth: usize) -> bool {
	const MAX_DEPTH: usize = 32;

	depth <= MAX_DEPTH
		&& table.props().any(|candidate| {
			candidate.as_ptr() == prop.as_ptr()
				|| candidate
					.data_table()
					.is_some_and(|nested| holds_prop(nested, prop, depth + 1))
		})
}

/// Has the engine encode every entity whose class's send table holds `prop`
/// anew.
fn resend_all(server: Server<'_>, prop: NonNull<sys::SendProp>) {
	let Ok(engine) = server.valve_engine() else {
		return;
	};

	// Tables checked so far, and whether each holds the property. Levels have
	// a few hundred classes at most.
	let mut checked: Vec<(*mut sys::SendTable, bool)> = Vec::new();

	for index in 0..MAX_EDICTS {
		let Some(edict) = engine.edict_of_index(index) else {
			continue;
		};

		let Some(table) = edict
			.entity()
			.and_then(|entity| entity.server_class())
			.and_then(|class| class.table())
		else {
			continue;
		};

		let holds = match checked
			.iter()
			.find(|(checked, _)| *checked == table.as_ptr())
		{
			Some(&(_, holds)) => holds,

			None => {
				let holds = holds_prop(table, prop, 0);

				checked.push((table.as_ptr(), holds));
				holds
			}
		};

		if holds {
			edict.full_state_changed(engine);
		}
	}
}

/// Puts back the game's proxy of every property this plugin overrides, as a
/// plugin must before its library unloads, when it might not have dropped
/// every [`SendProxyOverride`]: the engine would otherwise call into the
/// unloaded library. The overrides left do nothing as they drop.
///
/// A proxy another plugin replaced since stays in place, sending what the
/// game's sends, as the [module](self) describes.
pub fn restore_all(_server: Server<'_>) {
	// SAFETY: The server is scoped to a callback on the main thread, which
	// never runs while the engine packs entities, and the properties are
	// statics of the game DLL.
	unsafe { raw::restore_all() };
}

/// Keeps [`EntityValue`] from being implemented outside this crate.
mod sealed {
	pub trait Sealed {}

	impl Sealed for bool {}

	impl Sealed for f32 {}

	impl Sealed for i32 {}

	impl Sealed for u32 {}
}
