//! TF2's attributes: the item schema's named modifiers, such as
//! `damage bonus`, on economy items and players.
//!
//! # Items
//!
//! Weapons and wearables are economy items (`CEconEntity`). Each has static
//! attributes from its item definition and a runtime list a plugin can
//! change, which overrides static attributes of the same definition. The game
//! networks the runtime list with the item, so clients predict and describe
//! the item with it. [`ItemAttributes`] reads that list directly, at offsets
//! it checks against the game's own networking tables, and changes it through
//! the game's native `AddAttribute` and `RemoveAttribute`, which refresh the
//! caches of the item, its owner and their clients.
//!
//! Item attributes are the item's, not the player's: TF2 replaces weapons it
//! did not hand out itself on every resupply, respawn, class change and team
//! change, which loses their attributes, and a dropped weapon keeps its
//! attributes for whoever picks it up. To keep attributes on a player's
//! weapons, apply an [`AttributeSet`] again whenever the game fires
//! [`PostInventoryApplication`], after it hands out weapons.
//!
//! # Types and the schema
//!
//! The game looks an attribute up by name in the running item schema, and
//! then stores the value as 32 bits that every later read interprets
//! according to the attribute's type. Only TF2's legacy default type, a
//! plain number, is safe to store this way: string and other blob types read
//! the bits as a pointer. The schema the server runs can also differ from the
//! one it shipped with, since the game coordinator can send a newer one that
//! TF2 applies at the next level change.
//!
//! The safe setters therefore only take definitions from the [`catalog`],
//! which this crate vetted against the shipped schema, and a [`SchemaToken`]
//! from [`trust_shipped_schema`], whose caller vouches that the running
//! schema keeps their types, hooks and formats, and that every runtime
//! attribute the game may iterate has a type it can iterate. Each write then
//! checks that the running schema mapped the name to the catalog's definition
//! index, and undoes the write before the game reads the list again if it
//! did not.
//!
//! # Players
//!
//! [`PlayerAttributes`] covers TF2's custom player attributes, which can
//! expire and which the game clears whenever the player spawns or dies.
//!
//! # Limitations
//!
//! - Only an item's runtime list can be listed, with
//!   [`ItemAttributes::runtime`]. The static attributes of its item
//!   definition and economy item, which the game reaches through
//!   `IEconItemAttributeIterator`, are read one at a time by
//!   [`ItemAttributes::get`].
//! - [`PlayerAttributes`] does not validate the player's attribute layout,
//!   and has no typed or safe setter: its writes are unchecked.
//! - The catalog leaves out attributes whose values index game tables, such
//!   as particle effects, item definitions or condition masks.
//! - [`AttributeSet::apply`] does not undo the attributes it set before it
//!   fails.
//!
//! [`PostInventoryApplication`]: crate::tf2::game_events::GameEventId::PostInventoryApplication

pub mod catalog;
mod definition;
mod layout;

use crate::NotThreadSafe;
use crate::entities::Entity;
use crate::tf2::attributes::definition::RawDef;
use crate::tf2::attributes::layout::ItemLayout;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::tf2::weapons::ItemDefinitionIndex;
use crate::{Game, Server};
use sdk_raw::tf2::attributes::DEFAULT_CUSTOM_ATTRIBUTE_DURATION;
use std::ffi::CStr;
use std::marker::PhantomData;

pub use definition::{
	Amount, AttributeDef, AttributeIndex, AttributeValue, Combine, DescriptionFormat, Flag,
	Multiplier, Seconds,
};

pub(crate) use layout::item_definition;

/// The most runtime attributes the game networks per item
/// (`MAX_ATTRIBUTES_PER_ITEM`). The game applies further entries on the
/// server but silently does not send them, so clients mispredict the item.
#[doc(alias("MAX_ATTRIBUTES_PER_ITEM"))]
pub const MAX_RUNTIME_ATTRIBUTES: usize = sdk_raw::tf2::attributes::MAX_ATTRIBUTES_PER_ITEM;

