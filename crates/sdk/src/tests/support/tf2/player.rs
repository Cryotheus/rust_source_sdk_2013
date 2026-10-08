//! A fake TF2 player, or another networked entity: the native script methods
//! a test declares, answered by the test, and the networked variables it
//! declares, which a test reads and writes by name.

use super::script_binding::{SCRIPT_DESCRIPTION_SLOT, class_description, member_binding};
use crate::Module;
use crate::datatables::PropFlags;
use crate::entities::{Entity, EntityHandle};
use crate::interfaces::ValveEngine;

use crate::test_support::datatables::{
	direct_table, int8_proxy, int32_proxy, prop, table, table_prop,
};

use crate::test_support::edicts::{change_accessor, shared_change_info};
use crate::test_support::entities::MOCK_EFLAGS_OFFSET;
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::export;
use sdk_raw::edicts::{FL_EDICT_CHANGED, FL_FULL_EDICT_CHANGED};
use sdk_raw::entities::{NUM_NETWORKED_EHANDLE_BITS, NUM_SERIAL_NUM_SHIFT_BITS};
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::script_binding::{BOOL, FLOAT, HANDLE, INT, STRING, VOID};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::mem::offset_of;
use std::ptr::{NonNull, null, null_mut};

const _: () = assert!(offset_of!(FakePlayer, flags) == MOCK_EFLAGS_OFFSET);
const _: () = assert!(offset_of!(FakePlayer, instance) == INSTANCE_OFFSET);
const _: () = assert!(offset_of!(FakePlayer, script_id) == SCRIPT_ID_OFFSET);

/// Where fake entities hold `m_hScriptInstance`, just before `m_iszScriptId`.
///
/// The crate finds the instance once per process, so every fake entity of
/// its tests holds it here, as `tests/tf2/script_instances.rs`'s do.
const INSTANCE_OFFSET: usize = 72;

/// Where fake entities hold `m_iszScriptId`, as their datamap declares.
const SCRIPT_ID_OFFSET: usize = 80;

/// How many 32-bit slots fake entities have for their networked variables.
const SLOTS: usize = 64;

thread_local! {
	/// The edict index the next fake entity takes.
	static NEXT_INDEX: Cell<c_int> = const { Cell::new(1) };
}

/// What a fake entity's method answers a call with: `None` to reject it, as an
/// adapter does for arguments of the wrong type.
type Handler = Box<dyn FnMut(&FakePlayer, &CStr, &[Value]) -> Option<Value>>;

/// A TF2 player, or another networked entity, whose methods and networked
/// variables a test declares.
///
/// For tests only. Fake entities are leaked, and the game's code reaches them
/// only through raw pointers.
#[repr(C)]
pub struct FakePlayer {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	description: *mut sys::ScriptClassDesc_t,
	/// `m_iEFlags`, at [`MOCK_EFLAGS_OFFSET`].
	flags: Cell<c_int>,
	class_name: *const c_char,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	/// The handle `GetRefEHandle` points to.
	handle: sys::CBaseHandle,
	/// `m_hScriptInstance`, at [`INSTANCE_OFFSET`].
	instance: Cell<sys::HSCRIPT>,
	/// `m_iszScriptId`, at [`SCRIPT_ID_OFFSET`].
	script_id: *const u8,
	/// The networked variables, each at the slots `layout` gives it.
	slots: [Cell<u32>; SLOTS],
	/// Each variable's name, and its first slot.
	layout: Vec<(&'static CStr, usize)>,
	/// The calls of the entity's methods, in order.
	calls: RefCell<Vec<(&'static CStr, Vec<Value>)>>,
	/// What answers the calls.
	handler: RefCell<Option<Handler>>,
}

impl FakePlayer {
	/// A TF2 player, of class name `player` and server class `CTFPlayer`,
	/// whose `CTFPlayer` script class declares `methods`, and whose send table
	/// declares `vars`. Its datamaps chain `CTFPlayer`, `CBasePlayer` and
	/// `CBaseEntity`.
	pub fn new(methods: &[Method], vars: &[Var]) -> &'static Self {
		let base = data_map(c"CBaseEntity", base_entity_fields(), null_mut());
		let map = data_map(c"CTFPlayer", vec![], data_map(c"CBasePlayer", vec![], base));

		Self::build(
			c"player",
			(c"CTFPlayer", c"DT_TFPlayer"),
			map,
			Some(c"CTFPlayer"),
			methods,
			vars,
		)
	}

	/// Another networked entity, of class name `class_name` and of server
	/// class and datamap `class`, whose send table, named `table`, declares
	/// `vars`. Its script class is `CBaseEntity`.
	pub fn other(
		class_name: &'static CStr,
		class: &'static CStr,
		table: &'static CStr,
		vars: &[Var],
	) -> &'static Self {
		let base = data_map(c"CBaseEntity", base_entity_fields(), null_mut());

		Self::build(
			class_name,
			(class, table),
			data_map(class, vec![], base),
			None,
			&[],
			vars,
		)
	}

