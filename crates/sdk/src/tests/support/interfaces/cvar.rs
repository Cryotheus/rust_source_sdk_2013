//! Console commands and variables as other modules declare them, and a mock
//! `ICvar` registry listing them.

use crate::commands::{CommandBaseKind, CommandFlags};
use sdk_raw::commands::ICONVAR_OFFSET;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::RefCell;
use std::ffi::{CStr, c_char};
use std::ptr::null_mut;

/// A registry whose `GetCommands` returns its head, whose `FindVar` finds
/// its variables, and which keeps the global change callbacks installed in
/// it.
#[repr(C)]
struct MockCvar {
	interface: sys::ICvar,
	head: *mut sys::ConCommandBase,
	vars: Vec<*mut sys::ConVar>,
	callbacks: RefCell<Vec<sys::FnChangeCallback_t>>,
}

/// `ICvar::CallGlobalChangeCallbacks`, which calls the installed callbacks in
/// order with the variable's `IConVar` subobject, as `CCvar` converts the
/// `ConVar *` it is given. As `CCvar` does, it counts the callbacks before
/// calling the first; one removed meanwhile ends the walk here, where `CCvar`
/// would read past the end of its list.
unsafe extern "C" fn call_global_change_callbacks(
	this: *mut sys::ICvar,
	var: *mut sys::ConVar,
	old_value: *const c_char,
	old_float: f32,
) {
	// SAFETY: As for `find_var`.
	let cvar = unsafe { &*this.cast::<MockCvar>() };

	let interface = match var.is_null() {
		true => null_mut(),

		// SAFETY: The caller passes a live variable, which the subobject lies
		// within.
		false => unsafe { &raw mut (*var)._base_1 },
	};

	let count = cvar.callbacks.borrow().len();

	for index in 0..count {
		// The list is not borrowed while a callback runs, since the callback may
		// install or remove one.
		let Some(callback) = cvar.callbacks.borrow().get(index).copied() else {
			break;
		};

		if let Some(callback) = callback {
			// SAFETY: Callbacks are installed to be called with a variable's
			// interface, the string it held, and the float it held, which the
			// caller passes.
			unsafe { callback(interface, old_value, old_float) };
		}
	}
}

/// `ICvar::FindVar`, which finds a variable of the registry by name,
/// ignoring ASCII case, as `CCvar` does.
unsafe extern "C" fn find_var(this: *mut sys::ICvar, name: *const c_char) -> *mut sys::ConVar {
	// SAFETY: The wrappers pass NUL-terminated names, and every registry is a
	// `MockCvar`, which `mock_cvar` leaked.
	let (name, cvar) = unsafe { (CStr::from_ptr(name), &*this.cast::<MockCvar>()) };

	cvar.vars
		.iter()
		.copied()
		.find(|&var| {
			// SAFETY: The registry's variables are leaked, and named by string
			// literals.
			let listed = unsafe { CStr::from_ptr((*var)._base.m_pszName) };

			listed.to_bytes().eq_ignore_ascii_case(name.to_bytes())
		})
		.unwrap_or(null_mut())
}

/// `ICvar::GetCommands`, which returns the head of the registry's list.
unsafe extern "C" fn get_commands(this: *mut sys::ICvar) -> *mut sys::ConCommandBase {
	// SAFETY: As for `find_var`.
	unsafe { (*this.cast::<MockCvar>()).head }
}

/// `ConCommandBase::GetName`, which returns the name it was declared with.
unsafe extern "C" fn get_name(this: *const sys::ConCommandBase) -> *const c_char {
	// SAFETY: Every mock command or variable is a live `ConCommandBase`.
	unsafe { (&raw const (*this).m_pszName).read() }
}

/// `ICvar::InstallGlobalChangeCallback`, which adds a callback at the end of
/// the list, even one installed already, as `CCvar` does.
unsafe extern "C" fn install_global_change_callback(
	this: *mut sys::ICvar,
	callback: sys::FnChangeCallback_t,
) {
	// SAFETY: As for `find_var`.
	let cvar = unsafe { &*this.cast::<MockCvar>() };

	cvar.callbacks.borrow_mut().push(callback);
}

/// `IConVar::GetName` of a variable, which converts the interface back to the
/// variable, as C++'s thunk does, and returns its parent's name, as tier1's
/// `ConVar::GetName` does.
unsafe extern "C" fn interface_get_name(this: *const sys::IConVar) -> *const c_char {
	// SAFETY: Every mock variable is a live `ConVar`, whose `IConVar` lies at
	// `ICONVAR_OFFSET`, and whose parent is live.
	unsafe {
		let var = this.byte_sub(ICONVAR_OFFSET).cast::<sys::ConVar>();
		let parent = (&raw const (*var).m_pParent).read();

		(&raw const (*parent)._base.m_pszName).read()
	}
}

/// `ConCommandBase::IsCommand` of a command.
///
/// # Safety
///
/// None: it reads no argument. It is `unsafe` to fit the vtable slot.
pub unsafe extern "C" fn is_command(_: *const sys::ConCommandBase) -> bool {
	true
}

