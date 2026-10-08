//! Tests of entities' health members, found through `CBaseEntity`'s datamap,
//! of the change tracking their setters reach, of TF2 buildings' float
//! health, and of the health methods called through entities' vtables.

use super::*;
use crate::interfaces::ValveEngine;
use crate::server::Module;
use crate::test_support::edicts::{set_change_accessor, set_shared_change_info};

use crate::test_support::entities::{
	MOCK_HEALTH_OFFSET, MOCK_LIFE_STATE_OFFSET, MOCK_MAX_HEALTH_BONUS, MOCK_MAX_HEALTH_OFFSET,
	MOCK_TAKE_DAMAGE_OFFSET, MockEntity, base_entity_fields, health_fields, set_datamap,
	set_networking, take_health_calls,
};

use crate::test_support::sdk_core::change_tracking_engine;
use crate::test_support::server::{export, mock_server};
use sdk_raw::edicts::FL_EDICT_CHANGED;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::data_map;
use std::alloc::Layout;
use std::mem::{offset_of, zeroed};
use std::ptr::null_mut;

#[test]
fn buildings_keep_their_float_health_in_step() {
	let mut mock = MockEntity::with_layout(5, Layout::new::<sys::CBaseObject>());

	set_datamap(health_maps(&[c"CObjectSentrygun", c"CBaseObject"]));
	export_engine();

	let scope = ();
	let server = mock_server(&scope);
	let float_health = |mock: &mut MockEntity| {
		// SAFETY: The mock's storage has the layout of a `CBaseObject`.
		unsafe {
			mock.as_ptr()
				.byte_add(offset_of!(sys::CBaseObject, m_flHealth))
				.cast::<f32>()
				.read()
		}
	};

	mock.entity().set_health(server, 150).unwrap();
	assert_eq!(float_health(&mut mock), 150.0);
	assert_eq!(mock.int(MOCK_HEALTH_OFFSET), 150);

	// The integer health is the float rounded up, as `CBaseObject::SetHealth`
	// sets it.
	mock.entity().set_health(server, 16_777_217).unwrap();
	assert_eq!(float_health(&mut mock), 16_777_216.0);
	assert_eq!(mock.int(MOCK_HEALTH_OFFSET), 16_777_216);

	// TF2 cannot convert a float of 2^31 back.
	assert_eq!(
		mock.entity().set_health(server, 2_147_483_584),
		Err(HealthError::UnrepresentableHealth(2_147_483_584))
	);
	assert_eq!(
		mock.entity().set_max_health(server, c_int::MAX),
		Err(HealthError::UnrepresentableHealth(c_int::MAX))
	);
	assert_eq!(float_health(&mut mock), 16_777_216.0);

	// `TakeHealth` only adds to the integer health, which the float follows.
	mock.entity().set_health(server, 100).unwrap();
	assert_eq!(mock.entity().heal(server, 25.0), Ok(25));
	assert_eq!(float_health(&mut mock), 125.0);
}

/// Makes the mock server's factories on this thread export an engine whose
/// change tracking is set through `set_change_accessor` and
/// `set_shared_change_info`.
fn export_engine() {
	export(
		Module::Engine,
		ValveEngine::VERSION,
		change_tracking_engine().as_ptr(),
	);
}

