//! Entities in the server's entity list.
//!
//! [`ServerTools`](crate::interfaces::ServerTools) finds entities, including
//! server-only ones. Properties use the native entity interfaces instead of
//! key values, and networked variables are reached through
//! [`NetProp`](crate::datatables::NetProp).

use crate::datatables::ServerClass;
use crate::edicts::Edict;
use crate::ffi::{NotThreadSafe, borrow_cstr, vcall};
use crate::math::{QAngle, Vector};
use std::ffi::{CStr, c_char, c_int, c_short};
use std::fmt::{self, Display, Formatter};
use std::marker::PhantomData;
use std::mem::{align_of, size_of, transmute};
use std::ptr::{self, NonNull};
use std::sync::OnceLock;

/// The most data description maps an entity's chain is followed through.
const MAX_DATA_MAPS: usize = 64;

/// The most fields a data description map is trusted to hold.
const MAX_DATA_FIELDS: usize = 4096;

/// The deepest nesting of embedded objects searched for key fields.
const MAX_EMBEDDING_DEPTH: usize = 16;

/// `FTYPEDESC_KEY` from `public/datamap.h`: the field is set by a key value.
pub(crate) const FTYPEDESC_KEY: c_short = 0x0004;

/// `EFL_KILLME` from `game/shared/shareddefs.h`.
const EFL_KILLME: c_int = 1 << 0;

/// An entity cannot be teleported as requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TeleportError {
	#[error("the destination contains a non-finite component")]
	NonFinite,

	#[error("the entity is marked for deletion")]
	MarkedForDeletion,
}

