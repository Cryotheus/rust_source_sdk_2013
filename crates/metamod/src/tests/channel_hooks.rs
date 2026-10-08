//! Tests of `crate::channel_hooks`: pre hooks of `SendNetMsg`, `SendData` and
//! `SendDatagram` on a mock channel, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::net::messages::Print;
use source_sdk_2013::net::{MessageId, NetMessage};
use source_sdk_2013::raw::abi::VTABLE_SLOT_SIZE;
use source_sdk_2013::test_support::net::MockMessage;
use std::cell::RefCell;
use std::ffi::{CStr, c_void};
use std::ptr;

thread_local! {
	/// What happened during the sends since the last one began, in order.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
}

/// A channel of a C++ class, as far as hooks know it.
#[repr(C)]
struct Channel {
	vtable: *mut *mut c_void,
}

impl Channel {
	fn new() -> Box<Self> {
		let vtable = mock_vtable::<sys::INetChannel__bindgen_vtable>(&[
			(SEND_NET_MSG_SLOT, channel_send_net_msg as *mut c_void),
			(SEND_DATA_SLOT, channel_send_data as *mut c_void),
			(SEND_DATAGRAM_SLOT, channel_send_datagram as *mut c_void),
		]);

		Box::new(Self { vtable })
	}

	fn ptr(&mut self) -> NonNull<sys::INetChannel> {
		NonNull::from(self).cast()
	}
}

/// Something the channel or the listener noticed.
#[derive(Debug, Clone, PartialEq)]
enum Seen {
	/// The listener saw a message.
	Message {
		/// The address of the channel.
		channel: usize,

		/// The message's type.
		id: Option<MessageId>,

		/// Whether the message is reliable itself.
		reliable: bool,

		/// What the message encoded to.
		bits: Option<BitWriter>,

		/// Whether the sender forced the message into the reliable stream.
		force_reliable: bool,

		/// Whether the message was for the voice stream.
		voice: bool,
	},

	/// The listener saw, on the channel at this address, data of this many
	/// bits, which copied to these, for the reliable stream or not.
	Data(usize, usize, Option<BitWriter>, bool),

	/// The listener saw, on the channel at this address, a datagram with
	/// messages of its own, which copied to these, or without.
	Datagram(usize, Option<Option<BitWriter>>),

