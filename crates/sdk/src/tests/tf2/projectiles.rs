//! Tests of TF2's projectiles and flame managers through fake entities'
//! datamaps, networked variables and damage methods.

use super::*;
use crate::Module;
use crate::datatables::PropFlags;
use crate::interfaces::{ServerTools, ValveEngine};

use crate::test_support::datatables::{
	direct_table, int8_proxy, int32_proxy, prop, table, table_prop,
};

use crate::test_support::edicts::{change_accessor, shared_change_info};
use crate::test_support::entities::MOCK_EFLAGS_OFFSET;
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::edicts::FL_FULL_EDICT_CHANGED;
use sdk_raw::entities::{EFL_KILLME, GET_DATA_DESC_MAP_SLOT, NUM_NETWORKED_EHANDLE_BITS};
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::vtable_slot;
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void};
use std::mem::offset_of;
use std::ptr::{NonNull, null_mut};

const _: () = assert!(offset_of!(FakeEntity, flags) == MOCK_EFLAGS_OFFSET);

/// A null handle, in a fake entity's fields.
const NULL: u32 = u32::MAX;

/// A projectile, flame manager, player or weapon, whose datamaps and send
/// tables describe the fields of its class.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	/// Keeps `flags` at the offset every mock entity has it.
	_collideable: *const c_void,
	flags: c_int,
	handle: u32,
	class_name: &'static CStr,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	/// What `GetDamage` returns, and `SetDamage` sets.
	damage: f32,

	// The projectiles' networked variables.
	owner: u32,
	original_launcher: u32,
	launcher: u32,
	thrower: u32,
	deflect_owner: u32,
	deflected: c_int,
	pipebomb_type: c_int,
	projectile_type: c_int,
	critical: u8,
	touched: u8,
	alight: u8,

	// The flame managers'.
	weapon: u32,
	attacker: u32,
	firing: u8,
}

thread_local! {
	/// The fake entities, in the order the entity list holds them.
	static ENTITIES: RefCell<Vec<*mut FakeEntity>> = const { RefCell::new(Vec::new()) };

	/// `gEntList`, which `ServerTools::entity_by_handle` reads.
	static ENTITY_LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };
}

/// The server classes of the fake entities, whose send tables describe their
/// fields.
struct Classes {
	arrow: *mut sys::ServerClass,
	energy_ball: *mut sys::ServerClass,
	flame_manager: *mut sys::ServerClass,
	pipebomb: *mut sys::ServerClass,
	prop: *mut sys::ServerClass,
	rocket: *mut sys::ServerClass,
	syringe: *mut sys::ServerClass,
}

