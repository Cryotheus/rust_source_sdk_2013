//! Item creation through stand-ins for the game's item generation functions.

use super::*;
use std::cell::Cell;

/// The definition index the stand-in schema has.
const KNOWN: u16 = 18;

/// The arguments the stand-in `SpawnItem` was last called with: the
/// singleton, the definition, the level, the quality, and whether a classname
/// was given.
type Spawned = (usize, c_int, c_int, sys::entityquality_t, bool);

/// What the stand-in `GetItemDefinition` returns for unknown indices.
static mut DEFAULT_DEFINITION: u8 = 0;

/// What the stand-in `SpawnItem` creates.
static mut ENTITY: u8 = 0;

/// What the stand-in `GetItemDefinition` returns for [`KNOWN`].
static mut KNOWN_DEFINITION: u8 = 0;

/// The stand-in `CItemGeneration` singleton.
static mut SINGLETON: [u8; SINGLETON_LEN] = [0; SINGLETON_LEN];

/// What the stand-in schema getter returns, holding the schema
/// `SCHEMA_OFFSET` bytes in.
static mut SYSTEM: [u8; platform::SCHEMA_OFFSET + 8] = [0; platform::SCHEMA_OFFSET + 8];

thread_local! {
	static SPAWNED: Cell<Option<Spawned>> = const { Cell::new(None) };
}

unsafe extern "C" fn get_item_definition(
	_this: *mut sys::CEconItemSchema,
	index: c_int,
) -> *mut sys::CEconItemDefinition {
	if index == c_int::from(KNOWN) {
		(&raw mut KNOWN_DEFINITION).cast()
	} else {
		(&raw mut DEFAULT_DEFINITION).cast()
	}
}

unsafe extern "C" fn schema_getter() -> *mut c_void {
	(&raw mut SYSTEM).cast()
}

unsafe extern "C" fn spawn_item(
	this: *mut sys::CItemGeneration,
	definition: c_int,
	_origin: *const sys::Vector,
	_angles: *const sys::QAngle,
	level: c_int,
	quality: sys::entityquality_t,
	classname: *const c_char,
) -> *mut sys::CBaseEntity {
	SPAWNED.set(Some((
		this.addr(),
		definition,
		level,
		quality,
		!classname.is_null(),
	)));

	(&raw mut ENTITY).cast()
}

/// Item generation through the stand-ins.
fn generation() -> ItemGeneration {
	ItemGeneration {
		get_item_definition,
		schema_getter,
		singleton: NonZeroUsize::new((&raw mut SINGLETON).expose_provenance()).unwrap(),
		spawn_item,
	}
}

fn origin() -> sys::Vector {
	sys::Vector {
		x: 0.0,
		y: 0.0,
		z: 0.0,
	}
}

#[test]
fn items_are_spawned_at_the_level_and_quality_asked_for() {
	let generation = generation();
	let singleton = (&raw mut SINGLETON).addr();

	// SAFETY: The stand-ins are plain functions over this test's statics.
	let entity = unsafe { generation.spawn(KNOWN, origin(), None) }.unwrap();

	assert_eq!(entity.as_ptr().cast::<u8>(), &raw mut ENTITY);

	assert_eq!(
		SPAWNED.take(),
		Some((
			singleton,
			c_int::from(KNOWN),
			1,
			sys::EEconItemQuality_AE_UNIQUE,
			false
		))
	);

	// SAFETY: As above.
	unsafe {
		generation.spawn_with_quality(
			KNOWN,
			origin(),
			Some(c"tf_weapon_rocketlauncher"),
			100,
			sys::EEconItemQuality_AE_STRANGE,
		)
	}
	.unwrap();

	assert_eq!(
		SPAWNED.take(),
		Some((
			singleton,
			c_int::from(KNOWN),
			100,
			sys::EEconItemQuality_AE_STRANGE,
			true
		))
	);
}

#[test]
fn unknown_definitions_are_not_spawned() {
	// SAFETY: The stand-ins are plain functions over this test's statics.
	let result = unsafe {
		generation().spawn_with_quality(
			KNOWN + 1,
			origin(),
			None,
			5,
			sys::EEconItemQuality_AE_VINTAGE,
		)
	};

	assert_eq!(result, Err(ItemGenerationError::UnknownDefinition));
	assert_eq!(SPAWNED.take(), None);
}