/// An attribute operation could not be performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttributeError {
	/// A value was not finite, a duration was not finite and positive, or a
	/// value read from the game does not fit the definition's value type.
	#[error("the attribute value or duration is invalid")]
	InvalidValue,

	/// The entity is pending deletion, so it is not modified or queried.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The value lies outside the definition's bounds.
	#[error("the value is outside the attribute's supported range")]
	OutOfDomain,

	/// The native method's binding adapter reported failure, or the game
	/// stored a value other than the one written. A rejected write was undone.
	#[error("the game rejected the attribute change")]
	Rejected,

	/// The item already has [`MAX_RUNTIME_ATTRIBUTES`] runtime attributes, so
	/// another would not be networked to clients.
	#[error("the item already has the most runtime attributes the game networks")]
	RuntimeListFull,

	/// The running item schema maps the definition's name to another index.
	/// A write was undone; a removal removed the other attribute's entry.
	#[error("the running item schema maps the attribute's name to index {found}, not {expected}")]
	SchemaMismatch {
		/// The index the catalog expects.
		expected: AttributeIndex,

		/// The index of the entry the game changed.
		found: AttributeIndex,
	},

	/// The game DLL's `IServerGameDLL`, through which an item's networked
	/// variables are found, is unavailable.
	#[error("the server game dll interface is unavailable")]
	Unavailable,

	/// The runtime list changed in a way the native method does not change
	/// it, so it was left as it is.
	#[error("the runtime attribute list changed unexpectedly")]
	UnexpectedChange,

	/// The native method changed nothing: the running item schema has no
	/// attribute of this name, or the name selects another runtime entry that
	/// already held the value.
	#[error("the game did not apply the attribute; the running item schema may lack it")]
	UnknownAttribute,

	/// The item's runtime list is not linked to its attribute container: the
	/// list's manager is not the container, or the container's outer entity
	/// is not the item. The game links them when the item spawns
	/// (`CAttributeContainer::InitializeAttributes`), and copying an item
	/// view unlinks its list (`CAttributeList::operator=`).
	#[error("the item's runtime attribute list is not linked to its attribute container")]
	Unlinked,

	/// The entity's data maps include neither `CTFPlayer` nor `CEconEntity`,
	/// as the wrapper requires.
	#[error("the entity is not a TF2 player or economy item, as required")]
	UnsupportedEntity,

	/// The server is not running Team Fortress 2.
	#[error("attributes require Team Fortress 2")]
	UnsupportedGame,

	/// The item's networked attribute storage does not match the SDK's layout
	/// for this ABI. Its class is not networked; its send tables do not place
	/// the attribute container, item, definition index, runtime list, the
	/// list's vector or a field of a list entry where the generated layout
	/// does, or give entries another size; or its list holds an implausible
	/// count or allocation, or an entry that is not a `CEconItemAttribute`.
	#[error("the item's attribute storage does not match the sdk layout")]
	UnsupportedLayout,

	/// The entity's script class descriptors lack the native method, or its
	/// signature differs from the SDK's.
	#[error("the game does not expose the expected native attribute method")]
	UnsupportedMethod,
}

impl From<BindingError> for AttributeError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// Catalog attributes and values to apply together, such as each time the
/// game hands a player new weapons.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttributeSet {
	entries: Vec<(RawDef, f32)>,
}

impl AttributeSet {
	/// An empty set.
	pub const fn new() -> Self {
		Self {
			entries: Vec::new(),
		}
	}

	/// Sets every attribute on `item`, in insertion order, as
	/// [`ItemAttributes::set`] does.
	///
	/// Fails with [`AttributeError::RuntimeListFull`] before writing anything
	/// if the item has no room for the set's new definitions, counting those
	/// without an entry at their catalog index. Otherwise it stops at the
	/// first error, and does not undo the attributes it already set.
	///
	/// The game decides whether a weapon provides its attributes to its owner
	/// when it is equipped, so a set with [`catalog::PROVIDE_ON_ACTIVE`]
	/// needs [`ItemAttributes::reapply_provision`] afterwards, as
	/// [`PlayerWeapons::give_item_with`] does.
	///
	/// [`PlayerWeapons::give_item_with`]: crate::tf2::weapons::PlayerWeapons::give_item_with
	pub fn apply<'s>(
		&self,
		token: SchemaToken<'s>,
		item: ItemAttributes<'s>,
	) -> Result<(), AttributeError> {
		item.check_live()?;

		let runtime = item.layout.snapshot()?;
		let added = self
			.entries
			.iter()
			.filter(|(def, _)| !runtime.iter().any(|entry| entry.index == def.index))
			.count();

		if added != 0 && runtime.len() + added > MAX_RUNTIME_ATTRIBUTES {
			return Err(AttributeError::RuntimeListFull);
		}

		for &(def, stored) in &self.entries {
			item.set_raw(token, def, stored)?;
		}

		Ok(())
	}

	/// Adds an attribute, or replaces the value of one the set has. Fails with
	/// [`AttributeError::OutOfDomain`] outside the definition's bounds.
	pub fn insert<V: AttributeValue>(
		&mut self,
		def: &AttributeDef<V>,
		value: V,
	) -> Result<(), AttributeError> {
		let stored = def.stored(value)?;
		let def = def.raw();

		match self
			.entries
			.iter_mut()
			.find(|(existing, _)| existing.index == def.index)
		{
			Some(entry) => *entry = (def, stored),
			None => self.entries.push((def, stored)),
		}

		Ok(())
	}

	/// Whether the set has no attributes.
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	/// How many attributes the set has.
	pub fn len(&self) -> usize {
		self.entries.len()
	}

	/// Removes an attribute, returning whether the set had it.
	pub fn remove<V: AttributeValue>(&mut self, def: &AttributeDef<V>) -> bool {
		let len = self.entries.len();

		self.entries
			.retain(|(existing, _)| existing.index != def.index());

		self.entries.len() != len
	}

	/// As [`Self::insert`], by value, for chaining.
	pub fn with<V: AttributeValue>(
		mut self,
		def: &AttributeDef<V>,
		value: V,
	) -> Result<Self, AttributeError> {
		self.insert(def, value)?;

		Ok(self)
	}
}