impl Classes {
	fn new() -> Self {
		let offset = |offset: usize| c_int::try_from(offset).unwrap();
		let int = |name, at| {
			prop(
				name,
				sys::SendPropType_DPT_Int,
				offset(at),
				PropFlags::default(),
				Some(int32_proxy),
			)
		};
		let flag = |name, at| {
			prop(
				name,
				sys::SendPropType_DPT_Int,
				offset(at),
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			)
		};
		let handle = |name, at| {
			let mut prop = prop(
				name,
				sys::SendPropType_DPT_Int,
				offset(at),
				PropFlags::UNSIGNED,
				Some(handle_proxy),
			);

			prop.m_nBits = NUM_NETWORKED_EHANDLE_BITS as c_int;
			prop
		};
		let derived = |name, base, props: Vec<sys::SendProp>| {
			let mut all = vec![table_prop(c"baseclass", 0, base, Some(direct_table))];

			all.extend(props);
			leak(table(name, all.leak()))
		};

		let entity_props = vec![handle(c"m_hOwnerEntity", offset_of!(FakeEntity, owner))];
		let entity = leak(table(c"DT_BaseEntity", entity_props.leak()));
		let projectile = derived(
			c"DT_BaseProjectile",
			entity,
			vec![handle(
				c"m_hOriginalLauncher",
				offset_of!(FakeEntity, original_launcher),
			)],
		);

		let rocket_base = derived(
			c"DT_TFBaseRocket",
			projectile,
			vec![
				int(c"m_iDeflected", offset_of!(FakeEntity, deflected)),
				handle(c"m_hLauncher", offset_of!(FakeEntity, launcher)),
			],
		);
		let rocket = derived(
			c"DT_TFProjectile_Rocket",
			rocket_base,
			vec![flag(c"m_bCritical", offset_of!(FakeEntity, critical))],
		);
		let arrow = derived(
			c"DT_TFProjectile_Arrow",
			rocket_base,
			vec![
				flag(c"m_bArrowAlight", offset_of!(FakeEntity, alight)),
				flag(c"m_bCritical", offset_of!(FakeEntity, critical)),
				int(
					c"m_iProjectileType",
					offset_of!(FakeEntity, projectile_type),
				),
			],
		);
		let energy_ball = derived(c"DT_TFProjectile_EnergyBall", rocket_base, vec![]);

		let grenade_base = derived(
			c"DT_BaseGrenade",
			projectile,
			vec![handle(c"m_hThrower", offset_of!(FakeEntity, thrower))],
		);
		let grenade = derived(
			c"DT_TFWeaponBaseGrenadeProj",
			grenade_base,
			vec![
				flag(c"m_bCritical", offset_of!(FakeEntity, critical)),
				int(c"m_iDeflected", offset_of!(FakeEntity, deflected)),
				handle(c"m_hDeflectOwner", offset_of!(FakeEntity, deflect_owner)),
			],
		);
		let pipebomb = derived(
			c"DT_TFProjectile_Pipebomb",
			grenade,
			vec![
				flag(c"m_bTouched", offset_of!(FakeEntity, touched)),
				int(c"m_iType", offset_of!(FakeEntity, pipebomb_type)),
				handle(c"m_hLauncher", offset_of!(FakeEntity, launcher)),
			],
		);

		let nail = derived(
			c"DT_TFBaseProjectile",
			projectile,
			vec![handle(c"m_hLauncher", offset_of!(FakeEntity, launcher))],
		);
		let syringe = derived(c"DT_TFProjectile_Syringe", nail, vec![]);

		let flame_manager = derived(
			c"DT_TFFlameManager",
			entity,
			vec![
				handle(c"m_hWeapon", offset_of!(FakeEntity, weapon)),
				handle(c"m_hAttacker", offset_of!(FakeEntity, attacker)),
				flag(c"m_bIsFiring", offset_of!(FakeEntity, firing)),
			],
		);
		let prop = derived(c"DT_DynamicProp", entity, vec![]);

		let class = |name: &'static CStr, table, id| {
			leak(sys::ServerClass {
				m_pNetworkName: name.as_ptr(),
				m_pTable: table,
				m_pNext: null_mut(),
				m_ClassID: id,
				m_InstanceBaselineIndex: 0,
			})
		};

		Self {
			arrow: class(c"CTFProjectile_Arrow", arrow, 1),
			energy_ball: class(c"CTFProjectile_EnergyBall", energy_ball, 2),
			flame_manager: class(c"CTFFlameManager", flame_manager, 3),
			pipebomb: class(c"CTFGrenadePipebombProjectile", pipebomb, 4),
			prop: class(c"CDynamicProp", prop, 5),
			rocket: class(c"CTFProjectile_Rocket", rocket, 6),
			syringe: class(c"CTFProjectile_Syringe", syringe, 7),
		}
	}
}

/// The datamaps of the fake entities' classes.
struct Maps {
	arrow: *mut sys::datamap_t,
	flame_manager: *mut sys::datamap_t,
	pipebomb: *mut sys::datamap_t,
	prop: *mut sys::datamap_t,
	rocket: *mut sys::datamap_t,
	syringe: *mut sys::datamap_t,
}

