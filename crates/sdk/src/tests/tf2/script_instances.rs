//! Tests of `crate::tf2::script_instances`: fake entities whose
//! `ValidateScriptScope` binding registers their script instances, and a fake
//! entity list to find them in.

use super::*;
use crate::Module;
use crate::interfaces::ServerTools;
use crate::test_support::entities::{MOCK_EFLAGS_OFFSET, base_entity_fields};
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding,
};

use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::script_binding::{BOOL, INT, VECTOR};
use std::cell::Cell;
use std::ffi::{c_int, c_void};
use std::mem::offset_of;
use std::ptr::{null, null_mut};

/// Where fake entities hold `m_hScriptInstance`.
const INSTANCE_OFFSET: usize = 72;

/// Where fake entities hold `m_iszScriptId`, as their datamap declares.
const SCRIPT_ID_OFFSET: usize = 80;

thread_local! {
	/// The entities the fake entity list holds, in order.
	static LISTED: Cell<&'static [*mut sys::CBaseEntity]> = const { Cell::new(&[]) };
}

/// An entity with a `CBaseEntity` datamap and script descriptor, whose
/// `ValidateScriptScope` registers an instance for it.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	description: *mut sys::ScriptClassDesc_t,
	padding: usize,
	/// `m_iEFlags`, at [`MOCK_EFLAGS_OFFSET`].
	flags: Cell<c_int>,
	/// The members of other tests' mock entities, which these leave zero.
	others: [usize; 4],
	/// `m_hScriptInstance`, at [`INSTANCE_OFFSET`].
	instance: Cell<sys::HSCRIPT>,
	/// `m_iszScriptId`, at [`SCRIPT_ID_OFFSET`].
	script_id: *const u8,
	/// The instance `ValidateScriptScope` registers, or null to register none.
	registers: Cell<sys::HSCRIPT>,
	/// What `ValidateScriptScope` returns: false without a script VM.
	has_vm: Cell<bool>,
	/// How often `ValidateScriptScope` ran.
	validations: Cell<usize>,
}