/// An entity in the server's entity list, as referred to by a `CBaseEntity *`.
///
/// Removing an entity frees it at the end of the frame, so a handle is bound
/// to the scope that produced it. To refer to an entity across callbacks, keep
/// its [`EntityHandle`] and look it up again.
#[doc(alias = "CBaseEntity")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity<'s> {
	raw: NonNull<sys::CBaseEntity>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> Entity<'s> {
	/// # Safety
	///
	/// `raw` must point to an entity in the entity list that stays allocated
	/// for `'s`.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::CBaseEntity>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::CBaseEntity {
		self.raw.as_ptr()
	}

	/// The handle that identifies this entity across frames.
	#[doc(alias = "GetRefEHandle")]
	pub fn handle(self) -> EntityHandle {
		// SAFETY: `CBaseEntity`'s primary base is `IServerEntity`, which derives
		// from `IServerUnknown`, so the pointers coincide.
		let handle = unsafe { vcall!(self.unknown() => IServerUnknown_GetRefEHandle()) };

		// SAFETY: The handle is a member of the live entity.
		NonNull::new(handle.cast_mut()).map_or(EntityHandle::INVALID, |handle| {
			EntityHandle(unsafe { (&raw const (*handle.as_ptr()).m_Index).read() })
		})
	}

	/// The entity's slot in the entity list, which is its edict index if it is
	/// networked.
	#[doc(alias = "entindex")]
	pub fn index(self) -> Option<usize> {
		self.handle().index()
	}

	/// The entity's edict, or `None` for a server-only entity.
	#[doc(alias = "GetEdict")]
	pub fn edict(self) -> Option<Edict<'s>> {
		let networkable = self.networkable()?;

		// SAFETY: The networkable belongs to the live entity.
		let edict = unsafe { vcall!(networkable.as_ptr() => IServerNetworkable_GetEdict()) };

		// SAFETY: The engine's edict table outlives `'s`.
		NonNull::new(edict).map(|edict| unsafe { Edict::from_raw(edict) })
	}

	/// The entity's class name, such as `tf_player`.
	#[doc(alias = "GetClassname")]
	pub fn class_name(self) -> &'s CStr {
		let Some(networkable) = self.networkable() else {
			return c"";
		};

		// SAFETY: The networkable belongs to the live entity. Class names are
		// pooled strings, which live until the level ends.
		unsafe { borrow_cstr(vcall!(networkable.as_ptr() => IServerNetworkable_GetClassName())) }
			.unwrap_or_default()
	}

	/// The class describing how the entity is networked.
	#[doc(alias = "GetServerClass")]
	pub fn server_class(self) -> Option<ServerClass<'s>> {
		let networkable = self.networkable()?;

		// SAFETY: The networkable belongs to the live entity.
		let class = unsafe { vcall!(networkable.as_ptr() => IServerNetworkable_GetServerClass()) };

		// SAFETY: Server classes are statics of the game DLL.
		NonNull::new(class).map(|class| unsafe { ServerClass::from_raw(class) })
	}

	/// Reads the absolute origin without key-value conversion.
	///
	/// Source's collision property returns its owner's `GetAbsOrigin()` here.
	#[doc(alias = "GetAbsOrigin")]
	pub fn position(self) -> Option<Vector> {
		// SAFETY: As for `handle`.
		let collideable =
			NonNull::new(unsafe { vcall!(self.unknown() => IServerUnknown_GetCollideable()) })?;

		// SAFETY: The collideable belongs to the live entity.
		let origin = NonNull::new(
			unsafe { vcall!(collideable.as_ptr() => ICollideable_GetCollisionOrigin()) }.cast_mut(),
		)?;

		// SAFETY: The origin is a member of the live entity, copied immediately.
		Some(unsafe { origin.as_ptr().read() }.into())
	}

	/// Whether Source has marked this entity for deferred deletion.
	#[doc(alias = "IsMarkedForDeletion")]
	pub fn is_marked_for_deletion(self) -> bool {
		static EFLAGS_OFFSET: OnceLock<usize> = OnceLock::new();

		let offset = *EFLAGS_OFFSET.get_or_init(|| {
			find_base_entity_field(
				self,
				c"m_iEFlags",
				sys::_fieldtypes_FIELD_INTEGER,
				size_of::<c_int>(),
			)
			.expect("Source's CBaseEntity datamap does not contain m_iEFlags")
		});

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base.
		let flags = unsafe { self.as_ptr().byte_add(offset).cast::<c_int>().read() };

		flags & EFL_KILLME != 0
	}

	/// Moves the entity through Source's `Teleport` method, found at `slot` of
	/// its vtable. Each argument left as `None` is unchanged.
	pub(crate) fn teleport(
		self,
		slot: usize,
		origin: Option<Vector>,
		angles: Option<QAngle>,
		velocity: Option<Vector>,
	) -> Result<(), TeleportError> {
		type Teleport = unsafe extern "C" fn(
			*mut sys::CBaseEntity,
			*const sys::Vector,
			*const sys::QAngle,
			*const sys::Vector,
		);

		let finite_angles = |angles: QAngle| {
			angles.pitch.is_finite() && angles.yaw.is_finite() && angles.roll.is_finite()
		};

		if !(origin.is_none_or(|origin| origin.is_finite())
			&& angles.is_none_or(finite_angles)
			&& velocity.is_none_or(|velocity| velocity.is_finite()))
		{
			return Err(TeleportError::NonFinite);
		}

		if self.is_marked_for_deletion() {
			return Err(TeleportError::MarkedForDeletion);
		}

		let origin = origin.map(sys::Vector::from);
		let angles = angles.map(sys::QAngle::from);
		let velocity = velocity.map(sys::Vector::from);

		// SAFETY: `Game` selected the slot of `Teleport` in the game's vtable.
		let teleport: Teleport = unsafe { transmute(self.vtable_slot(slot)) };

		// SAFETY: The entity is live and every pointer is null or a local.
		unsafe {
			teleport(
				self.as_ptr(),
				origin.as_ref().map_or(ptr::null(), ptr::from_ref),
				angles.as_ref().map_or(ptr::null(), ptr::from_ref),
				velocity.as_ref().map_or(ptr::null(), ptr::from_ref),
			)
		};

		Ok(())
	}

	/// The entity's name field, `m_iName`, which the `targetname` key sets.
	///
	/// Returns `None` if the `CBaseEntity` datamap has no such field.
	pub(crate) fn name_field(self) -> Option<*mut sys::string_t> {
		static NAME_OFFSET: OnceLock<Option<usize>> = OnceLock::new();

		let offset = (*NAME_OFFSET.get_or_init(|| {
			find_base_entity_field(
				self,
				c"m_iName",
				sys::_fieldtypes_FIELD_STRING,
				size_of::<sys::string_t>(),
			)
		}))?;

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base.
		Some(unsafe { self.as_ptr().byte_add(offset).cast() })
	}

	/// The `string_t` field `GetKeyValue` reads for a key, or `None` if the key
	/// finds no field, or one of another type.
	///
	/// Fields are searched as `ExtractKeyvalue` does: from the entity's own
	/// class towards its bases, and through each embedded object before the
	/// fields after it, for the first key field whose name matches, ignoring
	/// ASCII case.
	pub(crate) fn string_key_field(self, key: &CStr) -> Option<*const sys::string_t> {
		let (field, offset) = find_key_field(self.data_maps(), key.to_bytes(), 0)?;

		let is_string = matches!(
			field.fieldType,
			sys::_fieldtypes_FIELD_STRING
				| sys::_fieldtypes_FIELD_MODELNAME
				| sys::_fieldtypes_FIELD_SOUNDNAME
		);

		// SAFETY: The offset was found in the entity's own datamap chain.
		(is_string && offset.is_multiple_of(align_of::<sys::string_t>()))
			.then(|| unsafe { self.as_ptr().byte_add(offset).cast_const().cast() })
	}

	/// The entity's data description maps, from its own class to its bases.
	pub(crate) fn data_maps(self) -> DataMaps<'s> {
		type GetDataDescMap = unsafe extern "C" fn(*mut sys::CBaseEntity) -> *mut sys::datamap_t;

		// SAFETY: `GetDataDescMap` occupies this slot under both ABIs.
		let get_map: GetDataDescMap =
			unsafe { transmute(self.vtable_slot(sys::CBASEENTITY_DATAMAP_VTABLE_SLOT)) };

		// SAFETY: The entity is live.
		DataMaps::starting_at(unsafe { get_map(self.as_ptr()) })
	}

	/// Calls Source's `AcceptInput`, which runs the input's handler before
	/// returning, and returns whether it found the input and converted the
	/// value to the input's type.
	///
	/// # Safety
	///
	/// The input, and everything it runs, must free entities only through
	/// deferred deletion. Its handler must accept the activator and caller.
	/// A string value must be pooled, since handlers may keep it.
	pub(crate) unsafe fn accept_input(
		self,
		input: &CStr,
		mut value: sys::variant_t,
		activator: Option<Entity<'_>>,
		caller: Option<Entity<'_>>,
	) -> bool {
		// `variant_t` has a user-provided copy constructor through its handle,
		// so both ABIs pass it as a pointer to a copy the caller owns, which
		// `AcceptInput` converts in place.
		type AcceptInput = unsafe extern "C" fn(
			this: *mut sys::CBaseEntity,
			input: *const c_char,
			activator: *mut sys::CBaseEntity,
			caller: *mut sys::CBaseEntity,
			value: *mut sys::variant_t,
			output_id: c_int,
		) -> bool;

		// The generated slot has the same signature.
		let _: fn(&sys::CBaseEntity__bindgen_vtable) -> AcceptInput =
			|vtable| vtable.CBaseEntity_AcceptInput;

		// SAFETY: `AcceptInput` occupies this slot in every game's vtable.
		let accept_input: AcceptInput =
			unsafe { transmute(self.vtable_slot(sys::CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT)) };

		// SAFETY: The entities are live, the name is only read during the call,
		// and the value is a local. The output ID is 0, as for the game's own
		// calls and VScript's `AcceptInput`. The caller upholds the rest.
		unsafe {
			accept_input(
				self.as_ptr(),
				input.as_ptr(),
				activator.map_or(ptr::null_mut(), Entity::as_ptr),
				caller.map_or(ptr::null_mut(), Entity::as_ptr),
				&raw mut value,
				0,
			)
		}
	}

	fn unknown(self) -> *mut sys::IServerUnknown {
		self.raw.as_ptr().cast()
	}

	fn networkable(self) -> Option<NonNull<sys::IServerNetworkable>> {
		// SAFETY: As for `handle`.
		NonNull::new(unsafe { vcall!(self.unknown() => IServerUnknown_GetNetworkable()) })
	}

	fn vtable_slot(self, slot: usize) -> *const () {
		// SAFETY: `CBaseEntity` has `IServerEntity` as its primary, zero-offset
		// base, so the entity starts with its primary vtable pointer.
		let vtable = unsafe { self.as_ptr().cast::<*const *const ()>().read() };

		// SAFETY: Callers only pass slots within `CBaseEntity`'s vtable.
		unsafe { vtable.add(slot).read() }
	}
}