impl Maps {
	fn new() -> Self {
		let mut flags = field(
			c"m_iEFlags",
			sys::_fieldtypes_FIELD_INTEGER,
			MOCK_EFLAGS_OFFSET,
		);

		flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

		let base = data_map(c"CBaseEntity", vec![flags], null_mut());
		let rocket = data_map(c"CTFBaseRocket", vec![], base);
		let grenade = data_map(c"CBaseGrenade", vec![], base);
		let grenade = data_map(c"CTFWeaponBaseGrenadeProj", vec![], grenade);

		Self {
			arrow: data_map(c"CTFProjectile_Arrow", vec![], rocket),
			flame_manager: data_map(c"CTFFlameManager", vec![], base),
			pipebomb: data_map(c"CTFGrenadePipebombProjectile", vec![], grenade),
			prop: data_map(c"CDynamicProp", vec![], base),
			rocket,
			syringe: data_map(c"CTFBaseProjectile", vec![], base),
		}
	}
}

/// Mock interfaces exported on this thread, and fake entities of every
/// class.
struct World {
	arrow: *mut FakeEntity,
	demoman: *mut FakeEntity,
	energy_ball: *mut FakeEntity,
	flame_manager: *mut FakeEntity,
	flame_thrower: *mut FakeEntity,
	launcher: *mut FakeEntity,
	medic: *mut FakeEntity,
	/// An entity that is neither a projectile nor a flame manager.
	prop: *mut FakeEntity,
	pyro: *mut FakeEntity,
	/// A rocket the pyro deflected.
	rocket: *mut FakeEntity,
	soldier: *mut FakeEntity,
	/// A stickybomb the pyro deflected.
	stickybomb: *mut FakeEntity,
	syringe: *mut FakeEntity,
}

