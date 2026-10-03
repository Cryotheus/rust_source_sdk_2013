//! Where an economy item keeps its runtime attributes, checked against the
//! game's own networking tables before any of it is read.
//!
//! The generated bindings give `CEconEntity`'s layout for the target's ABI.
//! [`ItemLayout::validate`] confirms it for the running game: the send tables
//! the game networks the item with must place the attribute container, item,
//! list, the list's vector and every field of a list entry at the generated
//! offsets, and give entries the generated size. Each read then checks that
//! the list belongs to this entity's container, whose address the game itself
//! stored in the list.

use crate::datatables::{NetProp, PropFlags, PropKind, SendProp, SendTable, Storage};
use crate::entities::Entity;
use crate::interfaces::ServerGameDll;

use crate::tf2::attributes::{
	AttributeError, AttributeIndex, MAX_RUNTIME_ATTRIBUTES, RuntimeAttribute,
};

use crate::tf2::weapons::ItemDefinitionIndex;
use std::ffi::{CStr, c_int, c_void};
use std::mem::{offset_of, size_of};

/// `CEconEntity::m_AttributeManager`, the item's `CAttributeContainer`.
const CONTAINER: usize = offset_of!(sys::CEconEntity, m_AttributeManager);

/// `CEconItemView::m_iItemDefinitionIndex`, within the entity.
const DEFINITION: usize = ITEM + offset_of!(sys::CEconItemView, m_iItemDefinitionIndex);

/// `CAttributeContainer::m_Item`, the item's `CEconItemView`, within the entity.
const ITEM: usize = CONTAINER + offset_of!(sys::CAttributeContainer, m_Item);

/// `CEconItemView::m_AttributeList`, the item's runtime `CAttributeList`,
/// within the entity.
const LIST: usize = ITEM + offset_of!(sys::CEconItemView, m_AttributeList);

/// The longest list read. Lists the game builds hold a few dozen entries at
/// most; anything longer is taken as a layout mismatch.
const MAX_LIST_LEN: c_int = 4096;

/// `CAttributeManager::m_hOuter`, the entity the container belongs to, within
/// the entity.
const OUTER: usize = CONTAINER + offset_of!(sys::CAttributeManager, m_hOuter);

/// An entity whose runtime attribute list was found where the generated
/// `CEconEntity` layout puts it.
#[derive(Debug, Clone, Copy)]
pub(super) struct ItemLayout<'s> {
	entity: Entity<'s>,
}

impl<'s> ItemLayout<'s> {
	/// Checks the generated offsets against the networked variables of
	/// `entity`'s class, which must have a `CEconEntity` datamap.
	///
	/// The container, item and list must be nested tables at the generated
	/// offsets, behind proxies that pass their data through unchanged, with
	/// the item's definition index at its generated offset and width. The
	/// list's vector must be networked as [`vector_entries`] describes, with
	/// each entry's index, raw value bits and refundable currency at the
	/// generated offsets of `CEconItemAttribute` and stored with their
	/// generated widths.
	pub(super) fn validate(
		dll: ServerGameDll<'s>,
		entity: Entity<'s>,
	) -> Result<Self, AttributeError> {
		let (container, item) = item_tables(dll, entity)?;

		check_offset(child(container, c"m_hOuter")?, OUTER)?;

		let list = child(item, c"m_AttributeList")?;

		check_table(list, LIST)?;

		let entry = vector_entries(child(list, c"m_Attributes")?)?;

		let proxies = dll
			.standard_send_proxies()
			.ok_or(AttributeError::UnsupportedLayout)?;

		// Entry properties are relative to the entry, which the game reaches
		// through a relocating proxy, so only their offsets and storage are used.
		let field = |name: &CStr, offset: usize, storage: Storage| {
			let prop = NetProp::resolve(entry, name, proxies)
				.map_err(|_| AttributeError::UnsupportedLayout)?;

			check_offset(prop, offset)?;
			check_storage(prop.storage(), storage)
		};

		field(
			c"m_iAttributeDefinitionIndex",
			offset_of!(sys::CEconItemAttribute, m_iAttributeDefinitionIndex),
			Storage::U16,
		)?;

		// The value is networked as its 32 raw bits.
		field(
			c"m_iRawValue32",
			offset_of!(sys::CEconItemAttribute, m_flValue),
			Storage::U32,
		)?;

		field(
			c"m_nRefundableCurrency",
			offset_of!(sys::CEconItemAttribute, m_nRefundableCurrency),
			Storage::I32,
		)?;

		Ok(Self { entity })
	}

	/// The item's raw definition index, `m_iItemDefinitionIndex`.
	pub(super) fn definition_index(self) -> u16 {
		// SAFETY: `validate` checked the field with `item_tables`.
		unsafe { read_definition_index(self.entity) }
	}