/// How a native call changed a runtime list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
	/// An entry for a definition the list lacked was appended.
	Appended(RuntimeAttribute),

	/// The value of the entry at `position` changed from `before`'s.
	Changed {
		position: usize,
		before: RuntimeAttribute,
	},

	/// Nothing changed.
	None,

	/// Anything else.
	Other,
}

impl Change {
	/// Compares a list before and after a native `AddAttribute`.
	fn between(before: &[RuntimeAttribute], after: &[RuntimeAttribute]) -> Self {
		if before == after {
			return Self::None;
		}

		if after.len() == before.len() + 1 && after.starts_with(before) {
			let added = after[before.len()];

			// `SetRuntimeAttributeValue` overwrites an existing entry instead.
			return if before.iter().any(|entry| entry.index == added.index) {
				Self::Other
			} else {
				Self::Appended(added)
			};
		}

		if after.len() == before.len() {
			let mut changed = before
				.iter()
				.zip(after)
				.enumerate()
				.filter(|(_, (old, new))| old != new);

			if let (Some((position, (old, new))), None) = (changed.next(), changed.next())
				&& old.index == new.index
				&& old.refundable_currency == new.refundable_currency
			{
				return Self::Changed {
					position,
					before: *old,
				};
			}
		}

		Self::Other
	}
}

/// The attributes of a TF2 economy item, such as a weapon or wearable, scoped
/// to one engine callback.
///
/// Reads of the runtime list copy it straight from the item, after checking
/// where it lies (see [`Self::new`]). Writes call the game's native methods,
/// then read the list again to confirm what changed. Methods fail with
/// [`AttributeError::MarkedForDeletion`] for an item pending deletion,
/// [`AttributeError::Unlinked`] if its list does not belong to its attribute
/// container, and [`AttributeError::UnsupportedMethod`] when the game lacks a
/// native method.
///
/// Some effects apply only when the game next looks at them, such as movement
/// speed after a weapon switch; the [`catalog`] notes them.
#[doc(alias("CEconEntity", "CAttributeList"))]
#[derive(Debug, Clone, Copy)]
pub struct ItemAttributes<'s> {
	layout: ItemLayout<'s>,
}

impl<'s> ItemAttributes<'s> {
	/// Wraps a TF2 economy item, identified by `CEconEntity` in `entity`'s
	/// datamap chain. Fails with [`AttributeError::UnsupportedGame`] outside
	/// TF2, [`AttributeError::UnsupportedEntity`] for other entities, and
	/// [`AttributeError::Unavailable`] without the game DLL's interface.
	///
	/// Fails with [`AttributeError::UnsupportedLayout`] unless the networked
	/// variables of `entity`'s class place its attribute container, item,
	/// definition index, runtime list and the list's vector, and each field
	/// of a list entry, where the generated SDK layout does, and give the
	/// vector entries of the generated size.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, AttributeError> {
		if server.game() != Game::TeamFortress2 {
			return Err(AttributeError::UnsupportedGame);
		}

		if !entity
			.data_maps()
			.any(|map| map.class_name() == Some(c"CEconEntity"))
		{
			return Err(AttributeError::UnsupportedEntity);
		}

		let dll = server
			.server_game_dll()
			.map_err(|_| AttributeError::Unavailable)?;