impl World {
	fn new() -> Self {
		let slot = |field: usize| field / size_of::<usize>();
		let get_damage = vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_GetDamage);
		let set_damage = vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_SetDamage);
		let mut vtable = vec![unexpected_call as *const (); get_damage.max(set_damage) + 1];

		vtable[GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		))] = networkable as *const ();
		vtable[get_damage] = damage as *const ();
		vtable[set_damage] = set_damage_fn as *const ();

		let vtable = vtable.leak().as_ptr();

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let networkable = Box::leak(unsafe {
			mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerNetworkable_GetClassName).write(class_name);
					(&raw mut (*vtable).IServerNetworkable_GetServerClass).write(server_class);
					(&raw mut (*vtable).IServerNetworkable_GetEdict).write(edict);
				},
			)
		});

		let maps = Maps::new();
		let classes = Classes::new();

		export_interfaces();
		ENTITIES.take();

		let spawn = |index: u32, class_name, map, class| {
			let fake = leak(FakeEntity {
				vtable,
				map,
				networkable: sys::IServerNetworkable {
					vtable_: networkable,
				},
				_collideable: null_mut(),
				flags: 0,
				handle: index | 1 << 16,
				class_name,
				class,
				edict: leak(mock_edict(index.cast_signed(), false)),
				damage: 0.0,
				owner: NULL,
				original_launcher: NULL,
				launcher: NULL,
				thrower: NULL,
				deflect_owner: NULL,
				deflected: 0,
				pipebomb_type: 0,
				projectile_type: 0,
				critical: 0,
				touched: 0,
				alight: 0,
				weapon: NULL,
				attacker: NULL,
				firing: 0,
			});

			// SAFETY: The entity list is a leaked, zeroed `CGlobalEntityList`,
			// and the slots are within its `m_EntPtrArray`.
			unsafe {
				let info = (&raw mut (*ENTITY_LIST.get())._base.m_EntPtrArray)
					.cast::<sys::CEntInfo>()
					.add(index as usize);

				(&raw mut (*info).m_pEntity).write(fake.cast());
				(&raw mut (*info).m_SerialNumber).write(1);
			}

			ENTITIES.with_borrow_mut(|entities| entities.push(fake));
			fake
		};

		let player = |index| spawn(index, c"player", maps.prop, classes.prop);
		let weapon = |index| spawn(index, c"tf_weapon", maps.prop, classes.prop);

		let world = Self {
			soldier: player(1),
			pyro: player(2),
			demoman: player(3),
			medic: player(4),
			launcher: weapon(5),
			flame_thrower: weapon(6),
			rocket: spawn(7, c"tf_projectile_rocket", maps.rocket, classes.rocket),
			arrow: spawn(8, c"tf_projectile_arrow", maps.arrow, classes.arrow),
			energy_ball: spawn(
				9,
				c"tf_projectile_energy_ball",
				maps.rocket,
				classes.energy_ball,
			),
			stickybomb: spawn(
				10,
				c"tf_projectile_pipe_remote",
				maps.pipebomb,
				classes.pipebomb,
			),
			syringe: spawn(11, c"tf_projectile_syringe", maps.syringe, classes.syringe),
			flame_manager: spawn(
				12,
				c"tf_flame_manager",
				maps.flame_manager,
				classes.flame_manager,
			),
			prop: spawn(13, c"prop_dynamic", maps.prop, classes.prop),
		};

		// SAFETY: Fake entities are leaked, and the mock game is not running
		// while the test writes their fields.
		unsafe {
			let handle = |fake: *mut FakeEntity| (*fake).handle;

			*world.rocket = FakeEntity {
				owner: handle(world.pyro),
				launcher: handle(world.flame_thrower),
				original_launcher: handle(world.launcher),
				deflected: 1,
				critical: 1,
				damage: 90.0,
				..world.rocket.read()
			};
			*world.arrow = FakeEntity {
				owner: handle(world.soldier),
				projectile_type: raw::TF_PROJECTILE_BUILDING_REPAIR_BOLT,
				alight: 1,
				..world.arrow.read()
			};
			*world.stickybomb = FakeEntity {
				thrower: handle(world.demoman),
				deflect_owner: handle(world.pyro),
				deflected: 2,
				pipebomb_type: raw::TF_GL_MODE_REMOTE_DETONATE,
				touched: 1,
				..world.stickybomb.read()
			};
			*world.syringe = FakeEntity {
				owner: handle(world.medic),
				..world.syringe.read()
			};
			*world.flame_manager = FakeEntity {
				owner: handle(world.flame_thrower),
				weapon: handle(world.flame_thrower),
				attacker: handle(world.pyro),
				firing: 1,
				..world.flame_manager.read()
			};
		}

		world
	}

	/// Whether the engine was told that a fake entity's networked variables
	/// changed.
	fn changed(&self, fake: *mut FakeEntity) -> bool {
		// SAFETY: As for the fields set in `new`.
		unsafe { (*(*fake).edict)._base.m_fStateFlags & FL_FULL_EDICT_CHANGED != 0 }
	}

	/// Reads a field of a fake entity.
	fn get<T>(&self, fake: *mut FakeEntity, field: impl FnOnce(&FakeEntity) -> T) -> T {
		// SAFETY: As for the fields set in `new`.
		field(unsafe { &*fake })
	}

	/// Sets a field of a fake entity, as the game would have.
	fn set(&self, fake: *mut FakeEntity, change: impl FnOnce(&mut FakeEntity)) {
		// SAFETY: As for the fields set in `new`.
		change(unsafe { &mut *fake });
	}
}

unsafe extern "C" fn class_name(this: *const sys::IServerNetworkable) -> *const c_char {
	// SAFETY: Callers pass the networkable of a fake entity.
	unsafe { (*fake_of(this)).class_name.as_ptr() }
}

#[test]
fn criticals_and_damage_are_written() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let rocket = projectile(server, world.rocket);

	assert_eq!(rocket.damage(), 90.0);

	rocket.set_damage(150.0).unwrap();
	assert_eq!(rocket.damage(), 150.0);

	for invalid in [-1.0, f32::NAN, f32::INFINITY] {
		assert!(matches!(
			rocket.set_damage(invalid),
			Err(ProjectileError::InvalidDamage(_))
		));
	}

	assert_eq!(world.get(world.rocket, |rocket| rocket.damage), 150.0);
	assert!(!world.changed(world.rocket));

	rocket.set_critical(false).unwrap();
	assert_eq!(world.get(world.rocket, |rocket| rocket.critical), 0);
	assert!(world.changed(world.rocket));
	assert!(!rocket.is_critical().unwrap());

	// Grenades network whether they are critical too, but Cow Mangler 5000
	// shots do not.
	let stickybomb = projectile(server, world.stickybomb);

	stickybomb.set_critical(true).unwrap();
	assert!(stickybomb.is_critical().unwrap());

	let energy_ball = projectile(server, world.energy_ball);

	assert!(matches!(
		energy_ball.set_critical(true),
		Err(ProjectileError::NetProp(_))
	));
	assert!(!world.changed(world.energy_ball));
}

