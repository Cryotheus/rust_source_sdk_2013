//! Mock network channels and engine messages, and the clients' messages and
//! engines of the `net` submodules.

pub mod cheats;
pub mod incoming;

use crate::bitbuf::BitWriter;
use sdk_raw::bitbuf::BfWrite;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr::NonNull;

thread_local! {
	static ACCEPTS: Cell<bool> = const { Cell::new(true) };
	static SENT: RefCell<Vec<(BitWriter, bool)>> = const { RefCell::new(Vec::new()) };
}

/// A channel whose `SendData` records what it is given.
///
/// For tests only. The channel is leaked, so its pointer stays valid for the
/// test.
pub struct MockChannel {
	channel: *mut sys::INetChannel,
}

impl MockChannel {
	/// A channel that accepts what it is sent, forgetting what earlier mocks
	/// on this thread recorded.
	pub fn new() -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes a slot of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::INetChannel__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetChannel_SendData).write(record_send_data);
				},
			)
		};

		let channel = Box::into_raw(Box::new(sys::INetChannel {
			vtable_: Box::into_raw(vtable),
		}));

		SENT.set(Vec::new());
		ACCEPTS.set(true);

		Self { channel }
	}

	/// The mock, as the engine would return it.
	pub const fn as_ptr(&self) -> *mut sys::INetChannel {
		self.channel
	}

	/// A handle to the mock, as the engine would return it.
	#[cfg(test)]
	pub(crate) fn channel(&self) -> crate::net::NetChannel<'_> {
		// SAFETY: The channel is leaked, so it outlives the borrow, and its
		// vtable answers `SendData`.
		unsafe { crate::net::NetChannel::from_raw(NonNull::new(self.channel).unwrap()) }
	}

	/// Makes `SendData` report that the stream had no room.
	pub fn refuse(&self) {
		ACCEPTS.set(false);
	}

	/// Takes the data `SendData` was given so far, in order, each with whether
	/// it was reliable.
	pub fn take_sent(&self) -> Vec<(BitWriter, bool)> {
		SENT.take()
	}
}

impl Default for MockChannel {
	fn default() -> Self {
		Self::new()
	}
}

/// `INetChannel::SendData`, which records the data for
/// [`MockChannel::take_sent`].
unsafe extern "C" fn record_send_data(
	_: *mut sys::INetChannel,
	buffer: *mut sys::bf_write,
	reliable: bool,
) -> bool {
	// SAFETY: `NetChannel::send_encoded` passes a live `bf_write` it wrote.
	let bits = unsafe { BfWrite::read_back(NonNull::new(buffer.cast()).unwrap()) }
		.map(BitWriter::from)
		.expect("a readable buffer");

	SENT.with_borrow_mut(|sent| sent.push((bits, reliable)));
	ACCEPTS.get()
}

/// One of the engine's message objects, whose `WriteToBuffer` appends the
/// bits it was made with to the buffer it is given.
///
/// For tests only. The message is leaked, so its pointer stays valid for the
/// test.
pub struct MockMessage {
	message: *mut MessageObject,
}

impl MockMessage {
	/// A reliable message of type `id` in the traffic group `group`, named
	/// `name`, whose `WriteToBuffer` appends `bits`, which start with the
	/// type, and reports whether they fit. Its `ToString` is its name, then
	/// its size in bits.
	pub fn new(id: c_int, group: c_int, name: &'static CStr, bits: BitWriter) -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::INetMessage__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetMessage_GetGroup).write(message_group);
					(&raw mut (*vtable).INetMessage_GetName).write(message_name);
					(&raw mut (*vtable).INetMessage_GetType).write(message_type);
					(&raw mut (*vtable).INetMessage_IsReliable).write(message_is_reliable);
					(&raw mut (*vtable).INetMessage_ToString).write(message_to_string);
					(&raw mut (*vtable).INetMessage_WriteToBuffer).write(message_write);
				},
			)
		};

		let description = format!("{}: {} bits", name.to_string_lossy(), bits.len());

		let message = Box::into_raw(Box::new(MessageObject {
			base: sys::INetMessage {
				vtable_: Box::into_raw(vtable),
			},
			id,
			group,
			name,
			description: CString::new(description).unwrap(),
			bits,
			reliable: Cell::new(true),
			fails: Cell::new(false),
			writes: Cell::new(0),
		}));

		Self { message }
	}

	/// The mock, as the engine would pass it.
	pub const fn as_ptr(&self) -> *mut sys::INetMessage {
		self.message.cast()
	}

	/// Makes `WriteToBuffer` write nothing and report failure, as the engine's
	/// messages do when a field is out of range.
	pub fn fail(&self) {
		self.object().fails.set(true);
	}

	/// Makes `IsReliable` report that the message is unreliable.
	pub fn make_unreliable(&self) {
		self.object().reliable.set(false);
	}

	fn object(&self) -> &MessageObject {
		// SAFETY: The object is leaked, and its methods only read it, but for
		// its cells.
		unsafe { &*self.message }
	}

	/// The number of times `WriteToBuffer` was called.
	pub fn writes(&self) -> usize {
		self.object().writes.get()
	}
}

/// What a [`MockMessage`] points to: the `INetMessage`, then what its methods
/// answer.
#[repr(C)]
struct MessageObject {
	base: sys::INetMessage,
	id: c_int,
	group: c_int,
	name: &'static CStr,
	description: CString,
	bits: BitWriter,
	reliable: Cell<bool>,
	fails: Cell<bool>,
	writes: Cell<usize>,
}

/// The mock message's `GetGroup`.
unsafe extern "C" fn message_group(this: *const sys::INetMessage) -> c_int {
	// SAFETY: Only mock messages have this vtable.
	unsafe { (*this.cast::<MessageObject>()).group }
}

/// The mock message's `IsReliable`.
unsafe extern "C" fn message_is_reliable(this: *const sys::INetMessage) -> bool {
	// SAFETY: As above.
	unsafe { (*this.cast::<MessageObject>()).reliable.get() }
}

/// The mock message's `GetName`.
unsafe extern "C" fn message_name(this: *const sys::INetMessage) -> *const c_char {
	// SAFETY: As above.
	unsafe { (*this.cast::<MessageObject>()).name.as_ptr() }
}

/// The mock message's `ToString`.
unsafe extern "C" fn message_to_string(this: *const sys::INetMessage) -> *const c_char {
	// SAFETY: As above.
	unsafe { (*this.cast::<MessageObject>()).description.as_ptr() }
}

/// The mock message's `GetType`.
unsafe extern "C" fn message_type(this: *const sys::INetMessage) -> c_int {
	// SAFETY: As above.
	unsafe { (*this.cast::<MessageObject>()).id }
}

/// The mock message's `WriteToBuffer`, which appends the message's bits, or
/// fails as [`MockMessage::fail`] makes it.
unsafe extern "C" fn message_write(
	this: *mut sys::INetMessage,
	buffer: *mut sys::bf_write,
) -> bool {
	// SAFETY: As above.
	let message = unsafe { &*this.cast::<MessageObject>() };

	message.writes.set(message.writes.get() + 1);

	if message.fails.get() {
		return false;
	}

	let buffer = BfWrite::from_sys(NonNull::new(buffer).expect("a buffer"));

	// SAFETY: The code under test passes a live buffer, whose storage nothing
	// else accesses during the call.
	unsafe { BfWrite::append(buffer, message.bits.as_words(), message.bits.len()) }
}
