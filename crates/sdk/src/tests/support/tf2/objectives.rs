//! Fake objective entities: a mock entity of any objective class, with the
//! data description fields, inputs and networked variables a test gives it,
//! and the mock interfaces its wrappers reach.

use super::super::datatables::{int8_proxy, int32_proxy, prop, table, vector_proxy};

use super::super::entities::{
	MOCK_NAME_OFFSET, MockEntity, base_entity_fields, set_datamap, set_networking,
};

use super::super::interfaces::player_info_manager::{global_vars, serve_global_vars};
use super::super::interfaces::server_game_dll::export_standard_proxies;
use super::super::leak;
use super::super::server::export;
use crate::datatables::PropFlags;
use crate::interfaces::{PlayerInfoManager, ServerTools};
use crate::server::Module;
use sdk_raw::entities::datamap::{FTYPEDESC_INPUT, FTYPEDESC_KEY};
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::alloc::Layout;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr::null_mut;

/// Where fake objective entities' own fields may start, past those every
/// mock entity stores.
pub const FIELDS_OFFSET: usize = 128;

/// The bytes a fake objective entity holds.
pub const STORAGE_SIZE: usize = size_of::<[usize; 512]>();

thread_local! {
	/// The world entity, at index 0, which strings are pooled through.
	static WORLD: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };

	/// The entities `FindEntityByName` finds, in order, with their names.
	static NAMED: RefCell<Vec<(*mut sys::CBaseEntity, CString)>> = const { RefCell::new(Vec::new()) };

	/// Strings pooled into the world's name.
	static POOL: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };

	/// Key values set on entities other than through pooling.
	static KEY_VALUES: RefCell<Vec<(*mut sys::CBaseEntity, CString, CString)>> = const { RefCell::new(Vec::new()) };
}

/// A fake objective's class: its data descriptions, from its own class's
/// towards `CBaseEntity`'s, and its send table, if it is networked.
pub struct FakeClass {
	/// Each class's name and the fields its own data description declares,
	/// from the most derived class to the last base before `CBaseEntity`.
	pub maps: Vec<(&'static CStr, Vec<sys::typedescription_t>)>,

	/// Fields `CBaseEntity`'s data description declares besides those every
	/// mock entity stores, such as `m_iTeamNum`.
	pub base_fields: Vec<sys::typedescription_t>,

	/// The send table's name and properties, or `None` for an entity without
	/// an edict.
	pub table: Option<(&'static CStr, Vec<sys::SendProp>)>,
}

/// A mock entity of a [`FakeClass`], with the world entity strings are
/// pooled through, and the mock interfaces exported for it.
pub struct FakeObjective {
	/// The objective.
	pub mock: MockEntity,

	/// The world entity.
	pub world: MockEntity,

	/// The engine's globals, whose game time is 0.
	pub globals: *mut sys::CGlobalVars,

	/// The objective's edict, or null if it is not networked.
	pub edict: *mut sys::edict_t,

	/// The objective's address.
	address: *mut sys::CBaseEntity,
}

impl FakeObjective {
	/// Builds an objective of `class`, and exports a mock `IServerTools`, the
	/// standard send proxies, and a mock `IPlayerInfoManager` with zeroed
	/// globals. Strings sent to its inputs are pooled into the world's name,
	/// and other key values are recorded for [`take_key_values`].
	pub fn new(class: FakeClass) -> Self {
		let mut world = MockEntity::new(0);
		let mut mock = MockEntity::with_layout(5 | 1 << 16, Layout::new::<[usize; 512]>());
		let address = mock.as_ptr();

		let mut base_fields = Vec::from(base_entity_fields());

		base_fields.extend(class.base_fields);

		let chain = class.maps.into_iter().rev().fold(
			data_map(c"CBaseEntity", base_fields, null_mut()),
			|base, (name, fields)| data_map(name, fields, base),
		);

		set_datamap(chain);

		let edict = match class.table {
			Some((name, props)) => {
				let props = props.leak();
				let class = leak(sys::ServerClass {
					m_pNetworkName: name.as_ptr(),
					m_pTable: leak(table(name, props)),
					m_pNext: null_mut(),
					m_ClassID: 1,
					m_InstanceBaselineIndex: 0,
				});
				let edict = leak(mock_edict(5, false));

				set_networking(class, edict);
				edict
			}

			None => null_mut(),
		};

		WORLD.set(world.as_ptr());
		NAMED.take();
		KEY_VALUES.take();
		export_tools();
		export_standard_proxies();

		let globals = serve_global_vars(32);

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes a slot of
		// the vtable being built.
		let manager = Box::leak(unsafe {
			mock_vtable::<sys::IPlayerInfoManager__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IPlayerInfoManager_GetGlobalVars).write(global_vars);
				},
			)
		});

