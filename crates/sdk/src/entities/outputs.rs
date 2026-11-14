//! The outputs of entities: the events an entity fires, such as a trigger's
//! `OnTrigger`, each with the actions it takes as it fires. Each action sends
//! an input to the entities a target names, as a map's connections and the
//! `AddOutput` input add them.
//!
//! An output is a field of its entity (`CBaseEntityOutput`), which its class
//! declares in its data description map with the output's name, and holds a
//! list of its actions (`CEventAction`). [`Entity::output_actions`] copies
//! that list.

#[cfg(test)]
#[path = "../tests/entities/outputs.rs"]
mod tests;

use crate::entities::Entity;
use sdk_raw::entities::datamap::{DataField, DataMaps, FTYPEDESC_OUTPUT};
use sdk_raw::util::cstr::copy_cstr;
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::ptr::NonNull;

/// The most actions [`Entity::output_actions`] reads of one output.
pub const MAX_ACTIONS: usize = 4096;

/// The most outputs [`Entity::outputs`] lists of one entity.
pub const MAX_OUTPUTS: usize = 1024;

/// An output of an entity, by the name a map's connections give it, with the
/// actions it takes as it fires, as [`Entity::outputs`] lists them.
#[doc(alias("CBaseEntityOutput"))]
#[derive(Debug, Clone, PartialEq)]
pub struct Output<'s> {
	/// The output's name, such as `OnTrigger`, as the data description map of
	/// the entity's class or a base declares it. Connections and the
	/// `AddOutput` input name it ignoring ASCII case.
	pub name: &'s CStr,

	/// The actions the output takes as it fires, in order.
	pub actions: Vec<OutputAction>,
}

/// An action an output takes as it fires: sending an input to the entities a
/// target names (`CEventAction`).
#[doc(alias("CEventAction"))]
#[derive(Debug, Clone, PartialEq)]
pub struct OutputAction {
	/// What the input is sent to: a name, which matches the entities so named,
	/// ignoring ASCII case, and may end in a `*` wildcard, or a procedural name
	/// such as `!activator`. The game sends the input to every entity of the
	/// name, or if none has it, to every entity of the class of that name.
	pub target: CString,

	/// The name of the input sent.
	pub input: CString,

	/// The input's parameter, or empty to send the value the output fires
	/// with.
	pub parameter: CString,

	/// Seconds after the output fires that the input is sent.
	pub delay: f32,

	/// How many more times the action is taken before the game removes it, or
	/// `None` for every time the output fires.
	pub times_to_fire: Option<u32>,
}

impl<'s> Entity<'s> {
	/// The actions of the entity's output named `name`, such as `OnTrigger`, in
	/// the order the output takes them. Returns `None` if the entity has no
	/// output of that name.
	///
	/// The output is the field the entity's data description maps declare with
	/// that external name, found as `KeyValue` finds the output a map's
	/// connection adds its action to, ignoring ASCII case. At most
	/// [`MAX_ACTIONS`] are read.
	#[doc(alias("CBaseEntityOutput", "m_ActionList"))]
	pub fn output_actions(self, name: &CStr) -> Option<Vec<OutputAction>> {
		let (field, offset) = self.data_maps().find_key_field(name.to_bytes())?;

		// SAFETY: The field and its offset were found in the entity's own
		// datamaps.
		unsafe { self.output_actions_at(field, offset) }
	}

	/// Copies the actions of the output `field` declares at `offset`, or
	/// returns `None` unless it declares one there (see [`is_output`]). At most
	/// [`MAX_ACTIONS`] are read.
	///
	/// # Safety
	///
	/// `field` must be declared by the entity's own datamaps, and `offset` be
	/// its offset in the entity, as `DataMaps::find_key_field` and
	/// `DataMaps::key_fields` give them.
	unsafe fn output_actions_at(
		self,
		field: &DataField,
		offset: usize,
	) -> Option<Vec<OutputAction>> {
		if !is_output(field, offset) {
			return None;
		}

		// SAFETY: The entity's own datamaps declare an output at the offset, as
		// the caller promises and `is_output` checked.
		Some(unsafe { self.actions_at(offset) })
	}

