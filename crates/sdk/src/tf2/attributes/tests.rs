use super::*;
use crate::datatables::PropFlags;

use crate::datatables::test_support::{
	custom_proxy, direct_table, int16_proxy, int32_proxy, pointer_table, prop, proxies, table,
	table_prop,
};

use crate::entities::test_support::{
	MOCK_EFLAGS_OFFSET, base_entity_fields, data_map, field, leak,
};

use crate::server::test_support::{export, mock_server};
use crate::tf2::weapons::{Weapon, WeaponError};
use crate::{InterfaceFactory, Module};
use sdk_raw::util::mock::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, c_void};
use std::mem::{offset_of, size_of, zeroed};
use std::ptr::{NonNull, null, null_mut};

/// Entries a mock item's list has room for.
const CAPACITY: usize = 32;

/// Where mock items keep their attribute container.
const CONTAINER: usize = offset_of!(sys::CEconEntity, m_AttributeManager);

/// The word of a mock entity holding its datamap.
const DATAMAP_WORD: usize = 1;

/// Where mock items keep their item definition index.
const DEFINITION: usize = ITEM + offset_of!(sys::CEconItemView, m_iItemDefinitionIndex);

/// The word of a mock entity holding its script class descriptor.
const DESCRIPTION_WORD: usize = 2;

/// Where mock entities keep the handle `GetRefEHandle` points to.
const HANDLE_OFFSET: usize = 40;

/// Where mock items keep their `CEconItemView`.
const ITEM: usize = CONTAINER + offset_of!(sys::CAttributeContainer, m_Item);

/// Where mock items keep their runtime list.
const LIST: usize = ITEM + offset_of!(sys::CEconItemView, m_AttributeList);

/// The word of a mock entity holding its networkable.
const NETWORKABLE_WORD: usize = 3;

/// Where mock weapons keep `m_hOwner`, as their datamap declares.
const OWNER_OFFSET: usize = 60;

thread_local! {
	/// The values native `AddAttribute` was called with.
	static ADDED: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
	/// Native methods called on mock entities, by name.
	static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
	/// Makes the named native method also change the first entry's
	/// refundable currency, which it never does itself.
	static DISTURB: Cell<Option<&'static str>> = const { Cell::new(None) };
	static ELEMENT_VTABLE: Cell<*const sys::CEconItemAttribute__bindgen_vtable> = const { Cell::new(null()) };
	static EXPORTED: Cell<bool> = const { Cell::new(false) };
	/// Makes native `RemoveAttribute` do nothing.
	static IGNORE_REMOVE: Cell<bool> = const { Cell::new(false) };
	/// Calls of the containers' `OnAttributeValuesChanged`.
	static NOTIFIES: Cell<usize> = const { Cell::new(0) };
	static PROXIES: Cell<*mut sys::CStandardSendProxies> = const { Cell::new(null_mut()) };
	/// The running schema's attribute names and indices.
	static SCHEMA: RefCell<Vec<(&'static CStr, u16)>> = const { RefCell::new(Vec::new()) };
	/// Added to every value native `AddAttribute` stores.
	static SKEW: Cell<f32> = const { Cell::new(0.0) };
	/// Static item-definition values, by index.
	static STATICS: RefCell<Vec<(u16, f32)>> = const { RefCell::new(Vec::new()) };
}

/// A mock economy item: a zeroed buffer the size of a `CEconEntity`, whose
/// attribute container, item and list sit at the generated offsets, and
/// whose class's send table is built from a [`Spec`]. Its allocations are
/// leaked and only reached through raw pointers, like the engine's objects.
struct Item {
	entity: *mut sys::CBaseEntity,
}

impl Item {
	fn new(spec: Spec, map: *mut sys::datamap_t) -> Self {
		export_game_dll();

		let words = size_of::<sys::CEconEntity>().div_ceil(size_of::<u64>()) + 8;
		let storage = vec![0u64; words].leak().as_mut_ptr();
		let entity = storage.cast::<sys::CBaseEntity>();
		let slot = |field: usize| field / size_of::<usize>();
		let mut vtable =
			vec![unexpected_call as *const (); sdk_raw::entities::GET_DATA_DESC_MAP_SLOT + 2];

		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT + 1] = description as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		))] = networkable as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetRefEHandle
		))] = handle as *const ();

		let class = leak(sys::ServerClass {
			m_pNetworkName: c"CMockItem".as_ptr(),
			m_pTable: send_table(spec),
			m_pNext: null_mut(),
			m_ClassID: 1,
			m_InstanceBaselineIndex: 0,
		});

		let networkable_vtable = Box::leak(unsafe {
			mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerNetworkable_GetServerClass).write(server_class);
					(&raw mut (*vtable).IServerNetworkable_GetClassName).write(class_name);
				},
			)
		});

		let networkable = leak(MockNetworkable {
			vtable_: networkable_vtable,
			class,
		});

		let container_vtable = Box::leak(unsafe {
			mock_vtable::<sys::CAttributeContainer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).CAttributeContainer_OnAttributeValuesChanged).write(notify);
				},
			)
		});

		let element_vtable = Box::leak(unsafe {
			mock_vtable::<sys::CEconItemAttribute__bindgen_vtable>(
				unexpected_call as *const (),
				|_| {},
			)
		});

		ELEMENT_VTABLE.set(element_vtable);

		let elements = (0..CAPACITY)
			.map(|_| unsafe { zeroed::<sys::CEconItemAttribute>() })
			.collect::<Vec<_>>()
			.leak()
			.as_mut_ptr();

		unsafe {
			storage
				.cast::<*const *const ()>()
				.write(vtable.leak().as_ptr());
			storage
				.add(DATAMAP_WORD)
				.cast::<*mut sys::datamap_t>()
				.write(map);
			storage
				.add(DESCRIPTION_WORD)
				.cast::<*mut sys::ScriptClassDesc_t>()
				.write(script_description());
			storage
				.add(NETWORKABLE_WORD)
				.cast::<*mut MockNetworkable>()
				.write(networkable);
			entity.byte_add(HANDLE_OFFSET).cast::<u32>().write(7);

			let container = entity
				.byte_add(CONTAINER)
				.cast::<sys::CAttributeContainer>();
			let list = entity.byte_add(LIST).cast::<sys::CAttributeList>();

			(&raw mut (*container)._base.vtable_).write((&raw const *container_vtable).cast());
			(&raw mut (*container)._base.m_hOuter.m_Value._base.m_Index).write(7);
			(&raw mut (*list).m_pManager).write(container.cast());
			(&raw mut (*list).m_Attributes.m_Memory.m_pMemory).write(elements);
			(&raw mut (*list).m_Attributes.m_Memory.m_nAllocationCount)
				.write(c_int::try_from(CAPACITY).unwrap());
		}

		assert_eq!(MOCK_EFLAGS_OFFSET, 32);

		Self { entity }
	}

	/// Empties the runtime list.
	fn clear(&self) {
		unsafe { (&raw mut (*self.list()).m_Attributes.m_Size).write(0) };
	}

	fn entity(&self) -> Entity<'_> {
		unsafe { Entity::from_raw(NonNull::new(self.entity).unwrap()) }
	}

	/// The list's entries, as indices and values.
	fn entries(&self) -> Vec<(u16, f32)> {
		unsafe { entries(self.list()) }
	}

	fn list(&self) -> *mut sys::CAttributeList {
		unsafe { self.entity.byte_add(LIST).cast() }
	}

	/// Sets or appends an entry, as `SetRuntimeAttributeValue` does.
	fn push(&self, index: u16, value: f32) {
		unsafe { set_runtime(self.list(), index, value) };
	}

	fn set_definition(&self, index: u16) {
		unsafe { self.entity.byte_add(DEFINITION).cast::<u16>().write(index) };
	}

	fn set_eflags(&self, flags: c_int) {
		unsafe {
			self.entity
				.byte_add(MOCK_EFLAGS_OFFSET)
				.cast::<c_int>()
				.write(flags)
		};
	}
}