	/// Sets what answers the entity's method calls, other than
	/// `ValidateScriptScope`, which registers its script instance.
	pub fn answer(&self, handler: impl FnMut(&Self, &CStr, &[Value]) -> Option<Value> + 'static) {
		self.handler.replace(Some(Box::new(handler)));
	}

	fn build(
		class_name: &'static CStr,
		(class, table_name): (&'static CStr, &'static CStr),
		map: *mut sys::datamap_t,
		script_class: Option<&'static CStr>,
		methods: &[Method],
		vars: &[Var],
	) -> &'static Self {
		let validate = Box::leak(Box::new([member_binding(
			c"ValidateScriptScope",
			BOOL,
			&mut [],
			Some(dispatch),
		)]));

		validate[0].m_pFunction.val_0 = c"ValidateScriptScope".as_ptr() as isize;

		let mut description = leak(class_description(c"CBaseEntity", validate, null_mut()));

		if let Some(script_class) = script_class {
			let bindings = methods
				.iter()
				.map(|method| {
					let parameters = Box::leak(method.parameters.to_vec().into_boxed_slice());
					let mut binding =
						member_binding(method.name, method.returns, parameters, Some(dispatch));

					binding.m_pFunction.val_0 = method.name.as_ptr() as isize;
					binding
				})
				.collect::<Vec<_>>()
				.leak();

			description = leak(class_description(script_class, bindings, description));
		}

		let mut layout = Vec::new();
		let mut props = Vec::new();
		let mut next = 0;

		for var in vars {
			let offset = offset_of!(Self, slots) + next * size_of::<u32>();
			let offset_int = c_int::try_from(offset).unwrap();

			layout.push((var.name, next));
			next += var.kind.slots();
			assert!(next <= SLOTS, "too many networked variables");

			props.push(match var.kind {
				VarKind::Bool => prop(
					var.name,
					sys::SendPropType_DPT_Int,
					offset_int,
					PropFlags::UNSIGNED,
					Some(int8_proxy),
				),

				VarKind::Float => prop(
					var.name,
					sys::SendPropType_DPT_Float,
					offset_int,
					PropFlags::default(),
					Some(int32_proxy),
				),

				VarKind::Floats(len) => {
					let elements = (0..len)
						.map(|index| {
							let name = CString::new(format!("{index:03}")).unwrap();

							prop(
								Box::leak(name.into_boxed_c_str()),
								sys::SendPropType_DPT_Float,
								c_int::try_from(index * size_of::<f32>()).unwrap(),
								PropFlags::default(),
								Some(int32_proxy),
							)
						})
						.collect::<Vec<_>>()
						.leak();

					table_prop(
						var.name,
						offset_int,
						leak(table(var.name, elements)),
						Some(direct_table),
					)
				}

				VarKind::Handle => {
					let mut handle = prop(
						var.name,
						sys::SendPropType_DPT_Int,
						offset_int,
						PropFlags::UNSIGNED,
						Some(handle_proxy),
					);

					handle.m_nBits = NUM_NETWORKED_EHANDLE_BITS as c_int;
					handle
				}

				VarKind::Int | VarKind::UInt => prop(
					var.name,
					sys::SendPropType_DPT_Int,
					offset_int,
					if var.kind == VarKind::UInt {
						PropFlags::UNSIGNED
					} else {
						PropFlags::default()
					},
					Some(int32_proxy),
				),
			});
		}

