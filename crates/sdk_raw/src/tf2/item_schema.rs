//! TF2's item schema's sorted map of its definitions (`m_mapItemsSorted`, a
//! `CUtlMap<int, CEconItemDefinition *, int>`), which the generated bindings
//! leave opaque. Its red-black tree is laid out here by hand, from
//! `public/tier1/utlrbtree.h` and `public/tier1/utlmap.h`.
//!
//! [`sorted_definitions`] checks the map before reading any of its nodes, and
//! each node as it walks the tree, so that a schema laid out otherwise than
//! the bindings say is refused instead of read out of bounds: the schema's
//! default definition, which `GetItemDefinition` finds, must be held by a slot
//! after the map (the bindings name `m_pDefaultItemDefinition`, but the game's
//! build can put fields between); the tree's copy of its node pointer must
//! match the pointer itself; its counts must agree; and every node must be in
//! use, hold a definition with its own index, and come in order.

#[cfg(test)]
#[path = "../tests/tf2/item_schema.rs"]
mod tests;

use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

// The tree must fill the opaque map exactly, as the map adds no fields to it.
const _: () = {
	assert!(size_of::<Tree>() == size_of::<sys::CEconItemSchema_SortedItemDefinitionMap_t>());
	assert!(align_of::<Tree>() == align_of::<sys::CEconItemSchema_SortedItemDefinitionMap_t>());
	assert!(size_of::<Node>() == 32);
};

/// `CUtlRBTree::InvalidIndex()`, the index of no node.
const INVALID: c_int = -1;

/// The most nodes a map is believed to allocate: far more than the schema's
/// definitions, so that a misread count cannot make a walk run away.
const MAX_NODES: c_int = 1 << 20;

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

/// Where, after the sorted map and within the bindings' size of the schema,
/// the schema holds the pointer to its default definition, as an offset from
/// the schema, or `None` if it holds it nowhere there. The bindings place the
/// pointer at `m_pDefaultItemDefinition`, but the game's own build may put
/// fields between the map and it, so any pointer-sized slot after the map
/// counts.
///
/// # Safety
///
/// `schema` points to the game's live item schema, as for
/// [`sorted_definitions`].
unsafe fn default_offset(
	schema: *const sys::CEconItemSchema,
	default: NonNull<sys::CEconItemDefinition>,
) -> Option<usize> {
	let after_map =
		std::mem::offset_of!(sys::CEconItemSchema, m_mapItemsSorted) + size_of::<Tree>();

	(after_map..=size_of::<sys::CEconItemSchema>().saturating_sub(size_of::<usize>()))
		.step_by(size_of::<usize>())
		.find(|&offset| {
			// SAFETY: The slot lies within the bindings' size of the schema, past
			// its map, and holds plain data; the game's schema is at least as large.
			let slot = unsafe {
				schema
					.cast::<u8>()
					.add(offset)
					.cast::<usize>()
					.read_unaligned()
			};

			slot == default.as_ptr().addr()
		})
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
	let tree = unsafe {
		(&raw const (*schema).m_mapItemsSorted)
			.cast::<Tree>()
			.read()
	};

	// The default definition comes after the map, so finding it there places
	// the fields before it, the map among them.
	// SAFETY: As above; the search stays within the schema.
	if unsafe { default_offset(schema, default) }.is_none() {
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

/// What the checks of [`sorted_definitions`] read, as text, for finding out
/// which of them fails: the map's header, each check on it, the first nodes,
/// and what the walk makes of the map.
///
/// # Safety
///
/// As for [`sorted_definitions`].
pub unsafe fn sorted_definitions_report(
	schema: NonNull<sys::CEconItemSchema>,
	default: NonNull<sys::CEconItemDefinition>,
) -> String {
	use std::fmt::Write;

	let schema_ptr = schema.as_ptr();

	// SAFETY: As for `sorted_definitions`.
	let (tree, schema_default) = unsafe {
		(
			(&raw const (*schema_ptr).m_mapItemsSorted)
				.cast::<Tree>()
				.read(),
			(&raw const (*schema_ptr).m_pDefaultItemDefinition).read(),
		)
	};

	let mut report = String::new();

	let _ = writeln!(
		report,
		"map at offset {}, default definition at offset {}: schema holds default {:p}, lookup gave {:p}",
		std::mem::offset_of!(sys::CEconItemSchema, m_mapItemsSorted),
		std::mem::offset_of!(sys::CEconItemSchema, m_pDefaultItemDefinition),
		schema_default,
		default.as_ptr(),
	);

	let _ = writeln!(
		report,
		"tree: less {:p}, memory {:p}, elements {:p}, allocation_count {}, grow_size {}, root {}, count {}, first_free {}, last_alloc {}",
		tree.less,
		tree.memory,
		tree.elements,
		tree.allocation_count,
		tree.grow_size,
		tree.root,
		tree.count,
		tree.first_free,
		tree.last_alloc,
	);

	let _ = writeln!(
		report,
		"checks: default found at offset {:?}, memory set {}, aligned {}, elements == memory {}, allocation_count in range {}, last_alloc in range {}, count in range {}, root in range {}",
		// SAFETY: As for this function.
		unsafe { default_offset(schema_ptr, default) },
		!tree.memory.is_null(),
		tree.memory.is_aligned(),
		tree.elements == tree.memory,
		(0..=MAX_NODES).contains(&tree.allocation_count),
		(INVALID..tree.allocation_count).contains(&tree.last_alloc),
		(1..=tree.last_alloc + 1).contains(&tree.count),
		(0..=tree.last_alloc).contains(&tree.root),
	);

	if !tree.memory.is_null() && tree.memory.is_aligned() && tree.last_alloc >= 0 {
		for index in 0..=tree.last_alloc.min(4) {
			// SAFETY: The node lies within the allocation the header claims; a
			// diagnostic read of plain data.
			let node = unsafe { tree.memory.add(index as usize).read() };

			let _ = writeln!(
				report,
				"node {index}: left {}, right {}, parent {}, tag {}, key {}, definition {:p}",
				node.left, node.right, node.parent, node.tag, node.key, node.definition,
			);
		}
	}

	// SAFETY: As for this function.
	let walked = unsafe { sorted_definitions(schema, default) };

	let _ = match &walked {
		Some(definitions) => writeln!(report, "walk: {} definitions", definitions.len()),
		None => writeln!(report, "walk: refused"),
	};

	report
}