/// `CBaseEntity::GetDamage`.
unsafe extern "C" fn damage(this: *mut sys::CBaseEntity) -> f32 {
	// SAFETY: As for `datamap`.
	unsafe { (*this.cast::<FakeEntity>()).damage }
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { (*entity.cast::<FakeEntity>()).map }
}

unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this)).edict }
}

/// A callback-scoped entity for a fake one.
fn entity<'s>(fake: *mut FakeEntity) -> Entity<'s> {
	// SAFETY: Fake entities are leaked, so they outlive every scope, and their
	// vtables answer what the wrappers call of an entity.
	unsafe { Entity::from_raw(NonNull::new(fake).unwrap().cast()) }
}

unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
	ENTITY_LIST.get()
}

/// Exports the engine and game interfaces the module uses.
fn export_interfaces() {
	export_standard_proxies();

	ENTITY_LIST.set(Box::leak(Box::<sys::CGlobalEntityList>::new_zeroed()).as_mut_ptr());

	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let tools = Box::leak(unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
			(&raw mut (*vtable).IServerTools_FindEntityByClassname).write(find_by_class_name);
		})
	});
	export(
		Module::GameServer,
		ServerTools::VERSION,
		leak(sys::IServerTools { vtable_: tools }),
	);

	// SAFETY: As for the tools' vtable.
	let engine = Box::leak(unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_GetChangeAccessor).write(change_accessor);
			(&raw mut (*vtable).IVEngineServer_GetSharedEdictChangeInfo).write(shared_change_info);
		})
	});
	export(
		Module::Engine,
		ValveEngine::VERSION,
		leak(sys::IVEngineServer { vtable_: engine }),
	);
}

/// The fake entity whose networkable is `this`.
fn fake_of(this: *const sys::IServerNetworkable) -> *mut FakeEntity {
	// SAFETY: Callers pass the networkable of a fake entity, so the entity
	// starts that far before it.
	unsafe { this.byte_sub(offset_of!(FakeEntity, networkable)) }
		.cast_mut()
		.cast()
}

/// `IServerTools::FindEntityByClassname`, which finds the fake entities whose
/// class names match, in order, as the game does, with a `*` ending the
/// pattern matching any rest.
unsafe extern "C" fn find_by_class_name(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	pattern: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The tools are passed a NUL-terminated class name.
	let pattern = unsafe { CStr::from_ptr(pattern) }.to_bytes();
	let matches = |name: &CStr| match pattern.strip_suffix(b"*") {
		Some(prefix) => name.to_bytes().starts_with(prefix),
		None => name.to_bytes() == pattern,
	};

	ENTITIES.with_borrow(|entities| {
		let start = entities
			.iter()
			.position(|&entity| entity.cast() == after)
			.map_or(0, |index| index + 1);

		entities[start..]
			.iter()
			// SAFETY: Fake entities are leaked.
			.find(|&&entity| matches(unsafe { (*entity).class_name }))
			.map_or(null_mut(), |&entity| entity.cast())
	})
}

#[test]
fn flame_managers_report_whose_flames_they_carry() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let manager = FlameManager::new(server, entity(world.flame_manager)).unwrap();
	let address = |entity: Option<Entity<'_>>| entity.map(|entity| entity.as_ptr().addr());

	assert_eq!(
		address(manager.weapon().unwrap()),
		Some(world.flame_thrower.addr())
	);
	assert_eq!(
		address(manager.attacker().unwrap()),
		Some(world.pyro.addr())
	);
	assert!(manager.is_firing().unwrap());
	assert_eq!(manager.entity().as_ptr().addr(), world.flame_manager.addr());

	// Flame managers are not projectiles, nor projectiles flame managers.
	assert!(matches!(
		Projectile::new(server, entity(world.flame_manager)),
		Err(ProjectileError::NotProjectile)
	));
	assert!(matches!(
		FlameManager::new(server, entity(world.rocket)),
		Err(ProjectileError::NotFlameManager)
	));
	assert!(matches!(
		FlameManager::new(
			null_server(Game::SourceSdk2013, &scope),
			entity(world.flame_manager)
		),
		Err(ProjectileError::WrongGame)
	));
}

