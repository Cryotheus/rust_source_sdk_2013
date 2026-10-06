//! Tests of reading the messages clients send, from the engine's own objects.
//!
//! The tests share one test binary, since the first message read fixes the
//! size of the engine's `CNetMessage` for the process.

use source_sdk_2013_raw::abi::CppDestructors;
use source_sdk_2013_raw::bitbuf::{BfRead, BfWrite};
use source_sdk_2013_raw::net::incoming::{
	ClientMessage, ConVarEntry, MessageClass, MoveFields, SMALLEST_MESSAGE_BASE, SetConVarFields,
	StringCmdFields, TickFields, read_message,
};
use source_sdk_2013_raw::test_support::net::mock_message;
use source_sdk_2013_raw::util::cstr::buffer_from_cstr;
use std::ffi::{CStr, c_char};
use std::ptr::{NonNull, null_mut};

/// The size of `CNetMessage` in mock messages. Every mock that is read
/// shares it, since the first message read fixes the size for the process.
const BASE: usize = SMALLEST_MESSAGE_BASE;

/// The handler mock messages are passed to, which none of them names, so that
/// only their own pointers can confirm the size of `CNetMessage`.
const HANDLER: NonNull<sys::IClientMessageHandler> = NonNull::dangling();

/// Copies `value` into a zeroed buffer.
fn buffer<const N: usize>(value: &CStr) -> [c_char; N] {
	buffer_from_cstr(value).unwrap()
}

/// Reads a `NET_StringCmd`, so that the size of `CNetMessage` is known to be
/// [`BASE`] before a test reads a message that cannot confirm it.
fn confirm_base() {
	let message = string_cmd(c"status");

	// SAFETY: The mock is live and unchanged.
	let read = unsafe { read_message(MessageClass::StringCmd, HANDLER, message) };

	assert!(matches!(read, Some(ClientMessage::StringCmd(_))));
}

#[test]
fn convar_vectors_must_be_consistent() {
	confirm_base();

	let mut entries = [ConVarEntry {
		name: buffer(c"cl_interp"),
		value: buffer(c"0.1"),
	}; 2];
	let elements = entries.as_mut_ptr();

	let set = |capacity, count, elements| {
		let message = mock_message(BASE + size_of::<SetConVarFields>());

		// SAFETY: The mock holds the fields past `BASE`, aligned for them.
		unsafe {
			message
				.as_ptr()
				.byte_add(BASE)
				.cast::<SetConVarFields>()
				.write(SetConVarFields {
					handler: null_mut(),
					convars: sys::CUtlVector {
						_phantom_0: Default::default(),
						_phantom_1: Default::default(),
						m_Memory: sys::CUtlMemory {
							_phantom_0: Default::default(),
							m_pMemory: elements,
							m_nAllocationCount: capacity,
							m_nGrowSize: 0,
						},
						m_Size: count,
						m_pElements: elements,
					},
				});
		}

		// SAFETY: The mock is live and unchanged.
		unsafe { read_message(MessageClass::SetConVar, HANDLER, message) }
	};

	let Some(ClientMessage::SetConVar(convars)) = set(2, 2, elements) else {
		panic!("a consistent vector is read");
	};

	assert_eq!(convars.len(), 2);
	assert_eq!(convars[1].value, entries[1].value);
	assert!(set(1, 2, elements).is_none());
	assert!(set(2, -1, elements).is_none());
	assert!(set(2, 2, null_mut()).is_none());
	assert!(matches!(set(0, 0, null_mut()), Some(ClientMessage::SetConVar(v)) if v.is_empty()));
}

#[test]
fn messages_of_another_base_are_refused() {
	confirm_base();

	let message = mock_message(BASE + 8 + size_of::<TickFields>());

	// SAFETY: The mock is live and unchanged.
	assert!(unsafe { read_message(MessageClass::Tick, HANDLER, message) }.is_none());

	let message = mock_message(BASE - 8 + size_of::<TickFields>());

	// SAFETY: The mock is live and unchanged.
	assert!(unsafe { read_message(MessageClass::Tick, HANDLER, message) }.is_none());
}

#[test]
fn moves_copy_their_payload() {
	confirm_base();

	let packet = [0b1010_0000_u8, 0b0000_0111];
	let message = mock_message(BASE + size_of::<MoveFields>());

	let reader = BfRead {
		data: packet.as_ptr(),
		data_bytes: 2,
		data_bits: 16,
		cur_bit: 5,
		overflow: 0,
		assert_on_overflow: 0,
		debug_name: std::ptr::null(),
	};

	let mut storage = [0];
	let fields = message
		.as_ptr()
		.wrapping_byte_add(BASE)
		.cast::<MoveFields>();

	let mut write = |length| {
		// SAFETY: The mock holds the fields past `BASE`, aligned for them.
		unsafe {
			fields.write(MoveFields {
				handler: null_mut(),
				backup_commands: 2,
				new_commands: 1,
				length,
				data_in: reader,
				data_out: BfWrite::empty(&mut storage),
			});
		}
	};

	write(6);

	// SAFETY: The mock is live and unchanged, and its reader describes
	// `packet`.
	let Some(ClientMessage::Move { fields: read, data }) =
		(unsafe { read_message(MessageClass::Move, HANDLER, message) })
	else {
		panic!("the move is read");
	};

	assert_eq!((read.backup_commands, read.new_commands), (2, 1));
	assert_eq!((data.len(), data.as_words()), (6, &[0b11_1101][..]));

	// Past the packet's bits.
	write(12);

	// SAFETY: The mock is live and unchanged, and its reader describes
	// `packet`.
	assert!(unsafe { read_message(MessageClass::Move, HANDLER, message) }.is_none());
}

#[test]
fn process_slots_follow_the_abi() {
	for (index, class) in MessageClass::ALL.into_iter().enumerate() {
		assert_eq!(class.process_slot(), CppDestructors::VTABLE_SLOTS + index);
	}
}

/// A `NET_StringCmd` holding `command`, pointing at its own buffer as the
/// engine's do.
fn string_cmd(command: &CStr) -> NonNull<sys::INetMessage> {
	let message = mock_message(BASE + size_of::<StringCmdFields>());
	let fields = message
		.as_ptr()
		.wrapping_byte_add(BASE)
		.cast::<StringCmdFields>();

	// SAFETY: The mock holds the fields past `BASE`, aligned for them.
	unsafe {
		fields.write(StringCmdFields {
			handler: null_mut(),
			command: (&raw const (*fields).command_buffer).cast(),
			command_buffer: buffer(command),
		});
	}

	message
}

#[test]
fn string_commands_confirm_the_base() {
	let message = string_cmd(c"say hi");

	// SAFETY: The mock is live and unchanged.
	let Some(ClientMessage::StringCmd(fields)) =
		(unsafe { read_message(MessageClass::StringCmd, HANDLER, message) })
	else {
		panic!("the command is read");
	};

	assert_eq!(fields.command_buffer, buffer::<1024>(c"say hi"));

	// Without its self-pointer, nothing confirms the layout, so the message
	// is only read because the base is already known to be `BASE`.
	let message = mock_message(BASE + size_of::<StringCmdFields>());

	// SAFETY: The mock is live and unchanged.
	assert!(unsafe { read_message(MessageClass::StringCmd, HANDLER, message) }.is_some());
}
