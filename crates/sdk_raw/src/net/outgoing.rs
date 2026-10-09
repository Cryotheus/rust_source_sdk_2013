//! Hand-written ABI of the `INetChannel` methods through which everything the
//! server sends a client passes: `SendNetMsg`, `SendData` and `SendDatagram`,
//! which hooks patch at [`SEND_NET_MSG_SLOT`], [`SEND_DATA_SLOT`] and
//! [`SEND_DATAGRAM_SLOT`].
//!
//! `INetChannel` derives from `INetChannelInfo`, whose 26 methods fill the
//! first slots of its vtable. Its own virtual destructor follows, which
//! occupies one slot under MSVC and two under the Itanium ABI, then its other
//! methods in the order of `public/inetchannel.h`. The slots of the three
//! methods, and of the `INetMessage` methods called on the messages
//! `SendNetMsg` sends, are checked against the generated vtables, so a
//! regenerated binding cannot silently dispatch to another method.

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::c_int;

// `INetChannelInfo`'s methods take the slots from `GetName` (0) to
// `GetTimeoutSeconds` (25), and `INetChannel`'s destructor those after them.
const _: () = {
	let destructor = CppDestructors::VTABLE_SLOTS;

	assert!(vtable_slot!(sys::INetChannel__bindgen_vtable, INetChannel_GetName) == 0);
	assert!(
		vtable_slot!(
			sys::INetChannel__bindgen_vtable,
			INetChannel_GetTimeoutSeconds
		) == 25
	);
	assert!(
		vtable_slot!(sys::INetChannel__bindgen_vtable, INetChannel_SetDataRate) == 26 + destructor
	);
	assert!(SEND_NET_MSG_SLOT == 36 + destructor);
	assert!(SEND_DATA_SLOT == 37 + destructor);
	assert!(SEND_DATAGRAM_SLOT == 42 + destructor);
};

// `INetMessage` declares a virtual destructor first, then its methods in the
// order of `public/inetmessage.h`.
const _: () = {
	let destructor = CppDestructors::VTABLE_SLOTS;

	assert!(
		vtable_slot!(sys::INetMessage__bindgen_vtable, INetMessage_SetNetChannel) == destructor
	);
	assert!(
		vtable_slot!(sys::INetMessage__bindgen_vtable, INetMessage_WriteToBuffer) == destructor + 4
	);
	assert!(
		vtable_slot!(sys::INetMessage__bindgen_vtable, INetMessage_IsReliable) == destructor + 5
	);
	assert!(vtable_slot!(sys::INetMessage__bindgen_vtable, INetMessage_GetType) == destructor + 6);
	assert!(vtable_slot!(sys::INetMessage__bindgen_vtable, INetMessage_GetGroup) == destructor + 7);
	assert!(vtable_slot!(sys::INetMessage__bindgen_vtable, INetMessage_GetName) == destructor + 8);
	assert!(
		vtable_slot!(sys::INetMessage__bindgen_vtable, INetMessage_ToString) == destructor + 10
	);
};

// The generated bindings have these signatures.
const _: fn(&sys::INetChannel__bindgen_vtable) -> SendDataFn = |vtable| vtable.INetChannel_SendData;

const _: fn(&sys::INetChannel__bindgen_vtable) -> SendDatagramFn =
	|vtable| vtable.INetChannel_SendDatagram;

const _: fn(&sys::INetChannel__bindgen_vtable) -> SendNetMsgFn =
	|vtable| vtable.INetChannel_SendNetMsg;

/// The slot of [`SendDataFn`] in `INetChannel`'s vtable, from the generated
/// binding.
#[doc(alias("SendData"))]
pub const SEND_DATA_SLOT: usize =
	vtable_slot!(sys::INetChannel__bindgen_vtable, INetChannel_SendData);

/// The slot of [`SendDatagramFn`] in `INetChannel`'s vtable, from the
/// generated binding.
#[doc(alias("SendDatagram"))]
pub const SEND_DATAGRAM_SLOT: usize =
	vtable_slot!(sys::INetChannel__bindgen_vtable, INetChannel_SendDatagram);

/// The slot of [`SendNetMsgFn`] in `INetChannel`'s vtable, from the generated
/// binding.
#[doc(alias("SendNetMsg"))]
pub const SEND_NET_MSG_SLOT: usize =
	vtable_slot!(sys::INetChannel__bindgen_vtable, INetChannel_SendNetMsg);

/// `bool INetChannel::SendData(bf_write &msg, bool bReliable)`: appends the
/// bits written to `data`, which are whole encoded messages, to the channel's
/// reliable stream if `reliable` is set, and to its unreliable stream
/// otherwise. Returns false if they do not fit.
///
/// The C++ reference is passed as a pointer.
#[doc(alias("SendData"))]
pub type SendDataFn = unsafe extern "C" fn(
	this: *mut sys::INetChannel,
	data: *mut sys::bf_write,
	reliable: bool,
) -> bool;

/// `int INetChannel::SendDatagram(bf_write *data)`: sends the client a packet
/// at once, with what the channel's streams hold and, unless `data` is null,
/// the messages written to it, which go in this packet alone, such as a
/// snapshot that updates the client's last. Returns the packet's sequence
/// number.
#[doc(alias("SendDatagram"))]
pub type SendDatagramFn =
	unsafe extern "C" fn(this: *mut sys::INetChannel, data: *mut sys::bf_write) -> c_int;

/// `bool INetChannel::SendNetMsg(INetMessage &msg, bool bForceReliable, bool
/// bVoice)`: has `message` encode itself with its `WriteToBuffer` into one of
/// the channel's streams: the voice stream if `voice` is set, the reliable
/// stream if the message is reliable or `force_reliable` is set, and the
/// unreliable stream otherwise. Returns false if it does not fit.
///
/// The C++ reference is passed as a pointer.
#[doc(alias("SendNetMsg"))]
pub type SendNetMsgFn = unsafe extern "C" fn(
	this: *mut sys::INetChannel,
	message: *mut sys::INetMessage,
	force_reliable: bool,
	voice: bool,
) -> bool;