/// Identifies an entity across frames without keeping it alive, like the
/// game's `CBaseHandle` and `EHANDLE`.
///
/// A handle combines the entity's slot in the entity list with the serial
/// number the slot had when the entity was created. Looking a handle up with
/// [`ServerTools::entity_by_handle`](crate::interfaces::ServerTools::entity_by_handle)
/// after its entity was removed finds nothing, rather than whatever entity
/// reused the slot.
#[doc(alias = "CBaseHandle")]
#[doc(alias = "EHANDLE")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct EntityHandle(u32);

impl EntityHandle {
	/// `INVALID_EHANDLE_INDEX`, which refers to no entity.
	pub const INVALID: Self = Self(u32::MAX);

	/// `NUM_ENT_ENTRIES`, the number of slots in the entity list.
	pub const SLOTS: usize = 1 << 13;

	/// `NUM_SERIAL_NUM_SHIFT_BITS`.
	const SERIAL_NUMBER_SHIFT: u32 = 16;

	pub const fn from_raw(raw: u32) -> Self {
		Self(raw)
	}

	pub const fn to_raw(self) -> u32 {
		self.0
	}

	pub const fn is_valid(self) -> bool {
		self.0 != Self::INVALID.0
	}

	/// The entity's slot in the entity list, or `None` for an invalid handle.
	#[doc(alias = "GetEntryIndex")]
	pub const fn index(self) -> Option<usize> {
		if self.is_valid() {
			Some((self.0 & ((1 << Self::SERIAL_NUMBER_SHIFT) - 1)) as usize)
		} else {
			None
		}
	}

