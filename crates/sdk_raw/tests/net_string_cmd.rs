//! Tests of building `NET_StringCmd`s in place, laid out as the engine reads
//! the commands clients send, and of passing them to a client's handler.
//!
//! A test binary of its own, since the first message read fixes the size of
//! the engine's `CNetMessage` for the process.

use source_sdk_2013_raw::net::incoming::{
	ClientMessage, MAX_STRING_CMD_LEN, MessageClass, StringCmdError, StringCmdFields,
	StringCmdMessage, engine_factory_of_client, read_message,
};
use source_sdk_2013_raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013_raw::util;
use source_sdk_2013_raw::util::cstr::cstring_from_buffer;
use source_sdk_2013_raw::vcall;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_int};
use std::ptr::NonNull;

/// The size of `CNetMessage` in the mock class: larger than the SDK's 2013
/// release declares it, as the TF2 engine's may be. Every message read here
/// has it, since the first message read fixes the size for the process.
const BASE: usize = 40;

/// What the mock class's `GetType` reports for string commands
/// (`net_StringCmd`).
const STRING_CMD_TYPE: c_int = 4;

/// The mock class's `CNetMessage`: what the SDK's 2013 release declares,
/// then fields of the engine's own.
#[repr(C)]
struct NetMessage {
	vtable: *const sys::INetMessage__bindgen_vtable,
	reliable: bool,
	channel: *mut sys::INetChannel,
	engine_fields: [u8; BASE - 24],
}

const _: () = assert!(size_of::<NetMessage>() == BASE);

thread_local! {
	/// What the mock class's `GetSize` reports on this thread.
	static SIZE: Cell<usize> = const { Cell::new(BASE + size_of::<StringCmdFields>()) };

	/// What the mock class's `GetType` reports on this thread.
	static TYPE: Cell<c_int> = const { Cell::new(STRING_CMD_TYPE) };

	/// What the mock handler's `ProcessStringCmd` returns on this thread.
	static HANDLED: Cell<bool> = const { Cell::new(true) };

	/// How many times the mock class's `Process` was called on this thread.
	static PROCESS_CALLS: Cell<usize> = const { Cell::new(0) };

	/// The command of each message the mock handler was passed on this
	/// thread, as a hook reads it, or `None` if it could not be read.
	static PROCESSED: RefCell<Vec<Option<CString>>> = const { RefCell::new(Vec::new()) };
}

/// A channel, which nothing here dereferences.
fn channel() -> NonNull<sys::INetChannel> {
	NonNull::from(Box::leak(Box::new(0_u64))).cast()
}

/// A message of the mock class holding `command`, for `handler`.
fn build(
	handler: NonNull<sys::IClientMessageHandler>,
	command: &CStr,
) -> Result<StringCmdMessage, StringCmdError> {
	// SAFETY: The mock class's `GetType` and `GetSize` read no field, and its
	// setters only store their argument in its `CNetMessage`. Only the mock
	// handler's method uses the handler, and nothing uses the channel.
	unsafe { StringCmdMessage::new(string_cmd_vtable(), handler, channel(), command) }
}

/// `INetMessage::GetSize`, which reports [`SIZE`].
unsafe extern "C" fn get_size(_: *const sys::INetMessage) -> usize {
	SIZE.get()
}

/// `INetMessage::GetType`, which reports [`TYPE`].
unsafe extern "C" fn get_type(_: *const sys::INetMessage) -> c_int {
	TYPE.get()
}

/// A leaked client message handler, whose `ProcessStringCmd` is
/// [`process_string_cmd`].
fn handler() -> NonNull<sys::IClientMessageHandler> {
	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes a slot of
	// the vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IClientMessageHandler__bindgen_vtable>(
			unexpected_call as *const (),
			|vtable| {
				(&raw mut (*vtable).IClientMessageHandler_ProcessStringCmd)
					.write(process_string_cmd);
			},
		)
	};

	NonNull::from(Box::leak(Box::new(sys::IClientMessageHandler {
		vtable_: Box::leak(vtable),
	})))
}