	/// The entity whose layout was validated.
	pub(super) const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// The item's `CAttributeList`, checked to belong to its container: the
	/// game sets the list's manager to the container when it initializes the
	/// item's attributes (`CAttributeContainer::InitializeAttributes`), and the
	/// container's outer entity to the item itself.
	fn list(self) -> Result<*mut sys::CAttributeList, AttributeError> {
		let base = self.entity.as_ptr();

		// SAFETY: `validate` placed the container and list at these offsets.
		let (container, list) = unsafe {
			(
				base.byte_add(CONTAINER).cast::<sys::CAttributeContainer>(),
				base.byte_add(LIST).cast::<sys::CAttributeList>(),
			)
		};

		// SAFETY: Both fields lie within the validated container and list, and
		// are read without forming references.
		let (manager, outer) = unsafe {
			(
				(&raw const (*list).m_pManager).read(),
				(&raw const (*container)._base.m_hOuter.m_Value._base.m_Index).read(),
			)
		};

		if manager.cast() != container || outer != self.entity.handle().to_raw() {
			return Err(AttributeError::Unlinked);
		}

		Ok(list)
	}

	/// Tells the item's container that attribute values changed, as the
	/// game's own setters do: it clears the cached results of its own hooks
	/// and those of the entities the item provides to, such as its owner
	/// (`CAttributeManager::ClearCache`, which walks `m_Receivers`), bumps the
	/// parity that makes clients do the same, and marks the item for
	/// networking. It does not iterate any attributes.
	#[doc(alias = "OnAttributeValuesChanged")]
	pub(super) fn notify(self) -> Result<(), AttributeError> {
		let list = self.list()?;

		// SAFETY: `list` confirmed that the game linked this list to the
		// container, which C++ declares as a `CAttributeContainer`
		// (`CEconEntity::m_AttributeManager`). Its embedded network variable
		// class overrides only `NetworkStateChanged`, so the generated slot of
		// `OnAttributeValuesChanged` holds. The call clears caches and flags
		// networking; it neither deletes entities nor calls plugins.
		unsafe {
			let container = (&raw const (*list).m_pManager)
				.read()
				.cast::<sys::CAttributeContainer>();
			let vtable = (&raw const (*container)._base.vtable_)
				.read()
				.cast::<sys::CAttributeContainer__bindgen_vtable>();

			((*vtable).CAttributeContainer_OnAttributeValuesChanged)(container);
		}

		Ok(())
	}

	/// Overwrites the raw value bits of the entry at `position`, which the
	/// last [`Self::snapshot`] returned, without notifying the container.
	///
	/// # Safety
	///
	/// No game code may have run since that snapshot, and the bits must be
	/// valid for the entry's attribute type, such as the bits it held before.
	pub(super) unsafe fn restore_bits(
		self,
		position: usize,
		bits: u32,
	) -> Result<(), AttributeError> {
		let (memory, len) = self.vector()?;

		if position >= len {
			return Err(AttributeError::UnexpectedChange);
		}

		// SAFETY: `vector` bounds the entry within the list's allocation. The
		// caller vouches for the bits, which the game reads as raw storage.
		unsafe {
			(&raw mut (*memory.add(position)).m_flValue.m_Value)
				.cast::<u32>()
				.write(bits)
		};

		Ok(())
	}

	/// Copies the runtime list, as the game networks it, without dispatching
	/// on any attribute's type.
	pub(super) fn snapshot(self) -> Result<Vec<RuntimeAttribute>, AttributeError> {
		let (memory, len) = self.vector()?;
		let mut entries = Vec::with_capacity(len);
		let mut vtable = None;

		for position in 0..len {
			// SAFETY: `vector` bounds the entry within the list's allocation.
			// Fields are copied without forming references.
			let (entry_vtable, index, bits, refundable_currency) = unsafe {
				let entry = memory.add(position);

				(
					(&raw const (*entry).vtable_).read(),
					(&raw const (*entry).m_iAttributeDefinitionIndex.m_Value).read(),
					(&raw const (*entry).m_flValue.m_Value).cast::<u32>().read(),
					(&raw const (*entry).m_nRefundableCurrency.m_Value).read(),
				)
			};

			// Entries are all exactly `CEconItemAttribute`, so share its vtable.
			if entry_vtable.is_null() || *vtable.get_or_insert(entry_vtable) != entry_vtable {
				return Err(AttributeError::UnsupportedLayout);
			}

			entries.push(RuntimeAttribute {
				index: AttributeIndex::new(index).ok_or(AttributeError::UnsupportedLayout)?,
				bits,
				refundable_currency,
			});
		}

		Ok(entries)
	}

