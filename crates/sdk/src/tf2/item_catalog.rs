//! More of what the item schema says of its definitions, and questions over
//! all of them: the attributes a definition carries, whether it is enabled,
//! hidden, a stock item or a tool, and which items a class equips where.
//!
//! # Layout
//!
//! These fields are read at the offsets of the generated
//! `CEconItemDefinition`, which TF2's own build can lay out otherwise, and no
//! known definition fixes their values the way [`ItemSchema::new`]'s checked
//! definitions fix the others'. So they are checked on the whole schema
//! instead, once, the first time one is read: in every definition the
//! attribute list's header must be consistent, with its elements where its
//! memory is, and its attribute indices nonzero; the flags must each be
//! exactly 0 or 1; and the schema as a whole must have attributes, and both
//! enabled and disabled definitions, as a layout reading none says nothing.
//!
//! A schema that fails the scan fails every later read of these fields with
//! [`ItemSchemaError::UnsupportedLayout`], until the plugin is reloaded, and
//! leaves the rest of [`item_schema`](crate::tf2::item_schema) as it was.
//! [`ItemSchema::catalog_report`] says what the scan read and what it found
//! wrong.

#[cfg(test)]
#[path = "../tests/tf2/item_catalog.rs"]
mod tests;

use crate::tf2::PlayerClass;
use crate::tf2::item_schema::{ItemDefinition, ItemSchema, ItemSchemaError, LoadoutPosition};
use std::ffi::{CStr, c_int};
use std::fmt;
use std::sync::OnceLock;

/// The most edits (insertions, deletions and substitutions of a byte) between
/// a name and a query for [`ItemSchema::similar_definition_names`] to suggest
/// the name for a query it neither starts with nor contains.
pub const MAX_NAME_DISTANCE: usize = 3;

/// The most static attributes a definition is believed to carry: far more
/// than the schema's definitions have, so that a misread count is refused.
const MAX_STATIC_ATTRIBUTES: c_int = 128;

/// The result of the whole-schema scan, once it ran.
static CATALOG: OnceLock<Result<CatalogStats, CatalogFault>> = OnceLock::new();

/// A value of a definition's static attribute, which the game stores in a
/// union of a float, a 32-bit integer and a pointer. Static attributes come
/// from the schema's text, so they hold the first two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeBits(u32);

impl AttributeBits {
	pub const fn from_bits(bits: u32) -> Self {
		Self(bits)
	}

	/// The value as the attribute's `float` type has it.
	pub const fn as_f32(self) -> f32 {
		f32::from_bits(self.0)
	}

	/// The value as the attribute's integer type has it.
	pub const fn as_u32(self) -> u32 {
		self.0
	}
}

/// What the scan found wrong, and in which definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("definition {index}: {reason}")]
pub struct CatalogFault {
	/// The definition's index, or 0 for a fault of the schema as a whole.
	pub index: u16,
	pub reason: FaultReason,
}

/// What the scan of the schema read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatalogStats {
	/// How many definitions it read.
	pub definitions: usize,

	/// How many of them have static attributes.
	pub with_attributes: usize,

	/// How many static attributes they have together.
	pub attributes: usize,

	/// How many definitions are enabled.
	pub enabled: usize,

	/// How many are hidden.
	pub hidden: usize,

	/// How many are base (stock) items.
	pub base_items: usize,

	/// How many are tools.
	pub tools: usize,
}

/// Why a definition failed the scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FaultReason {
	#[error("the attribute list's size {0} is out of range")]
	AttributeCount(c_int),

	#[error("the attribute list's memory is not where its elements are")]
	AttributeMemory,

	#[error("the attribute list's memory is null or misaligned for {0} attributes")]
	AttributeElements(c_int),

	#[error("the attribute list holds fewer slots ({0}) than attributes")]
	AttributeAllocation(c_int),

	#[error("an attribute has index 0")]
	AttributeIndex,

	#[error("a flag reads {0:#x}, neither 0 nor 1")]
	Flag(u8),

	#[error("every definition reads enabled")]
	NoneDisabled,

	#[error("the schema has no enabled definitions")]
	NoneEnabled,

	#[error("no definition has a static attribute")]
	NoAttributes,
}

