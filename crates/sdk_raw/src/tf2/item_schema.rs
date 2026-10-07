//! TF2's item schema's sorted map of its definitions (`m_mapItemsSorted`, a
//! `CUtlMap<int, CEconItemDefinition *, int>`), which the generated bindings
//! leave opaque. Its red-black tree is laid out here by hand, from
//! `public/tier1/utlrbtree.h` and `public/tier1/utlmap.h`.
//!
//! [`sorted_definitions`] checks the map before reading any of its nodes, and
//! each node as it walks the tree, so that a schema laid out otherwise than
//! the bindings say is refused instead of read out of bounds: the schema's
//! default definition must sit where `GetItemDefinition` finds it, after the
//! map; the tree's copy of its node pointer must match the pointer itself;
//! its counts must agree; and every node must be in use, hold a definition
//! with its own index, and come in order.

#[cfg(test)]
#[path = "../tests/tf2/item_schema.rs"]
mod tests;

use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

/// `CUtlRBTree::InvalidIndex()`, the index of no node.
const INVALID: c_int = -1;

/// The most nodes a map is believed to allocate: far more than the schema's
/// definitions, so that a misread count cannot make a walk run away.
const MAX_NODES: c_int = 1 << 20;

// The tree must fill the opaque map exactly, as the map adds no fields to it.
const _: () = {
	assert!(size_of::<Tree>() == size_of::<sys::CEconItemSchema_SortedItemDefinitionMap_t>());
	assert!(align_of::<Tree>() == align_of::<sys::CEconItemSchema_SortedItemDefinitionMap_t>());
	assert!(size_of::<Node>() == 32);
};

/// `UtlRBTreeNode_t<CUtlMap::Node_t, int>`: a node's links, then the map's key
/// and element.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Node {
	/// `m_Left`, the node's own index while it is free.
	left: c_int,

	/// `m_Right`, the next free node while it is free.
	right: c_int,

	/// `m_Parent`.
	parent: c_int,

	/// `m_Tag`, the node's color.
	tag: c_int,

	/// `key`, the definition's index.
	key: c_int,

	/// `elem`, the definition.
	definition: *mut sys::CEconItemDefinition,
}

/// `CUtlRBTree<CUtlMap::Node_t, int, CKeyLess>`, with its `CUtlMemory` of
/// nodes inline.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Tree {
	/// `m_LessFunc`, the map's comparison.
	less: *const c_void,

	/// `m_Elements.m_pMemory`, the nodes.
	memory: *mut Node,

	/// `m_Elements.m_nAllocationCount`, how many nodes `memory` holds.
	allocation_count: c_int,

	/// `m_Elements.m_nGrowSize`.
	grow_size: c_int,

	/// `m_Root`.
	root: c_int,

	/// `m_NumElements`, how many nodes are in the tree.
	count: c_int,

	/// `m_FirstFree`.
	first_free: c_int,

	/// `m_LastAlloc`, the last node ever allocated.
	last_alloc: c_int,

	/// `m_pElements`, a copy of `memory` the tree keeps for debuggers.
	elements: *mut Node,
}

/// A walk of a tree, which reads at most a budget of nodes.
struct Walk {
	/// How many more nodes the walk may read.
	budget: usize,

	/// The tree.
	tree: Tree,
}

impl Walk {
	/// The leftmost node under `index`, the first in order.
	fn leftmost(&mut self, mut index: c_int) -> Option<(c_int, Node)> {
		loop {
			let node = self.node(index)?;

			if node.left == INVALID {
				return Some((index, node));
			}

			index = node.left;
		}
	}

	/// The node at `index`, if the tree allocated it, it is in use, and the
	/// budget allows reading it.
	fn node(&mut self, index: c_int) -> Option<Node> {
		self.budget = self.budget.checked_sub(1)?;

		if !(0..=self.tree.last_alloc).contains(&index) {
			return None;
		}

		// SAFETY: The caller of `sorted_definitions` vouches for the live tree,
		// whose header was checked: the node lies within its allocation.
		let node = unsafe { self.tree.memory.add(index as usize).read() };

		// A free node is its own left child.
		(node.left != index).then_some(node)
	}

