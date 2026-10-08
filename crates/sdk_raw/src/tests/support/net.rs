//! Fakes of the network messages clients send, and of the clients.

use super::{mock_vtable, unexpected_call};
use crate::abi::VTABLE_SLOT_SIZE;
use crate::interfaces::game_event::{FIRE_GAME_EVENT_SLOT, FireGameEventFn};
use crate::net::incoming::SMALLEST_MESSAGE_BASE;
use std::ptr::{self, NonNull};

/// `INetMessage::GetSize` of a [`mock_message`], which keeps its size after
/// its vtable pointer.
unsafe extern "C" fn get_size(this: *const sys::INetMessage) -> usize {
	// SAFETY: Every mock message keeps its reported size in `CNetMessage`'s
	// fields, after its vtable pointer.
	unsafe { this.add(1).cast::<usize>().read() }
}

/// A leaked mock of one of the engine's clients, whose run-time type
/// information names its class `class`, where that of the engine's own names
/// `CGameClient`. Like theirs, it starts with its `IGameEventListener2` base,
/// then its `IClient` and `IClientMessageHandler` bases, each a vtable
/// pointer.
///
/// For tests only. Every slot of the three vtables holds [`unexpected_call`],
/// but the listener's `FireGameEvent`, which holds `fire_game_event`. Returns
/// the client's `IClient` base, as the engine's server hands it out.
///
/// # Panics
///
/// If `class` is longer than 32 bytes, or holds a NUL.
pub fn mock_game_client(class: &str, fire_game_event: FireGameEventFn) -> NonNull<sys::IClient> {
	assert!(class.len() <= 32 && !class.contains('\0'));

	let slots = |size: usize| vec![unexpected_call as *const (); size / VTABLE_SLOT_SIZE];
	let mut listener = slots(size_of::<sys::IGameEventListener2__bindgen_vtable>());

	listener[FIRE_GAME_EVENT_SLOT] = fire_game_event as *const ();

	let bases = [
		listener,
		slots(size_of::<sys::IClient__bindgen_vtable>()),
		slots(size_of::<sys::IClientMessageHandler__bindgen_vtable>()),
	];

	let prefixes = vtable_prefixes(class);

	let vtables: [*mut *const (); 3] = std::array::from_fn(|base| {
		let table = Vec::leak([prefixes[base].as_slice(), &bases[base]].concat());

		// SAFETY: The table holds the base's prefix, then its slots.
		unsafe { table.as_mut_ptr().add(prefixes[base].len()) }
	});

	let object = Box::into_raw(Box::new(vtables)).cast::<*mut *const ()>();

	// SAFETY: The object holds the three bases' vtable pointers, the `IClient`
	// base's second.
	NonNull::new(unsafe { object.add(1) }).unwrap().cast()
}

/// A leaked, zeroed message object of `size` bytes, aligned for pointers,
/// which reports that size and answers no other virtual call.
///
/// For tests only. A test writes a message's fields past its `CNetMessage`,
/// whose size the message's reported size implies.
///
/// # Panics
///
/// If `size` is smaller than [`SMALLEST_MESSAGE_BASE`], the smallest
/// `CNetMessage` the engine's messages are read with.
pub fn mock_message(size: usize) -> NonNull<sys::INetMessage> {
	assert!(size >= SMALLEST_MESSAGE_BASE);

	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes a slot of
	// the vtable being built.
	let vtable = Box::leak(unsafe {
		mock_vtable::<sys::INetMessage__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).INetMessage_GetSize).write(get_size);
		})
	});
	let object = Box::leak(vec![0_u64; size.div_ceil(8)].into_boxed_slice());
	let this = object.as_mut_ptr().cast::<sys::INetMessage>();

	// SAFETY: The object is at least `SMALLEST_MESSAGE_BASE` bytes and aligned
	// for pointers, so the vtable pointer and the size fit before its fields.
	unsafe {
		this.write(sys::INetMessage { vtable_: vtable });
		this.add(1).cast::<usize>().write(size);
	}

	NonNull::new(this).unwrap()
}

/// What the vtable of each base of a [`mock_game_client`] holds before its
/// first slot, in the order of the bases, under the Itanium ABI: the offset
/// from the base back to the object's start, then the class's type
/// information.
#[cfg(not(target_os = "windows"))]
fn vtable_prefixes(class: &str) -> [Vec<*const ()>; 3] {
	use crate::util::rtti::{ClassTypeInfo, class_type_info_vtable};
	use std::ffi::CString;

	let name = CString::new(format!("{}{class}", class.len())).unwrap();

	let type_info: &'static ClassTypeInfo = Box::leak(Box::new(ClassTypeInfo {
		vtable: class_type_info_vtable(),
		name: name.into_raw().cast_const(),
	}));

	std::array::from_fn(|base| {
		let offset_to_top = (base * size_of::<*const ()>()) as isize;

		vec![
			ptr::without_provenance(offset_to_top.wrapping_neg() as usize),
			ptr::from_ref(type_info).cast(),
		]
	})
}

/// What the vtable of each base of a [`mock_game_client`] holds before its
/// first slot, in the order of the bases, under MSVC's ABI: the base's
/// complete object locator, which references the class's type descriptor
/// from the start of the records holding both, as from an image's base.
#[cfg(target_os = "windows")]
fn vtable_prefixes(class: &str) -> [Vec<*const ()>; 3] {
	use crate::util::rtti::{CompleteObjectLocator, TypeDescriptor};
	use std::mem::offset_of;

	/// The records the locators of a client's vtables reference.
	#[repr(C)]
	struct TypeInformation {
		/// Keeps every offset above 0, which could read as absent.
		_image_start: u64,

		/// Room for a name of 32 bytes, decorated, with its terminator.
		type_descriptor: TypeDescriptor<40>,

		locators: [CompleteObjectLocator; 3],
	}

	let decorated = format!(".?AV{class}@@");
	let mut name = [0; 40];

	name[..decorated.len()].copy_from_slice(decorated.as_bytes());

	let information = Box::into_raw(Box::new(TypeInformation {
		_image_start: 0,
		type_descriptor: TypeDescriptor {
			vtable: ptr::null(),
			undecorated_name: ptr::null_mut(),
			name,
		},
		locators: std::array::from_fn(|base| CompleteObjectLocator {
			signature: CompleteObjectLocator::SIGNATURE,
			offset: (base * size_of::<*const ()>()) as u32,
			constructor_displacement: 0,
			type_descriptor: offset_of!(TypeInformation, type_descriptor) as u32,
			// No class hierarchy, which the readers of locators do not follow.
			class_descriptor: 0,
			this: (offset_of!(TypeInformation, locators)
				+ base * size_of::<CompleteObjectLocator>()) as u32,
		}),
	}));

	std::array::from_fn(|base| {
		// SAFETY: The records are leaked, and hold a locator for each base.
		vec![unsafe { &raw const (*information).locators[base] }.cast()]
	})
}