#[repr(C)]
struct MockNetworkable {
	vtable_: *const sys::IServerNetworkable__bindgen_vtable,
	class: *mut sys::ServerClass,
}

#[repr(C)]
struct MockPlayer {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	description: *mut sys::ScriptClassDesc_t,
	padding: usize,
	flags: i32,
	value: Cell<f32>,
	duration: Cell<f32>,
	calls: Cell<usize>,
}

/// The networked offsets a mock item's class reports, each relative to its
/// own table as the game's are, and how its vector is described.
#[derive(Clone, Copy)]
struct Spec {
	container: c_int,
	/// The refundable currency's offset, or `None` to leave it unnetworked.
	currency: Option<c_int>,
	definition: c_int,
	entries: usize,
	/// Makes the last entry's property nest another table.
	foreign_element: bool,
	index: c_int,
	index_proxy: sys::SendVarProxyFn,
	item: c_int,
	item_proxy: sys::SendTableProxyFn,
	list: c_int,
	outer: c_int,
	/// The vector's element size, as its extra data gives it.
	stride: c_int,
	value: c_int,
	/// The vector's offset within the list, as its extra data gives it.
	vector: c_int,
}

impl Default for Spec {
	fn default() -> Self {
		let offset = |offset: usize| c_int::try_from(offset).unwrap();

		Self {
			container: offset(CONTAINER),
			currency: Some(offset(offset_of!(
				sys::CEconItemAttribute,
				m_nRefundableCurrency
			))),
			definition: offset(offset_of!(sys::CEconItemView, m_iItemDefinitionIndex)),
			entries: MAX_RUNTIME_ATTRIBUTES,
			foreign_element: false,
			index: offset(offset_of!(
				sys::CEconItemAttribute,
				m_iAttributeDefinitionIndex
			)),
			index_proxy: Some(int16_proxy),
			item: offset(offset_of!(sys::CAttributeContainer, m_Item)),
			item_proxy: Some(direct_table),
			list: offset(offset_of!(sys::CEconItemView, m_AttributeList)),
			outer: offset(offset_of!(sys::CAttributeManager, m_hOuter)),
			stride: offset(size_of::<sys::CEconItemAttribute>()),
			value: offset(offset_of!(sys::CEconItemAttribute, m_flValue)),
			vector: offset(offset_of!(sys::CAttributeList, m_Attributes)),
		}
	}
}

/// `CSendPropExtra_UtlVector`, as `SendPropUtlVector` shares it between the
/// properties of a vector's table.
#[repr(C)]
struct UtlVectorExtra {
	functions: [*const c_void; 3],
	element_stride: c_int,
	offset: c_int,
	max_elements: c_int,
}

/// The native methods of `CEconEntity`'s script descriptor, as the game
/// implements them on the list (`econ_entity.h`, `econ_item_view.cpp`).
unsafe extern "C" fn adapter(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	_: i32,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	let list = unsafe { object.byte_add(LIST).cast::<sys::CAttributeList>() };
	let index = || {
		let name = unsafe { CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString) };

		SCHEMA.with_borrow(|schema| {
			schema
				.iter()
				.find(|(known, _)| *known == name)
				.map(|&(_, index)| index)
		})
	};

	let method = match function.val_0 {
		0 => "AddAttribute",
		1 => "RemoveAttribute",
		2 => "GetAttribute",
		3 => "ReapplyProvision",
		_ => unreachable!(),
	};

	CALLS.with_borrow_mut(|calls| calls.push(method));

	match function.val_0 {
		0 => {
			let value = unsafe { (*arguments.add(1)).__bindgen_anon_1.m_float };

			ADDED.with_borrow_mut(|added| added.push(value));

			if let Some(index) = index() {
				unsafe { set_runtime(list, index, value + SKEW.get()) };
			}
		}

		1 => {
			if let Some(index) = index()
				&& !IGNORE_REMOVE.get()
			{
				unsafe { remove_runtime(list, index) };
			}
		}

		2 => {
			let fallback = unsafe { (*arguments.add(1)).__bindgen_anon_1.m_float };
			let value = index()
				.and_then(|index| {
					unsafe { entries(list) }
						.into_iter()
						.find(|&(entry, _)| entry == index)
						.map(|(_, value)| value)
						.or_else(|| {
							STATICS.with_borrow(|statics| {
								statics
									.iter()
									.find(|&&(entry, _)| entry == index)
									.map(|&(_, value)| value)
							})
						})
				})
				.unwrap_or(fallback);

			unsafe { result.write(binding::float(value)) };
		}

		_ => {}
	}

	if DISTURB.get() == Some(method) {
		unsafe {
			let memory = (*list).m_Attributes.m_Memory.m_pMemory;

			if (*list).m_Attributes.m_Size > 0 {
				(*memory).m_nRefundableCurrency.m_Value += 1;
			}
		}
	}

	true
}