	/// The node after `node`, at `index`, in order, with its index, or
	/// `Some(None)` after the last. `None` if the links disagree.
	fn successor(&mut self, index: c_int, node: Node) -> Option<Option<(c_int, Node)>> {
		if node.right != INVALID {
			return self.leftmost(node.right).map(Some);
		}

		let mut child = index;
		let mut parent = node.parent;

		// Climbs to the first ancestor that the node is left of.
		while parent != INVALID {
			let above = self.node(parent)?;

			if above.left == child {
				return Some(Some((parent, above)));
			}

			if above.right != child {
				return None;
			}

			child = parent;
			parent = above.parent;
		}

		Some(None)
	}
}

/// The definitions of the item schema's sorted map, by index, in order of
/// their index, or `None` if the map is not laid out as the
/// [module documentation](self) says it must be.
///
/// The walk reads the whole map, which holds a node per definition. The
/// definitions are the schema's own, which it frees when the game applies a
/// newer schema.
///
/// # Safety
///
/// - `schema` points to the game's live item schema, which is not changed
///   during the call, as it is not on the server's main thread outside the
///   game's own schema updates.
/// - `default` is the schema's default definition, which
///   `CEconItemSchema::GetItemDefinition` returns for an index the schema
///   does not have.
#[doc(alias("m_mapItemsSorted", "GetSortedItemDefinitionMap"))]
pub unsafe fn sorted_definitions(
	schema: NonNull<sys::CEconItemSchema>,
	default: NonNull<sys::CEconItemDefinition>,
) -> Option<Vec<(u16, NonNull<sys::CEconItemDefinition>)>> {
	let schema = schema.as_ptr();

	// SAFETY: The schema is live. Both fields lie within it at the bindings'
	// offsets, and are plain data: the map's bytes and a pointer.
	let (tree, schema_default) = unsafe {
		(
			(&raw const (*schema).m_mapItemsSorted)
				.cast::<Tree>()
				.read(),
			(&raw const (*schema).m_pDefaultItemDefinition).read(),
		)
	};

	// The default definition comes after the map, so finding it places the
	// fields before it, the map among them.
	if schema_default != default.as_ptr() {
		return None;
	}

	// The header must be consistent before any node is read.
	let consistent = !tree.memory.is_null()
		&& tree.memory.is_aligned()
		&& tree.elements == tree.memory
		&& (0..=MAX_NODES).contains(&tree.allocation_count)
		&& (INVALID..tree.allocation_count).contains(&tree.last_alloc)
		&& (1..=tree.last_alloc + 1).contains(&tree.count)
		&& (0..=tree.last_alloc).contains(&tree.root);

	if !consistent {
		return None;
	}

	let count = tree.count as usize;

	// An in-order walk reads each node once going down, and climbs each link
	// at most once.
	let mut walk = Walk {
		budget: count * 2,
		tree,
	};

	let mut definitions = Vec::with_capacity(count);
	let mut next = Some(walk.leftmost(tree.root)?);

	while let Some((index, node)) = next {
		if definitions.len() == count {
			return None;
		}

		let key = u16::try_from(node.key).ok()?;

		if definitions.last().is_some_and(|&(last, _)| last >= key) {
			return None;
		}

		let definition = NonNull::new(node.definition)?;

		// SAFETY: The node is in use, and its element is a live definition, whose
		// index lies at the bindings' offset.
		if unsafe { (&raw const (*definition.as_ptr()).m_nDefIndex).read() } != key {
			return None;
		}

		definitions.push((key, definition));
		next = walk.successor(index, node)?;
	}

	(definitions.len() == count).then_some(definitions)
}
