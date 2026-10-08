//! Data description maps (`datamap_t`, `public/datamap.h`), with which the
//! game describes the fields, key values, and inputs of its entity classes.
//!
//! A game DLL declares each class's map as a static, completes it before the
//! first entity of the class exists, and never changes it afterwards. The
//! handles here rely on that: they are made by `unsafe` constructors whose
//! callers vouch for it, after which reading the maps, their fields, and the
//! fields' names is safe.

use crate::util::cstr::borrow_cstr;
use std::ffi::{CStr, c_short};
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;

/// `FTYPEDESC_INPUT` from `public/datamap.h`: the field is an input, which
/// `AcceptInput` finds by its external name.
pub const FTYPEDESC_INPUT: c_short = 0x0008;

/// `FTYPEDESC_KEY` from `public/datamap.h`: the field is set by a key value,
/// which `KeyValue` finds by its external name.
pub const FTYPEDESC_KEY: c_short = 0x0004;

/// `FTYPEDESC_OUTPUT` from `public/datamap.h`: the field is an output, a
/// `CBaseEntityOutput` (`DEFINE_OUTPUT`), to which the key value of its
/// external name adds an action.
pub const FTYPEDESC_OUTPUT: c_short = 0x0010;

/// `TD_OFFSET_NORMAL` from `public/datamap.h`: the index, in a field's
/// `fieldOffset`, of its offset in the object its map describes. The other
/// offset is the one prediction packs fields at.
pub const TD_OFFSET_NORMAL: usize = 0;

/// A field a [`DataMap`] declares (`typedescription_t`).
///
/// Fields are only reached through [`DataMap::fields`], borrowed for as long
/// as their map, which is what makes reading their names safe. The raw
/// description is readable through `Deref`.
#[doc(alias("typedescription_t"))]
#[derive(Debug)]
#[repr(transparent)]
pub struct DataField(sys::typedescription_t);

impl DataField {
	/// The maps of the object the field embeds (`td`), which only a
	/// `FIELD_EMBEDDED` field has. Other fields give none.
	pub fn embedded(&self) -> DataMaps<'_> {
		// SAFETY: A field is only reached through its map, whose embedded maps
		// stay allocated and unmodified for as long as the map does, as
		// `DataMaps::new` requires.
		unsafe { DataMaps::new(self.0.td) }
	}

	/// The name the field's key value or input is found by (`externalName`),
	/// such as `targetname`, or `None` if it has none.
	pub fn external_name(&self) -> Option<&CStr> {
		// SAFETY: As for `embedded`, the name is a string of the map's module,
		// which stays allocated and unmodified for as long as the map does.
		unsafe { borrow_cstr(self.0.externalName) }
	}

	/// The member's name in its class (`fieldName`), such as `m_iHealth`, or
	/// `None` if it has none.
	pub fn name(&self) -> Option<&CStr> {
		// SAFETY: As for `external_name`.
		unsafe { borrow_cstr(self.0.fieldName) }
	}

	/// The member's offset in the object its map describes, or `None` if it is
	/// negative.
	pub fn offset(&self) -> Option<usize> {
		usize::try_from(self.0.fieldOffset[TD_OFFSET_NORMAL]).ok()
	}
}

impl Deref for DataField {
	type Target = sys::typedescription_t;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

/// One class's data description map (`datamap_t`), which declares the fields
/// of that class, not those of its bases.
#[doc(alias("datamap_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataMap<'a> {
	raw: NonNull<sys::datamap_t>,
	_maps: PhantomData<&'a sys::datamap_t>,
}

impl<'a> DataMap<'a> {
	/// The most fields a map is trusted to declare. A map claiming more is
	/// read as declaring none.
	pub const MAX_FIELDS: usize = 4096;