#[test]
fn attribute_sets_check_room_before_writing_anything() {
	let scope = ();
	let server = mock_server(&scope);
	let token = unsafe { trust_shipped_schema(server) };
	let item = Item::new(Spec::default(), item_map());
	let attributes = ItemAttributes::new(server, item.entity()).unwrap();

	schema(&[(c"damage bonus", 2), (c"critboost on kill", 31)]);

	let mut set = AttributeSet::new()
		.with(&catalog::DAMAGE_BONUS, multiplier(2.0))
		.unwrap()
		.with(&catalog::CRITBOOST_ON_KILL, Seconds::new(3.0).unwrap())
		.unwrap();

	// Inserting a definition again replaces its value.
	set.insert(&catalog::DAMAGE_BONUS, multiplier(2.5)).unwrap();
	assert_eq!(set.len(), 2);
	assert_eq!(
		set.insert(&catalog::DAMAGE_BONUS, multiplier(20.0)),
		Err(AttributeError::OutOfDomain)
	);

	for index in 100..119 {
		item.push(index, 1.0);
	}

	assert_eq!(
		set.apply(token, attributes),
		Err(AttributeError::RuntimeListFull)
	);
	assert!(take_calls().is_empty());

	item.clear();
	set.apply(token, attributes).unwrap();
	assert_eq!(item.entries(), [(2, 2.5), (31, 3.0)]);

	assert!(set.remove(&catalog::DAMAGE_BONUS));
	assert!(!set.remove(&catalog::DAMAGE_BONUS));
	assert_eq!(set.len(), 1);
	assert!(!set.is_empty());
}

#[test]
fn attribute_sets_stop_at_the_first_failure_and_keep_earlier_entries() {
	let scope = ();
	let server = mock_server(&scope);
	let token = unsafe { trust_shipped_schema(server) };
	let item = Item::new(Spec::default(), item_map());
	let attributes = ItemAttributes::new(server, item.entity()).unwrap();
	let set = AttributeSet::new()
		.with(&catalog::DAMAGE_BONUS, multiplier(2.0))
		.unwrap()
		.with(&catalog::CLIP_SIZE_BONUS, multiplier(2.0))
		.unwrap()
		.with(&catalog::CRITBOOST_ON_KILL, Seconds::new(3.0).unwrap())
		.unwrap();

	// The running schema lacks the second attribute.
	schema(&[(c"damage bonus", 2), (c"critboost on kill", 31)]);
	assert_eq!(
		set.apply(token, attributes),
		Err(AttributeError::UnknownAttribute)
	);
	assert_eq!(item.entries(), [(2, 2.0)]);

	// It renumbers the second attribute, whose write is undone.
	item.clear();
	schema(&[
		(c"damage bonus", 2),
		(c"clip size bonus", 3),
		(c"critboost on kill", 31),
	]);
	assert_eq!(
		set.apply(token, attributes),
		Err(AttributeError::SchemaMismatch {
			expected: index(4),
			found: index(3),
		})
	);
	assert_eq!(item.entries(), [(2, 2.0)]);
}

unsafe extern "C" fn class_name(_: *const sys::IServerNetworkable) -> *const c_char {
	c"tf_weapon_mock".as_ptr()
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	unsafe {
		entity
			.cast::<*mut sys::datamap_t>()
			.add(DATAMAP_WORD)
			.read()
	}
}

unsafe extern "C" fn description(entity: *mut sys::CBaseEntity) -> *mut sys::ScriptClassDesc_t {
	unsafe {
		entity
			.cast::<*mut sys::ScriptClassDesc_t>()
			.add(DESCRIPTION_WORD)
			.read()
	}
}

#[test]
fn effective_values_provision_and_unchecked_writes_go_through_native_methods() {
	let scope = ();
	let server = mock_server(&scope);
	let token = unsafe { trust_shipped_schema(server) };
	let item = Item::new(Spec::default(), item_map());
	let attributes = ItemAttributes::new(server, item.entity()).unwrap();

	schema(&[
		(c"damage bonus", 2),
		(c"provide on active", 128),
		(c"custom attr", 900),
		(c"other attr", 901),
	]);
	STATICS.set(vec![(2, 1.5), (128, 0.5)]);

	// Static values apply until a runtime entry overrides them.
	assert_eq!(
		attributes.get(token, &catalog::DAMAGE_BONUS),
		Ok(Some(multiplier(1.5)))
	);
	assert_eq!(attributes.runtime_value(&catalog::DAMAGE_BONUS), Ok(None));
	item.push(2, 3.0);
	assert_eq!(
		attributes.get(token, &catalog::DAMAGE_BONUS),
		Ok(Some(multiplier(3.0)))
	);
	assert_eq!(
		attributes.runtime_value(&catalog::DAMAGE_BONUS),
		Ok(Some(multiplier(3.0)))
	);
	assert_eq!(attributes.get(token, &catalog::CLIP_SIZE_BONUS), Ok(None));
	assert_eq!(
		attributes.get(token, &catalog::PROVIDE_ON_ACTIVE),
		Err(AttributeError::InvalidValue)
	);

	let notifies = NOTIFIES.get();

	attributes.refresh().unwrap();
	assert_eq!(NOTIFIES.get(), notifies + 1);
	take_calls();
	attributes.reapply_provision(token).unwrap();
	assert_eq!(take_calls(), ["ReapplyProvision"]);

	item.set_definition(127);
	assert_eq!(
		attributes.definition(),
		Ok(crate::tf2::weapons::ItemDefinitionIndex::new(127))
	);
	item.set_definition(u16::MAX);
	assert_eq!(attributes.definition(), Ok(None));

	// Unchecked writes report the entry the game changed, if any.
	let entry = |index: u16, value: f32| RuntimeAttribute {
		index: self::index(index),
		bits: value.to_bits(),
		refundable_currency: 0,
	};

	assert_eq!(
		unsafe { attributes.set_by_name_unchecked(c"custom attr", 3.0) },
		Ok(Some(entry(900, 3.0)))
	);
	assert_eq!(
		unsafe { attributes.set_by_name_unchecked(c"custom attr", 4.0) },
		Ok(Some(entry(900, 4.0)))
	);
	assert_eq!(
		unsafe { attributes.set_by_name_unchecked(c"custom attr", 4.0) },
		Ok(None)
	);
	assert_eq!(
		unsafe { attributes.set_by_name_unchecked(c"missing attr", 4.0) },
		Ok(None)
	);
	assert_eq!(
		unsafe { attributes.set_by_name_unchecked(c"custom attr", f32::NAN) },
		Err(AttributeError::InvalidValue)
	);
	assert_eq!(item.entries(), [(2, 3.0), (900, 4.0)]);

	// The 21st entry is undone rather than left unnetworked.
	for index in 100..118 {
		item.push(index, 1.0);
	}

	let full = item.entries();

	assert_eq!(full.len(), MAX_RUNTIME_ATTRIBUTES);
	assert_eq!(
		unsafe { attributes.set_by_name_unchecked(c"other attr", 1.0) },
		Err(AttributeError::RuntimeListFull)
	);
	assert_eq!(item.entries(), full);
	assert_eq!(
		attributes.runtime().unwrap()[1],
		entry(900, 4.0),
		"runtime copies entries in list order"
	);
}