/// Stands in for `SendProxy_EHandleToInt`, which no test reaches: the wrappers
/// read handles as stored.
unsafe extern "C" fn handle_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	_: *const c_void,
	_: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	unexpected_call();
}

#[test]
fn kinds_keep_tf2s_classes_and_numbers() {
	let names = ProjectileKind::ALL.map(ProjectileKind::class_name);

	assert!(names.is_sorted());

	for kind in ProjectileKind::ALL {
		assert!(kind.class_name().to_bytes().starts_with(b"tf_projectile_"));
		assert_eq!(
			ProjectileKind::from_class_name(kind.class_name()),
			Some(kind)
		);
	}

	assert_eq!(ProjectileKind::from_class_name(c"tf_flame_manager"), None);
	assert_eq!(
		ProjectileKind::BallOfFire.family(),
		ProjectileFamily::Rocket
	);
	assert_eq!(ProjectileKind::EnergyRing.family(), ProjectileFamily::Nail);
	assert_eq!(ProjectileKind::StunBall.family(), ProjectileFamily::Grenade);

	let arrows = [
		(ArrowKind::Arrow, 8),
		(ArrowKind::HealingBolt, 11),
		(ArrowKind::BuildingRepairBolt, 18),
		(ArrowKind::FestiveArrow, 19),
		(ArrowKind::FestiveHealingBolt, 23),
		(ArrowKind::GrapplingHook, 26),
	];

	for (kind, number) in arrows {
		assert_eq!(kind.to_raw(), number);
		assert_eq!(ArrowKind::from_raw(number), Some(kind));
	}

	assert_eq!(ArrowKind::from_raw(raw::TF_PROJECTILE_ROCKET), None);
	assert_eq!(raw::TF_PROJECTILE_FLAME_BALL, raw::TF_NUM_PROJECTILES - 1);

	let pipebombs = [
		PipebombKind::Grenade,
		PipebombKind::Stickybomb,
		PipebombKind::PracticeStickybomb,
		PipebombKind::Cannonball,
	];

	for (number, kind) in (0..).zip(pipebombs) {
		assert_eq!(kind.to_raw(), number);
		assert_eq!(PipebombKind::from_raw(number), Some(kind));
	}

	assert_eq!(PipebombKind::from_raw(4), None);
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { &raw mut (*entity.cast::<FakeEntity>()).networkable }
}

/// A callback-scoped projectile for a fake one.
fn projectile<'s>(server: Server<'s>, fake: *mut FakeEntity) -> Projectile<'s> {
	Projectile::new(server, entity(fake)).unwrap()
}

#[test]
fn projectiles_are_found_by_class() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let found = || {
		Projectile::all(server)
			.unwrap()
			.map(|projectile| projectile.entity().as_ptr().addr())
			.collect::<Vec<_>>()
	};

	assert_eq!(
		found(),
		[
			world.rocket.addr(),
			world.arrow.addr(),
			world.energy_ball.addr(),
			world.stickybomb.addr(),
			world.syringe.addr(),
		]
	);

	// Projectiles marked for deletion are gone as far as the game is
	// concerned.
	world.set(world.arrow, |arrow| arrow.flags |= EFL_KILLME);
	assert_eq!(
		found(),
		[
			world.rocket.addr(),
			world.energy_ball.addr(),
			world.stickybomb.addr(),
			world.syringe.addr(),
		]
	);

	assert!(matches!(
		Projectile::all(null_server(Game::SourceSdk2013, &scope)),
		Err(ProjectileError::WrongGame)
	));
}

