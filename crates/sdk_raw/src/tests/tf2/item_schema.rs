//! Walks of hand-built sorted maps of item definitions.

use super::*;
use std::mem::MaybeUninit;
use std::ptr;

/// The definitions' indices, by node.
const KEYS: [c_int; 5] = [5, 0, 18, 738, 49];

/// The node no definition is in, which is free.
const FREE: c_int = 1;

/// A schema whose sorted map holds 5, 18, 49 and 738, stored out of order
/// with a free node among them:
///
/// ```text
///       18 (2)
///      /      \
///   5 (0)    738 (3)
///            /
///        49 (4)
/// ```
struct Schema {
	default: Box<MaybeUninit<sys::CEconItemDefinition>>,
	definitions: Vec<MaybeUninit<sys::CEconItemDefinition>>,
	nodes: Vec<Node>,
	schema: Box<MaybeUninit<sys::CEconItemSchema>>,

	/// The default definition the schema holds.
	stored_default: *mut sys::CEconItemDefinition,

	tree: Tree,
}

impl Schema {
	fn new() -> Self {
		let links = [
			(INVALID, INVALID, 2),
			(FREE, INVALID, INVALID),
			(0, 3, INVALID),
			(4, INVALID, 2),
			(INVALID, INVALID, 3),
		];

		let mut definitions: Vec<_> = KEYS
			.iter()
			.map(|&key| {
				let mut definition = MaybeUninit::<sys::CEconItemDefinition>::zeroed();

				// SAFETY: The definition is allocated, and its index is plain data.
				unsafe { (&raw mut (*definition.as_mut_ptr()).m_nDefIndex).write(key as u16) };

				definition
			})
			.collect();

		let mut nodes: Vec<_> = links
			.into_iter()
			.zip(&mut definitions)
			.zip(KEYS)
			.map(|(((left, right, parent), definition), key)| Node {
				left,
				right,
				parent,
				tag: 0,
				key,
				definition: definition.as_mut_ptr(),
			})
			.collect();

		// Room the tree allocated without using it.
		nodes.extend([nodes[FREE as usize]; 3]);

		let memory = nodes.as_mut_ptr();
		let mut default = Box::new_zeroed();

		let mut schema = Self {
			stored_default: default.as_mut_ptr(),
			default,
			definitions,
			nodes,
			schema: Box::new_zeroed(),
			tree: Tree {
				less: ptr::null(),
				memory,
				allocation_count: 8,
				grow_size: 0,
				root: 2,
				count: 4,
				first_free: FREE,
				last_alloc: 4,
				elements: memory,
			},
		};

		schema.store();
		schema
	}

	/// The definitions the map holds, by index, in order.
	fn expected(&mut self) -> Vec<(u16, NonNull<sys::CEconItemDefinition>)> {
		[0, 2, 4, 3]
			.into_iter()
			.map(|node| {
				(
					KEYS[node] as u16,
					NonNull::new(self.definitions[node].as_mut_ptr()).unwrap(),
				)
			})
			.collect()
	}

	/// Writes the tree and the default definition into the schema.
	fn store(&mut self) {
		let schema = self.schema.as_mut_ptr();

		// SAFETY: The schema is allocated, and both fields are plain data.
		unsafe {
			(&raw mut (*schema).m_mapItemsSorted)
				.cast::<Tree>()
				.write(self.tree);
			(&raw mut (*schema).m_pDefaultItemDefinition).write(self.stored_default);
		}
	}

	fn walk(&mut self) -> Option<Vec<(u16, NonNull<sys::CEconItemDefinition>)>> {
		self.store();

		// SAFETY: The schema, its nodes and its definitions live in `self`.
		unsafe {
			sorted_definitions(
				NonNull::new(self.schema.as_mut_ptr()).unwrap(),
				NonNull::new(self.default.as_mut_ptr()).unwrap(),
			)
		}
	}
}

#[test]
fn definitions_are_walked_in_order_of_their_index() {
	let mut schema = Schema::new();
	let expected = schema.expected();

	assert_eq!(schema.walk(), Some(expected));
}

#[test]
fn maps_laid_out_otherwise_are_refused() {
	type Change = fn(&mut Schema);

	let changes: [(&str, Change); 10] = [
		("another default definition", |schema| {
			schema.stored_default = schema.definitions[0].as_mut_ptr();
		}),
		("a debug pointer that differs", |schema| {
			schema.tree.elements = schema.tree.elements.wrapping_add(1);
		}),
		("no nodes", |schema| schema.tree.memory = ptr::null_mut()),
		("more nodes than counted", |schema| schema.tree.count = 3),
		("fewer nodes than counted", |schema| schema.tree.count = 5),
		("a root never allocated", |schema| schema.tree.root = 5),
		("a link to a free node", |schema| {
			schema.nodes[4].left = FREE
		}),
		("a cycle", |schema| schema.nodes[4].right = 3),
		("keys out of order", |schema| {
			schema.nodes[0].key = 20;

			// SAFETY: The definition is allocated, and its index is plain data.
			unsafe { (&raw mut (*schema.definitions[0].as_mut_ptr()).m_nDefIndex).write(20) };
		}),
		("a key its definition does not hold", |schema| {
			// SAFETY: The definition is allocated, and its index is plain data.
			unsafe { (&raw mut (*schema.definitions[4].as_mut_ptr()).m_nDefIndex).write(48) };
		}),
	];

	for (name, change) in changes {
		let mut schema = Schema::new();

		change(&mut schema);

		assert_eq!(schema.walk(), None, "{name}");
	}
}