	/// Wraps a map.
	///
	/// # Safety
	///
	/// `raw` must point to a map that stays allocated and unmodified for `'a`,
	/// with everything reachable from it, as [`DataMaps::new`] requires of its
	/// first map.
	pub const unsafe fn from_raw(raw: NonNull<sys::datamap_t>) -> Self {
		Self {
			raw,
			_maps: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *const sys::datamap_t {
		self.raw.as_ptr().cast_const()
	}

	/// The map of the class's base class (`baseMap`), or `None` for a class
	/// without one.
	#[doc(alias("baseMap"))]
	pub fn base(self) -> Option<Self> {
		// SAFETY: The map is allocated and unmodified for `'a`, as `from_raw`
		// requires, and its field is read without forming a reference.
		let base = unsafe { (&raw const (*self.as_ptr()).baseMap).read() };

		// SAFETY: The base map is reachable from this one, so it stays
		// allocated and unmodified for `'a` too.
		NonNull::new(base).map(|base| unsafe { Self::from_raw(base) })
	}

	/// The name of the class the map describes (`dataClassName`), such as
	/// `CBaseEntity`, or `None` if it has none.
	#[doc(alias("dataClassName"))]
	pub fn class_name(self) -> Option<&'a CStr> {
		// SAFETY: As for `base`, and the name is a string of the map's module,
		// which stays allocated and unmodified for `'a`.
		unsafe { borrow_cstr((&raw const (*self.as_ptr()).dataClassName).read()) }
	}

	/// The offset of the field of `field_type` named `name` that the map
	/// declares, not counting its bases' fields, or `None` if it declares none
	/// or its offset is negative.
	pub fn field_offset(self, name: &CStr, field_type: sys::fieldtype_t) -> Option<usize> {
		self.fields()
			.iter()
			.find(|field| field.fieldType == field_type && field.name() == Some(name))
			.and_then(DataField::offset)
	}

	/// The fields the map declares (`dataDesc`), not including its bases'.
	///
	/// A map whose array is null or misaligned, or whose count is not within
	/// 1 to [`Self::MAX_FIELDS`], declares none.
	#[doc(alias("dataDesc"))]
	pub fn fields(self) -> &'a [DataField] {
		// SAFETY: As for `base`.
		let (fields, count) = unsafe {
			(
				(&raw const (*self.as_ptr()).dataDesc).read(),
				(&raw const (*self.as_ptr()).dataNumFields).read(),
			)
		};

		match usize::try_from(count) {
			Ok(count @ 1..=Self::MAX_FIELDS) if !fields.is_null() && fields.is_aligned() => {
				// SAFETY: The map declares `count` fields in the array, which is
				// reachable from the map, so it stays allocated and unmodified
				// for `'a`. `DataField` is a transparent wrapper of a field's
				// description.
				unsafe {
					std::slice::from_raw_parts(fields.cast_const().cast::<DataField>(), count)
				}
			}

			_ => &[],
		}
	}
}

/// A class's data description maps, from its own towards its bases'.
///
/// The chain is followed through at most [`Self::MAX_MAPS`] maps.
#[derive(Debug, Clone)]
pub struct DataMaps<'a> {
	next: Option<DataMap<'a>>,
	remaining: usize,
}

impl<'a> DataMaps<'a> {
	/// The deepest nesting of embedded objects [`Self::find_key_field`]
	/// searches.
	pub const MAX_EMBEDDING_DEPTH: usize = 16;

	/// The most maps a chain is followed through.
	pub const MAX_MAPS: usize = 64;

	/// Follows the chain of maps from `first`, or none for null.
	///
	/// # Safety
	///
	/// `first` must be null, or point to a `datamap_t` that stays allocated and
	/// unmodified for `'a`, with every map, field array, and string reachable
	/// from it: through `baseMap`, `dataDesc`, and `dataClassName`, and
	/// through its fields' names and embedded maps (`td`). A loaded game DLL's
	/// maps are such statics once the first entity of their class exists, for
	/// as long as the DLL stays loaded.
	pub const unsafe fn new(first: *const sys::datamap_t) -> Self {
		Self {
			next: match NonNull::new(first.cast_mut()) {
				// SAFETY: The caller upholds the contract.
				Some(first) => Some(unsafe { DataMap::from_raw(first) }),

				None => None,
			},
			remaining: Self::MAX_MAPS,
		}
	}

	/// Finds the field `CBaseEntity::ExtractKeyvalue` reads for `key` in the
	/// object these maps describe, and its offset in that object.
	///
	/// Fields are searched as `ExtractKeyvalue` does: from the class's own map
	/// towards its bases', and through each embedded object (a single
	/// `FIELD_EMBEDDED` field) before the fields after it, for the first
	/// [`FTYPEDESC_KEY`] field whose external name matches, ignoring ASCII
	/// case. Objects nested deeper than [`Self::MAX_EMBEDDING_DEPTH`] are not
	/// searched.
	pub fn find_key_field(self, key: &[u8]) -> Option<(&'a DataField, usize)> {
		self.find_key_field_at(key, 0)
	}