/// A datamap chain from the first of `classes` to the last, and from there to
/// a `CBaseEntity` map declaring the members mock entities store, health
/// among them.
fn health_maps(classes: &[&'static CStr]) -> *mut sys::datamap_t {
	let mut fields = Vec::from(base_entity_fields());

	fields.extend(health_fields());

	classes.iter().rev().fold(
		data_map(c"CBaseEntity", fields, null_mut()),
		|base, &class| data_map(class, vec![], base),
	)
}

#[test]
fn health_members_are_read_and_written_at_their_datamap_offsets() {
	let mut mock = MockEntity::new(5);

	set_datamap(health_maps(&[c"CTFPlayer"]));
	mock.set_int(MOCK_HEALTH_OFFSET, 125);
	mock.set_int(MOCK_MAX_HEALTH_OFFSET, 150);
	mock.set_byte(MOCK_LIFE_STATE_OFFSET, LIFE_DYING);
	mock.set_byte(MOCK_TAKE_DAMAGE_OFFSET, raw::DAMAGE_YES);

	let entity = mock.entity();

	assert_eq!(entity.health(), Ok(125));
	assert_eq!(entity.stored_max_health(), Ok(150));
	assert_eq!(entity.life_state(), Ok(LifeState::Dying));
	assert_eq!(entity.damage_mode(), Ok(DamageMode::Vulnerable));

	mock.set_byte(MOCK_LIFE_STATE_OFFSET, 7);
	mock.set_byte(MOCK_TAKE_DAMAGE_OFFSET, 9);

	let entity = mock.entity();

	assert_eq!(entity.life_state(), Err(HealthError::UnknownLifeState(7)));
	assert_eq!(entity.damage_mode(), Err(HealthError::UnknownDamageMode(9)));

	// Each write lands at the member's offset, and the networked entity's
	// edict records the offset as changed.
	// SAFETY: Zero is valid for every field of `CSharedEdictChangeInfo`.
	let mut shared = Box::new(unsafe { zeroed::<sys::CSharedEdictChangeInfo>() });
	let mut accessor = sys::IChangeInfoAccessor {
		m_iChangeInfo: 0,
		m_iChangeInfoSerialNumber: 0,
	};
	let mut edict = mock_edict(5, false);

	shared.m_iSerialNumber = 7;
	set_change_accessor(&raw mut accessor);
	set_shared_change_info(&raw mut *shared);
	set_networking(null_mut(), &raw mut edict);
	export_engine();

	let scope = ();
	let server = mock_server(&scope);
	let entity = mock.entity();

	entity.set_health(server, -20).unwrap();
	entity.set_max_health(server, 300).unwrap();
	entity
		.set_life_state(server, LifeState::Respawnable)
		.unwrap();
	entity.set_damage_mode(server, DamageMode::Immune).unwrap();

	assert_eq!(mock.int(MOCK_HEALTH_OFFSET), -20);
	assert_eq!(mock.int(MOCK_MAX_HEALTH_OFFSET), 300);
	assert_eq!(mock.byte(MOCK_LIFE_STATE_OFFSET), LIFE_RESPAWNABLE);
	assert_eq!(mock.byte(MOCK_TAKE_DAMAGE_OFFSET), raw::DAMAGE_NO);
	assert_ne!(edict._base.m_fStateFlags & FL_EDICT_CHANGED, 0);

	let changes = &shared.m_ChangeInfos[0];
	let offsets = [
		MOCK_HEALTH_OFFSET,
		MOCK_MAX_HEALTH_OFFSET,
		MOCK_LIFE_STATE_OFFSET,
		MOCK_TAKE_DAMAGE_OFFSET,
	]
	.map(|offset| u16::try_from(offset).unwrap());

	assert_eq!(changes.m_nChangeOffsets, 4);
	assert_eq!(changes.m_ChangeOffsets[..4], offsets);

	set_change_accessor(null_mut());
	set_shared_change_info(null_mut());
	set_networking(null_mut(), null_mut());
}

#[test]
fn health_methods_are_called_through_the_entitys_vtable() {
	let mut mock = MockEntity::new(5);

	set_datamap(health_maps(&[c"CTFPlayer"]));
	mock.set_int(MOCK_HEALTH_OFFSET, 100);
	mock.set_int(MOCK_MAX_HEALTH_OFFSET, 150);

	let scope = ();
	let server = mock_server(&scope);

	assert_eq!(mock.entity().heal(server, 10.75), Ok(10));
	assert_eq!(take_health_calls(), [(10.75, raw::DMG_GENERIC)]);
	assert_eq!(mock.int(MOCK_HEALTH_OFFSET), 110);
	assert_eq!(
		mock.entity().max_health(server),
		Ok(150 + MOCK_MAX_HEALTH_BONUS)
	);

	assert!(mock.entity().is_alive());
	mock.set_byte(MOCK_LIFE_STATE_OFFSET, LIFE_DEAD);
	assert!(!mock.entity().is_alive());

	#[cfg(feature = "tf2")]
	{
		use crate::tf2::damage::DamageType;
		use sdk_raw::tf2::damage::DMG_IGNORE_MAXHEALTH;

		let overheal = DamageType::IGNORE_MAX_HEALTH;

		assert_eq!(mock.entity().take_health(server, 5.0, overheal), Ok(5));
		assert_eq!(take_health_calls()[1], (5.0, DMG_IGNORE_MAXHEALTH));
	}
}