/// `INetMessage::Process`, which counts the call in [`PROCESS_CALLS`], then
/// passes the message to the `ProcessStringCmd` method of the handler its
/// fields name, as the engine's does.
unsafe extern "C" fn process(this: *mut sys::INetMessage) -> bool {
	PROCESS_CALLS.set(PROCESS_CALLS.get() + 1);

	// SAFETY: Every message of the mock class holds its fields past its
	// `CNetMessage` of `BASE` bytes, aligned for them.
	let handler = unsafe { (*this.byte_add(BASE).cast::<StringCmdFields>()).handler };

	// SAFETY: The fields name a mock handler, whose `ProcessStringCmd` only
	// reads the message.
	unsafe {
		vcall!(handler.cast::<sys::IClientMessageHandler>() => IClientMessageHandler_ProcessStringCmd(this.cast()))
	}
}

/// `IClientMessageHandler::ProcessStringCmd`, which reads the message as a
/// hook on the method does, records its command in [`PROCESSED`], and returns
/// [`HANDLED`].
unsafe extern "C" fn process_string_cmd(
	this: *mut sys::IClientMessageHandler,
	message: *mut sys::NET_StringCmd,
) -> bool {
	let (Some(this), Some(message)) = (NonNull::new(this), NonNull::new(message)) else {
		PROCESSED.with_borrow_mut(|processed| processed.push(None));
		return false;
	};

	// SAFETY: The message was passed to this method of `this`, and stays alive
	// and unchanged for the call.
	let read = unsafe { read_message(MessageClass::StringCmd, this, message.cast()) };

	let command = match read {
		Some(ClientMessage::StringCmd(fields)) => Some(cstring_from_buffer(&fields.command_buffer)),
		_ => None,
	};

	PROCESSED.with_borrow_mut(|processed| processed.push(command));
	HANDLED.get()
}

/// `INetMessage::SetNetChannel`, which stores the channel as `CNetMessage`
/// does.
unsafe extern "C" fn set_net_channel(this: *mut sys::INetMessage, channel: *mut sys::INetChannel) {
	// SAFETY: Every message of the mock class starts with its `CNetMessage`.
	unsafe { (&raw mut (*this.cast::<NetMessage>()).channel).write(channel) };
}

/// `INetMessage::SetReliable`, which stores the flag as `CNetMessage` does.
unsafe extern "C" fn set_reliable(this: *mut sys::INetMessage, reliable: bool) {
	// SAFETY: As for `set_net_channel`.
	unsafe { (&raw mut (*this.cast::<NetMessage>()).reliable).write(reliable) };
}

/// A leaked vtable of the mock class, a `NET_StringCmd` whose `CNetMessage`
/// is [`BASE`] bytes.
fn string_cmd_vtable() -> NonNull<sys::INetMessage__bindgen_vtable> {
	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::INetMessage__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).INetMessage_GetSize).write(get_size);
			(&raw mut (*vtable).INetMessage_GetType).write(get_type);
			(&raw mut (*vtable).INetMessage_Process).write(process);
			(&raw mut (*vtable).INetMessage_SetNetChannel).write(set_net_channel);
			(&raw mut (*vtable).INetMessage_SetReliable).write(set_reliable);
		})
	};

	NonNull::from(Box::leak(vtable))
}

#[test]
fn classes_must_report_a_string_commands_type_and_size() {
	let fields = size_of::<StringCmdFields>();

	let refused = |size, id| {
		SIZE.set(size);
		TYPE.set(id);

		let built = build(handler(), c"kill");

		SIZE.set(BASE + fields);
		TYPE.set(STRING_CMD_TYPE);

		matches!(built, Err(StringCmdError::UnexpectedLayout))
	};

	// Another type of message.
	assert!(refused(BASE + fields, STRING_CMD_TYPE + 1));

	// Too small for the fields, or for any `CNetMessage`.
	assert!(refused(fields - 8, STRING_CMD_TYPE));
	assert!(refused(16 + fields, STRING_CMD_TYPE));

	// A `CNetMessage` that leaves the fields unaligned.
	assert!(refused(BASE + 4 + fields, STRING_CMD_TYPE));

	// A larger `CNetMessage` than any accepted.
	assert!(refused(264 + fields, STRING_CMD_TYPE));
}