/// The flags of a definition the scan checks, in the order of
/// [`RawCatalogEntry::flags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
enum Flags {
	Enabled,
	Hidden,
	BaseItem,
	Tool,
	PackBundle,
}

/// What a definition reads as, before the scan accepts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawCatalogEntry {
	pub attributes: VectorHeader,

	/// The flags, in the order of [`Flags`].
	pub flags: [u8; 5],
}

impl RawCatalogEntry {
	/// Reads the header and flags of `definition`.
	fn read(definition: ItemDefinition<'_>) -> Self {
		let raw = definition.raw.as_ptr();

		// SAFETY: The schema keeps the definition through the callback it is
		// scoped to. The fields are plain data within it at the bindings'
		// offsets; the flags are read as bytes, as the layout is not yet known
		// to hold booleans.
		unsafe {
			let list = &raw const (*raw).m_vecStaticAttributes;

			let flag = |field: *const bool| field.cast::<u8>().read();

			Self {
				attributes: VectorHeader {
					memory: (&raw const (*list).m_Memory.m_pMemory).read().addr(),
					allocation: (&raw const (*list).m_Memory.m_nAllocationCount).read(),
					size: (&raw const (*list).m_Size).read(),
					elements: (&raw const (*list).m_pElements).read().addr(),
				},

				flags: [
					flag(&raw const (*raw).m_bEnabled),
					flag(&raw const (*raw).m_bHidden),
					flag(&raw const (*raw).m_bBaseItem),
					flag(&raw const (*raw).m_bIsTool),
					flag(&raw const (*raw).m_bIsPackBundle),
				],
			}
		}
	}

	/// Whether the entry is consistent, as the [module
	/// documentation](self#layout) says each definition must be.
	fn check(self) -> Result<(), FaultReason> {
		self.attributes
			.check(align_of::<sys::static_attrib_t>(), MAX_STATIC_ATTRIBUTES)?;

		match self.flags.into_iter().find(|&flag| flag > 1) {
			Some(flag) => Err(FaultReason::Flag(flag)),
			None => Ok(()),
		}
	}

	fn flag(self, flag: Flags) -> bool {
		self.flags[flag as usize] == 1
	}
}

/// An attribute the schema gives all items of a definition
/// (`static_attrib_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(alias("static_attrib_t"))]
pub struct StaticAttribute {
	/// The attribute's definition index (`iDefIndex`).
	pub index: u16,

	/// Its value (`m_value`), whose type the attribute's definition says.
	pub value: AttributeBits,
}

/// The header of a `CUtlVector`: its memory's pointer and slot count, its size
/// and its element pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VectorHeader {
	pub memory: usize,
	pub allocation: c_int,
	pub size: c_int,
	pub elements: usize,
}

impl VectorHeader {
	/// Whether the header describes a vector of at most `max` elements of
	/// `alignment`, which can be read through `elements`.
	pub(crate) fn check(self, alignment: usize, max: c_int) -> Result<(), FaultReason> {
		if !(0..=max).contains(&self.size) {
			return Err(FaultReason::AttributeCount(self.size));
		}

		// The vector keeps a second copy of its memory's pointer.
		if self.memory != self.elements {
			return Err(FaultReason::AttributeMemory);
		}

		if self.size > 0 && (self.memory == 0 || !self.memory.is_multiple_of(alignment)) {
			return Err(FaultReason::AttributeElements(self.size));
		}

		if self.allocation < self.size {
			return Err(FaultReason::AttributeAllocation(self.allocation));
		}

		Ok(())
	}
}

