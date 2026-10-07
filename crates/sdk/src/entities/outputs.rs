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
use sdk_raw::entities::datamap::FTYPEDESC_OUTPUT;
use sdk_raw::util::cstr::copy_cstr;
use std::ffi::{CStr, CString};
use std::ptr::NonNull;

/// The most actions [`Entity::output_actions`] reads of one output.
pub const MAX_ACTIONS: usize = 4096;

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

impl Entity<'_> {
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

		let is_output = field.flags & FTYPEDESC_OUTPUT != 0
			&& field.fieldType == sys::_fieldtypes_FIELD_CUSTOM
			&& offset.is_multiple_of(align_of::<sys::CBaseEntityOutput>());

		if !is_output {
			return None;
		}

		// SAFETY: The entity is live during `'s`, on the main thread, and its own
		// datamaps declare an output at the offset, which `DEFINE_OUTPUT` only
		// declares for a `CBaseEntityOutput`. The member is read without forming
		// a reference, as the game writes it too.
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

		Some(actions)
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