#[test]
fn commands_must_fit_the_buffer_with_their_terminator() {
	let longest = CString::new(vec![b'a'; MAX_STRING_CMD_LEN]).unwrap();
	let too_long = CString::new(vec![b'a'; MAX_STRING_CMD_LEN + 1]).unwrap();

	assert_eq!(MAX_STRING_CMD_LEN, 1023);
	assert!(build(handler(), &longest).is_ok());
	assert_eq!(
		build(handler(), &too_long).unwrap_err(),
		StringCmdError::TooLong
	);
}

#[test]
fn messages_are_laid_out_as_the_engine_reads_them() {
	let handler = handler();
	let vtable = string_cmd_vtable();
	let channel = channel();

	// SAFETY: As for `build`.
	let message =
		unsafe { StringCmdMessage::new(vtable, handler, channel, c"say I withdraw") }.unwrap();

	let this = message.as_ptr();
	let fields = this
		.as_ptr()
		.wrapping_byte_add(BASE)
		.cast::<StringCmdFields>();

	// SAFETY: The message is a live object of the mock class: its `CNetMessage`
	// of `BASE` bytes, then the fields, aligned for both.
	let (head, read) = unsafe { (this.cast::<NetMessage>().read(), fields.read()) };

	assert_eq!(head.vtable, vtable.as_ptr().cast_const());
	assert!(head.reliable);
	assert_eq!(head.channel, channel.as_ptr());
	assert_eq!(head.engine_fields, [0; BASE - 24]);

	assert_eq!(read.handler, handler.as_ptr().cast());
	// SAFETY: The projection stays within the fields.
	assert_eq!(
		read.command,
		unsafe { &raw const (*fields).command_buffer }.cast()
	);
	assert_eq!(
		cstring_from_buffer(&read.command_buffer).as_c_str(),
		c"say I withdraw"
	);
	assert_eq!(message.handler(), handler);

	// A hook on the handler reads it as one a client sent.
	// SAFETY: The message is live and unchanged.
	let Some(ClientMessage::StringCmd(decoded)) =
		(unsafe { read_message(MessageClass::StringCmd, handler, this) })
	else {
		panic!("the command is read");
	};

	assert_eq!(decoded.command_buffer, read.command_buffer);
}

#[test]
fn no_engine_factory_is_found_where_no_module_exports_one() {
	/// A vtable in this test binary, which exports no `CreateInterface`.
	static VTABLE: [usize; 1] = [0];

	// One on the heap, which no module holds.
	let heap = Box::new([0_usize; 1]);

	for vtable in [VTABLE.as_ptr(), heap.as_ptr()] {
		let mut client = sys::IClient {
			vtable_: vtable.cast(),
		};

		// SAFETY: The client is live, and its vtable is only located, never
		// read.
		let found = unsafe { engine_factory_of_client(NonNull::from(&mut client)) };

		assert!(matches!(found, Err(util::Error::InvalidImage)));
	}
}

#[test]
fn processing_passes_the_message_to_its_handlers_method() {
	let mut message = build(handler(), c"say_team medic").unwrap();

	PROCESS_CALLS.set(0);
	HANDLED.set(true);

	// SAFETY: The mock class's `Process` only passes the message on, and the
	// mock handler only reads it.
	assert!(unsafe { message.process() });

	HANDLED.set(false);

	// SAFETY: As above.
	assert!(!unsafe { message.process() });

	// Each went through the class's own `Process`, as the engine's do.
	assert_eq!(PROCESS_CALLS.get(), 2);
	assert_eq!(
		PROCESSED.take(),
		vec![Some(c"say_team medic".to_owned()); 2]
	);
}

#[test]
fn the_size_learned_from_the_engines_messages_must_match() {
	let handler = handler();
	let message = build(handler, c"status").unwrap();

	// Fixes the size of `CNetMessage` at `BASE`, as a hook reading the
	// engine's messages would.
	// SAFETY: The message is live and unchanged.
	assert!(unsafe { read_message(MessageClass::StringCmd, handler, message.as_ptr()) }.is_some());

	// A size that would be accepted had no size been learned.
	SIZE.set(BASE + 8 + size_of::<StringCmdFields>());

	let built = build(handler, c"status");

	SIZE.set(BASE + size_of::<StringCmdFields>());
	assert!(matches!(built, Err(StringCmdError::UnexpectedLayout)));
}