/// A list's entries, as indices and values.
unsafe fn entries(list: *mut sys::CAttributeList) -> Vec<(u16, f32)> {
	unsafe {
		let memory = (*list).m_Attributes.m_Memory.m_pMemory;
		let len = usize::try_from((*list).m_Attributes.m_Size).unwrap();

		(0..len)
			.map(|position| {
				let entry = memory.add(position);

				(
					(*entry).m_iAttributeDefinitionIndex.m_Value,
					(*entry).m_flValue.m_Value,
				)
			})
			.collect()
	}
}

/// Exports a game DLL whose standard send proxies the mock tables use, once
/// per thread.
fn export_game_dll() {
	unsafe extern "C" fn standard_send_proxies(
		_: *mut sys::IServerGameDLL,
	) -> *mut sys::CStandardSendProxies {
		PROXIES.get()
	}

	if EXPORTED.replace(true) {
		return;
	}

	PROXIES.set(leak(proxies(null_mut())));

	let vtable = Box::leak(unsafe {
		mock_vtable::<sys::IServerGameDLL__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerGameDLL_GetStandardSendProxies).write(standard_send_proxies);
		})
	});

	export(
		Module::GameServer,
		c"ServerGameDLL012",
		leak(sys::IServerGameDLL { vtable_: vtable }),
	);
}

unsafe extern "C" fn handle(entity: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
	unsafe { entity.byte_add(HANDLE_OFFSET).cast() }
}

fn index(raw: u16) -> AttributeIndex {
	AttributeIndex::new(raw).unwrap()
}

/// A datamap chain declaring a `CEconEntity`.
fn item_map() -> *mut sys::datamap_t {
	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());

	data_map(c"CEconEntity", vec![], base)
}

#[test]
fn items_need_their_networked_layout_to_match_the_generated_one() {
	let scope = ();
	let server = mock_server(&scope);
	let item = Item::new(Spec::default(), item_map());
	let attributes = ItemAttributes::new(server, item.entity()).unwrap();

	assert_eq!(attributes.runtime(), Ok(vec![]));
	assert_eq!(attributes.entity(), item.entity());

	let corruptions: [fn(&mut Spec); 15] = [
		|spec| spec.container += 8,
		|spec| spec.currency = spec.currency.map(|offset| offset + 4),
		// An entry field the entry's table does not network.
		|spec| spec.currency = None,
		|spec| spec.definition += 2,
		|spec| spec.entries -= 1,
		|spec| spec.foreign_element = true,
		|spec| spec.index += 2,
		// An entry field stored with another width.
		|spec| spec.index_proxy = Some(int32_proxy),
		|spec| spec.item += 8,
		|spec| spec.item_proxy = Some(pointer_table),
		|spec| spec.list += 8,
		|spec| spec.outer += 4,
		|spec| spec.stride += 8,
		|spec| spec.value += 4,
		|spec| spec.vector += 8,
	];

	for corrupt in corruptions {
		let mut spec = Spec::default();

		corrupt(&mut spec);

		let item = Item::new(spec, item_map());

		assert_eq!(
			ItemAttributes::new(server, item.entity()).err(),
			Some(AttributeError::UnsupportedLayout)
		);
	}

	// Each read checks that the list belongs to this item's container.
	let list = item.list();
	let container = unsafe { (&raw const (*list).m_pManager).read() };

	unsafe { (&raw mut (*list).m_pManager).write(container.byte_add(8)) };
	assert_eq!(attributes.runtime(), Err(AttributeError::Unlinked));
	assert_eq!(attributes.refresh(), Err(AttributeError::Unlinked));

	// As `CAttributeList::operator=` leaves a copied list.
	unsafe { (&raw mut (*list).m_pManager).write(null_mut()) };
	assert_eq!(attributes.runtime(), Err(AttributeError::Unlinked));
	unsafe { (&raw mut (*list).m_pManager).write(container) };

	let outer = unsafe {
		item.entity
			.byte_add(CONTAINER)
			.cast::<sys::CAttributeManager>()
	};

	unsafe { (&raw mut (*outer).m_hOuter.m_Value._base.m_Index).write(8) };
	assert_eq!(attributes.runtime(), Err(AttributeError::Unlinked));
	unsafe { (&raw mut (*outer).m_hOuter.m_Value._base.m_Index).write(7) };

	// Implausible counts and storage are refused rather than read.
	let memory = unsafe { (&raw const (*list).m_Attributes.m_Memory.m_pMemory).read() };

	for (size, pointer) in [
		(-1, memory),
		(c_int::try_from(CAPACITY).unwrap() + 1, memory),
		(1, null_mut()),
	] {
		unsafe {
			(&raw mut (*list).m_Attributes.m_Size).write(size);
			(&raw mut (*list).m_Attributes.m_Memory.m_pMemory).write(pointer);
		}

		assert_eq!(attributes.runtime(), Err(AttributeError::UnsupportedLayout));
	}

	unsafe { (&raw mut (*list).m_Attributes.m_Memory.m_pMemory).write(memory) };
	item.clear();

	// Entries must all be `CEconItemAttribute`s.
	item.push(2, 1.0);
	item.push(3, 1.0);
	unsafe { (&raw mut (*memory.add(1)).vtable_).write(null()) };
	assert_eq!(attributes.runtime(), Err(AttributeError::UnsupportedLayout));
	item.clear();

	let factory = InterfaceFactory::new(no_interfaces);
	let other_game = unsafe { Server::new(factory, factory, Game::SourceSdk2013, &scope) };

	assert_eq!(
		ItemAttributes::new(other_game, item.entity()).err(),
		Some(AttributeError::UnsupportedGame)
	);

	// Without the game DLL's interface, the layout cannot be checked.
	let bare = unsafe { Server::new(factory, factory, Game::TeamFortress2, &scope) };

	assert_eq!(
		ItemAttributes::new(bare, item.entity()).err(),
		Some(AttributeError::Unavailable)
	);

	let plain = Item::new(
		Spec::default(),
		data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut()),
	);

	assert_eq!(
		ItemAttributes::new(server, plain.entity()).err(),
		Some(AttributeError::UnsupportedEntity)
	);
}

/// Leaks a table of `props`.
fn leak_table(name: &'static CStr, props: Vec<sys::SendProp>) -> *mut sys::SendTable {
	leak(table(name, props.leak()))
}

fn multiplier(factor: f32) -> Multiplier {
	Multiplier::new(factor).unwrap()
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	unsafe {
		entity
			.cast::<*mut sys::IServerNetworkable>()
			.add(NETWORKABLE_WORD)
			.read()
	}
}

unsafe extern "C" fn no_interfaces(_: *const c_char, _: *mut c_int) -> *mut c_void {
	null_mut()
}