	#[doc(alias = "GetSerialNumber")]
	pub const fn serial_number(self) -> u32 {
		self.0 >> Self::SERIAL_NUMBER_SHIFT
	}
}

impl Display for EntityHandle {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self.index() {
			Some(index) => write!(f, "{index}:{}", self.serial_number()),
			None => f.write_str("invalid"),
		}
	}
}

/// An entity's data description maps, from [`Entity::data_maps`].
pub(crate) struct DataMaps<'s> {
	next: *const sys::datamap_t,
	remaining: usize,
	_scope: PhantomData<&'s ()>,
}

impl DataMaps<'_> {
	/// Follows the chain from `first`, a map of the game DLL, or null.
	fn starting_at(first: *const sys::datamap_t) -> Self {
		Self {
			next: first,
			remaining: MAX_DATA_MAPS,
			_scope: PhantomData,
		}
	}
}

impl<'s> Iterator for DataMaps<'s> {
	type Item = &'s sys::datamap_t;

	fn next(&mut self) -> Option<Self::Item> {
		self.remaining = self.remaining.checked_sub(1)?;

		// SAFETY: Datamaps are statics of the game DLL, which are complete once
		// the first entity of their class exists and never change afterwards.
		let map = unsafe { self.next.as_ref() }?;

		self.next = map.baseMap;
		Some(map)
	}
}

/// The name of the class a data description map describes.
pub(crate) fn data_map_class(map: &sys::datamap_t) -> Option<&CStr> {
	// SAFETY: Class names are string literals of the game DLL.
	unsafe { borrow_cstr(map.dataClassName) }
}