#[test]
fn projectiles_are_wrapped_by_family() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let wrapped = |fake| {
		let projectile = projectile(server, fake);

		(projectile.family(), projectile.kind())
	};

	assert_eq!(
		wrapped(world.rocket),
		(ProjectileFamily::Rocket, Some(ProjectileKind::Rocket))
	);
	assert_eq!(
		wrapped(world.arrow),
		(ProjectileFamily::Rocket, Some(ProjectileKind::Arrow))
	);
	assert_eq!(
		wrapped(world.stickybomb),
		(ProjectileFamily::Grenade, Some(ProjectileKind::PipeRemote))
	);
	assert_eq!(
		wrapped(world.syringe),
		(ProjectileFamily::Nail, Some(ProjectileKind::Syringe))
	);

	// A projectile of a class this crate does not know is still one.
	world.set(world.energy_ball, |ball| {
		ball.class_name = c"tf_projectile_energy_ball_2";
	});
	assert_eq!(wrapped(world.energy_ball), (ProjectileFamily::Rocket, None));

	assert!(matches!(
		Projectile::new(server, entity(world.prop)),
		Err(ProjectileError::NotProjectile)
	));
	assert!(matches!(
		Projectile::new(
			null_server(Game::SourceSdk2013, &scope),
			entity(world.rocket)
		),
		Err(ProjectileError::WrongGame)
	));
}

#[test]
fn projectiles_read_who_fired_them_and_how() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let address = |entity: Option<Entity<'_>>| entity.map(|entity| entity.as_ptr().addr());
	fn missing<T>(result: Result<T, ProjectileError>) -> bool {
		matches!(result, Err(ProjectileError::NetProp(_)))
	}

	// The pyro deflected the soldier's rocket with their flame thrower.
	let rocket = projectile(server, world.rocket);

	assert_eq!(address(rocket.owner().unwrap()), Some(world.pyro.addr()));
	assert_eq!(
		address(rocket.launcher().unwrap()),
		Some(world.flame_thrower.addr())
	);
	assert_eq!(
		address(rocket.original_launcher().unwrap()),
		Some(world.launcher.addr())
	);
	assert_eq!(rocket.deflections().unwrap(), 1);
	assert!(rocket.is_critical().unwrap());
	assert!(missing(rocket.thrower().map(address)));
	assert!(missing(rocket.deflected_by().map(address)));
	assert!(missing(rocket.pipebomb_kind()));
	assert!(missing(rocket.arrow_kind()));

	let arrow = projectile(server, world.arrow);

	assert_eq!(
		arrow.arrow_kind().unwrap(),
		Some(ArrowKind::BuildingRepairBolt)
	);
	assert!(arrow.is_alight().unwrap());
	assert!(!arrow.is_critical().unwrap());
	assert_eq!(address(arrow.launcher().unwrap()), None);

	// The pyro deflected the demoman's stuck stickybomb twice.
	let stickybomb = projectile(server, world.stickybomb);

	assert_eq!(address(stickybomb.owner().unwrap()), None);
	assert_eq!(
		address(stickybomb.thrower().unwrap()),
		Some(world.demoman.addr())
	);
	assert_eq!(
		address(stickybomb.deflected_by().unwrap()),
		Some(world.pyro.addr())
	);
	assert_eq!(stickybomb.deflections().unwrap(), 2);
	assert_eq!(
		stickybomb.pipebomb_kind().unwrap(),
		Some(PipebombKind::Stickybomb)
	);
	assert!(stickybomb.has_touched().unwrap());
	assert!(missing(stickybomb.is_alight()));

	let syringe = projectile(server, world.syringe);

	assert_eq!(address(syringe.owner().unwrap()), Some(world.medic.addr()));
	assert!(missing(syringe.deflections()));
	assert!(missing(syringe.is_critical()));
	assert!(missing(syringe.has_touched()));

	// Handles to entities that no longer exist resolve to none.
	world.set(world.stickybomb, |stickybomb| {
		stickybomb.thrower = 40 | 1 << 16;
	});
	assert_eq!(address(stickybomb.thrower().unwrap()), None);
}

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this)).class }
}

/// `CBaseEntity::SetDamage`.
unsafe extern "C" fn set_damage_fn(this: *mut sys::CBaseEntity, damage: f32) {
	// SAFETY: As for `datamap`.
	unsafe { (*this.cast::<FakeEntity>()).damage = damage };
}