unsafe extern "C" fn notify(_: *mut sys::CAttributeContainer) {
	NOTIFIES.set(NOTIFIES.get() + 1);
}

unsafe extern "C" fn player_adapter(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	_: i32,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	let object = unsafe { &*object.cast::<MockPlayer>() };
	object.calls.set(object.calls.get() + 1);
	let name = unsafe { CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString) };

	match function.val_0 {
		0 => {
			let fallback = unsafe { (*arguments.add(1)).__bindgen_anon_1.m_float };
			let value = if name == c"move speed bonus" && !object.value.get().is_nan() {
				object.value.get()
			} else {
				fallback
			};

			unsafe { result.write(binding::float(value)) };
		}

		1 => {
			assert!(result.is_null());

			if name == c"move speed bonus" {
				object
					.value
					.set(unsafe { (*arguments.add(1)).__bindgen_anon_1.m_float });
				object
					.duration
					.set(unsafe { (*arguments.add(2)).__bindgen_anon_1.m_float });
			}
		}

		2 => {
			assert!(result.is_null());
			object.value.set(f32::NAN);
		}

		_ => unreachable!(),
	}

	true
}

#[test]
fn player_attributes_dispatch_typed_methods_and_reject_invalid_values() {
	let names = [
		c"GetCustomAttribute",
		c"AddCustomAttribute",
		c"RemoveCustomAttribute",
	];
	let mut parameters = [
		vec![binding::STRING, binding::FLOAT],
		vec![binding::STRING, binding::FLOAT, binding::FLOAT],
		vec![binding::STRING],
	];
	let mut functions: [sys::ScriptFunctionBinding_t; 3] = unsafe { zeroed() };

	for (i, function) in functions.iter_mut().enumerate() {
		function.m_desc.m_pszScriptName = names[i].as_ptr();
		function.m_desc.m_ReturnType = if i == 0 {
			binding::FLOAT
		} else {
			binding::VOID
		};
		function.m_desc.m_Parameters = vector(&mut parameters[i]);
		function.m_flags = 1;
		function.m_pfnBinding = Some(player_adapter);
		function.m_pFunction.val_0 = i as isize;
	}

	let mut description: sys::ScriptClassDesc_t = unsafe { zeroed() };

	description.m_pszClassname = c"CTFPlayer".as_ptr();
	description.m_FunctionBindings = vector(&mut functions);

	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
	let map = data_map(c"CTFPlayer", vec![], base);
	let mut table = [null(); 16];

	table[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = player_datamap as *const ();
	table[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT + 1] = player_description as *const ();

	let mut object = MockPlayer {
		vtable: table.as_ptr(),
		map,
		description: &raw mut description,
		padding: 0,
		flags: 0,
		value: Cell::new(f32::NAN),
		duration: Cell::new(0.0),
		calls: Cell::new(0),
	};

	assert_eq!(offset_of!(MockPlayer, flags), MOCK_EFLAGS_OFFSET);

	let scope = ();
	let server = mock_server(&scope);
	let token = unsafe { trust_shipped_schema(server) };
	let entity = unsafe { Entity::from_raw(NonNull::from(&mut object).cast()) };
	let attributes = PlayerAttributes::new(server, entity).unwrap();

	assert_eq!(attributes.player(), entity);
	assert_eq!(attributes.get(token, c"move speed bonus"), Ok(None));
	// SAFETY: The mock schema implements this numeric attribute only.
	assert!(unsafe { attributes.set_unchecked(c"move speed bonus", 1.5) }.unwrap());
	assert_eq!(attributes.get(token, c"move speed bonus"), Ok(Some(1.5)));
	assert_eq!(object.duration.get(), -1.0);
	assert!(!unsafe { attributes.set_unchecked(c"unknown", 1.5) }.unwrap());

	let calls = object.calls.get();

	assert_eq!(
		unsafe { attributes.set_unchecked(c"move speed bonus", f32::NAN) },
		Err(AttributeError::InvalidValue)
	);
	assert_eq!(
		unsafe { attributes.set_for_unchecked(c"move speed bonus", 2.0, Some(0.0)) },
		Err(AttributeError::InvalidValue)
	);
	assert_eq!(object.calls.get(), calls);
	assert!(unsafe { attributes.set_for_unchecked(c"move speed bonus", 2.0, Some(5.0)) }.unwrap());
	assert_eq!(object.duration.get(), 5.0);
	attributes.remove(token, c"move speed bonus").unwrap();
	assert_eq!(attributes.get(token, c"move speed bonus"), Ok(None));

	// Items are not players.
	let item = Item::new(Spec::default(), item_map());

	assert_eq!(
		PlayerAttributes::new(server, item.entity()).err(),
		Some(AttributeError::UnsupportedEntity)
	);
	assert_eq!(
		ItemAttributes::new(server, entity).err(),
		Some(AttributeError::UnsupportedEntity)
	);
}

unsafe extern "C" fn player_datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	unsafe { (*entity.cast::<MockPlayer>()).map }
}

unsafe extern "C" fn player_description(
	entity: *mut sys::CBaseEntity,
) -> *mut sys::ScriptClassDesc_t {
	unsafe { (*entity.cast::<MockPlayer>()).description }
}

#[test]
fn removal_needs_an_entry_and_reports_which_was_removed() {
	let scope = ();
	let server = mock_server(&scope);
	let item = Item::new(Spec::default(), item_map());
	let attributes = ItemAttributes::new(server, item.entity()).unwrap();

	schema(&[(c"damage bonus", 2)]);

	// Without an entry, nothing is called.
	assert_eq!(attributes.remove(&catalog::DAMAGE_BONUS), Ok(false));
	assert!(take_calls().is_empty());

	item.push(1, 0.5);
	item.push(2, 2.0);
	item.push(3, 0.5);
	assert_eq!(attributes.remove(&catalog::DAMAGE_BONUS), Ok(true));
	assert_eq!(take_calls(), ["RemoveAttribute"]);
	assert_eq!(item.entries(), [(1, 0.5), (3, 0.5)]);

	// A renumbered name removes another definition's entry.
	item.push(2, 2.0);
	schema(&[(c"damage bonus", 1)]);
	assert_eq!(
		attributes.remove(&catalog::DAMAGE_BONUS),
		Err(AttributeError::SchemaMismatch {
			expected: index(2),
			found: index(1),
		})
	);
	assert_eq!(item.entries(), [(3, 0.5), (2, 2.0)]);

	schema(&[]);
	assert_eq!(
		attributes.remove(&catalog::DAMAGE_BONUS),
		Err(AttributeError::UnknownAttribute)
	);

	// Any runtime attribute can be removed by name, and is reported.
	schema(&[(c"custom attr", 900)]);
	item.clear();
	item.push(2, 2.0);
	item.push(900, 4.0);
	item.push(3, 0.5);
	take_calls();
	assert_eq!(
		attributes.remove_by_name(c"custom attr"),
		Ok(Some(RuntimeAttribute {
			index: index(900),
			bits: 4.0f32.to_bits(),
			refundable_currency: 0,
		}))
	);
	assert_eq!(take_calls(), ["RemoveAttribute"]);
	assert_eq!(item.entries(), [(2, 2.0), (3, 0.5)]);
	assert_eq!(attributes.remove_by_name(c"custom attr"), Ok(None));
	assert_eq!(attributes.remove_by_name(c"missing attr"), Ok(None));

	item.set_eflags(1);
	assert_eq!(
		attributes.remove(&catalog::DAMAGE_BONUS),
		Err(AttributeError::MarkedForDeletion)
	);
	assert_eq!(
		attributes.remove_by_name(c"custom attr"),
		Err(AttributeError::MarkedForDeletion)
	);
	item.set_eflags(0);
}