		Ok(Self {
			layout: ItemLayout::validate(dll, entity)?,
		})
	}

	/// Native `AddAttribute`, which looks `name` up in the running item schema
	/// and, if it finds it, sets or appends the runtime entry of its
	/// definition and notifies the attribute container. Durations are ignored
	/// for items, so the entry stays until removed.
	///
	/// # Safety
	///
	/// If the running schema has an attribute named `name`, `value` must be
	/// valid for its type and gameplay domain, or the entry must be undone
	/// before game code reads the list again.
	unsafe fn add(self, name: &CStr, value: f32) -> Result<(), AttributeError> {
		// SAFETY: The method copies nothing it keeps from the name, and only
		// changes the list through `SetRuntimeAttributeValue`, which notifies
		// the container (`econ_entity.h`, `econ_item_view.cpp`). Neither
		// iterates attributes or deletes entities. The caller vouches for the
		// value.
		unsafe {
			self.call(
				c"AddAttribute",
				&mut [
					binding::string(name),
					binding::float(value),
					binding::float(DEFAULT_CUSTOM_ATTRIBUTE_DURATION),
				],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// Calls one of `CEconEntity`'s native attribute methods.
	///
	/// # Safety
	///
	/// As for [`binding::call`].
	unsafe fn call(
		self,
		method: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
		result: sys::ScriptDataType_t,
	) -> Result<sys::ScriptVariant_t, AttributeError> {
		// SAFETY: The caller upholds `call`'s contract.
		Ok(unsafe { binding::call(self.entity(), c"CEconEntity", method, arguments, result) }?)
	}

	/// Refuses items marked for deletion.
	fn check_live(self) -> Result<(), AttributeError> {
		if self.entity().is_marked_for_deletion() {
			Err(AttributeError::MarkedForDeletion)
		} else {
			Ok(())
		}
	}

	/// The item's definition index (`m_iItemDefinitionIndex`), or `None` for
	/// an item without one.
	#[doc(alias("m_iItemDefinitionIndex"))]
	pub fn definition(self) -> Result<Option<ItemDefinitionIndex>, AttributeError> {
		self.check_live()?;

		Ok(ItemDefinitionIndex::new(self.layout.definition_index()))
	}

	/// The item's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.layout.entity()
	}

	/// The attribute's effective value on the item, as the native scripting
	/// getter reads it: the runtime entry, or else the item definition's
	/// static value. `None` when the running schema has no attribute of the
	/// definition's name, when the item has neither value, or when it stores
	/// NaN, which the getter uses to signal absence.
	///
	/// The getter looks the attribute up by name and does not check the
	/// index it finds: should the running schema renumber the name, this is
	/// the value of whatever attribute the schema now gives it, converted to
	/// `V`. Only the setters detect renumbering. Fails with
	/// [`AttributeError::InvalidValue`] if the value does not fit `V`.
	///
	/// The getter iterates the item's attributes through their types, which
	/// the token vouches for.
	#[doc(alias("GetAttribute"))]
	pub fn get<V: AttributeValue>(
		self,
		token: SchemaToken<'s>,
		def: &AttributeDef<V>,
	) -> Result<Option<V>, AttributeError> {
		let _ = token;

		self.check_live()?;

		// SAFETY: The getter iterates the item's attributes, whose types the
		// token vouches for, and keeps no pointer to the name.
		let result = unsafe {
			self.call(
				c"GetAttribute",
				&mut [binding::string(def.name()), binding::float(f32::NAN)],
				binding::FLOAT,
			)
		}?;

		// SAFETY: `call` checked FIELD_FLOAT before returning.
		let value = unsafe { result.__bindgen_anon_1.m_float };

		if value.is_nan() {
			return Ok(None);
		}

		V::from_stored(value)
			.map(Some)
			.ok_or(AttributeError::InvalidValue)
	}

	/// The effective value on the item of the attribute named `name`, as for
	/// [`Self::get`], but for any attribute, as the 32 bits the game stores read
	/// as a float: the value of TF2's legacy default type. `None` when the
	/// running schema has no attribute of that name, when the item has no value
	/// for it, or when it stores NaN.
	///
	/// The getter only finds values of the legacy default type, whatever their
	/// meaning: an integer stored as such (`stored_as_integer`) reads as the
	/// float of the same bits, and a string or other blob type as absent.
	/// Besides the runtime list and the item definition's static attributes, it
	/// reads the attributes of the economy item a player's inventory holds, such
	/// as paint (`set item tint rgb`).
	///
	/// The getter iterates the item's attributes through their types, which
	/// the token vouches for.
	#[doc(alias("GetAttribute"))]
	pub fn get_by_name(
		self,
		token: SchemaToken<'s>,
		name: &CStr,
	) -> Result<Option<f32>, AttributeError> {
		let _ = token;

		self.check_live()?;

		// SAFETY: The getter iterates the item's attributes, whose types the
		// token vouches for, and keeps no pointer to the name.
		let result = unsafe {
			self.call(
				c"GetAttribute",
				&mut [binding::string(name), binding::float(f32::NAN)],
				binding::FLOAT,
			)
		}?;

		// SAFETY: `call` checked FIELD_FLOAT before returning.
		let value = unsafe { result.__bindgen_anon_1.m_float };

		Ok((!value.is_nan()).then_some(value))
	}

	/// Native `ReapplyProvision`, which links the item's attributes to its
	/// current owner, honoring [`catalog::PROVIDE_ON_ACTIVE`] on weapons. Use
	/// it after changing that attribute or the item's owner. It can also
	/// change the item's model to suit the owner's class.
	///
	/// TF2's weapons evaluate `provide_on_active` to decide, which iterates,
	/// through their types, the runtime attributes of the weapon, of its
	/// owner and of every item providing to the owner. The token vouches for
	/// them.
	#[doc(alias("ReapplyProvision"))]
	pub fn reapply_provision(self, token: SchemaToken<'s>) -> Result<(), AttributeError> {
		let _ = token;

		self.check_live()?;

		// SAFETY: The method relinks attribute providers and may set the
		// item's model; it deletes no entities. The attribute hook it
		// evaluates iterates the attributes of the item, its owner and the
		// owner's other providers, whose types the token vouches for.
		unsafe { self.call(c"ReapplyProvision", &mut [], binding::VOID) }?;

		Ok(())
	}

	/// Tells the item's attribute container that its values changed, as every
	/// native change does: the cached results of the attribute hooks of the
	/// item and of the entities it provides to, such as its owner, are
	/// cleared, and clients are made to do the same. It reads no attributes.
	#[doc(alias("OnAttributeValuesChanged"))]
	pub fn refresh(self) -> Result<(), AttributeError> {
		self.check_live()?;
		self.layout.notify()
	}

	/// Removes the attribute's runtime entry, returning whether the item had
	/// one. A static value from the item definition can still apply after.
	///
	/// Without an entry for the definition's index, nothing is called: this
	/// check is keyed by the catalog's index, so should the running schema
	/// renumber the name, an entry at its new index stays. Otherwise native
	/// `RemoveAttribute` removes the entry the running schema maps the name
	/// to. If that is another definition's, the removal cannot be undone and
	/// fails with [`AttributeError::SchemaMismatch`].
	#[doc(alias("RemoveAttribute"))]
	pub fn remove<V: AttributeValue>(self, def: &AttributeDef<V>) -> Result<bool, AttributeError> {
		self.check_live()?;

		let def = def.raw();
		let before = self.layout.snapshot()?;

		if !before.iter().any(|entry| entry.index == def.index) {
			return Ok(false);
		}

		self.remove_native(def.name)?;

		let after = self.layout.snapshot()?;

		match removed_entry(&before, &after)? {
			None => Err(AttributeError::UnknownAttribute),
			Some(entry) if entry.index == def.index => Ok(true),

			Some(entry) => Err(AttributeError::SchemaMismatch {
				expected: def.index,
				found: entry.index,
			}),
		}
	}

	/// Removes the runtime entry of any attribute by name, such as one
	/// [`Self::set_by_name_unchecked`] added, through native
	/// `RemoveAttribute`. Returns the entry removed, or `None` when the
	/// running schema has no such attribute or the item has no entry for it.
	///
	/// Removal reads no attribute through its type: the game compares the
	/// entries' definitions, then notifies the container.
	#[doc(alias("RemoveAttribute"))]
	pub fn remove_by_name(self, name: &CStr) -> Result<Option<RuntimeAttribute>, AttributeError> {
		self.check_live()?;

		let before = self.layout.snapshot()?;

		self.remove_native(name)?;

		let after = self.layout.snapshot()?;

		removed_entry(&before, &after)
	}

	/// Native `RemoveAttribute`, which removes the runtime entry of the
	/// definition the running schema maps `name` to, if the item has one.
	fn remove_native(self, name: &CStr) -> Result<(), AttributeError> {
		// SAFETY: The method compares definitions to remove an entry and then
		// notifies the container (`econ_entity.h`, `econ_item_view.cpp`). It
		// neither iterates attributes nor deletes entities, and keeps no
		// pointer to the name.
		unsafe {
			self.call(
				c"RemoveAttribute",
				&mut [binding::string(name)],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// Copies the item's runtime attributes, in the order the game keeps and
	/// networks them, without reading any through its type.
	#[doc(alias("m_AttributeList"))]
	pub fn runtime(self) -> Result<Vec<RuntimeAttribute>, AttributeError> {
		self.check_live()?;
		self.layout.snapshot()
	}

	/// The value of the runtime entry at the definition's index, or `None`
	/// without one. Fails with [`AttributeError::InvalidValue`] if the entry's
	/// value does not fit `V`.
	///
	/// This is keyed by the catalog's index, not the name: should the running
	/// schema renumber the name, it reads whichever attribute the schema now
	/// gives the index.
	pub fn runtime_value<V: AttributeValue>(
		self,
		def: &AttributeDef<V>,
	) -> Result<Option<V>, AttributeError> {
		self.runtime()?
			.into_iter()
			.find(|entry| entry.index == def.index())
			.map(|entry| V::from_stored(entry.value()).ok_or(AttributeError::InvalidValue))
			.transpose()
	}

	/// Sets a catalog attribute's runtime value, and refreshes the caches of
	/// the item, its owner and their clients.
	///
	/// Fails, without changing anything, with [`AttributeError::OutOfDomain`]
	/// outside the definition's bounds, or [`AttributeError::RuntimeListFull`]
	/// when the item has no entry at the definition's index and no room for
	/// one.
	///
	/// Otherwise native `AddAttribute` sets the value by name, and the list is
	/// read again. The write is undone, before any game code reads the list,
	/// if the game changed another definition's entry
	/// ([`AttributeError::SchemaMismatch`]) or stored other bits
	/// ([`AttributeError::Rejected`]). [`AttributeError::UnknownAttribute`]
	/// means it changed nothing.
	///
	/// Writing the value an entry at the definition's index already holds
	/// would change nothing, and so not show which entry the running schema
	/// maps the name to. Another value within the bounds is set first, which
	/// shows it, then the value itself.
	#[doc(alias("AddAttribute"))]
	pub fn set<V: AttributeValue>(
		self,
		token: SchemaToken<'s>,
		def: &AttributeDef<V>,
		value: V,
	) -> Result<(), AttributeError> {
		let stored = def.stored(value)?;

		self.set_raw(token, def.raw(), stored)
	}

	/// Sets a runtime attribute by name, with no type or domain checks, and
	/// returns the entry the game added or changed. `None` means the list did
	/// not change: the running schema has no such attribute, or its entry
	/// already held the value. A write that would make the list longer than
	/// [`MAX_RUNTIME_ATTRIBUTES`] is undone and fails with
	/// [`AttributeError::RuntimeListFull`]. [`Self::remove_by_name`] removes
	/// the entry again.
	///
	/// # Safety
	///
	/// If `name` exists in the running item schema, its attribute must have
	/// TF2's legacy default type (`CSchemaAttributeType_Default`): the game
	/// reads every other type's stored bits as a pointer whenever it next
	/// iterates the item's attributes, on the server and, once networked, on
	/// clients. `value` must also be valid for the attribute's gameplay
	/// domain; being finite does not stop an extreme value overflowing the
	/// integer conversions of the code that reads it. A value that is
	/// stored as an integer must be the integer's bits as a float.
	#[doc(alias("AddAttribute"))]
	pub unsafe fn set_by_name_unchecked(
		self,
		name: &CStr,
		value: f32,
	) -> Result<Option<RuntimeAttribute>, AttributeError> {
		self.check_live()?;

		if !value.is_finite() {
			return Err(AttributeError::InvalidValue);
		}

		let before = self.layout.snapshot()?;

		// SAFETY: The caller vouches for the attribute's type and the value.
		unsafe { self.add(name, value) }?;

		let after = self.layout.snapshot()?;

		match Change::between(&before, &after) {
			Change::Appended(_) if after.len() > MAX_RUNTIME_ATTRIBUTES => {
				self.undo_append(name, &before)?;

				Err(AttributeError::RuntimeListFull)
			}

			Change::Appended(entry) => Ok(Some(entry)),
			Change::Changed { position, .. } => Ok(Some(after[position])),
			Change::None => Ok(None),
			Change::Other => Err(AttributeError::UnexpectedChange),
		}
	}

	/// As [`Self::set`], with a checked stored value.
	fn set_raw(
		self,
		token: SchemaToken<'s>,
		def: RawDef,
		stored: f32,
	) -> Result<(), AttributeError> {
		let _ = token;

		self.check_live()?;

		let bits = stored.to_bits();
		let before = self.layout.snapshot()?;

		match before.iter().find(|entry| entry.index == def.index) {
			Some(entry) if entry.bits == bits => {
				// Another value within the bounds, which catalog definitions
				// always have, changes the entry the name maps to, or else is
				// undone. Each call then has an entry that differs from the
				// value it writes, so this recurses only once. Should the value
				// itself then fail, the entry keeps the other, equally valid one.
				let probe = if def.min.to_bits() == bits {
					def.max
				} else {
					def.min
				};

				self.set_raw(token, def, probe)?;

				return self.set_raw(token, def, stored);
			}

			None if before.len() >= MAX_RUNTIME_ATTRIBUTES => {
				return Err(AttributeError::RuntimeListFull);
			}

			_ => {}
		}

		// SAFETY: The token vouches that an attribute with the catalog's name
		// and index has the default type, for which any float in the
		// definition's bounds is valid. If the running schema maps the name to
		// another index instead, the change is undone below, before game code
		// reads the list: setting the value only notifies the container, which
		// clears caches without iterating attributes (`ClearCache`), and the
		// item's description is not built on the server.
		unsafe { self.add(def.name, stored) }?;

		let after = self.layout.snapshot()?;

		let check = |entry: RuntimeAttribute| {
			if entry.index != def.index {
				Err(AttributeError::SchemaMismatch {
					expected: def.index,
					found: entry.index,
				})
			} else if entry.bits != bits {
				Err(AttributeError::Rejected)
			} else {
				Ok(())
			}
		};

		match Change::between(&before, &after) {
			Change::Appended(entry) => {
				let outcome = match check(entry) {
					Ok(()) if after.len() > MAX_RUNTIME_ATTRIBUTES => {
						Err(AttributeError::RuntimeListFull)
					}

					outcome => outcome,
				};

				if outcome.is_err() {
					self.undo_append(def.name, &before)?;
				}

				outcome
			}

			Change::Changed {
				position,
				before: previous,
			} => {
				let outcome = check(after[position]);

				if outcome.is_err() {
					// SAFETY: No game code ran since `after` was read, and the
					// entry's previous bits are valid for its own type.
					unsafe { self.layout.restore_bits(position, previous.bits) }?;
					self.layout.notify()?;
				}

				outcome
			}

			Change::None => Err(AttributeError::UnknownAttribute),
			Change::Other => Err(AttributeError::UnexpectedChange),
		}
	}

	/// Undoes native `AddAttribute` appending an entry for `name` to a list
	/// that was `before`, before game code reads the list again.
	///
	/// Native `RemoveAttribute` removes it: the appended entry is the only one
	/// of the definition the name maps to, since `SetRuntimeAttributeValue`
	/// would otherwise have overwritten an existing one. Should the list still
	/// end with it, it is dropped directly instead.
	fn undo_append(self, name: &CStr, before: &[RuntimeAttribute]) -> Result<(), AttributeError> {
		// The list itself shows whether removal worked.
		let _ = self.remove_native(name);
		let after = self.layout.snapshot()?;

		if after == before {
			return Ok(());
		}

		if after.len() == before.len() + 1 && after.starts_with(before) {
			// SAFETY: No game code ran since `after` was read, and its last
			// entry is the one this plugin's `AddAttribute` call appended.
			unsafe { self.layout.truncate(before.len()) }?;

			return self.layout.notify();
		}

		Err(AttributeError::UnexpectedChange)
	}
}

/// The attributes of a TF2 player, scoped to one engine callback.
///
/// These are TF2's custom player attributes, which the game clears whenever
/// the player spawns or dies, and which can expire. Names are the item
/// schema's attribute names, such as `move speed bonus`; TF2 looks each up in
/// the running schema. Values supplied by the player's items are separate.
///
/// Methods fail with [`AttributeError::MarkedForDeletion`] for a player
/// pending deletion, [`AttributeError::UnsupportedMethod`] when the game lacks
/// the expected native method, and [`AttributeError::Rejected`] when the
/// method's binding reports failure.
#[doc(alias("CTFPlayer"))]
#[derive(Debug, Clone, Copy)]
pub struct PlayerAttributes<'s> {
	player: Entity<'s>,
}

impl<'s> PlayerAttributes<'s> {
	/// Wraps a TF2 player, identified by `CTFPlayer` in `player`'s datamap
	/// chain. Fails with [`AttributeError::UnsupportedGame`] outside TF2, or
	/// [`AttributeError::UnsupportedEntity`] for any other entity.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, AttributeError> {
		if server.game() != Game::TeamFortress2 {
			return Err(AttributeError::UnsupportedGame);
		}

		if !player
			.data_maps()
			.any(|map| map.class_name() == Some(c"CTFPlayer"))
		{
			return Err(AttributeError::UnsupportedEntity);
		}

		Ok(Self { player })
	}

	/// Calls one of `CTFPlayer`'s native attribute methods.
	///
	/// # Safety
	///
	/// As for [`binding::call`].
	unsafe fn call(
		self,
		method: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
		result: sys::ScriptDataType_t,
	) -> Result<sys::ScriptVariant_t, AttributeError> {
		if self.player.is_marked_for_deletion() {
			return Err(AttributeError::MarkedForDeletion);
		}

		// SAFETY: The caller upholds `call`'s contract.
		Ok(unsafe { binding::call(self.player, c"CTFPlayer", method, arguments, result) }?)
	}

	/// Looks up the value of the player's own attribute as the native
	/// scripting getter does. `None` for an unknown name, an absent attribute,
	/// or a stored NaN, which the getter uses to signal absence. Attributes of
	/// the explicit `"float"` schema type are never found.
	///
	/// The getter iterates the player's attributes through their types, which
	/// the token vouches for.
	#[doc(alias("GetCustomAttribute"))]
	pub fn get(self, token: SchemaToken<'s>, name: &CStr) -> Result<Option<f32>, AttributeError> {
		let _ = token;

		// SAFETY: The token vouches for the types the getter iterates.
		unsafe { self.read(name) }
	}

	/// The player whose attributes these are.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Native `GetCustomAttribute`, as for [`Self::get`].
	///
	/// # Safety
	///
	/// The second condition of [`trust_shipped_schema`] must hold. The getter
	/// iterates the player's own attributes through their types.
	unsafe fn read(self, name: &CStr) -> Result<Option<f32>, AttributeError> {
		// SAFETY: The caller vouches for the types the getter iterates. It
		// keeps no pointer to the name.
		let result = unsafe {
			self.call(
				c"GetCustomAttribute",
				&mut [binding::string(name), binding::float(f32::NAN)],
				binding::FLOAT,
			)
		}?;

		// SAFETY: `call` checked FIELD_FLOAT before returning.
		let value = unsafe { result.__bindgen_anon_1.m_float };

		Ok((!value.is_nan()).then_some(value))
	}

	/// Removes a custom attribute added by [`Self::set_unchecked`],
	/// [`Self::set_for_unchecked`] or the game's `AddCustomAttribute`.
	///
	/// The game then recomputes the player's speed, whose attribute hooks
	/// iterate, through their types, the runtime attributes of the player and
	/// of every item providing to the player. The token vouches for them.
	#[doc(alias("RemoveCustomAttribute"))]
	pub fn remove(self, token: SchemaToken<'s>, name: &CStr) -> Result<(), AttributeError> {
		let _ = token;

		// SAFETY: The method removes a list entry, refreshes speed and caches,
		// and keeps no pointer to the name; it deletes no entities. The speed
		// hooks iterate the attributes of the player and its providers, whose
		// types the token vouches for.
		unsafe {
			self.call(
				c"RemoveCustomAttribute",
				&mut [binding::string(name)],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// As [`Self::set_unchecked`], with an expiry in seconds. Fails with
	/// [`AttributeError::InvalidValue`] for a non-finite value or a duration
	/// that is not finite and positive.
	///
	/// # Safety
	///
	/// The contract of [`Self::set_unchecked`] applies.
	#[doc(alias("AddCustomAttribute"))]
	pub unsafe fn set_for_unchecked(
		self,
		name: &CStr,
		value: f32,
		duration: Option<f32>,
	) -> Result<bool, AttributeError> {
		if !value.is_finite()
			|| duration.is_some_and(|seconds| !seconds.is_finite() || seconds <= 0.0)
		{
			return Err(AttributeError::InvalidValue);
		}

		// SAFETY: The method copies the name into its own map, sets the value
		// through the player's attribute manager, and recomputes speed; it
		// deletes no entities. The caller vouches for the types the speed
		// hooks read, on the player and every item providing to it.
		unsafe {
			self.call(
				c"AddCustomAttribute",
				&mut [
					binding::string(name),
					binding::float(value),
					binding::float(duration.unwrap_or(DEFAULT_CUSTOM_ATTRIBUTE_DURATION)),
				],
				binding::VOID,
			)
		}?;

		// SAFETY: The caller vouches for every type the getter iterates.
		Ok(unsafe { self.read(name) }?.is_some_and(|stored| stored == value))
	}

	/// Sets a custom attribute that lasts until removed or the game clears it,
	/// and returns whether reading it back gives `value`. False means an
	/// unknown or ignored attribute, or a value that did not read back equal;
	/// it does not undo the write.
	///
	/// # Safety
	///
	/// If `name` exists in the running item schema, its attribute must have
	/// TF2's legacy default type (`CSchemaAttributeType_Default`): the game
	/// reads every other type's stored bits as a pointer, starting with the
	/// speed update this method runs. `value` must be valid for the
	/// attribute's gameplay domain. The second condition of
	/// [`trust_shipped_schema`] must hold: the speed update iterates the
	/// attributes of the player and of every item providing to it, and the
	/// read-back the player's own.
	#[doc(alias("AddCustomAttribute"))]
	pub unsafe fn set_unchecked(self, name: &CStr, value: f32) -> Result<bool, AttributeError> {
		// SAFETY: The caller upholds the same contract.
		unsafe { self.set_for_unchecked(name, value, None) }
	}
}

/// One entry of an item's runtime attribute list, copied without reading the
/// value through its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeAttribute {
	/// The entry's attribute definition (`m_iAttributeDefinitionIndex`).
	pub index: AttributeIndex,

	/// The value's 32 raw bits (`m_flValue`), which the attribute's type
	/// interprets.
	pub bits: u32,

	/// The currency Mann vs. Machine refunds for an upgrade that added the
	/// entry (`m_nRefundableCurrency`).
	pub refundable_currency: i32,
}

impl RuntimeAttribute {
	/// The bits as a float, the meaning of every catalog attribute's value.
	pub const fn value(self) -> f32 {
		f32::from_bits(self.bits)
	}
}

/// Proof, for one callback, that the running item schema keeps the types of
/// the [`catalog`]'s attributes. Get one from [`trust_shipped_schema`].
#[derive(Debug, Clone, Copy)]
pub struct SchemaToken<'s> {
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

/// The entry native `RemoveAttribute` removed from a list that was `before`
/// and is now `after`, or `None` if it removed nothing.
/// [`AttributeError::UnexpectedChange`] reports any other change.
fn removed_entry(
	before: &[RuntimeAttribute],
	after: &[RuntimeAttribute],
) -> Result<Option<RuntimeAttribute>, AttributeError> {
	if after == before {
		return Ok(None);
	}

	// `CUtlVector::Remove` keeps the order of the remaining entries.
	let position = before
		.iter()
		.zip(after)
		.position(|(old, new)| old != new)
		.unwrap_or(after.len());

	if after.len() + 1 != before.len() || before[position + 1..] != after[position..] {
		return Err(AttributeError::UnexpectedChange);
	}

	Ok(Some(before[position]))
}

/// Vouches for the running item schema and the attributes on the server for
/// the scope of `server`, for the attribute methods whose soundness rests on
/// them.
///
/// # Safety
///
/// For all of `'s`:
///
/// 1. Every attribute of the running item schema that has both the name and
///    the definition index of a [`catalog`] entry keeps what the shipped
///    `items_game.txt` gives it, against which the catalog's bounds were
///    vetted: TF2's legacy default numeric type
///    (`CSchemaAttributeType_Default`), its `attribute_class` (the hook that
///    reads it), its `description_format` (how the game combines it) and
///    float storage (no `stored_as_integer`). The game coordinator can send
///    the server a newer schema, which TF2 applies at the next level change;
///    it may renumber or drop a catalog name, which the setters detect, but
///    must not change any of these for a name it keeps at its index.
/// 2. Every runtime attribute of every economy item and player on the
///    server, including those other plugins or map scripts added, has a type
///    the game can iterate: one that supports gameplay modification
///    (`BSupportsGameplayModificationAndNetworking`), the default type or
///    `"float"`. Evaluating an attribute hook iterates, through each
///    attribute's type, the lists of the entity it is evaluated on, of every
///    item providing to it, and then of its owner and the owner's providers
///    (`CAttributeManager::ApplyAttributeFloat`), reading a string or other
///    blob attribute's stored bits as a pointer.
pub const unsafe fn trust_shipped_schema<'s>(server: Server<'s>) -> SchemaToken<'s> {
	let _ = server;

	SchemaToken {
		_scope: PhantomData,
		_not_thread_safe: PhantomData,
	}
}