		let class = leak(sys::ServerClass {
			m_pNetworkName: class.as_ptr(),
			m_pTable: leak(table(table_name, props.leak())),
			m_pNext: null_mut(),
			m_ClassID: 1,
			m_InstanceBaselineIndex: 0,
		});

		let index = NEXT_INDEX.get();

		NEXT_INDEX.set(index + 1);

		Box::leak(Box::new(Self {
			vtable: entity_vtable(),
			map,
			networkable: sys::IServerNetworkable {
				vtable_: networkable_vtable(),
			},
			description,
			flags: Cell::new(0),
			class_name: class_name.as_ptr(),
			class,
			edict: leak(mock_edict(index, false)),
			instance: Cell::new(null_mut()),
			script_id: null(),
			handle: sys::CBaseHandle {
				m_Index: index.cast_unsigned() | 1 << NUM_SERIAL_NUM_SHIFT_BITS,
			},
			slots: [const { Cell::new(0) }; SLOTS],
			layout,
			calls: RefCell::new(Vec::new()),
			handler: RefCell::new(None),
		}))
	}

	/// The callback-scoped entity of the fake.
	pub fn entity(&'static self) -> Entity<'static> {
		// SAFETY: Fake entities are leaked, and their vtables answer what the
		// wrappers call of an entity.
		unsafe { Entity::from_raw(NonNull::from(self).cast()) }
	}

	/// The first slot of the variable `name`.
	///
	/// # Panics
	///
	/// If the entity declares no such variable.
	fn slot(&self, name: &CStr) -> usize {
		self.layout
			.iter()
			.find(|(declared, _)| *declared == name)
			.unwrap_or_else(|| panic!("no networked variable {name:?}"))
			.1
	}

	/// The float the variable `name` holds, or its element `index`.
	pub fn float(&self, name: &CStr, index: usize) -> f32 {
		f32::from_bits(self.slots[self.slot(name) + index].get())
	}

	/// The bits the variable `name` holds.
	pub fn get(&self, name: &CStr) -> u32 {
		self.slots[self.slot(name)].get()
	}

	/// The entity's handle, with serial number 1.
	pub fn handle(&self) -> EntityHandle {
		EntityHandle::from_raw(self.handle.m_Index)
	}

	/// The script instance `ValidateScriptScope` registered, or null.
	pub fn instance(&self) -> sys::HSCRIPT {
		self.instance.get()
	}

	/// Sets the float of the variable `name`, or of its element `index`.
	pub fn set_float(&self, name: &CStr, index: usize, value: f32) {
		self.slots[self.slot(name) + index].set(value.to_bits());
	}

	/// Sets the entity's `m_iEFlags`.
	pub fn set_flags(&self, flags: c_int) {
		self.flags.set(flags);
	}

	/// Sets the bits of the variable `name`. A `bool` keeps its value in the
	/// lowest byte.
	pub fn set(&self, name: &CStr, bits: u32) {
		self.slots[self.slot(name)].set(bits);
	}

	/// The calls of the entity's methods since the last take, with their
	/// arguments.
	pub fn take_calls(&self) -> Vec<(&'static CStr, Vec<Value>)> {
		self.calls.take()
	}

	/// Whether the engine was told that the entity's networked variables
	/// changed since the last take.
	pub fn take_changed(&self) -> bool {
		// SAFETY: The edict is leaked, and only the wrappers' writes of the
		// variables, on this thread, touch its flags.
		unsafe {
			let flags = &raw mut (*self.edict)._base.m_fStateFlags;
			let changed = flags.read() & (FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED) != 0;

			flags.write(0);
			changed
		}
	}
}