/// Removes an entry, as `CAttributeList::RemoveAttribute` does.
unsafe fn remove_runtime(list: *mut sys::CAttributeList, index: u16) {
	unsafe {
		let memory = (*list).m_Attributes.m_Memory.m_pMemory;
		let len = usize::try_from((*list).m_Attributes.m_Size).unwrap();

		if let Some(position) = (0..len)
			.find(|&position| (*memory.add(position)).m_iAttributeDefinitionIndex.m_Value == index)
		{
			std::ptr::copy(
				memory.add(position + 1),
				memory.add(position),
				len - position - 1,
			);
			(*list).m_Attributes.m_Size -= 1;
		}
	}
}

#[test]
fn safe_writes_check_the_schema_index_and_undo_mismatches() {
	let scope = ();
	let server = mock_server(&scope);
	let token = unsafe { trust_shipped_schema(server) };
	let item = Item::new(Spec::default(), item_map());
	let attributes = ItemAttributes::new(server, item.entity()).unwrap();

	// "fire rate bonus" is renumbered to the penalty's index 5.
	schema(&[(c"damage bonus", 2), (c"fire rate bonus", 5)]);

	attributes
		.set(token, &catalog::DAMAGE_BONUS, multiplier(2.0))
		.unwrap();
	assert_eq!(item.entries(), [(2, 2.0)]);
	assert_eq!(take_calls(), ["AddAttribute"]);

	// The same value again is confirmed with another value first.
	attributes
		.set(token, &catalog::DAMAGE_BONUS, multiplier(2.0))
		.unwrap();
	assert_eq!(take_calls(), ["AddAttribute", "AddAttribute"]);
	assert_eq!(item.entries(), [(2, 2.0)]);

	attributes
		.set(token, &catalog::DAMAGE_BONUS, multiplier(3.0))
		.unwrap();
	assert_eq!(item.entries(), [(2, 3.0)]);
	take_calls();

	// Values outside the definition's bounds never reach the game.
	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_BONUS, multiplier(0.5)),
		Err(AttributeError::OutOfDomain)
	);
	assert!(take_calls().is_empty());

	assert_eq!(
		attributes.set(token, &catalog::CLIP_SIZE_BONUS, multiplier(2.0)),
		Err(AttributeError::UnknownAttribute)
	);
	assert_eq!(take_calls(), ["AddAttribute"]);

	// A renumbered name's new entry is removed again.
	let mismatch = Err(AttributeError::SchemaMismatch {
		expected: index(6),
		found: index(5),
	});
	let notifies = NOTIFIES.get();

	assert_eq!(
		attributes.set(token, &catalog::FIRE_RATE_BONUS, multiplier(0.5)),
		mismatch
	);
	assert_eq!(take_calls(), ["AddAttribute", "RemoveAttribute"]);
	assert_eq!(item.entries(), [(2, 3.0)]);

	// Should native removal leave it, it is dropped directly.
	IGNORE_REMOVE.set(true);
	assert_eq!(
		attributes.set(token, &catalog::FIRE_RATE_BONUS, multiplier(0.5)),
		mismatch
	);
	IGNORE_REMOVE.set(false);
	assert_eq!(item.entries(), [(2, 3.0)]);
	assert_eq!(NOTIFIES.get(), notifies + 1);
	take_calls();

	// A renumbered name selecting an existing entry gets its value back.
	item.push(5, 2.0);
	assert_eq!(
		attributes.set(token, &catalog::FIRE_RATE_BONUS, multiplier(0.5)),
		mismatch
	);
	assert_eq!(take_calls(), ["AddAttribute"]);
	assert_eq!(item.entries(), [(2, 3.0), (5, 2.0)]);
	assert_eq!(NOTIFIES.get(), notifies + 2);

	// Other stored bits are undone too, for new and existing entries.
	SKEW.set(1.0);
	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_BONUS, multiplier(4.0)),
		Err(AttributeError::Rejected)
	);
	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_PENALTY, multiplier(0.5)),
		Err(AttributeError::UnknownAttribute)
	);
	schema(&[(c"damage penalty", 1)]);
	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_PENALTY, multiplier(0.5)),
		Err(AttributeError::Rejected)
	);
	SKEW.set(0.0);
	assert_eq!(item.entries(), [(2, 3.0), (5, 2.0)]);
	take_calls();

	// A full list takes no new definitions, but existing ones still change.
	schema(&[(c"damage bonus", 2)]);
	item.clear();

	for index in 100..120 {
		item.push(index, 1.0);
	}

	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_BONUS, multiplier(2.0)),
		Err(AttributeError::RuntimeListFull)
	);
	assert!(take_calls().is_empty());

	item.clear();
	item.push(2, 1.5);

	for index in 100..119 {
		item.push(index, 1.0);
	}

	attributes
		.set(token, &catalog::DAMAGE_BONUS, multiplier(2.0))
		.unwrap();
	assert_eq!(item.entries()[0], (2, 2.0));

	item.set_eflags(1);
	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_BONUS, multiplier(2.5)),
		Err(AttributeError::MarkedForDeletion)
	);
	item.set_eflags(0);
}

