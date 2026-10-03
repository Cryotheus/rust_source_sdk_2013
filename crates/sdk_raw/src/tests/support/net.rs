//! Fakes of the network messages clients send.

use super::{mock_vtable, unexpected_call};
use crate::net::incoming::SMALLEST_MESSAGE_BASE;
use std::ptr::NonNull;

/// `INetMessage::GetSize` of a [`mock_message`], which keeps its size after
/// its vtable pointer.
unsafe extern "C" fn get_size(this: *const sys::INetMessage) -> usize {
	// SAFETY: Every mock message keeps its reported size in `CNetMessage`'s
	// fields, after its vtable pointer.
	unsafe { this.add(1).cast::<usize>().read() }
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