/// The fields a data description map declares, not including its bases'.
pub(crate) fn data_fields(map: &sys::datamap_t) -> &[sys::typedescription_t] {
	match usize::try_from(map.dataNumFields) {
		Ok(count @ 1..=MAX_DATA_FIELDS) if !map.dataDesc.is_null() => {
			// SAFETY: The map holds `dataNumFields` fields, as immutable as the
			// map itself.
			unsafe { std::slice::from_raw_parts(map.dataDesc, count) }
		}
		_ => &[],
	}
}

/// Finds the offset of a field `CBaseEntity`'s own map declares.
fn find_base_entity_field(
	entity: Entity<'_>,
	name: &CStr,
	field_type: sys::fieldtype_t,
	size: usize,
) -> Option<usize> {
	let map = entity
		.data_maps()
		.find(|&map| data_map_class(map) == Some(c"CBaseEntity"))?;

	let field = data_fields(map).iter().find(|field| {
		field.fieldType == field_type
			&& usize::try_from(field.fieldSizeInBytes) == Ok(size)
			// SAFETY: Field names are string literals of the game DLL.
			&& unsafe { borrow_cstr(field.fieldName) } == Some(name)
	})?;

	let offset = usize::try_from(field.fieldOffset[0]).ok()?;

	(offset < 8192 && offset.is_multiple_of(size.min(align_of::<*const ()>()))).then_some(offset)
}