/// Sets the running schema's attribute names and indices.
fn schema(attributes: &[(&'static CStr, u16)]) {
	SCHEMA.set(attributes.to_vec());
}

/// A script class descriptor for `CEconEntity` with its four native
/// attribute methods, which [`adapter`] implements.
fn script_description() -> *mut sys::ScriptClassDesc_t {
	let names = [
		c"AddAttribute",
		c"RemoveAttribute",
		c"GetAttribute",
		c"ReapplyProvision",
	];
	let parameters = [
		vec![binding::STRING, binding::FLOAT, binding::FLOAT],
		vec![binding::STRING],
		vec![binding::STRING, binding::FLOAT],
		vec![],
	];
	let returns = [binding::VOID, binding::VOID, binding::FLOAT, binding::VOID];
	let functions = (0..names.len())
		.map(|i| {
			let mut function: sys::ScriptFunctionBinding_t = unsafe { zeroed() };

			function.m_desc.m_pszScriptName = names[i].as_ptr();
			function.m_desc.m_ReturnType = returns[i];
			function.m_desc.m_Parameters = vector(parameters[i].clone().leak());
			function.m_flags = 1;
			function.m_pfnBinding = Some(adapter);
			function.m_pFunction.val_0 = i as isize;
			function
		})
		.collect::<Vec<_>>();

	let mut description: sys::ScriptClassDesc_t = unsafe { zeroed() };

	description.m_pszClassname = c"CEconEntity".as_ptr();
	description.m_FunctionBindings = vector(functions.leak());
	leak(description)
}

/// Leaks the send table of a mock item class laid out as `spec` says.
fn send_table(spec: Spec) -> *mut sys::SendTable {
	let unsigned = |name: &'static CStr, offset: c_int, proxy: sys::SendVarProxyFn| {
		prop(
			name,
			sys::SendPropType_DPT_Int,
			offset,
			PropFlags::UNSIGNED,
			proxy,
		)
	};

	let entry_props = || {
		let mut props = vec![
			unsigned(c"m_iAttributeDefinitionIndex", spec.index, spec.index_proxy),
			unsigned(c"m_iRawValue32", spec.value, Some(int32_proxy)),
		];

		props.extend(
			spec.currency
				.map(|offset| unsigned(c"m_nRefundableCurrency", offset, Some(int32_proxy))),
		);

		props
	};

	let entry = leak_table(c"DT_ScriptCreatedAttribute", entry_props());
	let foreign = leak_table(c"DT_ScriptCreatedAttribute", entry_props());

	// `SendPropUtlVector` shares this between the vector's properties.
	let extra = leak(UtlVectorExtra {
		functions: [null(); 3],
		element_stride: spec.stride,
		offset: spec.vector,
		max_elements: c_int::try_from(MAX_RUNTIME_ATTRIBUTES).unwrap(),
	})
	.cast_const()
	.cast::<c_void>();

	let mut length_prop = unsigned(c"lengthprop20", 0, Some(custom_proxy));

	length_prop.m_pExtraData = extra;

	let length = leak_table(c"_LPT_m_Attributes_20", vec![length_prop]);
	let mut length_proxy = table_prop(c"lengthproxy", 0, length, Some(pointer_table));

	length_proxy.m_pExtraData = extra;

	let mut entries = vec![length_proxy];

	// Each entry's property holds its position in its element stride.
	entries.extend((0..spec.entries).map(|position| {
		let table = if spec.foreign_element && position + 1 == spec.entries {
			foreign
		} else {
			entry
		};

		let mut prop = table_prop(c"000", 0, table, Some(pointer_table));

		prop.m_pExtraData = extra;
		prop.m_ElementStride = c_int::try_from(position).unwrap();
		prop
	}));

	let entries = leak_table(c"_ST_m_Attributes_20", entries);
	let list = leak_table(
		c"DT_AttributeList",
		vec![table_prop(c"m_Attributes", 0, entries, Some(direct_table))],
	);

	let item = leak_table(
		c"DT_ScriptCreatedItem",
		vec![
			unsigned(
				c"m_iItemDefinitionIndex",
				spec.definition,
				Some(int16_proxy),
			),
			table_prop(c"m_AttributeList", spec.list, list, Some(direct_table)),
		],
	);

	let container = leak_table(
		c"DT_AttributeContainer",
		vec![
			prop(
				c"m_hOuter",
				sys::SendPropType_DPT_Int,
				spec.outer,
				PropFlags::default(),
				Some(custom_proxy),
			),
			table_prop(c"m_Item", spec.item, item, spec.item_proxy),
		],
	);

	let econ = leak_table(
		c"DT_EconEntity",
		vec![table_prop(
			c"m_AttributeManager",
			spec.container,
			container,
			Some(direct_table),
		)],
	);

	leak_table(
		c"DT_MockItem",
		vec![table_prop(c"baseclass", 0, econ, Some(direct_table))],
	)
}

unsafe extern "C" fn server_class(
	networkable: *mut sys::IServerNetworkable,
) -> *mut sys::ServerClass {
	unsafe { (*networkable.cast::<MockNetworkable>()).class }
}

/// Sets or appends an entry, as `CAttributeList::SetRuntimeAttributeValue`
/// does.
unsafe fn set_runtime(list: *mut sys::CAttributeList, index: u16, value: f32) {
	unsafe {
		let memory = (*list).m_Attributes.m_Memory.m_pMemory;
		let len = usize::try_from((*list).m_Attributes.m_Size).unwrap();

		for position in 0..len {
			let entry = memory.add(position);

			if (*entry).m_iAttributeDefinitionIndex.m_Value == index {
				(*entry).m_flValue.m_Value = value;
				return;
			}
		}

		let entry = memory.add(len);

		(*entry).vtable_ = ELEMENT_VTABLE.get();
		(*entry).m_iAttributeDefinitionIndex.m_Value = index;
		(*entry).m_flValue.m_Value = value;
		(*entry).m_nRefundableCurrency.m_Value = 0;
		(*list).m_Attributes.m_Size += 1;
	}
}

#[test]
fn setting_a_held_value_confirms_the_schema_index_with_another_value() {
	let scope = ();
	let server = mock_server(&scope);
	let token = unsafe { trust_shipped_schema(server) };
	let item = Item::new(Spec::default(), item_map());
	let attributes = ItemAttributes::new(server, item.entity()).unwrap();

	schema(&[(c"damage bonus", 2)]);
	item.push(2, 2.0);

	// The lower bound shows the entry the name maps to.
	attributes
		.set(token, &catalog::DAMAGE_BONUS, multiplier(2.0))
		.unwrap();
	assert_eq!(ADDED.take(), [1.0, 2.0]);
	assert_eq!(item.entries(), [(2, 2.0)]);

	// The lower bound itself is confirmed with the upper one.
	attributes
		.set(token, &catalog::DAMAGE_BONUS, multiplier(1.0))
		.unwrap();
	ADDED.take();
	attributes
		.set(token, &catalog::DAMAGE_BONUS, multiplier(1.0))
		.unwrap();
	assert_eq!(ADDED.take(), [10.0, 1.0]);
	assert_eq!(item.entries(), [(2, 1.0)]);

	// A renumbered name is caught even when the catalog's index holds the
	// value, and its new entry undone.
	schema(&[(c"damage bonus", 7)]);
	take_calls();
	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_BONUS, multiplier(1.0)),
		Err(AttributeError::SchemaMismatch {
			expected: index(2),
			found: index(7),
		})
	);
	assert_eq!(take_calls(), ["AddAttribute", "RemoveAttribute"]);
	assert_eq!(item.entries(), [(2, 1.0)]);
}