	/// Drops entries from the end of the list, without notifying the
	/// container, as `CUtlVector::Remove` does for its last element.
	///
	/// # Safety
	///
	/// No game code may have run since the last [`Self::snapshot`], and the
	/// dropped entries must be ones the plugin's own call just appended.
	pub(super) unsafe fn truncate(self, len: usize) -> Result<(), AttributeError> {
		let list = self.list()?;
		let (_, current) = self.vector()?;

		if len > current {
			return Err(AttributeError::UnexpectedChange);
		}

		// SAFETY: `len` fits, as it is at most the validated current length.
		// `CEconItemAttribute` has no destructor to run, so shrinking the count
		// is all `CUtlVector::Remove` does to a trailing entry.
		unsafe { (&raw mut (*list).m_Attributes.m_Size).write(len as c_int) };

		Ok(())
	}

	/// The list's entries and their count, checked for plausibility. The
	/// entries' size is the one `validate` matched against the vector's
	/// networked element size, so the count bounds them within the vector's
	/// allocation.
	fn vector(self) -> Result<(*mut sys::CEconItemAttribute, usize), AttributeError> {
		let list = self.list()?;

		// SAFETY: The vector lies within the validated list.
		let (memory, allocated, len) = unsafe {
			(
				(&raw const (*list).m_Attributes.m_Memory.m_pMemory).read(),
				(&raw const (*list).m_Attributes.m_Memory.m_nAllocationCount).read(),
				(&raw const (*list).m_Attributes.m_Size).read(),
			)
		};

		let plausible = (0..=MAX_LIST_LEN).contains(&len)
			&& len <= allocated
			&& (len == 0 || (!memory.is_null() && memory.is_aligned()));

		if !plausible {
			return Err(AttributeError::UnsupportedLayout);
		}

		Ok((memory, len as usize))
	}
}

/// `CSendPropExtra_UtlVector` (`dt_utlvector_send.cpp`), which
/// `SendPropUtlVector` allocates for each networked vector and shares between
/// the properties of the table it builds for it. The class is private to
/// that file, so the generated bindings lack it. It has no bases or virtual
/// methods, so both supported ABIs lay it out as C does.
#[repr(C)]
struct UtlVectorExtra {
	/// `m_DataTableProxyFn`, `m_ProxyFn` and `m_EnsureCapacityFn`, unused.
	_functions: [*const c_void; 3],

	/// `m_ElementStride`: the size of each element.
	element_stride: c_int,

	/// `m_Offset`: bytes from the structure the vector's property belongs to,
	/// here the list, to the vector.
	offset: c_int,

	/// `m_nMaxElements`: the most elements networked.
	max_elements: c_int,
}

/// Checks that a property lies at `offset` from the entity.
fn check_offset(prop: NetProp<'_>, offset: usize) -> Result<(), AttributeError> {
	if prop.offset() == offset {
		Ok(())
	} else {
		Err(AttributeError::UnsupportedLayout)
	}
}

/// Checks that a variable is stored compatibly with `expected`.
fn check_storage(storage: Storage, expected: Storage) -> Result<(), AttributeError> {
	if storage.is_compatible(expected) {
		Ok(())
	} else {
		Err(AttributeError::UnsupportedLayout)
	}
}

/// Checks that a property is a nested table at `offset` from the entity.
fn check_table(prop: NetProp<'_>, offset: usize) -> Result<(), AttributeError> {
	if prop.prop().kind() == PropKind::DataTable {
		check_offset(prop, offset)
	} else {
		Err(AttributeError::UnsupportedLayout)
	}
}

/// The variable named `name` that a nested table property's own table
/// declares. [`NetProp::element`] refuses it unless the parent's proxy passes
/// its data through unchanged.
fn child<'s>(parent: NetProp<'s>, name: &CStr) -> Result<NetProp<'s>, AttributeError> {
	let index = parent
		.prop()
		.data_table()
		.and_then(|table| {
			table.props().position(|prop| {
				prop.name() == name
					&& !prop.flags().contains(PropFlags::EXCLUDE)
					&& !prop.flags().contains(PropFlags::INSIDE_ARRAY)
			})
		})
		.ok_or(AttributeError::UnsupportedLayout)?;

	parent
		.element(index)
		.map_err(|_| AttributeError::UnsupportedLayout)
}

/// A send property's extra data (`m_pExtraData`), if it is set and aligned
/// for a [`UtlVectorExtra`].
fn extra_data(prop: SendProp<'_>) -> Option<*const UtlVectorExtra> {
	// SAFETY: The property belongs to one of the game DLL's send tables, and
	// the field is read without forming a reference.
	let extra =
		unsafe { (&raw const (*prop.as_ptr()).m_pExtraData).read() }.cast::<UtlVectorExtra>();

	(!extra.is_null() && extra.is_aligned()).then_some(extra)
}

