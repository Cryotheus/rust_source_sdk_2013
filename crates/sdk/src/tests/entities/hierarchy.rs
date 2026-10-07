//! Tests of parenting entities: the inputs sent, the cycles refused, and the
//! attachments checked.

use super::*;
use crate::entities::EntityHandle;

use crate::test_support::entities::{
	MockEntity, input, set_accepts, set_datamap, state_maps, take_inputs,
};

use crate::test_support::server_tools::MockTools;
use std::ffi::CString;
use std::ptr::NonNull;

const CHILD: u32 = 7 | 3 << 16;
const GRANDPARENT: u32 = 9 | 2 << 16;
const PARENT: u32 = 8 | 1 << 16;

struct Mocks {
	tools: MockTools,
	child: MockEntity,
	parent: MockEntity,
	grandparent: MockEntity,
}

#[test]
fn attachments_are_followed_once_found() {
	let mut mocks = mocks();
	let tools = mocks.tools.tools();
	let child = entity(&mut mocks.child);
	let parent = entity(&mut mocks.parent);

	// The game sets the attachment only once it finds it, which the mock
	// entity's `AcceptInput` leaves to the test.
	mocks.child.state().parent_attachment = 2;
	assert_eq!(child.parent_attachment(), Ok(NonZeroU8::new(2)));
	tools
		.set_parent_attachment(child, parent, c"head", false)
		.unwrap();
	tools
		.set_parent_attachment(child, parent, c"head", true)
		.unwrap();

	let set_parent = (
		c"SetParent".to_owned(),
		parent.as_ptr(),
		child.as_ptr(),
		Some(c"!activator".to_owned()),
	);
	let attach = |input: &CStr| {
		(
			input.to_owned(),
			parent.as_ptr(),
			child.as_ptr(),
			Some(c"head".to_owned()),
		)
	};

	assert_eq!(
		received(child),
		[
			set_parent.clone(),
			attach(c"SetParentAttachment"),
			set_parent,
			attach(c"SetParentAttachmentMaintainOffset"),
		]
	);

	mocks.child.state().parent_attachment = 0;
	assert_eq!(child.parent_attachment(), Ok(None));
	assert_eq!(
		tools.set_parent_attachment(child, parent, c"missing", false),
		Err(ParentError::UnknownAttachment)
	);
	assert_eq!(received(child).len(), 2);

	// Nothing is sent once the parent is refused.
	assert_eq!(
		tools.set_parent_attachment(child, child, c"head", false),
		Err(ParentError::Cycle)
	);
	assert!(received(child).is_empty());
}

#[test]
fn cycles_are_refused() {
	let mut mocks = mocks();
	let tools = mocks.tools.tools();
	let child = entity(&mut mocks.child);
	let parent = entity(&mut mocks.parent);

	assert_eq!(tools.set_parent(child, child), Err(ParentError::Cycle));

	// The child is the parent's grandparent.
	mocks.parent.state().move_parent = GRANDPARENT;
	mocks.grandparent.state().move_parent = CHILD;
	assert_eq!(tools.set_parent(child, parent), Err(ParentError::Cycle));

	// A chain that loops without the child is longer than the entity list.
	mocks.grandparent.state().move_parent = PARENT;
	assert_eq!(tools.set_parent(child, parent), Err(ParentError::Cycle));
	assert!(take_inputs().is_empty());

	// A chain ends at an entity without a parent, or with a stale one.
	mocks.grandparent.state().move_parent = EntityHandle::INVALID.to_raw();
	tools.set_parent(child, parent).unwrap();
	mocks.grandparent.state().move_parent = CHILD + (1 << 16);
	tools.set_parent(child, parent).unwrap();
	assert_eq!(received(child).len(), 2);
}

/// A mock entity, for as long as the mocks live.
fn entity(mock: &mut MockEntity) -> Entity<'static> {
	// SAFETY: Mock entities are leaked, and their vtables answer what
	// parenting calls of a `CBaseEntity`.
	unsafe { Entity::from_raw(NonNull::new(mock.as_ptr()).unwrap()) }
}

/// A child, its parent-to-be and that one's parent-to-be, listed in the
/// entity list without move parents, whose class declares the parenting
/// inputs, and a world to pool strings into.
fn mocks() -> Mocks {
	use sys::{_fieldtypes_FIELD_STRING as STRING, _fieldtypes_FIELD_VOID as VOID};

	let mut world = MockEntity::new(0);
	let mut mocks = Mocks {
		tools: MockTools::new(world.as_ptr()),
		child: MockEntity::new(CHILD),
		parent: MockEntity::new(PARENT),
		grandparent: MockEntity::new(GRANDPARENT),
	};

	set_datamap(state_maps(vec![(
		c"CTestEntity",
		vec![
			input(c"ClearParent", VOID),
			input(c"SetParent", STRING),
			input(c"SetParentAttachment", STRING),
			input(c"SetParentAttachmentMaintainOffset", STRING),
		],
	)]));

	for (mock, handle) in [
		(&mut mocks.child, CHILD),
		(&mut mocks.parent, PARENT),
		(&mut mocks.grandparent, GRANDPARENT),
	] {
		mocks
			.tools
			.list(mock.as_ptr(), EntityHandle::from_raw(handle));
		mock.state().move_parent = EntityHandle::INVALID.to_raw();
	}

	set_accepts(true);
	take_inputs();
	mocks
}

#[test]
fn parents_are_set_and_cleared_through_inputs() {
	let mut mocks = mocks();
	let tools = mocks.tools.tools();
	let child = entity(&mut mocks.child);
	let parent = entity(&mut mocks.parent);

	tools.set_parent(child, parent).unwrap();
	tools.clear_parent(child).unwrap();

	assert_eq!(
		received(child),
		[
			(
				c"SetParent".to_owned(),
				parent.as_ptr(),
				child.as_ptr(),
				Some(c"!activator".to_owned())
			),
			(
				c"ClearParent".to_owned(),
				child.as_ptr(),
				child.as_ptr(),
				None
			),
		]
	);

	set_accepts(false);
	assert_eq!(
		tools.set_parent(child, parent),
		Err(ParentError::Input(InputError::Rejected))
	);
	assert_eq!(tools.clear_parent(child), Err(InputError::Rejected));
}

/// The name, activator, caller and string of each input `target` received.
fn received(
	target: Entity<'_>,
) -> Vec<(
	CString,
	*mut sys::CBaseEntity,
	*mut sys::CBaseEntity,
	Option<CString>,
)> {
	take_inputs()
		.into_iter()
		.map(|input| {
			assert_eq!(input.target, target.as_ptr());

			// SAFETY: String values were pooled, and stay allocated for the
			// rest of the thread.
			let string = (!input.string.is_null())
				.then(|| unsafe { CStr::from_ptr(input.string) }.to_owned());

			(input.name, input.activator, input.caller, string)
		})
		.collect()
}