fn take_calls() -> Vec<&'static str> {
	CALLS.take()
}

#[test]
fn unexpected_list_changes_are_reported_and_left_alone() {
	let scope = ();
	let server = mock_server(&scope);
	let token = unsafe { trust_shipped_schema(server) };
	let item = Item::new(Spec::default(), item_map());
	let attributes = ItemAttributes::new(server, item.entity()).unwrap();
	let currency = || unsafe {
		(*(*item.list()).m_Attributes.m_Memory.m_pMemory)
			.m_nRefundableCurrency
			.m_Value
	};

	schema(&[
		(c"damage bonus", 2),
		(c"fire rate bonus", 5),
		(c"custom attr", 900),
	]);
	item.push(100, 1.0);

	// Each native call below also changes the first entry.
	DISTURB.set(Some("AddAttribute"));
	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_BONUS, multiplier(2.0)),
		Err(AttributeError::UnexpectedChange)
	);
	assert_eq!(
		unsafe { attributes.set_by_name_unchecked(c"custom attr", 1.0) },
		Err(AttributeError::UnexpectedChange)
	);
	assert_eq!(item.entries(), [(100, 1.0), (2, 2.0), (900, 1.0)]);
	assert_eq!(currency(), 2);

	DISTURB.set(Some("RemoveAttribute"));
	assert_eq!(
		attributes.remove(&catalog::DAMAGE_BONUS),
		Err(AttributeError::UnexpectedChange)
	);
	assert_eq!(
		attributes.remove_by_name(c"custom attr"),
		Err(AttributeError::UnexpectedChange)
	);
	assert_eq!(item.entries(), [(100, 1.0)]);

	// Undoing a renumbered name's new entry finds the list changed further.
	let notifies = NOTIFIES.get();

	assert_eq!(
		attributes.set(token, &catalog::FIRE_RATE_BONUS, multiplier(0.5)),
		Err(AttributeError::UnexpectedChange)
	);
	assert_eq!(item.entries(), [(100, 1.0)]);
	assert_eq!(NOTIFIES.get(), notifies, "nothing is truncated");
	assert_eq!(currency(), 5);
	DISTURB.set(None);
}

/// A vector over `values`, which must stay in place while it is used.
fn vector<T>(values: &mut [T]) -> sys::CUtlVector<T, sys::CUtlMemory<T>> {
	let len = c_int::try_from(values.len()).unwrap();
	let elements = values.as_mut_ptr();

	sys::CUtlVector {
		_phantom_0: Default::default(),
		_phantom_1: Default::default(),
		m_Memory: sys::CUtlMemory {
			_phantom_0: Default::default(),
			m_pMemory: elements,
			m_nAllocationCount: len,
			m_nGrowSize: 0,
		},
		m_Size: len,
		m_pElements: elements,
	}
}

/// A datamap chain declaring a TF2 weapon with an `m_hOwner`.
fn weapon_map() -> *mut sys::datamap_t {
	let mut owner = field();

	owner.fieldName = c"m_hOwner".as_ptr();
	owner.fieldType = sys::_fieldtypes_FIELD_EHANDLE;
	owner.fieldOffset[0] = c_int::try_from(OWNER_OFFSET).unwrap();
	owner.fieldSize = 1;
	owner.fieldSizeInBytes = 4;

	let combat = data_map(c"CBaseCombatWeapon", vec![owner], item_map());

	data_map(c"CTFWeaponBase", vec![], combat)
}

#[test]
fn weapons_reach_their_item_attributes() {
	let scope = ();
	let server = mock_server(&scope);
	let token = unsafe { trust_shipped_schema(server) };
	let item = Item::new(Spec::default(), weapon_map());
	let weapon = Weapon::new(server, item.entity()).unwrap();

	schema(&[(c"damage bonus", 2)]);
	item.set_definition(1151);
	assert_eq!(
		weapon.definition().unwrap(),
		crate::tf2::weapons::ItemDefinitionIndex::new(1151)
	);

	weapon
		.attributes()
		.unwrap()
		.set(token, &catalog::DAMAGE_BONUS, multiplier(2.0))
		.unwrap();
	assert_eq!(item.entries(), [(2, 2.0)]);

	let mut owner = field();

	owner.fieldName = c"m_hOwner".as_ptr();
	owner.fieldType = sys::_fieldtypes_FIELD_EHANDLE;
	owner.fieldOffset[0] = c_int::try_from(OWNER_OFFSET).unwrap();

	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
	let combat = data_map(c"CBaseCombatWeapon", vec![owner], base);
	let plain = Item::new(Spec::default(), data_map(c"CTFWeaponBase", vec![], combat));

	plain.set_definition(1151);

	let plain = Weapon::new(server, plain.entity()).unwrap();

	// The definition needs only the networked variables that place it.
	assert_eq!(
		plain.definition().unwrap(),
		crate::tf2::weapons::ItemDefinitionIndex::new(1151)
	);
	assert!(matches!(
		plain.attributes(),
		Err(AttributeError::UnsupportedEntity)
	));

	let unlisted = Item::new(
		Spec {
			stride: 8,
			..Spec::default()
		},
		weapon_map(),
	);

	unlisted.set_definition(1151);

	let unlisted = Weapon::new(server, unlisted.entity()).unwrap();

	assert_eq!(
		unlisted.definition().unwrap(),
		crate::tf2::weapons::ItemDefinitionIndex::new(1151)
	);
	assert!(matches!(
		unlisted.attributes(),
		Err(AttributeError::UnsupportedLayout)
	));

	let misplaced = Item::new(
		Spec {
			definition: Spec::default().definition + 2,
			..Spec::default()
		},
		weapon_map(),
	);

	assert!(matches!(
		Weapon::new(server, misplaced.entity())
			.unwrap()
			.definition(),
		Err(WeaponError::UnsupportedLayout)
	));

	let factory = InterfaceFactory::new(no_interfaces);
	let bare = unsafe { Server::new(factory, factory, Game::TeamFortress2, &scope) };

	assert!(matches!(
		Weapon::new(bare, item.entity()).unwrap().definition(),
		Err(WeaponError::Interface(_))
	));

	item.set_eflags(1);
	assert!(matches!(
		weapon.definition(),
		Err(WeaponError::MarkedForDeletion)
	));
	item.set_eflags(0);
}