/// An economy item's definition index (`m_iItemDefinitionIndex`), or `None`
/// for an item without one, read after checking only the networked
/// variables that place it. Fails with [`AttributeError::UnsupportedLayout`]
/// if they do not place it where the generated layout does.
pub(crate) fn item_definition<'s>(
	dll: ServerGameDll<'s>,
	entity: Entity<'s>,
) -> Result<Option<ItemDefinitionIndex>, AttributeError> {
	item_tables(dll, entity)?;

	// SAFETY: `item_tables` checked the field.
	Ok(ItemDefinitionIndex::new(unsafe {
		read_definition_index(entity)
	}))
}

/// The networked container and item of `entity`, checked to be nested tables
/// at the generated offsets behind proxies that pass their data through
/// unchanged, with the item's definition index at its generated offset and
/// width. An entity whose class is not networked fails.
fn item_tables<'s>(
	dll: ServerGameDll<'s>,
	entity: Entity<'s>,
) -> Result<(NetProp<'s>, NetProp<'s>), AttributeError> {
	let container = dll
		.entity_net_prop(entity, c"m_AttributeManager")
		.map_err(|_| AttributeError::UnsupportedLayout)?;

	check_table(container, CONTAINER)?;

	let item = child(container, c"m_Item")?;

	check_table(item, ITEM)?;

	let definition = child(item, c"m_iItemDefinitionIndex")?;

	check_offset(definition, DEFINITION)?;
	check_storage(definition.storage(), Storage::U16)?;

	Ok((container, item))
}

/// Reads an item's raw definition index.
///
/// # Safety
///
/// [`item_tables`] must have accepted `entity`, matching the field's
/// networked offset and width with the generated ones.
unsafe fn read_definition_index(entity: Entity<'_>) -> u16 {
	// SAFETY: The caller vouches for the field, which is read without forming
	// a reference, as the game writes it too.
	unsafe {
		let item = entity.as_ptr().byte_add(ITEM).cast::<sys::CEconItemView>();

		(&raw const (*item).m_iItemDefinitionIndex.m_Value).read()
	}
}

/// The table describing each entry of the list's vector, checked to describe
/// the generated vector.
///
/// `SendPropUtlVector` gives the vector's own property offset 0, behind a
/// proxy that passes the list through unchanged, and nests a table holding a
/// `lengthproxy` property, then one property per entry, each nesting the
/// entry's table and holding its position in its element stride. It keeps
/// where the vector lies within the list, and the size of its entries, in a
/// [`UtlVectorExtra`] every one of those properties shares
/// (`dt_utlvector_send.cpp`). That must give the generated offset of the
/// list's vector, the size of `CEconItemAttribute`, and 20 entries
/// (`MAX_ATTRIBUTES_PER_ITEM`, `econ_item_constants.h`).
fn vector_entries(vector: NetProp<'_>) -> Result<SendTable<'_>, AttributeError> {
	let unsupported = AttributeError::UnsupportedLayout;

	// The nested table's properties are then relative to the list itself.
	check_table(vector, LIST)?;
	vector.element(0).map_err(|_| unsupported)?;

	let props = vector
		.prop()
		.data_table()
		.filter(|props| props.len() == MAX_RUNTIME_ATTRIBUTES + 1)
		.ok_or(unsupported)?;

	let extra = props
		.prop(0)
		.filter(|prop| prop.kind() == PropKind::DataTable && prop.name() == c"lengthproxy")
		.and_then(extra_data)
		.ok_or(unsupported)?;

	let entry = props
		.prop(1)
		.and_then(|prop| prop.data_table())
		.ok_or(unsupported)?;

	for (position, prop) in props.props().enumerate().skip(1) {
		let element = prop.kind() == PropKind::DataTable
			&& prop.data_table().map(|table| table.as_ptr()) == Some(entry.as_ptr())
			&& extra_data(prop) == Some(extra)
			&& usize::try_from(prop.element_stride()) == Ok(position - 1);

		if !element {
			return Err(unsupported);
		}
	}

	// SAFETY: Only `SendPropUtlVector` gives send properties extra data: a
	// `CSendPropExtra_UtlVector` it allocates and never frees, and which every
	// property of the table it builds shares, as these do. It is copied
	// without forming a reference.
	let extra = unsafe { extra.read() };

	let generated = usize::try_from(extra.offset)
		== Ok(offset_of!(sys::CAttributeList, m_Attributes))
		&& usize::try_from(extra.element_stride) == Ok(size_of::<sys::CEconItemAttribute>())
		&& usize::try_from(extra.max_elements) == Ok(MAX_RUNTIME_ATTRIBUTES);

	if generated {
		Ok(entry)
	} else {
		Err(unsupported)
	}
}