	/// The channel's `SendData` or `SendDatagram` ran.
	Channel(&'static str),

	/// The channel's `SendNetMsg` had the message write these bits.
	Queued(BitWriter),
}

#[test]
fn a_panicking_listener_leaves_the_sends_alone() {
	on_both(|harness| {
		let api = harness.api();
		let mut channel = Channel::new();

		fn fail(_server: Server<'_>, _channel: NetChannel<'_>, _send: &ChannelSend<'_>) {
			panic!("the listener failed");
		}

		// SAFETY: The mock channel has the three methods at their slots, and its
		// vtable is leaked.
		let hooks =
			unsafe { api.install_channel_sends(channel.ptr(), tf2_binding(no_interfaces), fail) }
				.unwrap();

		assert_eq!(
			send_datagram(harness, &mut channel, None),
			(42, vec![Seen::Channel("SendDatagram")])
		);

		hooks.remove(api);
	});
}

#[test]
fn sends_an_earlier_hook_superseded_skip_the_listener() {
	on_both(|harness| {
		let api = harness.api();
		let mut channel = Channel::new();
		let data = print(c"skipped");

		fn supersede(_call: &HookCall<'_, SendData>) -> HookAction<bool> {
			HookAction::Supersede(false)
		}

		// SAFETY: As above.
		let hooks = unsafe {
			api.add_hook(
				SEND_DATA,
				HookTarget::class_of(channel.ptr()),
				HookTiming::Pre,
				&supersede,
			)
			.unwrap();
			api.install_channel_sends(channel.ptr(), tf2_binding(no_interfaces), on_send)
				.unwrap()
		};

		assert_eq!(
			send_data(harness, &mut channel, &data, true),
			(false, vec![])
		);

		hooks.remove(api);
	});
}

#[test]
fn sends_reach_the_listener_before_the_channel() {
	on_both(|harness| {
		let api = harness.api();
		let mut channel = Channel::new();
		let address = channel.ptr().addr().get();

		// SAFETY: As above.
		let hooks = unsafe {
			api.install_channel_sends(channel.ptr(), tf2_binding(no_interfaces), on_send)
		}
		.unwrap();

		// SAFETY: As above, and the handle is made on the test's thread, which
		// stands for the main thread.
		let handle = unsafe { NetChannel::from_raw(channel.ptr()) };

		// A second install is refused, so that each send is seen once.
		assert!(matches!(
			api.listen_channel_sends(handle, tf2_binding(no_interfaces), on_send),
			Err(HookError::AlreadyInstalled)
		));

		let bits = print(c"hello");
		let message = MockMessage::new(7, 0, c"svc_Print", bits.clone());

		message.make_unreliable();

		// The listener encodes the message, which still writes the same bits for
		// the channel.
		assert_eq!(
			send_message(harness, &mut channel, &message, true, false),
			(
				true,
				vec![
					Seen::Message {
						channel: address,
						id: Some(MessageId::PRINT),
						reliable: false,
						bits: Some(bits.clone()),
						force_reliable: true,
						voice: false,
					},
					Seen::Queued(bits.clone())
				]
			)
		);

		assert_eq!(message.writes(), 2);

		assert_eq!(
			send_data(harness, &mut channel, &bits, false),
			(
				true,
				vec![
					Seen::Data(address, bits.len(), Some(bits.clone()), false),
					Seen::Channel("SendData")
				]
			)
		);

		assert_eq!(
			send_datagram(harness, &mut channel, Some(&bits)),
			(
				42,
				vec![
					Seen::Datagram(address, Some(Some(bits.clone()))),
					Seen::Channel("SendDatagram")
				]
			)
		);

		// A packet of what the streams hold alone.
		assert_eq!(
			send_datagram(harness, &mut channel, None),
			(
				42,
				vec![Seen::Datagram(address, None), Seen::Channel("SendDatagram")]
			)
		);

		// Removed hooks pass the sends on unseen, and allow a replacement.
		hooks.remove(api);

		assert_eq!(
			send_datagram(harness, &mut channel, None),
			(42, vec![Seen::Channel("SendDatagram")])
		);

		let hooks = api
			.listen_channel_sends(handle, tf2_binding(no_interfaces), on_send)
			.unwrap();

		assert_eq!(
			send_datagram(harness, &mut channel, None),
			(
				42,
				vec![Seen::Datagram(address, None), Seen::Channel("SendDatagram")]
			)
		);

		hooks.remove(api);
	});
}

/// The mock channel's `SendData`, which notes the call and takes the data.
unsafe extern "C" fn channel_send_data(
	_this: *mut sys::INetChannel,
	_data: *mut sys::bf_write,
	_reliable: bool,
) -> bool {
	SEEN.with_borrow_mut(|seen| seen.push(Seen::Channel("SendData")));
	true
}

/// The mock channel's `SendDatagram`, which notes the call and reports the
/// packet's sequence number, 42.
unsafe extern "C" fn channel_send_datagram(
	_this: *mut sys::INetChannel,
	_data: *mut sys::bf_write,
) -> c_int {
	SEEN.with_borrow_mut(|seen| seen.push(Seen::Channel("SendDatagram")));
	42
}

/// The mock channel's `SendNetMsg`, which has the message write itself, as
/// the engine's channels do, and notes the bits.
unsafe extern "C" fn channel_send_net_msg(
	_this: *mut sys::INetChannel,
	message: *mut sys::INetMessage,
	_force_reliable: bool,
	_voice: bool,
) -> bool {
	let mut storage = [0; 64];
	let mut buffer = BfWrite::empty(&mut storage);

	// SAFETY: The tests send live mock messages, which write within the
	// buffer's storage.
	let written =
		unsafe { ((*(*message).vtable_).INetMessage_WriteToBuffer)(message, buffer.as_raw()) };

	// SAFETY: The buffer describes `storage`, which the call has finished
	// writing.
	let bits = unsafe { BfWrite::read_back(NonNull::from(&mut buffer)) }.unwrap();

	SEEN.with_borrow_mut(|seen| seen.push(Seen::Queued(BitWriter::from(bits))));
	written
}

/// A leaked vtable of `V`'s size, whose slots hold `functions` at their
/// indices, and [`unexpected_call`] elsewhere.
fn mock_vtable<V>(functions: &[(usize, *mut c_void)]) -> *mut *mut c_void {
	let mut slots = vec![unexpected_call as *mut c_void; size_of::<V>() / VTABLE_SLOT_SIZE];

	for &(slot, function) in functions {
		slots[slot] = function;
	}

	Vec::leak(slots).as_mut_ptr()
}

/// The listener, which notes what it saw, copying and encoding all of it.
fn on_send(_server: Server<'_>, channel: NetChannel<'_>, send: &ChannelSend<'_>) {
	let channel = channel.as_ptr().addr();

	let noticed = match *send {
		ChannelSend::Message {
			message,
			force_reliable,
			voice,
		} => Seen::Message {
			channel,
			id: message.id(),
			reliable: message.is_reliable(),
			bits: message.encode(),
			force_reliable,
			voice,
		},

		ChannelSend::Data { data, reliable } => {
			Seen::Data(channel, data.bit_len(), data.copy(), reliable)
		}

		ChannelSend::Datagram(data) => Seen::Datagram(channel, data.map(|data| data.copy())),
	};

	SEEN.with_borrow_mut(|seen| seen.push(noticed));
}

/// A `svc_Print` of `text`, encoded.
fn print(text: &CStr) -> BitWriter {
	Print { text }.encode().unwrap()
}

/// Has the mock channel send `data`, through its hooked vtable, and returns
/// what `SendData` returned and what happened.
fn send_data(
	harness: &Harness,
	channel: &mut Channel,
	data: &BitWriter,
	reliable: bool,
) -> (bool, Vec<Seen>) {
	let mut buffer = BfWrite::written(data.as_words(), data.len());

	SEEN.take();

	let sent = harness.call::<SendData>(
		channel.ptr().as_ptr(),
		SEND_DATA_SLOT,
		(buffer.as_raw(), reliable),
	);

	(sent, SEEN.take())
}

/// Has the mock channel send a packet with `data`, or with no messages of its
/// own, through its hooked vtable, and returns what `SendDatagram` returned
/// and what happened.
fn send_datagram(
	harness: &Harness,
	channel: &mut Channel,
	data: Option<&BitWriter>,
) -> (c_int, Vec<Seen>) {
	let mut buffer = data.map(|data| BfWrite::written(data.as_words(), data.len()));
	let raw = buffer.as_mut().map_or(ptr::null_mut(), BfWrite::as_raw);

	SEEN.take();

	let sequence = harness.call::<SendDatagram>(channel.ptr().as_ptr(), SEND_DATAGRAM_SLOT, (raw,));

	(sequence, SEEN.take())
}

/// Has the mock channel send `message`, through its hooked vtable, and returns
/// what `SendNetMsg` returned and what happened.
fn send_message(
	harness: &Harness,
	channel: &mut Channel,
	message: &MockMessage,
	force_reliable: bool,
	voice: bool,
) -> (bool, Vec<Seen>) {
	SEEN.take();

	let sent = harness.call::<SendNetMsg>(
		channel.ptr().as_ptr(),
		SEND_NET_MSG_SLOT,
		(message.as_ptr(), force_reliable, voice),
	);

	(sent, SEEN.take())
}

/// A slot no test expects to be called, which aborts the test process.
extern "C" fn unexpected_call() {
	panic!("unexpected virtual call");
}