/// A native method of a fake player's `CTFPlayer` script class.
#[derive(Debug, Clone, Copy)]
pub struct Method {
	name: &'static CStr,
	returns: sys::ScriptDataType_t,
	parameters: &'static [sys::ScriptDataType_t],
}

impl Method {
	/// The method `name`, returning `returns` and taking `parameters`.
	pub const fn new(
		name: &'static CStr,
		returns: sys::ScriptDataType_t,
		parameters: &'static [sys::ScriptDataType_t],
	) -> Self {
		Self {
			name,
			returns,
			parameters,
		}
	}
}

/// An argument or result of a fake entity's method.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
	Bool(bool),
	Float(f32),
	Handle(sys::HSCRIPT),
	Int(c_int),
	String(CString),
	Void,
}

impl Value {
	/// The value of an argument variant.
	///
	/// # Safety
	///
	/// The variant's union member is the one its type names, and a string's
	/// pointer is valid.
	unsafe fn of(variant: &sys::ScriptVariant_t) -> Self {
		let value = &variant.__bindgen_anon_1;

		// SAFETY: As the caller promises.
		unsafe {
			match c_int::from(variant.m_type) {
				BOOL => Self::Bool(value.m_bool),
				FLOAT => Self::Float(value.m_float),
				HANDLE => Self::Handle(value.m_hScript),
				INT => Self::Int(value.m_int),
				STRING => Self::String(CStr::from_ptr(value.m_pszString).to_owned()),
				VOID => Self::Void,
				other => panic!("unexpected argument type {other}"),
			}
		}
	}

	/// The result variant of the value.
	fn variant(&self) -> sys::ScriptVariant_t {
		use sdk_raw::tf2::script_binding::{boolean, float, handle, int};

		match *self {
			Self::Bool(value) => boolean(value),
			Self::Float(value) => float(value),
			Self::Handle(value) => handle(value),
			Self::Int(value) => int(value),
			Self::String(_) | Self::Void => panic!("not a result fake methods return"),
		}
	}
}

/// A networked variable of a fake entity.
#[derive(Debug, Clone, Copy)]
pub struct Var {
	name: &'static CStr,
	kind: VarKind,
}

impl Var {
	/// The variable `name`, declared and stored as `kind` says.
	pub const fn new(name: &'static CStr, kind: VarKind) -> Self {
		Self { name, kind }
	}
}

/// How a fake entity declares and stores a networked variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
	/// A `bool`, as `SendPropBool` declares one: unsigned, in a byte.
	Bool,

	/// A float.
	Float,

	/// That many floats, as `SendPropArray3` declares an array: a table of
	/// elements named by their index.
	Floats(usize),

	/// An entity handle, as `SendPropEHandle` declares one.
	Handle,

	/// A signed 32-bit integer.
	Int,

	/// A 32-bit integer networked as unsigned.
	UInt,
}

impl VarKind {
	/// The slots the variable takes.
	const fn slots(self) -> usize {
		match self {
			Self::Floats(len) => len,
			_ => 1,
		}
	}
}

/// The fields `CBaseEntity`'s map declares that fake entities store:
/// `m_iEFlags` and `m_iszScriptId`.
fn base_entity_fields() -> Vec<sys::typedescription_t> {
	let mut flags = field(
		c"m_iEFlags",
		sys::_fieldtypes_FIELD_INTEGER,
		MOCK_EFLAGS_OFFSET,
	);

	flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

	let mut script_id = field(
		c"m_iszScriptId",
		sys::_fieldtypes_FIELD_STRING,
		SCRIPT_ID_OFFSET,
	);

	script_id.fieldSizeInBytes = size_of::<sys::string_t>() as c_int;
	vec![flags, script_id]
}

unsafe extern "C" fn class_name(this: *const sys::IServerNetworkable) -> *const c_char {
	// SAFETY: Only fake entities' networkables have this method, and they are
	// fields of their leaked entities.
	unsafe { (*fake_of(this)).class_name }
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { (*entity.cast::<FakePlayer>()).map }
}