impl<'s> ItemDefinition<'s> {
	/// The flags of the definition, once the scan accepted the schema.
	fn catalog_entry(self) -> Result<RawCatalogEntry, ItemSchemaError> {
		match CATALOG.get() {
			Some(Ok(_)) => Ok(RawCatalogEntry::read(self)),
			_ => Err(ItemSchemaError::UnsupportedLayout),
		}
	}

	/// Whether the definition is a base item (`m_bBaseItem`), one of the stock
	/// items a class starts with.
	///
	/// Fails as [`Self::static_attributes`] does.
	#[doc(alias("m_bBaseItem", "IsBaseItem"))]
	pub fn is_base_item(self) -> Result<bool, ItemSchemaError> {
		Ok(self.catalog_entry()?.flag(Flags::BaseItem))
	}

	/// Whether the schema enables the definition (`m_bEnabled`). The game gives
	/// out only enabled definitions' items.
	///
	/// Fails as [`Self::static_attributes`] does.
	#[doc(alias("m_bEnabled", "IsEnabled"))]
	pub fn is_enabled(self) -> Result<bool, ItemSchemaError> {
		Ok(self.catalog_entry()?.flag(Flags::Enabled))
	}

	/// Whether the definition is hidden from the player's item lists
	/// (`m_bHidden`).
	///
	/// Fails as [`Self::static_attributes`] does.
	#[doc(alias("m_bHidden", "IsHidden"))]
	pub fn is_hidden(self) -> Result<bool, ItemSchemaError> {
		Ok(self.catalog_entry()?.flag(Flags::Hidden))
	}

	/// Whether the definition is a bundle of other items (`m_bIsPackBundle`).
	///
	/// Fails as [`Self::static_attributes`] does.
	#[doc(alias("m_bIsPackBundle", "IsPackBundle"))]
	pub fn is_pack_bundle(self) -> Result<bool, ItemSchemaError> {
		Ok(self.catalog_entry()?.flag(Flags::PackBundle))
	}

	/// Whether the definition is a tool (`m_bIsTool`), such as a key or a
	/// paint can, not equipped.
	///
	/// Fails as [`Self::static_attributes`] does.
	#[doc(alias("m_bIsTool", "IsTool"))]
	pub fn is_tool(self) -> Result<bool, ItemSchemaError> {
		Ok(self.catalog_entry()?.flag(Flags::Tool))
	}

	/// The attributes of the definition read as consistent by the scan, which
	/// callers of the public methods ran.
	///
	/// Only called with a list whose header [`VectorHeader::check`] accepted, so
	/// its elements lie in `size` valid slots.
	fn static_attribute_elements(self, size: c_int) -> Vec<StaticAttribute> {
		let raw = self.raw.as_ptr();

		// SAFETY: The schema keeps the definition, and its attribute list, through
		// the callback. The caller checked that the list's memory holds `size`
		// slots; the elements are plain data.
		unsafe {
			let elements = (&raw const (*raw).m_vecStaticAttributes.m_pElements).read();

			(0..size as usize)
				.map(|at| {
					let element = elements.add(at);

					StaticAttribute {
						index: (&raw const (*element).iDefIndex).read(),
						value: AttributeBits::from_bits(
							(&raw const (*element).m_value.asUint32).read(),
						),
					}
				})
				.collect()
		}
	}

	/// The attributes the schema gives every item of the definition
	/// (`m_vecStaticAttributes`, the `attributes` of `items_game.txt`), in the
	/// schema's order. Items can carry more of their own.
	///
	/// Fails with [`ItemSchemaError::UnsupportedLayout`] unless the schema
	/// passed the [scan](self#layout), which [`ItemSchema::scan_catalog`] runs.
	#[doc(alias("m_vecStaticAttributes", "GetStaticAttributes"))]
	pub fn static_attributes(self) -> Result<Vec<StaticAttribute>, ItemSchemaError> {
		let entry = self.catalog_entry()?;

		Ok(self.static_attribute_elements(entry.attributes.size))
	}
}