	/// [`Self::find_key_field`] in an object nested `depth` deep.
	fn find_key_field_at(self, key: &[u8], depth: usize) -> Option<(&'a DataField, usize)> {
		for map in self {
			for field in map.fields() {
				let offset = field.offset();

				// Embedded objects are searched before the field itself, but not
				// arrays of them.
				if field.fieldType == sys::_fieldtypes_FIELD_EMBEDDED
					&& field.fieldSize == 1
					&& depth < Self::MAX_EMBEDDING_DEPTH
					&& let Some((found, inner)) = field.embedded().find_key_field_at(key, depth + 1)
				{
					return Some((found, offset?.checked_add(inner)?));
				}

				if field.flags & FTYPEDESC_KEY != 0
					&& field
						.external_name()
						.is_some_and(|name| name.to_bytes().eq_ignore_ascii_case(key))
				{
					return Some((field, offset?));
				}
			}
		}

		None
	}

	/// The [`FTYPEDESC_KEY`] fields of the object these maps describe, each with
	/// its offset in that object, in the order [`Self::find_key_field`] searches
	/// them: from the class's own map towards its bases', and through each
	/// embedded object (a single `FIELD_EMBEDDED` field) before the fields after
	/// it, nested at most [`Self::MAX_EMBEDDING_DEPTH`] deep.
	///
	/// The offset is `None` if the field's, or that of an object embedding it,
	/// is negative, or they add up past `usize::MAX`.
	pub fn key_fields(self) -> KeyFields<'a> {
		KeyFields {
			objects: vec![SearchedObject {
				maps: self,
				fields: [].iter(),
				offset: Some(0),
				embedding: None,
			}],
		}
	}
}

impl<'a> Iterator for DataMaps<'a> {
	type Item = DataMap<'a>;

	fn next(&mut self) -> Option<Self::Item> {
		self.remaining = self.remaining.checked_sub(1)?;

		let map = self.next?;

		self.next = map.base();
		Some(map)
	}
}

/// The key fields of an object, with their offsets in it, as
/// [`DataMaps::key_fields`] gives them.
#[derive(Debug, Clone)]
pub struct KeyFields<'a> {
	/// The objects being searched, from the outermost to the most deeply
	/// embedded one.
	objects: Vec<SearchedObject<'a>>,
}

impl<'a> Iterator for KeyFields<'a> {
	type Item = (&'a DataField, Option<usize>);

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			let depth = self.objects.len().checked_sub(1)?;
			let object = self.objects.last_mut()?;

			let Some(field) = object.next_field() else {
				// An embedded object is searched before the field embedding it.
				let object = self.objects.pop()?;

				match object.embedding {
					Some(field) if field.flags & FTYPEDESC_KEY != 0 => {
						return Some((field, object.offset));
					}

					_ => continue,
				}
			};

			let offset = object
				.offset
				.zip(field.offset())
				.and_then(|(object, field)| object.checked_add(field));

			// Embedded objects are searched before the field itself, but not
			// arrays of them.
			if field.fieldType == sys::_fieldtypes_FIELD_EMBEDDED
				&& field.fieldSize == 1
				&& depth < DataMaps::MAX_EMBEDDING_DEPTH
			{
				self.objects.push(SearchedObject {
					maps: field.embedded(),
					fields: [].iter(),
					offset,
					embedding: Some(field),
				});
			} else if field.flags & FTYPEDESC_KEY != 0 {
				return Some((field, offset));
			}
		}
	}
}

/// An object [`KeyFields`] searches.
#[derive(Debug, Clone)]
struct SearchedObject<'a> {
	/// The object's maps left to search.
	maps: DataMaps<'a>,

	/// The fields left to search of the map being searched.
	fields: std::slice::Iter<'a, DataField>,

	/// The object's offset in the outermost object, or `None` if it is
	/// negative or past `usize::MAX`.
	offset: Option<usize>,

	/// The field embedding the object, or `None` for the outermost object.
	embedding: Option<&'a DataField>,
}

impl<'a> SearchedObject<'a> {
	/// The object's next field, from its class's own map towards its bases'.
	fn next_field(&mut self) -> Option<&'a DataField> {
		loop {
			if let Some(field) = self.fields.next() {
				return Some(field);
			}

			self.fields = self.maps.next()?.fields().iter();
		}
	}
}
