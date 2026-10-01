//! TF2's legacy/default numeric attributes, through the game's attribute manager.
//!
//! Mutations use the native methods that notify the manager and invalidate its
//! provider caches. Writing sendprops alone does not update those caches.
//! The native getters support the schema's default 32-bit gameplay attribute
//! type, not attributes declaring the separate `"float"` type or other types.

use crate::entities::{Entity, data_map_class};
use crate::script_binding::{self as binding, BindingError};
use crate::{Game, Server};
use std::ffi::CStr;

/// An attribute operation could not be performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttributeError {
	#[error("attributes require Team Fortress 2")]
	UnsupportedGame,

	#[error("the entity is neither a TF2 player nor an economy item")]
	UnsupportedEntity,

	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	#[error("attribute values must be finite and player durations must be positive")]
	InvalidValue,

	#[error("the game does not expose the expected native attribute method")]
	UnsupportedMethod,

	#[error("the native attribute method rejected the call")]
	Rejected,
}

impl From<BindingError> for AttributeError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// Callback-scoped access to attributes on a TF2 weapon, wearable, or player.
///
/// Names are the item schema's attribute names, such as `"damage bonus"` or
/// `"move speed bonus"`, rather than the attribute class/hook names. These
/// methods support the schema's legacy/default 32-bit numeric attribute type
/// (`CSchemaAttributeType_Default`), used by ordinary gameplay attributes.
/// This is distinct from an explicit schema `attribute_type` of `"float"`.
/// Explicit float, string, blob, and 64-bit schema types are unsupported.
#[derive(Debug, Clone, Copy)]
pub struct Attributes<'s> {
	entity: Entity<'s>,
	player: bool,
}

impl<'s> Attributes<'s> {
	pub fn new(server: &Server<'s>, entity: Entity<'s>) -> Result<Self, AttributeError> {
		if server.game() != Game::TeamFortress2 {
			return Err(AttributeError::UnsupportedGame);
		}

		for map in entity.data_maps() {
			match data_map_class(map) {
				Some(name) if name == c"CTFPlayer" => {
					return Ok(Self {
						entity,
						player: true,
					});
				}

				Some(name) if name == c"CEconEntity" => {
					return Ok(Self {
						entity,
						player: false,
					});
				}

				_ => {}
			}
		}

		Err(AttributeError::UnsupportedEntity)
	}

	fn check_live(self) -> Result<(), AttributeError> {
		if self.entity.is_marked_for_deletion() {
			Err(AttributeError::MarkedForDeletion)
		} else {
			Ok(())
		}
	}