		export(
			Module::GameServer,
			PlayerInfoManager::VERSION,
			leak(sys::IPlayerInfoManager { vtable_: manager }),
		);

		// Keep the world's name valid for pooling.
		world.set_name(sys::string_t {
			pszValue: c"worldspawn".as_ptr(),
		});

		Self {
			mock,
			world,
			globals,
			edict,
			address,
		}
	}

	/// The objective's address.
	pub fn as_ptr(&self) -> *mut sys::CBaseEntity {
		self.address
	}

	/// Reads the byte at `offset` into the objective as a `bool`.
	pub fn bool(&mut self, offset: usize) -> bool {
		self.read::<u8>(offset) != 0
	}

	/// A handle to the objective, which the leaked mock outlives.
	#[cfg(test)]
	pub(crate) fn entity(&self) -> crate::entities::Entity<'static> {
		let entity = std::ptr::NonNull::new(self.address).unwrap();

		// SAFETY: The mock is leaked, so it outlives every borrow, and its vtable
		// answers what the wrappers call of a `CBaseEntity`.
		unsafe { crate::entities::Entity::from_raw(entity) }
	}

	/// Reads the `float` at `offset` into the objective.
	pub fn float(&mut self, offset: usize) -> f32 {
		self.read(offset)
	}

	/// Reads the `int` at `offset` into the objective.
	pub fn int(&mut self, offset: usize) -> c_int {
		self.read(offset)
	}

	/// Reads a `T` at `offset` into the objective.
	///
	/// # Panics
	///
	/// If the value lies beyond [`STORAGE_SIZE`], or is misaligned.
	fn read<T: Copy>(&mut self, offset: usize) -> T {
		assert!(offset + size_of::<T>() <= STORAGE_SIZE);
		assert!(offset.is_multiple_of(align_of::<T>()));

		// SAFETY: The storage holds `STORAGE_SIZE` initialized bytes, and the
		// place is aligned.
		unsafe { self.mock.as_ptr().byte_add(offset).cast::<T>().read() }
	}

	/// Writes `value` as a byte at `offset` into the objective.
	pub fn set_bool(&mut self, offset: usize, value: bool) {
		self.write(offset, u8::from(value));
	}

	/// Writes the `float` at `offset` into the objective.
	pub fn set_float(&mut self, offset: usize, value: f32) {
		self.write(offset, value);
	}

	/// Writes the `int` at `offset` into the objective.
	pub fn set_int(&mut self, offset: usize, value: c_int) {
		self.write(offset, value);
	}

	/// Sets the game time.
	pub fn set_time(&mut self, time: f32) {
		// SAFETY: The globals are leaked, and only this thread uses them.
		unsafe { (&raw mut (*self.globals)._base.curtime).write(time) };
	}

	/// Writes `value` at `offset` into the objective.
	///
	/// # Panics
	///
	/// As for [`read`](Self::read).
	fn write<T: Copy>(&mut self, offset: usize, value: T) {
		assert!(offset + size_of::<T>() <= STORAGE_SIZE);
		assert!(offset.is_multiple_of(align_of::<T>()));

		// SAFETY: As for `read`.
		unsafe { self.mock.as_ptr().byte_add(offset).cast::<T>().write(value) };
	}
}

/// A networked `bool`, as `SendPropBool` declares one: an unsigned integer
/// stored in a byte.
pub fn bool_prop(name: &'static CStr, offset: usize) -> sys::SendProp {
	prop(
		name,
		sys::SendPropType_DPT_Int,
		c_int::try_from(offset).unwrap(),
		PropFlags::UNSIGNED,
		Some(int8_proxy),
	)
}

/// A field of `field_type` at `offset`, whose size is `size`.
pub fn data_field(
	name: &'static CStr,
	field_type: sys::fieldtype_t,
	offset: usize,
	size: usize,
) -> sys::typedescription_t {
	let mut field = field(name, field_type, offset);

	field.fieldSizeInBytes = c_int::try_from(size).unwrap();
	field.fieldSize = 1;
	field
}

/// `IServerTools::GetBaseEntityByEntIndex`, which finds only the world, at
/// index 0.
unsafe extern "C" fn entity_by_index(
	_: *mut sys::IServerTools,
	index: c_int,
) -> *mut sys::CBaseEntity {
	if index == 0 { WORLD.get() } else { null_mut() }
}

/// Exports a mock `IServerTools`, which finds the world at index 0, finds
/// [registered](register_name) names, and sets key values with
/// [`set_key_value`].
fn export_tools() {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let vtable = Box::leak(unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex).write(entity_by_index);
			(&raw mut (*vtable).IServerTools_SetKeyValue).write(set_key_value);
			(&raw mut (*vtable).IServerTools_FindEntityByName).write(find_by_name);
		})
	});

	export(
		Module::GameServer,
		ServerTools::VERSION,
		leak(sys::IServerTools { vtable_: vtable }),
	);
}

