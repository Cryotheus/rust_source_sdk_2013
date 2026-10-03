//! Mock network channels, and the clients' messages and engines of the `net`
//! submodules.

pub mod cheats;
pub mod incoming;

use crate::bitbuf::BitWriter;
use sdk_raw::bitbuf::BfWrite;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
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