impl<'s> ItemSchema<'s> {
	/// What the scan of [`Self::scan_catalog`] read and found, as text, for
	/// finding out why it fails: the counts it read, or its fault. It scans
	/// again, whether or not an earlier scan failed, and remembers nothing.
	///
	/// Fails like [`Self::definitions`].
	pub fn catalog_report(self) -> Result<String, ItemSchemaError> {
		use fmt::Write;

		let definitions = self.definitions()?;
		let mut report = String::new();

		let _ = match scan(&definitions) {
			Ok(stats) => writeln!(report, "catalog scan passed: {stats:?}"),
			Err(fault) => writeln!(report, "catalog scan refused: {fault}"),
		};

		let _ = writeln!(
			report,
			"flag offsets: enabled {}, hidden {}, base {}, tool {}, bundle {}; attribute list {}",
			std::mem::offset_of!(sys::CEconItemDefinition, m_bEnabled),
			std::mem::offset_of!(sys::CEconItemDefinition, m_bHidden),
			std::mem::offset_of!(sys::CEconItemDefinition, m_bBaseItem),
			std::mem::offset_of!(sys::CEconItemDefinition, m_bIsTool),
			std::mem::offset_of!(sys::CEconItemDefinition, m_bIsPackBundle),
			std::mem::offset_of!(sys::CEconItemDefinition, m_vecStaticAttributes),
		);

		let _ = writeln!(report, "remembered: {:?}", CATALOG.get());

		// The whole of a few definitions, to place fields by.
		for index in [0, 18, 49, 56, 738] {
			let Some(definition) = definitions
				.iter()
				.find(|definition| definition.index() == index)
			else {
				continue;
			};

			let _ = write!(report, "definition {index} words:");

			// SAFETY: A definition is a `CTFItemDefinition`, which the schema keeps
			// through the callback, and its size is the bindings'.
			let words = unsafe {
				std::slice::from_raw_parts(
					definition.raw.as_ptr().cast::<u32>(),
					size_of::<sys::CEconItemDefinition>() / 4,
				)
			};

			for (at, word) in words.iter().enumerate() {
				if at % 8 == 0 {
					let _ = write!(report, "\n  +{:#05x}:", at * 4);
				}

				let _ = write!(report, " {word:08x}");
			}

			report.push('\n');
		}

		if let Some(first) = definitions.first() {
			let entry = RawCatalogEntry::read(*first);

			let _ = writeln!(report, "first definition {}: {entry:?}", first.index(),);
		}

		Ok(report)
	}

	/// The definitions `class` can equip, each with the loadout position
	/// `class` equips it in, ordered by position (as the game numbers them) and
	/// then by index. Includes disabled, hidden and tool definitions; filter
	/// them with [`ItemDefinition::is_enabled`] and the like.
	///
	/// Fails like [`Self::definitions`] and [`ItemDefinition::loadout_position`].
	pub fn class_loadout(
		self,
		class: PlayerClass,
	) -> Result<Vec<(LoadoutPosition, ItemDefinition<'s>)>, ItemSchemaError> {
		let mut found = Vec::new();

		for definition in self.definitions()? {
			if !definition.is_used_by(class)? {
				continue;
			}

			if let Some(position) = definition.loadout_position(class)? {
				found.push((position, definition));
			}
		}

		// The definitions are in order of their index, which a stable sort keeps
		// within a position.
		found.sort_by_key(|(position, _)| position.to_raw());
		Ok(found)
	}

	/// Every named definition's index and name, in order of their index, for
	/// listing or suggesting names. Definitions without a name are left out.
	/// Walks every definition, as [`Self::definitions`] does.
	///
	/// Fails like [`Self::definitions`] and [`ItemDefinition::name`].
	pub fn definition_names(self) -> Result<Vec<(u16, &'s CStr)>, ItemSchemaError> {
		let mut names = Vec::new();

		for definition in self.definitions()? {
			if let Some(name) = definition.name()? {
				names.push((definition.index(), name));
			}
		}

		Ok(names)
	}