unsafe extern "C" fn description(entity: *mut sys::CBaseEntity) -> *mut sys::ScriptClassDesc_t {
	// SAFETY: As for `datamap`.
	unsafe { (*entity.cast::<FakePlayer>()).description }
}

/// The adapter of every fake method: notes the call, and answers it, or
/// registers the instance for `ValidateScriptScope`.
unsafe extern "C" fn dispatch(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	// SAFETY: Fake bindings keep their method's static name in the function's
	// first word, and are only declared by fake entities' descriptors, so the
	// object is a leaked fake entity, only ever shared.
	let (name, object) = unsafe {
		(
			CStr::from_ptr(function.val_0 as *const c_char),
			&*object.cast::<FakePlayer>(),
		)
	};

	let arguments = (0..usize::try_from(count).unwrap())
		// SAFETY: The caller passes `count` initialized arguments, of the types
		// the binding declares, whose pointers are valid for the call.
		.map(|index| unsafe { Value::of(&*arguments.add(index)) })
		.collect::<Vec<_>>();

	object.calls.borrow_mut().push((name, arguments.clone()));

	let answer = if name == c"ValidateScriptScope" {
		if object.instance.get().is_null() {
			object.instance.set(leak(0_u8).cast());
		}

		Some(Value::Bool(true))
	} else {
		match object.handler.take() {
			Some(mut handler) => {
				let answer = handler(object, name, &arguments);

				object.handler.replace(Some(handler));
				answer
			}

			None if result.is_null() => Some(Value::Void),
			None => panic!("no answer for {name:?}"),
		}
	};

	let Some(answer) = answer else {
		return false;
	};

	if !result.is_null() {
		// SAFETY: A method with a result is passed a writable one.
		unsafe { result.write(answer.variant()) };
	}

	true
}

unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this)).edict }
}

/// The vtable of fake entities.
fn entity_vtable() -> *const *const () {
	let slot = |field: usize| field / size_of::<usize>();
	let networkable_slot = slot(offset_of!(
		sys::IServerEntity__bindgen_vtable,
		IServerEntity_GetNetworkable
	));
	let handle_slot = slot(offset_of!(
		sys::IServerEntity__bindgen_vtable,
		IServerEntity_GetRefEHandle
	));
	let slots = SCRIPT_DESCRIPTION_SLOT
		.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT)
		.max(networkable_slot)
		.max(handle_slot)
		+ 1;
	let mut vtable = vec![unexpected_call as *const (); slots];

	vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
	vtable[networkable_slot] = networkable as *const ();
	vtable[handle_slot] = ref_handle as *const ();
	vtable[SCRIPT_DESCRIPTION_SLOT] = description as *const ();
	vtable.leak().as_ptr()
}

/// The fake entity whose networkable is `this`.
fn fake_of(this: *const sys::IServerNetworkable) -> *const FakePlayer {
	// SAFETY: Callers pass the networkable of a fake entity, so the entity
	// starts that far before it.
	unsafe { this.byte_sub(offset_of!(FakePlayer, networkable)) }.cast()
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

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { (&raw const (*entity.cast::<FakePlayer>()).networkable).cast_mut() }
}

/// The vtable of fake entities' networkables.
fn networkable_vtable() -> *mut sys::IServerNetworkable__bindgen_vtable {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	Box::leak(unsafe {
		mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
			unexpected_call as *const (),
			|vtable| {
				(&raw mut (*vtable).IServerNetworkable_GetClassName).write(class_name);
				(&raw mut (*vtable).IServerNetworkable_GetServerClass).write(server_class);
				(&raw mut (*vtable).IServerNetworkable_GetEdict).write(edict);
			},
		)
	})
}

unsafe extern "C" fn ref_handle(entity: *const sys::IServerEntity) -> *const sys::CBaseHandle {
	// SAFETY: As for `datamap`.
	unsafe { &raw const (*entity.cast::<FakePlayer>()).handle }
}

/// Exports the engine and the game DLL, for the networked variables of fake
/// entities on this thread.
pub fn serve_interfaces() {
	export_standard_proxies();

	// SAFETY: As for the networkables' vtable.
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

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this)).class }
}