	fn class(self) -> &'static CStr {
		if self.player {
			c"CTFPlayer"
		} else {
			c"CEconEntity"
		}
	}

	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Looks up the attribute's legacy numeric storage as a float, matching
	/// the native scripting getter. For items this includes static
	/// item-definition attributes as well as runtime overrides. For players it
	/// reads the player's own list, not values supplied by equipped weapons.
	/// An unknown name, absent attribute, unsupported schema type (including
	/// explicit `"float"`), or stored NaN returns `None`. The getter uses NaN
	/// as its missing-value sentinel and cannot distinguish these cases.
	pub fn get(self, name: &CStr) -> Result<Option<f32>, AttributeError> {
		self.check_live()?;

		let method = if self.player {
			c"GetCustomAttribute"
		} else {
			c"GetAttribute"
		};

		// SAFETY: These two native getters only iterate the respective attribute
		// lists. Strings remain alive for the synchronous lookup. A NaN fallback
		// distinguishes absence from all values accepted by `set`.
		let result = unsafe {
			binding::call(
				self.entity,
				self.class(),
				method,
				&mut [binding::string(name), binding::float(f32::NAN)],
				binding::FLOAT,
			)
		}?;
		// SAFETY: `call` checked FIELD_FLOAT before returning.
		let value = unsafe { result.__bindgen_anon_1.m_float };

		Ok((!value.is_nan()).then_some(value))
	}

	/// Removes a runtime override and refreshes the manager's caches. An item
	/// can still expose a static item-definition value afterwards. On players,
	/// this removes attributes registered by `set`/`AddCustomAttribute`.
	pub fn remove(self, name: &CStr) -> Result<(), AttributeError> {
		self.check_live()?;

		let method = if self.player {
			c"RemoveCustomAttribute"
		} else {
			c"RemoveAttribute"
		};

		// SAFETY: These native methods remove list entries and invalidate caches;
		// they do not destroy entities or retain the name pointer.
		unsafe {
			binding::call(
				self.entity,
				self.class(),
				method,
				&mut [binding::string(name)],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// As `set`, with a player-only expiry. Item attributes reject a duration.
	///
	/// # Safety
	/// The full contract of [`Self::set_unchecked`] applies, including its default schema
	/// type requirement and attribute-specific valid value domain.
	pub unsafe fn set_for_unchecked(
		self,
		name: &CStr,
		value: f32,
		duration: Option<f32>,
	) -> Result<bool, AttributeError> {
		self.check_live()?;

		if !value.is_finite()
			|| duration.is_some_and(|seconds| !seconds.is_finite() || seconds <= 0.0)
			|| (!self.player && duration.is_some())
		{
			return Err(AttributeError::InvalidValue);
		}

		let method = if self.player {
			c"AddCustomAttribute"
		} else {
			c"AddAttribute"
		};

		// SAFETY: Both native methods copy/consume the schema name and change
		// the attribute list via its manager. They do not destroy entities.
		unsafe {
			binding::call(
				self.entity,
				self.class(),
				method,
				&mut [
					binding::string(name),
					binding::float(value),
					binding::float(duration.unwrap_or(-1.0)),
				],
				binding::VOID,
			)
		}?;

		Ok(self.get(name)?.is_some_and(|stored| stored == value))
	}

	/// Sets a runtime numeric attribute and refreshes the manager's caches.
	/// Returns true when immediate value readback equals `value`. Under the
	/// schema contract below, false means an unknown/ignored attribute or a
	/// value that did not remain equal on readback; it does not roll back a write.
	///
	/// Player durations are seconds; `None` keeps the attribute until explicitly
	/// removed or the game resets it. Item attributes have no native expiry, so
	/// their duration is always permanent. Attributes are not saved to inventory
	/// and can be replaced by respawn/loadout regeneration.
	///
	/// # Safety
	/// If `name` exists in the item schema, it must use the legacy/default
	/// 32-bit numeric type (`CSchemaAttributeType_Default`) and support gameplay
	/// modification and networking (`BSupportsGameplayModificationAndNetworking`).
	/// The separate explicit `"float"` schema type is not supported by the
	/// native readback iterator. String/blob attributes are unsafe to write:
	/// the native runtime list would later treat the float's bits as a pointer.
	/// `value` must also be valid for that attribute's gameplay domain; merely
	/// being finite does not prevent an extreme multiplier overflowing later
	/// native damage, health, or movement calculations.
	pub unsafe fn set_unchecked(self, name: &CStr, value: f32) -> Result<bool, AttributeError> {
		// SAFETY: The caller vouches for the schema attribute's runtime type.
		unsafe { self.set_for_unchecked(name, value, None) }
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::InterfaceFactory;
	use crate::entities::test_support::{base_entity_fields, data_map};
	use std::cell::Cell;
	use std::ffi::{c_char, c_void};
	use std::mem::zeroed;
	use std::ptr::{NonNull, null_mut};

	#[repr(C)]
	struct FakeEntity {
		vtable: *const *const (),
		map: *mut sys::datamap_t,
		description: *mut sys::ScriptClassDesc_t,
		padding: usize,
		flags: i32,
		value: Cell<f32>,
		duration: Cell<f32>,
		calls: Cell<usize>,
	}

	unsafe extern "C" fn adapter(
		function: sys::ScriptFunctionBindingStorageType_t,
		object: *mut c_void,
		arguments: *mut sys::ScriptVariant_t,
		_: i32,
		result: *mut sys::ScriptVariant_t,
	) -> bool {
		let object = unsafe { &*object.cast::<FakeEntity>() };
		object.calls.set(object.calls.get() + 1);
		let name = unsafe { CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString) };
		match function.val_0 {
			0 => {
				let fallback = unsafe { (*arguments.add(1)).__bindgen_anon_1.m_float };
				let value = if name == c"damage bonus" && !object.value.get().is_nan() {
					object.value.get()
				} else {
					fallback
				};
				unsafe { result.write(binding::float(value)) };
			}

			1 => {
				assert!(result.is_null());
				if name == c"damage bonus" {
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

	unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
		unsafe { (*entity.cast::<FakeEntity>()).map }
	}

	unsafe extern "C" fn description(entity: *mut sys::CBaseEntity) -> *mut sys::ScriptClassDesc_t {
		unsafe { (*entity.cast::<FakeEntity>()).description }
	}

	unsafe extern "C" fn factory(_: *const c_char, _: *mut i32) -> *mut c_void {
		null_mut()
	}

	#[test]
	fn player_and_item_attributes_dispatch_typed_methods_and_reject_invalid_values() {
		for player in [true, false] {
			let names = if player {
				[
					c"GetCustomAttribute",
					c"AddCustomAttribute",
					c"RemoveCustomAttribute",
				]
			} else {
				[c"GetAttribute", c"AddAttribute", c"RemoveAttribute"]
			};
			let class = if player { c"CTFPlayer" } else { c"CEconEntity" };
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
				function.m_pfnBinding = Some(adapter);
				function.m_pFunction.val_0 = i as isize;
			}
			let mut description: sys::ScriptClassDesc_t = unsafe { zeroed() };
			description.m_pszClassname = class.as_ptr();
			description.m_FunctionBindings = vector(&mut functions);
			let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
			let map = data_map(class, vec![], base);
			let mut table = [std::ptr::null(); 16];
			table[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT] = datamap as *const ();
			table[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT + 1] = self::description as *const ();
			let mut object = FakeEntity {
				vtable: table.as_ptr(),
				map,
				description: &raw mut description,
				padding: 0,
				flags: 0,
				value: Cell::new(f32::NAN),
				duration: Cell::new(0.0),
				calls: Cell::new(0),
			};
			assert_eq!(
				std::mem::offset_of!(FakeEntity, flags),
				crate::entities::test_support::MOCK_EFLAGS_OFFSET
			);
			let scope = ();
			let factory = InterfaceFactory::new(factory);
			let server = unsafe { Server::new(factory, factory, Game::TeamFortress2, &scope) };
			let entity = unsafe { Entity::from_raw(NonNull::from(&mut object).cast()) };
			let attributes = Attributes::new(&server, entity).unwrap();
			assert_eq!(attributes.get(c"damage bonus").unwrap(), None);
			// SAFETY: The mock schema implements this numeric attribute only.
			assert!(unsafe { attributes.set_unchecked(c"damage bonus", 1.5) }.unwrap());
			assert_eq!(attributes.get(c"damage bonus").unwrap(), Some(1.5));
			assert_eq!(object.duration.get(), -1.0);
			assert!(!unsafe { attributes.set_unchecked(c"unknown", 1.5) }.unwrap());
			let calls = object.calls.get();
			assert_eq!(
				unsafe { attributes.set_unchecked(c"damage bonus", f32::NAN) },
				Err(AttributeError::InvalidValue)
			);
			assert_eq!(
				unsafe { attributes.set_for_unchecked(c"damage bonus", 2.0, Some(0.0)) },
				Err(AttributeError::InvalidValue)
			);
			assert_eq!(object.calls.get(), calls);
			if player {
				assert!(
					unsafe { attributes.set_for_unchecked(c"damage bonus", 2.0, Some(5.0)) }
						.unwrap()
				);
				assert_eq!(object.duration.get(), 5.0);
			} else {
				assert_eq!(
					unsafe { attributes.set_for_unchecked(c"damage bonus", 2.0, Some(5.0)) },
					Err(AttributeError::InvalidValue)
				);
			}
			attributes.remove(c"damage bonus").unwrap();
			assert_eq!(attributes.get(c"damage bonus").unwrap(), None);
		}
	}

	fn vector<T>(values: &mut [T]) -> sys::CUtlVector<T, sys::CUtlMemory<T>> {
		sys::CUtlVector {
			_phantom_0: Default::default(),
			_phantom_1: Default::default(),
			m_Memory: sys::CUtlMemory {
				_phantom_0: Default::default(),
				m_pMemory: values.as_mut_ptr(),
				m_nAllocationCount: values.len() as i32,
				m_nGrowSize: 0,
			},
			m_Size: values.len() as i32,
			m_pElements: values.as_mut_ptr(),
		}
	}
}