	/// The definitions whose item class (`item_class`) is `item_class`,
	/// compared ignoring ASCII case, in order of their index. Walks every
	/// definition, as [`Self::definitions`] does.
	///
	/// Fails like [`Self::definitions`] and [`ItemDefinition::item_class`].
	pub fn definitions_of_item_class(
		self,
		item_class: &CStr,
	) -> Result<Vec<ItemDefinition<'s>>, ItemSchemaError> {
		let mut found = Vec::new();

		for definition in self.definitions()? {
			let same = definition
				.item_class()?
				.is_some_and(|own| own.to_bytes().eq_ignore_ascii_case(item_class.to_bytes()));

			if same {
				found.push(definition);
			}
		}

		Ok(found)
	}

	/// The definitions `class` equips in `position`, in order of their index.
	/// Walks every definition, as [`Self::class_loadout`] does.
	///
	/// Fails like [`Self::class_loadout`].
	pub fn loadout_items(
		self,
		class: PlayerClass,
		position: LoadoutPosition,
	) -> Result<Vec<ItemDefinition<'s>>, ItemSchemaError> {
		Ok(self
			.class_loadout(class)?
			.into_iter()
			.filter(|&(own, _)| own == position)
			.map(|(_, definition)| definition)
			.collect())
	}

	/// Scans the whole schema for the fields of this module, if it has not been,
	/// and returns what it read. The result is remembered: a schema the game
	/// replaces later is not scanned again, and the readers of this module read
	/// it at the layout this scan accepted.
	///
	/// Fails like [`Self::definitions`], and with
	/// [`ItemSchemaError::UnsupportedLayout`] if the scan found a fault, which
	/// [`Self::catalog_report`] names.
	pub fn scan_catalog(self) -> Result<CatalogStats, ItemSchemaError> {
		if let Some(done) = CATALOG.get() {
			return done.map_err(|_| ItemSchemaError::UnsupportedLayout);
		}

		let definitions = self.definitions()?;
		let done = scan(&definitions);

		// Another call may have raced this one only on the main thread, where
		// both would have read the same schema.
		let done = *CATALOG.get_or_init(|| done);

		done.map_err(|_| ItemSchemaError::UnsupportedLayout)
	}

	/// Up to `limit` definitions whose names are closest to `query`, closest
	/// first, for suggesting what a mistyped name meant: names equal to it
	/// ignoring ASCII case, then names starting with it, then names containing
	/// it, then names within [`MAX_NAME_DISTANCE`] edits of it. Ties go to the
	/// lower index.
	///
	/// Fails like [`Self::definition_names`].
	pub fn similar_definition_names(
		self,
		query: &CStr,
		limit: usize,
	) -> Result<Vec<(u16, &'s CStr)>, ItemSchemaError> {
		Ok(rank_names(
			self.definition_names()?,
			query.to_bytes(),
			limit,
		))
	}

	/// The stock items `class` starts with: its enabled base items, with their
	/// positions, ordered as [`Self::class_loadout`] does.
	///
	/// Fails like [`Self::class_loadout`], and as [`ItemDefinition::is_base_item`]
	/// does, which includes scanning the schema first, as
	/// [`Self::scan_catalog`] does.
	pub fn stock_loadout(
		self,
		class: PlayerClass,
	) -> Result<Vec<(LoadoutPosition, ItemDefinition<'s>)>, ItemSchemaError> {
		self.scan_catalog()?;

		let mut stock = Vec::new();

		for (position, definition) in self.class_loadout(class)? {
			if definition.is_base_item()? && definition.is_enabled()? {
				stock.push((position, definition));
			}
		}

		Ok(stock)
	}
}