/// `IServerTools::FindEntityByName`, which finds the entities
/// [`register_name`] registered, in order, comparing names ignoring ASCII
/// case.
unsafe extern "C" fn find_by_name(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	name: *const c_char,
	_: *mut sys::CBaseEntity,
	_: *mut sys::CBaseEntity,
	_: *mut sys::CBaseEntity,
	_: *mut sys::IEntityFindFilter,
) -> *mut sys::CBaseEntity {
	// SAFETY: The wrappers pass a NUL-terminated name.
	let name = unsafe { CStr::from_ptr(name) }.to_bytes();

	NAMED.with_borrow(|named| {
		let start = named
			.iter()
			.position(|(entity, _)| *entity == after)
			.map_or(0, |index| index + 1);

		named[start.min(named.len())..]
			.iter()
			.find(|(_, entity_name)| entity_name.to_bytes().eq_ignore_ascii_case(name))
			.map_or(null_mut(), |(entity, _)| *entity)
	})
}

/// A networked `float`, stored as one.
pub fn float_prop(name: &'static CStr, offset: usize) -> sys::SendProp {
	prop(
		name,
		sys::SendPropType_DPT_Float,
		c_int::try_from(offset).unwrap(),
		PropFlags::default(),
		Some(int32_proxy),
	)
}

/// An input of `field_type`, named `name`.
pub fn input(name: &'static CStr, field_type: sys::fieldtype_t) -> sys::typedescription_t {
	// SAFETY: Zero is valid for every field of `typedescription_t`.
	let mut input: sys::typedescription_t = unsafe { std::mem::zeroed() };

	input.fieldType = field_type;
	input.externalName = name.as_ptr();
	input.flags = FTYPEDESC_INPUT;
	input
}

/// A networked `int`, stored as one.
pub fn int_prop(name: &'static CStr, offset: usize) -> sys::SendProp {
	prop(
		name,
		sys::SendPropType_DPT_Int,
		c_int::try_from(offset).unwrap(),
		PropFlags::default(),
		Some(int32_proxy),
	)
}

/// A field of `field_type` named `name` at `offset`, whose size is `size`,
/// which maps set with the key `key`.
pub fn key_field(
	name: &'static CStr,
	key: &'static CStr,
	field_type: sys::fieldtype_t,
	offset: usize,
	size: usize,
) -> sys::typedescription_t {
	let mut field = data_field(name, field_type, offset, size);

	field.externalName = key.as_ptr();
	field.flags = FTYPEDESC_KEY;
	field
}

/// Makes the mock `FindEntityByName` find `entity` by `name`, after the
/// entities registered before it.
pub fn register_name(entity: *mut sys::CBaseEntity, name: &CStr) {
	NAMED.with_borrow_mut(|named| named.push((entity, name.to_owned())));
}

/// `IServerTools::SetKeyValue`, which pools a `targetname` into the entity's
/// name case-insensitively, as `CGameStringPool` does, and records any other
/// key value for [`take_key_values`].
unsafe extern "C" fn set_key_value(
	_: *mut sys::IServerTools,
	entity: *mut sys::CBaseEntity,
	key: *const c_char,
	value: *const c_char,
) -> bool {
	// SAFETY: The wrappers pass NUL-terminated keys and values.
	let (key, value) = unsafe { (CStr::from_ptr(key), CStr::from_ptr(value)) };

	if key != c"targetname" {
		KEY_VALUES.with_borrow_mut(|set| set.push((entity, key.to_owned(), value.to_owned())));
		return true;
	}

	let pooled = POOL.with_borrow_mut(|pool| {
		let index = pool
			.iter()
			.position(|pooled| pooled.to_bytes().eq_ignore_ascii_case(value.to_bytes()))
			.unwrap_or_else(|| {
				pool.push(value.to_owned());
				pool.len() - 1
			});

		pool[index].as_ptr()
	});

	// SAFETY: Every entity the wrappers pool through is a mock entity, which
	// stores its name at this offset.
	unsafe {
		entity
			.byte_add(MOCK_NAME_OFFSET)
			.cast::<sys::string_t>()
			.write(sys::string_t { pszValue: pooled })
	};

	true
}

/// Takes the key values set on this thread other than through pooling: the
/// entity, the key and the value.
pub fn take_key_values() -> Vec<(*mut sys::CBaseEntity, CString, CString)> {
	KEY_VALUES.take()
}

/// A networked vector, stored as three floats.
pub fn vector_prop(name: &'static CStr, offset: usize) -> sys::SendProp {
	prop(
		name,
		sys::SendPropType_DPT_Vector,
		c_int::try_from(offset).unwrap(),
		PropFlags::default(),
		Some(vector_proxy),
	)
}