impl FakeEntity {
	fn entity(&'static self) -> Entity<'static> {
		// SAFETY: Fake entities are leaked, and their vtables answer the
		// datamap and script descriptor lookups the wrappers make.
		unsafe { Entity::from_raw(NonNull::from(self).cast()) }
	}
}

#[test]
fn arguments_carry_borrowed_vectors_and_angles() {
	let vector = sys::Vector {
		x: 1.0,
		y: 2.0,
		z: 3.0,
	};
	let angles = sys::QAngle {
		x: 10.0,
		y: 20.0,
		z: 30.0,
	};
	let vector_argument = raw::vector(&vector);
	let angles_argument = raw::qangle(&angles);

	assert_eq!(i32::from(vector_argument.m_type), VECTOR);
	assert_eq!(i32::from(angles_argument.m_type), raw::QANGLE);
	assert_eq!((vector_argument.m_flags, angles_argument.m_flags), (0, 0));

	// SAFETY: The builders set these union members, to the borrowed values.
	unsafe {
		assert_eq!(
			vector_argument.__bindgen_anon_1.m_pVector,
			&raw const vector
		);
		assert_eq!(
			angles_argument.__bindgen_anon_1.m_pData.cast_const(),
			(&raw const angles).cast()
		);
	}
}

/// `CBaseEntity::GetScriptDesc`, which returns the fake entity's descriptor.
unsafe extern "C" fn description(entity: *mut sys::CBaseEntity) -> *mut sys::ScriptClassDesc_t {
	// SAFETY: Only fake entities have this vtable, and they are leaked.
	unsafe { (*entity.cast::<FakeEntity>()).description }
}

/// `IServerTools::FirstEntity`, the first of [`LISTED`].
unsafe extern "C" fn first_entity(_: *mut sys::IServerTools) -> *mut sys::CBaseEntity {
	LISTED.get().first().copied().unwrap_or(null_mut())
}

/// A distinct, never dereferenced script instance handle.
fn handle() -> sys::HSCRIPT {
	leak(0_u8).cast()
}

#[test]
fn instances_are_found_again_among_the_entities() {
	let scope = ();
	let server = mock_server(&scope);
	let first = mock_entity(handle(), Some(script_id_field(SCRIPT_ID_OFFSET)));
	let second = mock_entity(handle(), Some(script_id_field(SCRIPT_ID_OFFSET)));
	let unlisted = mock_entity(handle(), Some(script_id_field(SCRIPT_ID_OFFSET)));

	serve_entities(&[first, second]);

	let instance = ScriptInstance::of(server, second.entity()).unwrap();

	assert_eq!(instance.entity(server), Ok(Some(second.entity())));
	assert_eq!(
		ScriptInstance::of(server, first.entity())
			.unwrap()
			.entity(server),
		Ok(Some(first.entity()))
	);

	// An entity removed since, or a script object that is no entity.
	let elsewhere = ScriptInstance::of(server, unlisted.entity()).unwrap();

	assert_eq!(elsewhere.entity(server), Ok(None));

	// SAFETY: The handle is never dereferenced.
	let object = unsafe { ScriptInstance::from_raw(handle()) }.unwrap();

	assert_eq!(object.entity(server), Ok(None));
}

#[test]
fn instances_are_made_once_through_validate_script_scope() {
	let scope = ();
	let server = mock_server(&scope);
	let registered = handle();
	let fake = mock_entity(registered, Some(script_id_field(SCRIPT_ID_OFFSET)));
	let entity = fake.entity();

	assert_eq!(ScriptInstance::existing(server, entity), Ok(None));
	assert_eq!(fake.validations.get(), 0);

	let instance = ScriptInstance::of(server, entity).unwrap();

	assert_eq!(instance.as_raw(), registered);
	assert_eq!(fake.validations.get(), 1);

	// The existing instance is returned as is.
	assert_eq!(ScriptInstance::of(server, entity), Ok(instance));
	assert_eq!(ScriptInstance::existing(server, entity), Ok(Some(instance)));
	assert_eq!(fake.validations.get(), 1);

	// SAFETY: Null is not wrapped.
	assert_eq!(unsafe { ScriptInstance::from_raw(null_mut()) }, None);
}

#[test]
fn instances_need_tf2_a_script_vm_and_a_live_entity() {
	let scope = ();
	let server = mock_server(&scope);
	let fake = mock_entity(handle(), Some(script_id_field(SCRIPT_ID_OFFSET)));
	let entity = fake.entity();

	let other = null_server(Game::SourceSdk2013, &scope);

	assert_eq!(
		ScriptInstance::of(other, entity),
		Err(ScriptInstanceError::WrongGame)
	);
	assert_eq!(
		ScriptInstance::existing(other, entity),
		Err(ScriptInstanceError::WrongGame)
	);

	// No VM, as under `-scripting`.
	fake.has_vm.set(false);
	assert_eq!(
		ScriptInstance::of(server, entity),
		Err(ScriptInstanceError::NoScriptVm)
	);
	assert_eq!(fake.validations.get(), 1);
	fake.has_vm.set(true);

	// A VM that registers nothing, as a layout the datamap misled would.
	let unregistered = mock_entity(null_mut(), Some(script_id_field(SCRIPT_ID_OFFSET)));

	assert_eq!(
		ScriptInstance::of(server, unregistered.entity()),
		Err(ScriptInstanceError::UnsupportedMethod)
	);

	// An entity marked for deletion has given its instance up.
	fake.flags.set(sdk_raw::entities::EFL_KILLME);
	assert_eq!(
		ScriptInstance::of(server, entity),
		Err(ScriptInstanceError::MarkedForDeletion)
	);
	assert_eq!(fake.validations.get(), 1);
	fake.flags.set(0);

	// A binding of another signature is not called.
	// SAFETY: The descriptor chain is leaked, and only read through these
	// pointers by the wrappers.
	unsafe {
		let base = (*fake.description).m_pBaseDesc;
		let binding = (*base).m_FunctionBindings.m_Memory.m_pMemory;

		(*binding).m_desc.m_ReturnType = INT;
	}

	assert_eq!(
		ScriptInstance::of(server, entity),
		Err(ScriptInstanceError::UnsupportedMethod)
	);
	assert_eq!(fake.validations.get(), 1);

	// Listing entities needs IServerTools.
	// SAFETY: The handle is never dereferenced.
	let instance = unsafe { ScriptInstance::from_raw(handle()) }.unwrap();

	assert_eq!(
		instance.entity(server),
		Err(ScriptInstanceError::NoServerTools)
	);
	assert_eq!(instance.entity(other), Err(ScriptInstanceError::WrongGame));
}

/// `CBaseEntity::GetDataDescMap`, which returns the fake entity's datamap.
unsafe extern "C" fn map(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake entities have this vtable, and they are leaked.
	unsafe { (*entity.cast::<FakeEntity>()).map }
}

/// A leaked entity whose `ValidateScriptScope` registers `registers`, and
/// whose datamap declares `script_id` as `m_iszScriptId` if it is given.
fn mock_entity(
	registers: sys::HSCRIPT,
	script_id: Option<sys::typedescription_t>,
) -> &'static FakeEntity {
	let bindings = Box::leak(Box::new([member_binding(
		c"ValidateScriptScope",
		BOOL,
		&mut [],
		Some(validate_script_scope),
	)]));

	let base = leak(class_description(c"CBaseEntity", bindings, null_mut()));
	let derived = leak(class_description(c"CTFPlayer", &mut [], base));
	let mut fields = Vec::from(base_entity_fields());

	fields.extend(script_id);