/// Finds the field `ExtractKeyvalue` reads for a key in the object `maps`
/// describe, and its offset in that object.
fn find_key_field<'s>(
	maps: DataMaps<'s>,
	key: &[u8],
	depth: usize,
) -> Option<(&'s sys::typedescription_t, usize)> {
	for map in maps {
		for field in data_fields(map) {
			let offset = usize::try_from(field.fieldOffset[0]).ok();

			// Embedded objects are searched before the field itself, but not
			// arrays of them.
			if field.fieldType == sys::_fieldtypes_FIELD_EMBEDDED
				&& field.fieldSize == 1
				&& depth < MAX_EMBEDDING_DEPTH
				&& let Some((found, inner)) =
					find_key_field(DataMaps::starting_at(field.td), key, depth + 1)
			{
				return Some((found, offset?.checked_add(inner)?));
			}

			// SAFETY: Key names are string literals of the game DLL.
			let name = unsafe { borrow_cstr(field.externalName) };

			if field.flags & FTYPEDESC_KEY != 0
				&& name.is_some_and(|name| name.to_bytes().eq_ignore_ascii_case(key))
			{
				return Some((field, offset?));
			}
		}
	}

	None
}

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;
	use crate::ffi::test_support::unexpected_call;
	use std::cell::{Cell, RefCell};
	use std::ffi::CString;
	use std::mem::offset_of;
	use std::ptr::null_mut;

	pub(crate) const MOCK_EFLAGS_OFFSET: usize = 32;
	const MOCK_HANDLE_OFFSET: usize = 40;
	pub(crate) const MOCK_NAME_OFFSET: usize = 48;

	/// An input a mock entity's `AcceptInput` received.
	#[derive(Debug, Clone, PartialEq)]
	pub(crate) struct ReceivedInput {
		pub(crate) target: *mut sys::CBaseEntity,
		pub(crate) name: CString,
		pub(crate) activator: *mut sys::CBaseEntity,
		pub(crate) caller: *mut sys::CBaseEntity,
		pub(crate) field_type: sys::fieldtype_t,
		/// The bytes every union member covers.
		pub(crate) payload: [u8; 12],
		/// The string pointer, read as a pointer, for string values.
		pub(crate) string: *const std::ffi::c_char,
		pub(crate) handle: u32,
		pub(crate) output_id: c_int,
	}

	thread_local! {
		static COLLIDEABLE: Cell<*mut sys::ICollideable> = const { Cell::new(null_mut()) };
		static NETWORKABLE: Cell<*mut sys::IServerNetworkable> = const { Cell::new(null_mut()) };
		static ORIGIN: Cell<sys::Vector> = const { Cell::new(sys::Vector { x: 1.0, y: 2.0, z: 3.0 }) };
		static TELEPORTS: Cell<usize> = const { Cell::new(0) };
		static DATA_MAP: Cell<*mut sys::datamap_t> = const { Cell::new(null_mut()) };
		static SERVER_CLASS: Cell<*mut sys::ServerClass> = const { Cell::new(null_mut()) };
		static EDICT: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
		static INPUTS: RefCell<Vec<ReceivedInput>> = const { RefCell::new(Vec::new()) };
		static ACCEPTS: Cell<bool> = const { Cell::new(true) };
	}

	/// Replaces the datamap chain mock entities on this thread report.
	pub(crate) fn set_datamap(map: *mut sys::datamap_t) {
		DATA_MAP.set(map);
	}

	/// Sets what mock entities' `AcceptInput` returns on this thread.
	pub(crate) fn set_accepts(accepts: bool) {
		ACCEPTS.set(accepts);
	}

	/// Takes the inputs mock entities received on this thread.
	pub(crate) fn take_inputs() -> Vec<ReceivedInput> {
		INPUTS.take()
	}

	unsafe extern "C" fn accept_input(
		this: *mut sys::CBaseEntity,
		name: *const std::ffi::c_char,
		activator: *mut sys::CBaseEntity,
		caller: *mut sys::CBaseEntity,
		value: *mut sys::variant_t,
		output_id: c_int,
	) -> bool {
		let received = unsafe {
			ReceivedInput {
				target: this,
				name: CStr::from_ptr(name).to_owned(),
				activator,
				caller,
				field_type: (&raw const (*value).fieldType).read(),
				payload: value.cast::<[u8; 12]>().read(),
				string: if (&raw const (*value).fieldType).read() == sys::_fieldtypes_FIELD_STRING {
					(&raw const (*value).__bindgen_anon_1.iszVal.pszValue).read()
				} else {
					std::ptr::null()
				},
				handle: (&raw const (*value).eVal._base.m_Index).read(),
				output_id,
			}
		};

		// `Convert` changes the caller's copy in place.
		unsafe { (&raw mut (*value).fieldType).write(sys::_fieldtypes_FIELD_VOID) };

		INPUTS.with_borrow_mut(|inputs| inputs.push(received));
		ACCEPTS.get()
	}

	/// Sets the server class and edict that mock entities on this thread report.
	pub(crate) fn set_networking(class: *mut sys::ServerClass, edict: *mut sys::edict_t) {
		SERVER_CLASS.set(class);
		EDICT.set(edict);
	}

	unsafe extern "C" fn get_server_class(
		_: *mut sys::IServerNetworkable,
	) -> *mut sys::ServerClass {
		SERVER_CLASS.get()
	}

	unsafe extern "C" fn get_edict(_: *const sys::IServerNetworkable) -> *mut sys::edict_t {
		EDICT.get()
	}

	unsafe extern "C" fn get_datamap(_: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
		DATA_MAP.get()
	}

	unsafe extern "C" fn get_collideable(_: *mut sys::IServerUnknown) -> *mut sys::ICollideable {
		COLLIDEABLE.get()
	}

	unsafe extern "C" fn get_networkable(
		_: *mut sys::IServerUnknown,
	) -> *mut sys::IServerNetworkable {
		NETWORKABLE.get()
	}

	unsafe extern "C" fn get_handle(this: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
		unsafe { this.byte_add(MOCK_HANDLE_OFFSET).cast() }
	}

	unsafe extern "C" fn get_origin(_: *const sys::ICollideable) -> *const sys::Vector {
		ORIGIN.with(Cell::as_ptr).cast_const()
	}

	unsafe extern "C" fn get_class_name(
		_: *const sys::IServerNetworkable,
	) -> *const std::ffi::c_char {
		c"tf_player".as_ptr()
	}

	unsafe extern "C" fn teleport_entity(
		_: *mut sys::CBaseEntity,
		position: *const sys::Vector,
		angles: *const sys::QAngle,
		velocity: *const sys::Vector,
	) {
		assert!(angles.is_null() && velocity.is_null());
		ORIGIN.set(unsafe { *position });
		TELEPORTS.set(TELEPORTS.get() + 1);
	}

	fn vtable_slots<T>() -> Vec<*const ()> {
		vec![unexpected_call as *const (); size_of::<T>() / size_of::<*const ()>()]
	}

	/// Leaks a value, so pointers into it stay valid however the test moves
	/// what it keeps.
	pub(crate) fn leak<T>(value: T) -> *mut T {
		Box::into_raw(Box::new(value))
	}

	/// An entity with a class name, origin, datamap, handle, and TF2 `Teleport`,
	/// whose teleports are counted by [`teleports`].
	///
	/// Its allocations are leaked, and only reached through raw pointers, like
	/// the engine's objects.
	pub(crate) struct MockEntity {
		storage: *mut [usize; 64],
	}

	/// A zeroed field description, to be filled in.
	pub(crate) fn field() -> sys::typedescription_t {
		// SAFETY: Zero is valid for every field of `typedescription_t`.
		unsafe { std::mem::zeroed() }
	}

	/// Builds a leaked map for `class`, deriving from `base`.
	pub(crate) fn data_map(
		class: &'static CStr,
		declared: Vec<sys::typedescription_t>,
		base: *mut sys::datamap_t,
	) -> *mut sys::datamap_t {
		let count = declared.len() as c_int;
		let declared = declared.leak();
		let map = leak(unsafe { std::mem::zeroed::<sys::datamap_t>() });

		unsafe {
			(*map).dataDesc = declared.as_mut_ptr();
			(*map).dataNumFields = count;
			(*map).dataClassName = class.as_ptr();
			(*map).baseMap = base;
		}

		map
	}

	impl MockEntity {
		pub(crate) fn new(handle: u32) -> Self {
			let slot_count = sys::CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT
				.max(sys::CBASEENTITY_DATAMAP_VTABLE_SLOT)
				.max(sys::CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT)
				+ 1;
			let mut vtable = vec![unexpected_call as *const (); slot_count];
			let slot = |field: usize| field / size_of::<usize>();

			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetCollideable
			))] = get_collideable as *const ();
			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetNetworkable
			))] = get_networkable as *const ();
			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetRefEHandle
			))] = get_handle as *const ();
			vtable[sys::CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT] = teleport_entity as *const ();
			vtable[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT] = get_datamap as *const ();
			vtable[sys::CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT] = accept_input as *const ();

			let storage = leak([0usize; 64]);
			let vtable = vtable.leak();

			// Written as a pointer, so the entity's vtable keeps its provenance.
			unsafe {
				storage.cast::<*const *const ()>().write(vtable.as_ptr());
				storage
					.cast::<usize>()
					.add(MOCK_HANDLE_OFFSET / size_of::<usize>())
					.write(handle as usize);
			}

			let fields = leak(base_entity_fields());
			let map = leak(unsafe { std::mem::zeroed::<sys::datamap_t>() });

			unsafe {
				(*map).dataDesc = fields.cast();
				(*map).dataNumFields = (*fields).len() as c_int;
				(*map).dataClassName = c"CBaseEntity".as_ptr();
			}

			let mut collideable_vtable = vtable_slots::<sys::ICollideable__bindgen_vtable>();
			collideable_vtable[slot(offset_of!(
				sys::ICollideable__bindgen_vtable,
				ICollideable_GetCollisionOrigin
			))] = get_origin as *const ();
			let collideable = leak(sys::ICollideable {
				vtable_: collideable_vtable.leak().as_ptr().cast(),
			});

			let mut networkable_vtable = vtable_slots::<sys::IServerNetworkable__bindgen_vtable>();
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetClassName
			))] = get_class_name as *const ();
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetServerClass
			))] = get_server_class as *const ();
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetEdict
			))] = get_edict as *const ();
			let networkable = leak(sys::IServerNetworkable {
				vtable_: networkable_vtable.leak().as_ptr().cast(),
			});

			DATA_MAP.set(map);
			COLLIDEABLE.set(collideable);
			NETWORKABLE.set(networkable);
			ORIGIN.set(sys::Vector {
				x: 1.0,
				y: 2.0,
				z: 3.0,
			});
			TELEPORTS.set(0);
			set_networking(null_mut(), null_mut());

			Self { storage }
		}

		pub(crate) fn as_ptr(&mut self) -> *mut sys::CBaseEntity {
			self.storage.cast()
		}

		pub(crate) fn entity(&mut self) -> Entity<'_> {
			unsafe { Entity::from_raw(NonNull::new(self.as_ptr()).unwrap()) }
		}

		pub(crate) fn set_name(&mut self, name: sys::string_t) {
			unsafe {
				self.as_ptr()
					.byte_add(MOCK_NAME_OFFSET)
					.cast::<sys::string_t>()
					.write(name)
			};
		}

		pub(crate) fn name(&mut self) -> sys::string_t {
			unsafe {
				self.as_ptr()
					.byte_add(MOCK_NAME_OFFSET)
					.cast::<sys::string_t>()
					.read()
			}
		}

		pub(crate) fn set_eflags(&mut self, flags: c_int) {
			unsafe {
				self.as_ptr()
					.byte_add(MOCK_EFLAGS_OFFSET)
					.cast::<c_int>()
					.write(flags)
			};
		}
	}

	pub(crate) fn teleports() -> usize {
		TELEPORTS.get()
	}

	/// The fields `CBaseEntity`'s map declares that mock entities store.
	pub(crate) fn base_entity_fields() -> [sys::typedescription_t; 2] {
		let mut flags = field();

		flags.fieldType = sys::_fieldtypes_FIELD_INTEGER;
		flags.fieldName = c"m_iEFlags".as_ptr();
		flags.fieldOffset[0] = MOCK_EFLAGS_OFFSET as c_int;
		flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

		let mut name = field();

		name.fieldType = sys::_fieldtypes_FIELD_STRING;
		name.fieldName = c"m_iName".as_ptr();
		name.fieldOffset[0] = MOCK_NAME_OFFSET as c_int;
		name.fieldSizeInBytes = size_of::<sys::string_t>() as c_int;

		[flags, name]
	}
}

