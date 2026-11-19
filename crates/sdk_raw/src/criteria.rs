//! The criteria sets of the response system (`AI_CriteriaSet`), which an
//! entity fills before the game picks what it says or does, such as a
//! player's taunt, and the virtual method that fills them.
//!
//! The game builds a fresh set for each concept an entity speaks: the
//! concept's name and modifiers first, then `CBaseEntity::ModifyOrAppendCriteria`
//! of the speaker, which classes override to append their own, and finally
//! asks the response system for the rule whose criteria the set matches best.
//!
//! A set is a red-black tree (`CUtlRBTree<CritEntry_t, short>`) that the
//! generated bindings lay out with a zero-sized `m_LastAlloc`, so it is laid
//! out here by hand, from `public/tier1/utlrbtree.h`. Each entry holds its
//! criterion's name as a symbol of the game's own symbol table
//! (`CUtlSymbol`), which is not exported: names can be compared between
//! entries, but not read. [`entries`] checks the tree's header before reading
//! any node, so that a set laid out otherwise is refused instead of read out
//! of bounds.

#[cfg(test)]
#[path = "tests/criteria.rs"]
mod tests;

use crate::vtable_slot;
use std::ffi::{c_short, c_void};
use std::ptr::NonNull;

/// The signature of `CBaseEntity::ModifyOrAppendCriteria`,
/// `void (AI_CriteriaSet &set)`, at [`MODIFY_OR_APPEND_CRITERIA_SLOT`].
#[doc(alias("ModifyOrAppendCriteria"))]
pub type ModifyOrAppendCriteriaFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, set: *mut sys::AI_CriteriaSet);

// TF2's players override it in the same slot.
#[cfg(feature = "tf2")]
const _: () = assert!(
	MODIFY_OR_APPEND_CRITERIA_SLOT
		== vtable_slot!(
			sys::CTFPlayer__bindgen_vtable,
			CTFPlayer_ModifyOrAppendCriteria
		)
);

// The hand-written tree must fill the generated set exactly, and its nodes
// hold the generated entry after their links.
const _: () = {
	assert!(size_of::<Tree>() == size_of::<sys::AI_CriteriaSet>());
	assert!(align_of::<Tree>() == align_of::<sys::AI_CriteriaSet>());
	assert!(std::mem::offset_of!(Node, entry) == 8);
	assert!(size_of::<sys::AI_CriteriaSet_CritEntry_t>() == 72);
};

/// `CUtlRBTree::InvalidIndex()` for `short` indices, the index of no node.
const INVALID: c_short = -1;

/// The slot of `CBaseEntity::ModifyOrAppendCriteria` in an entity's primary
/// vtable, from the generated binding: the method that appends the entity's
/// criteria to a set before the game picks a response for it.
#[doc(alias("ModifyOrAppendCriteria"))]
pub const MODIFY_OR_APPEND_CRITERIA_SLOT: usize = vtable_slot!(
	sys::CBaseEntity__bindgen_vtable,
	CBaseEntity_ModifyOrAppendCriteria
);

/// `UtlRBTreeNode_t<CritEntry_t, short>`: a node's links, then its entry.
#[derive(Debug)]
#[repr(C)]
struct Node {
	/// `m_Left`, the node's own index while it is free.
	left: c_short,

	/// `m_Right`.
	right: c_short,

	/// `m_Parent`.
	parent: c_short,

	/// `m_Tag`, the node's color.
	tag: c_short,

	/// `m_Data`.
	entry: sys::AI_CriteriaSet_CritEntry_t,
}

/// `CUtlRBTree<CritEntry_t, short>`, with its `CUtlMemory` of nodes inline.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Tree {
	/// `m_LessFunc`, `CritEntry_t::LessFunc`.
	less: *const c_void,

	/// `m_Elements.m_pMemory`, the nodes.
	memory: *mut Node,

	/// `m_Elements.m_nAllocationCount`, how many nodes `memory` holds.
	allocation_count: i32,

	/// `m_Elements.m_nGrowSize`.
	grow_size: i32,

	/// `m_Root`.
	root: c_short,

	/// `m_NumElements`, how many nodes are in the tree.
	count: c_short,

	/// `m_FirstFree`.
	first_free: c_short,

	/// `m_LastAlloc`, the last node ever allocated.
	last_alloc: c_short,

	/// `m_pElements`, a copy of `memory` the tree keeps for debuggers.
	elements: *mut Node,
}

/// The entries of `set`, in no particular order, or `None` if its tree is
/// not laid out as the [module documentation](self) says: its node pointer
/// and the copy it keeps must agree, the last node allocated must lie within
/// the allocation, and as many nodes must be in use as the tree counts.
///
/// # Safety
///
/// `set` points to a live criteria set of the game's, which nothing else
/// changes while the entries are used.
pub unsafe fn entries(
	set: NonNull<sys::AI_CriteriaSet>,
) -> Option<Vec<NonNull<sys::AI_CriteriaSet_CritEntry_t>>> {
	// SAFETY: As the caller promises, the set is live, and the tree fills it.
	let tree = unsafe { set.cast::<Tree>().read() };
	let count = usize::try_from(tree.count).ok()?;

	if count == 0 {
		return Some(Vec::new());
	}

	let memory = NonNull::new(tree.memory)?;

	if tree.elements != tree.memory
		|| tree.last_alloc == INVALID
		|| !(0..tree.allocation_count).contains(&i32::from(tree.last_alloc))
	{
		return None;
	}

	let mut entries = Vec::with_capacity(count);

	for index in 0..=tree.last_alloc {
		// SAFETY: The index lies within the allocation, as checked above, and
		// the nodes up to the last allocated are initialized.
		let node = unsafe { memory.add(index as usize) };

		// SAFETY: As above. Only the link is read.
		let left = unsafe { (&raw const (*node.as_ptr()).left).read() };

		// A free node is its own left child.
		if left == index {
			continue;
		}

		// SAFETY: The node is in use, and holds its entry after its links.
		entries.push(unsafe { NonNull::new_unchecked(&raw mut (*node.as_ptr()).entry) });
	}

	(entries.len() == count).then_some(entries)
}