	let vtable = Box::leak(Box::new([null::<()>(); SCRIPT_DESCRIPTION_SLOT + 1]));

	vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = map as *const ();
	vtable[SCRIPT_DESCRIPTION_SLOT] = description as *const ();

	let entity = Box::leak(Box::new(FakeEntity {
		vtable: vtable.as_ptr(),
		map: data_map(c"CBaseEntity", fields, null_mut()),
		description: derived,
		padding: 0,
		flags: Cell::new(0),
		others: [0; 4],
		instance: Cell::new(null_mut()),
		script_id: null(),
		registers: Cell::new(registers),
		has_vm: Cell::new(true),
		validations: Cell::new(0),
	}));

	assert_eq!(offset_of!(FakeEntity, flags), MOCK_EFLAGS_OFFSET);
	assert_eq!(offset_of!(FakeEntity, instance), INSTANCE_OFFSET);
	assert_eq!(offset_of!(FakeEntity, script_id), SCRIPT_ID_OFFSET);
	entity
}

/// `IServerTools::NextEntity`, the one after `previous` in [`LISTED`].
unsafe extern "C" fn next_entity(
	_: *mut sys::IServerTools,
	previous: *mut sys::CBaseEntity,
) -> *mut sys::CBaseEntity {
	let listed = LISTED.get();

	listed
		.iter()
		.position(|&entity| entity == previous)
		.and_then(|index| listed.get(index + 1))
		.copied()
		.unwrap_or(null_mut())
}

#[test]
fn offsets_need_an_aligned_script_id_after_the_instance() {
	let offset = |script_id: Option<sys::typedescription_t>| {
		let fake = mock_entity(null_mut(), script_id);

		raw::script_instance_offset(fake.entity().data_maps())
	};

	assert_eq!(
		offset(Some(script_id_field(SCRIPT_ID_OFFSET))),
		Some(INSTANCE_OFFSET)
	);
	assert_eq!(offset(None), None);

	// The instance handle would precede the entity, or be misaligned.
	assert_eq!(offset(Some(script_id_field(0))), None);
	assert_eq!(offset(Some(script_id_field(SCRIPT_ID_OFFSET + 4))), None);

	// The script ID must be a pooled string.
	let mut integer = script_id_field(SCRIPT_ID_OFFSET);

	integer.fieldType = sys::_fieldtypes_FIELD_INTEGER;
	assert_eq!(offset(Some(integer)), None);
}

/// The `m_iszScriptId` field fake entities declare, at `offset`.
fn script_id_field(offset: usize) -> sys::typedescription_t {
	let mut script_id = field(c"m_iszScriptId", sys::_fieldtypes_FIELD_STRING, offset);

	script_id.fieldSizeInBytes = size_of::<sys::string_t>() as c_int;
	script_id
}

/// Exports an `IServerTools` listing `entities`, for this thread's mock
/// servers.
fn serve_entities(entities: &[&'static FakeEntity]) {
	let listed = entities
		.iter()
		.map(|&entity| std::ptr::from_ref(entity).cast_mut().cast())
		.collect::<Vec<_>>();

	LISTED.set(listed.leak());

	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_FirstEntity).write(first_entity);
			(&raw mut (*vtable).IServerTools_NextEntity).write(next_entity);
		})
	};

	let tools = leak(sys::IServerTools {
		vtable_: Box::leak(vtable),
	});

	export(Module::GameServer, ServerTools::VERSION, tools);
}

/// `bool CBaseEntity::ValidateScriptScope()`, which registers the entity's
/// instance, as `GetScriptInstance` does, when the game has a script VM.
unsafe extern "C" fn validate_script_scope(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	_: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(count, 0);
	assert!(!result.is_null());

	// SAFETY: The binding is only declared by fake entities' descriptors, so
	// the object is a fake entity, which is leaked and only ever shared.
	let object = unsafe { &*object.cast::<FakeEntity>() };

	object.validations.set(object.validations.get() + 1);

	if object.has_vm.get() && object.instance.get().is_null() {
		object.instance.set(object.registers.get());
	}

	// SAFETY: The binding returns a bool, so the caller passes a writable
	// result.
	unsafe { result.write(sdk_raw::tf2::script_binding::boolean(object.has_vm.get())) };
	true
}

#[test]
fn vector_and_angle_results_are_refused_before_the_call() {
	let fake = mock_entity(handle(), Some(script_id_field(SCRIPT_ID_OFFSET)));

	for result_type in [VECTOR, raw::QANGLE] {
		// SAFETY: The fake entity is leaked, and the call is refused before any
		// binding is looked up.
		let result = unsafe {
			raw::call(
				NonNull::from(fake).cast(),
				c"CBaseEntity",
				c"ValidateScriptScope",
				&mut [],
				result_type,
			)
		};

		assert_eq!(result.err(), Some(BindingError::SignatureMismatch));
	}

	assert_eq!(fake.validations.get(), 0);
}