	/// Copies the actions of the output at `offset`, in order. At most
	/// [`MAX_ACTIONS`] are read.
	///
	/// # Safety
	///
	/// The entity's own datamaps must declare an output at `offset`, as
	/// [`is_output`] tells of a field they declare there.
	unsafe fn actions_at(self, offset: usize) -> Vec<OutputAction> {
		// SAFETY: The entity is live during `'s`, on the main thread, and its own
		// datamaps declare an output at the offset, as the caller promises, which
		// `DEFINE_OUTPUT` only declares for a `CBaseEntityOutput`. The member is
		// read without forming a reference, as the game writes it too.
		let mut next = unsafe {
			let output = self
				.as_ptr()
				.byte_add(offset)
				.cast::<sys::CBaseEntityOutput>();

			(&raw const (*output).m_ActionList).read()
		};

		let mut actions = Vec::new();

		while let Some(action) = NonNull::new(next)
			&& actions.len() < MAX_ACTIONS
		{
			// SAFETY: The game allocates the output's actions, and changes and frees
			// them only on the main thread, where this runs.
			let action = unsafe { action.read() };

			// SAFETY: The game names actions with pooled strings, or null.
			let (target, input, parameter) = unsafe {
				(
					copy_string(action.m_iTarget),
					copy_string(action.m_iTargetInput),
					copy_string(action.m_iParameter),
				)
			};

			actions.push(OutputAction {
				target,
				input,
				parameter,
				delay: action.m_flDelay,
				times_to_fire: u32::try_from(action.m_nTimesToFire).ok(),
			});

			next = action.m_pNext;
		}

		actions
	}

	/// Every output of the entity, such as a trigger's `OnStartTouch`, each with
	/// its actions in the order it takes them.
	///
	/// The outputs are the fields the entity's data description maps declare as
	/// outputs, in the order `KeyValue` searches them for the output a map's
	/// connection adds its action to: from its class's own map towards its
	/// bases', and through each embedded object before the fields after it, as
	/// [`DataMaps::key_fields`] gives them. A name is listed once, ignoring
	/// ASCII case, for the first field of that name, which is the one
	/// [`Self::output_actions`] reads: the game reaches no other. At most
	/// [`MAX_OUTPUTS`] are listed, each with at most [`MAX_ACTIONS`] actions.
	///
	/// Which fields of a class are outputs is found once and kept, since a
	/// class's maps never change; the actions are read from the entity each
	/// time.
	#[doc(alias("CBaseEntityOutput", "m_ActionList"))]
	pub fn outputs(self) -> Vec<Output<'s>> {
		// SAFETY: The entity is live.
		let map = unsafe { sdk_raw::entities::data_desc_map(self.as_ptr()) };

		// SAFETY: As for `Entity::data_maps`: the maps are statics of the game
		// DLL, complete since the first entity of the class exists, and the DLL
		// stays loaded for `'s` (`Server::new`).
		let maps = unsafe { DataMaps::new(map) };

		found::output_fields(map, || output_fields(maps))
			.into_iter()
			.filter_map(|(field, offset)| {
				Some(Output {
					name: field.external_name()?,
					// SAFETY: `output_fields` found an output at the offset in the
					// entity's own datamaps.
					actions: unsafe { self.actions_at(offset) },
				})
			})
			.collect()
	}
}

/// Copies a string the game names an action's target, input or parameter
/// with, or an empty one for null.
///
/// # Safety
///
/// `string` must be null, or point to a C string that stays allocated during
/// the call, as the game's pooled strings do.
unsafe fn copy_string(string: sys::string_t) -> CString {
	// SAFETY: As the caller promises.
	unsafe { copy_cstr(string.pszValue) }.unwrap_or_default()
}