/// `ConCommandBase::IsRegistered`, which reads `m_bRegistered`.
unsafe extern "C" fn is_registered(this: *const sys::ConCommandBase) -> bool {
	// SAFETY: As for `get_name`.
	unsafe { (&raw const (*this).m_bRegistered).read() }
}

/// `ConCommandBase::IsCommand` of a variable.
///
/// # Safety
///
/// None: it reads no argument. It is `unsafe` to fit the vtable slot.
pub unsafe extern "C" fn is_variable(_: *const sys::ConCommandBase) -> bool {
	false
}

/// A registered `ConCommandBase` another module declared, of `kind`, linked
/// to `next`, whose vtable answers `IsCommand`, `GetName` and
/// `IsRegistered`.
///
/// For tests only.
pub fn mock_base(
	name: &'static CStr,
	kind: CommandBaseKind,
	flags: CommandFlags,
	next: *mut sys::ConCommandBase,
) -> sys::ConCommandBase {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::ConCommandBase__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).ConCommandBase_IsCommand).write(match kind {
				CommandBaseKind::Command => is_command,
				CommandBaseKind::Variable => is_variable,
			});
			(&raw mut (*vtable).ConCommandBase_GetName).write(get_name);
			(&raw mut (*vtable).ConCommandBase_IsRegistered).write(is_registered);
		})
	};

	sys::ConCommandBase {
		vtable_: Box::leak(vtable),
		m_pNext: next,
		m_bRegistered: true,
		m_pszName: name.as_ptr(),
		m_pszHelpString: c"".as_ptr(),
		m_nFlags: flags.bits(),
	}
}

/// A leaked command another module declared, linked to `next`.
///
/// For tests only.
pub fn mock_command(
	name: &'static CStr,
	next: *mut sys::ConCommandBase,
) -> *mut sys::ConCommandBase {
	let base = mock_base(name, CommandBaseKind::Command, CommandFlags::NONE, next);

	Box::into_raw(Box::new(base))
}

/// A leaked registry whose list starts at `head`, whose `FindVar` finds
/// `vars`, and whose global change callbacks are installed, removed and
/// called as `CCvar`'s are.
///
/// For tests only. The list and the variables must stay alive while the
/// registry is used.
pub fn mock_cvar(head: *mut sys::ConCommandBase, vars: Vec<*mut sys::ConVar>) -> *mut sys::ICvar {
	// SAFETY: As for `mock_base`.
	let vtable = unsafe {
		mock_vtable::<sys::ICvar__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).ICvar_FindVar).write(find_var);
			(&raw mut (*vtable).ICvar_GetCommands).write(get_commands);
			(&raw mut (*vtable).ICvar_InstallGlobalChangeCallback)
				.write(install_global_change_callback);
			(&raw mut (*vtable).ICvar_RemoveGlobalChangeCallback)
				.write(remove_global_change_callback);
			(&raw mut (*vtable).ICvar_CallGlobalChangeCallbacks)
				.write(call_global_change_callbacks);
		})
	};

	Box::into_raw(Box::new(MockCvar {
		interface: sys::ICvar {
			vtable_: Box::leak(vtable),
		},
		head,
		vars,
		callbacks: RefCell::new(Vec::new()),
	}))
	.cast()
}

/// A leaked variable another module declared, which is its own parent,
/// linked to `next`, and whose `IConVar` answers `GetName`.
///
/// For tests only.
pub fn mock_var(
	name: &'static CStr,
	default: &'static CStr,
	value: &'static CStr,
	flags: CommandFlags,
	next: *mut sys::ConCommandBase,
) -> *mut sys::ConVar {
	// SAFETY: Zero is valid for every field of `ConVar`.
	let var = Box::into_raw(Box::new(unsafe { std::mem::zeroed::<sys::ConVar>() }));

	// SAFETY: As for `mock_base`.
	let interface = unsafe {
		mock_vtable::<sys::IConVar__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IConVar_GetName).write(interface_get_name);
		})
	};

	// SAFETY: `var` is the box just leaked, and nothing else refers to it yet.
	unsafe {
		(*var)._base = mock_base(name, CommandBaseKind::Variable, flags, next);
		(*var)._base_1.vtable_ = Box::leak(interface);
		(*var).m_pParent = var;
		(*var).m_pszDefaultValue = default.as_ptr();
		(*var).m_pszString = value.as_ptr().cast_mut();
	}

	var
}

/// `ICvar::RemoveGlobalChangeCallback`, which removes the first entry of a
/// callback from the list, if any, as `CCvar` does.
unsafe extern "C" fn remove_global_change_callback(
	this: *mut sys::ICvar,
	callback: sys::FnChangeCallback_t,
) {
	// SAFETY: As for `find_var`.
	let cvar = unsafe { &*this.cast::<MockCvar>() };
	let mut callbacks = cvar.callbacks.borrow_mut();

	// Callbacks are compared by address, as `CCvar` compares them.
	let address = |callback: sys::FnChangeCallback_t| callback.map(|callback| callback as usize);

	if let Some(index) = callbacks
		.iter()
		.position(|&listed| address(listed) == address(callback))
	{
		callbacks.remove(index);
	}
}