/// How closely a name matches a query: lower is closer, or `None` if it is
/// not close enough to suggest.
fn name_rank(name: &[u8], query: &[u8]) -> Option<(u8, usize)> {
	let name = normalize(name);
	let query = normalize(query);

	if query.is_empty() {
		None
	} else if name == query {
		Some((0, 0))
	} else if name.starts_with(&query) {
		Some((1, name.len() - query.len()))
	} else if name.windows(query.len()).any(|window| window == query) {
		Some((2, name.len() - query.len()))
	} else {
		let distance = substring_distance(&name, &query);

		(distance <= MAX_NAME_DISTANCE).then_some((3, distance))
	}
}

/// `text` as lowercase ASCII letters and digits only, so that `Rocket
/// Launcher`, `rocket_launcher` and `ROCKETLAUNCHER` compare equal.
fn normalize(text: &[u8]) -> Vec<u8> {
	text.iter()
		.filter(|byte| byte.is_ascii_alphanumeric())
		.map(u8::to_ascii_lowercase)
		.collect()
}

/// The `limit` names closest to `query`, closest first, by [`name_rank`].
fn rank_names<'s>(names: Vec<(u16, &'s CStr)>, query: &[u8], limit: usize) -> Vec<(u16, &'s CStr)> {
	let mut ranked: Vec<_> = names
		.into_iter()
		.filter_map(|(index, name)| Some((name_rank(name.to_bytes(), query)?, index, name)))
		.collect();

	ranked.sort_by_key(|&(rank, index, _)| (rank, index));
	ranked.truncate(limit);
	ranked
		.into_iter()
		.map(|(_, index, name)| (index, name))
		.collect()
}

/// Scans `definitions`, which must be the whole schema, for the faults the
/// [module documentation](self#layout) lists.
pub(crate) fn scan<'a, 's: 'a>(
	definitions: impl IntoIterator<Item = &'a ItemDefinition<'s>>,
) -> Result<CatalogStats, CatalogFault> {
	let mut stats = CatalogStats::default();

	for definition in definitions {
		let index = definition.index();
		let fault = |reason| CatalogFault { index, reason };
		let entry = RawCatalogEntry::read(*definition);

		entry.check().map_err(fault)?;

		if entry.attributes.size > 0 {
			let elements = definition.static_attribute_elements(entry.attributes.size);

			if elements.iter().any(|attribute| attribute.index == 0) {
				return Err(fault(FaultReason::AttributeIndex));
			}

			stats.with_attributes += 1;
			stats.attributes += elements.len();
		}

		stats.definitions += 1;
		stats.enabled += usize::from(entry.flag(Flags::Enabled));
		stats.hidden += usize::from(entry.flag(Flags::Hidden));
		stats.base_items += usize::from(entry.flag(Flags::BaseItem));
		stats.tools += usize::from(entry.flag(Flags::Tool));
	}

	let whole = |reason| CatalogFault { index: 0, reason };

	if stats.enabled == stats.definitions {
		return Err(whole(FaultReason::NoneDisabled));
	}

	if stats.enabled == 0 {
		return Err(whole(FaultReason::NoneEnabled));
	}

	if stats.attributes == 0 {
		return Err(whole(FaultReason::NoAttributes));
	}

	Ok(stats)
}

/// The fewest edits turning `query` into some part of `text`.
fn substring_distance(text: &[u8], query: &[u8]) -> usize {
	// Edit distance with a free start in the text, and a free end.
	let mut row: Vec<usize> = (0..=query.len()).collect();
	let mut best = row[query.len()];

	for &x in text {
		let mut diagonal = row[0];

		row[0] = 0;

		for (j, &y) in query.iter().enumerate() {
			let above = row[j + 1];

			row[j + 1] = (diagonal + usize::from(x != y))
				.min(above + 1)
				.min(row[j] + 1);
			diagonal = above;
		}

		best = best.min(row[query.len()]);
	}

	best
}