/// Whether `field`, declared at `offset` in an entity, is an output, as
/// `DEFINE_OUTPUT` declares them: a `FIELD_CUSTOM` field flagged
/// [`FTYPEDESC_OUTPUT`], at an offset aligned for a `CBaseEntityOutput`.
fn is_output(field: &DataField, offset: usize) -> bool {
	field.flags & FTYPEDESC_OUTPUT != 0
		&& field.fieldType == sys::_fieldtypes_FIELD_CUSTOM
		&& offset.is_multiple_of(align_of::<sys::CBaseEntityOutput>())
}

/// The outputs `maps` declare, each with its offset in the entity, in the order
/// [`Entity::outputs`] lists them: the first key field of each name, ignoring
/// ASCII case, that is an output. At most [`MAX_OUTPUTS`] are found.
fn output_fields(maps: DataMaps<'_>) -> Vec<(&DataField, usize)> {
	let mut names = HashSet::new();
	let mut fields = Vec::new();

	for (field, offset) in maps.key_fields() {
		let Some(name) = field.external_name() else {
			continue;
		};

		// The first field of a name hides the others, which may not even be
		// outputs.
		if !names.insert(name.to_bytes().to_ascii_lowercase()) {
			continue;
		}

		if let Some(offset) = offset
			&& is_output(field, offset)
		{
			fields.push((field, offset));

			if fields.len() == MAX_OUTPUTS {
				break;
			}
		}
	}

	fields
}

/// Output fields for builds with tests, found anew every time: tests build
/// maps that they free, and that later maps may replace at the same
/// addresses.
#[cfg(any(test, feature = "_test-support"))]
mod found {
	use sdk_raw::entities::datamap::DataField;

	/// What `find` returns.
	pub(super) fn output_fields<'a>(
		_map: *const sys::datamap_t,
		find: impl FnOnce() -> Vec<(&'a DataField, usize)>,
	) -> Vec<(&'a DataField, usize)> {
		find()
	}
}

/// The output fields of each class, kept for the rest of the process.
///
/// A class's data description maps are statics of the game server module,
/// complete before the first entity of the class exists and never changed
/// afterwards, and the module stays loaded as long as plugins are, so the
/// fields found in them stay true. The key is the address of the class's own
/// map.
#[cfg(not(any(test, feature = "_test-support")))]
mod found {
	use sdk_raw::entities::datamap::DataField;
	use std::collections::HashMap;
	use std::sync::{Mutex, PoisonError};

	/// The addresses and offsets of a class's output fields.
	type Fields = Box<[(usize, usize)]>;

	/// Each class's output fields, by the address of its own map.
	static OUTPUT_FIELDS: Mutex<Option<HashMap<usize, Fields>>> = Mutex::new(None);

	/// The output fields of the class whose own map is `map`, which `find`
	/// finds the first time.
	pub(super) fn output_fields<'a>(
		map: *const sys::datamap_t,
		find: impl FnOnce() -> Vec<(&'a DataField, usize)>,
	) -> Vec<(&'a DataField, usize)> {
		if map.is_null() {
			return find();
		}

		let key = map as usize;
		let mut kept = OUTPUT_FIELDS.lock().unwrap_or_else(PoisonError::into_inner);
		let kept = kept.get_or_insert_default();

		if let Some(fields) = kept.get(&key) {
			return fields
				.iter()
				.map(|&(field, offset)| {
					// SAFETY: The address is that of a field `find` found in the maps
					// from `map`, statics of the game server module, which stay
					// loaded and unchanged for `'a` as the maps `find` searches do.
					(unsafe { &*(field as *const DataField) }, offset)
				})
				.collect();
		}

		let fields = find();

		kept.insert(
			key,
			fields
				.iter()
				.map(|&(field, offset)| (std::ptr::from_ref(field) as usize, offset))
				.collect(),
		);

		fields
	}
}