#[cfg(test)]
mod tests {
	use super::test_support::{MockEntity, teleports};
	use super::*;
	use glam::Vec3;

	#[test]
	fn entities_read_native_properties_and_teleport() {
		let mut mock = MockEntity::new(5 | 7 << 16);
		let entity = mock.entity();

		assert_eq!(entity.class_name(), c"tf_player");
		assert_eq!(entity.position(), Some(Vector::new(1.0, 2.0, 3.0)));
		assert_eq!(entity.handle(), EntityHandle::from_raw(5 | 7 << 16));
		assert_eq!(entity.index(), Some(5));
		assert!(!entity.is_marked_for_deletion());

		let slot = sys::CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT;

		assert_eq!(
			entity.teleport(
				slot,
				Some(Vector(Vec3::new(f32::NAN, 0.0, 0.0))),
				None,
				None
			),
			Err(TeleportError::NonFinite)
		);

		entity
			.teleport(slot, Some(Vector::new(4.0, 5.0, 6.0)), None, None)
			.unwrap();
		assert_eq!(entity.position(), Some(Vector::new(4.0, 5.0, 6.0)));
		assert_eq!(teleports(), 1);

		mock.set_eflags(EFL_KILLME);
		let entity = mock.entity();

		assert!(entity.is_marked_for_deletion());
		assert_eq!(
			entity.teleport(slot, Some(Vector::new(7.0, 8.0, 9.0)), None, None),
			Err(TeleportError::MarkedForDeletion)
		);
		assert_eq!(teleports(), 1);
	}

	#[test]
	fn handles_split_into_slot_and_serial_number() {
		let handle = EntityHandle::from_raw(42 | 3 << 16);

		assert_eq!(handle.index(), Some(42));
		assert_eq!(handle.serial_number(), 3);
		assert_eq!(handle.to_string(), "42:3");
		assert_eq!(EntityHandle::INVALID.index(), None);
		assert_eq!(EntityHandle::INVALID.to_string(), "invalid");
	}
}
