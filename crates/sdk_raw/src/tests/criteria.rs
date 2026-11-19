//! Reads of hand-built criteria sets.

use super::*;
use std::mem::MaybeUninit;
use std::ptr;

#[test]
fn a_tree_laid_out_otherwise_is_refused() {
	let fresh = || [node(0, Some(7)), node(1, Some(9))];

	assert_eq!(
		read(&mut fresh(), |tree| tree.elements = ptr::null_mut()),
		None
	);
	assert_eq!(read(&mut fresh(), |tree| tree.allocation_count = 1), None);
	assert_eq!(read(&mut fresh(), |tree| tree.count = 3), None);
	assert_eq!(read(&mut fresh(), |tree| tree.last_alloc = INVALID), None);
}

#[test]
fn an_empty_set_reads_no_node() {
	assert_eq!(
		read(&mut [], |tree| tree.memory = ptr::null_mut()),
		Some(Vec::new())
	);
}

#[test]
fn free_nodes_are_skipped() {
	let mut nodes = [node(0, Some(7)), node(1, None), node(2, Some(3))];

	assert_eq!(read(&mut nodes, |_| {}), Some(vec![7, 3]));
}

/// A node holding `symbol`'s entry, or a free one, its own left child.
fn node(index: c_short, symbol: Option<u16>) -> Node {
	// SAFETY: An entry is plain data, valid when zeroed.
	let mut entry: sys::AI_CriteriaSet_CritEntry_t = unsafe { MaybeUninit::zeroed().assume_init() };

	entry.criterianame.m_Id = symbol.unwrap_or_default();

	Node {
		left: if symbol.is_some() { INVALID } else { index },
		right: INVALID,
		parent: INVALID,
		tag: 0,
		entry,
	}
}

/// The symbols of the entries `entries` reads from a tree over `nodes`.
fn read(nodes: &mut [Node], tree: impl FnOnce(&mut Tree)) -> Option<Vec<u16>> {
	let memory = nodes.as_mut_ptr();
	let mut header = Tree {
		less: ptr::null(),
		memory,
		allocation_count: nodes.len() as i32 + 2,
		grow_size: 0,
		root: 0,
		count: nodes.iter().filter(|node| node.left == INVALID).count() as c_short,
		first_free: INVALID,
		last_alloc: nodes.len() as c_short - 1,
		elements: memory,
	};

	tree(&mut header);

	let set = NonNull::from(&mut header).cast::<sys::AI_CriteriaSet>();

	// SAFETY: The set is the tree, over live nodes.
	let entries = unsafe { entries(set) }?;

	// SAFETY: The entries point into the live nodes.
	Some(
		entries
			.into_iter()
			.map(|entry| unsafe { entry.as_ref().criterianame.m_Id })
			.collect(),
	)
}
