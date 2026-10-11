//! Decoding the qualified serialized subset without native memory access.

use super::{Bounds, Bsp, Child, Error, Invalid, Leaf, Limits, Model, Node, Plane, Resource, Work};
use super::{bound, finite, raw, reserve};

fn checked_bounds(
	min: [f32; 3],
	max: [f32; 3],
	lump: usize,
	record: usize,
) -> Result<Bounds, Error> {
	if !finite(min) || !finite(max) || (0..3).any(|axis| min[axis] > max[axis]) {
		return Err(Invalid::Bounds { lump, record }.into());
	}
	Ok(Bounds { min, max })
}

fn child(
	value: i32,
	nodes: usize,
	leaves: usize,
	lump: usize,
	record: usize,
) -> Result<Child, Error> {
	if value >= 0 {
		return index(value, nodes, lump, record).map(Child::Node);
	}
	// Widen before negating: i32::MIN must report an invalid index, not panic.
	let leaf = -(i64::from(value) + 1);
	let leaf = usize::try_from(leaf).map_err(|_| Invalid::Reference { lump, record })?;
	if leaf >= leaves {
		return Err(Invalid::Reference { lump, record }.into());
	}
	Ok(Child::Leaf(leaf))
}

pub(super) fn decode(bytes: &[u8], limits: Limits) -> Result<Bsp, Error> {
	let mut work = Work::new(limits.work);
	let directory = super::lumps::Directory::read(bytes, limits, &mut work)?;
	let mut budget = super::lumps::Budget::new(limits);
	let planes_lump = directory.prepare(raw::LUMP_PLANES, &[0], &mut budget, &mut work)?;
	let nodes_lump = directory.prepare(raw::LUMP_NODES, &[0], &mut budget, &mut work)?;
	let leaves_lump = directory.prepare(raw::LUMP_LEAVES, &[0, 1], &mut budget, &mut work)?;
	let models_lump = directory.prepare(raw::LUMP_MODELS, &[0], &mut budget, &mut work)?;
	let leaf_stride = if leaves_lump.version == 0 {
		raw::LEAF_V0_BYTES
	} else {
		raw::LEAF_V1_BYTES
	};
	let required = [
		(raw::LUMP_PLANES, planes_lump.length, raw::PLANE_BYTES),
		(raw::LUMP_NODES, nodes_lump.length, raw::NODE_BYTES),
		(raw::LUMP_LEAVES, leaves_lump.length, leaf_stride),
		(raw::LUMP_MODELS, models_lump.length, raw::MODEL_BYTES),
	];
	let mut counts = [0; 4];
	let mut total = 0usize;
	for (slot, (id, length, stride)) in required.into_iter().enumerate() {
		if length % stride != 0 {
			return Err(Invalid::RecordSize { lump: id }.into());
		}
		let count = length / stride;
		total = total.checked_add(count).ok_or(Error::Budget {
			resource: Resource::Records,
			limit: limits.records,
		})?;
		bound(total, limits.records, Resource::Records)?;
		counts[slot] = count;
	}
	let [plane_count, node_count, leaf_count, model_count] = counts;
	if leaf_count == 0 || model_count == 0 {
		return Err(Invalid::MissingWorld.into());
	}

	let data = planes_lump.decode()?;
	let mut planes = Vec::new();
	reserve(&mut planes, plane_count, Resource::Records)?;
	for (record, value) in data.chunks_exact(raw::PLANE_BYTES).enumerate() {
		work.spend(1)?;
		let normal = vector(value, 0);
		let distance = float(value, 12);
		if !finite(normal) || !distance.is_finite() || normal == [0.0; 3] {
			return Err(Invalid::Plane { record }.into());
		}
		planes.push(Plane { normal, distance });
	}

	drop(data);

	let data = nodes_lump.decode()?;
	let mut nodes = Vec::new();
	reserve(&mut nodes, node_count, Resource::Records)?;
	for (record, value) in data.chunks_exact(raw::NODE_BYTES).enumerate() {
		work.spend(1)?;
		let lump = raw::LUMP_NODES;
		nodes.push(Node {
			plane: index(integer(value, 0), plane_count, lump, record)?,
			children: [
				child(integer(value, 4), node_count, leaf_count, lump, record)?,
				child(integer(value, 8), node_count, leaf_count, lump, record)?,
			],
			bounds: short_bounds(value, 12, lump, record)?,
		});
	}

	drop(data);

	let data = leaves_lump.decode()?;
	let mut leaves = Vec::new();
	reserve(&mut leaves, leaf_count, Resource::Records)?;
	for (record, value) in data.chunks_exact(leaf_stride).enumerate() {
		work.spend(1)?;
		let area_flags = unsigned_short(value, 6);
		leaves.push(Leaf {
			contents: word(value, 0),
			cluster: short(value, 4),
			area: area_flags & 0x01ff,
			flags: area_flags >> 9,
			bounds: short_bounds(value, 8, raw::LUMP_LEAVES, record)?,
		});
	}

	drop(data);

	let data = models_lump.decode()?;
	let mut models = Vec::new();
	reserve(&mut models, model_count, Resource::Records)?;
	for (record, value) in data.chunks_exact(raw::MODEL_BYTES).enumerate() {
		work.spend(1)?;
		let lump = raw::LUMP_MODELS;
		let origin = vector(value, 24);
		if !finite(origin) {
			return Err(Invalid::Bounds { lump, record }.into());
		}
		models.push(Model {
			bounds: checked_bounds(vector(value, 0), vector(value, 12), lump, record)?,
			origin,
			head: child(integer(value, 36), node_count, leaf_count, lump, record)?,
		});
	}

	drop(data);

	let depths = validate_nodes(&nodes, limits, &mut work)?;
	let world_depth = match models[0].head {
		Child::Node(node) => depths[node],
		Child::Leaf(_) => 0,
	};
	Ok(Bsp {
		version: directory.version,
		revision: directory.revision,
		planes,
		nodes,
		leaves,
		models,
		world_depth,
		limits,
	})
}

fn float(value: &[u8], at: usize) -> f32 {
	f32::from_bits(word(value, at))
}

fn index(value: i32, count: usize, lump: usize, record: usize) -> Result<usize, Error> {
	usize::try_from(value)
		.ok()
		.filter(|&value| value < count)
		.ok_or_else(|| Invalid::Reference { lump, record }.into())
}

fn integer(value: &[u8], at: usize) -> i32 {
	i32::from_le_bytes([value[at], value[at + 1], value[at + 2], value[at + 3]])
}

fn short(value: &[u8], at: usize) -> i16 {
	i16::from_le_bytes([value[at], value[at + 1]])
}

fn short_bounds(value: &[u8], at: usize, lump: usize, record: usize) -> Result<Bounds, Error> {
	let min = [0, 2, 4].map(|offset| f32::from(short(value, at + offset)));
	let max = [6, 8, 10].map(|offset| f32::from(short(value, at + offset)));
	checked_bounds(min, max, lump, record)
}

fn unsigned_short(value: &[u8], at: usize) -> u16 {
	u16::from_le_bytes([value[at], value[at + 1]])
}

fn validate_nodes(nodes: &[Node], limits: Limits, work: &mut Work) -> Result<Vec<usize>, Error> {
	let mut state = Vec::new();
	reserve(&mut state, nodes.len(), Resource::Records)?;
	state.resize(nodes.len(), 0u8);
	let mut depths = Vec::new();
	reserve(&mut depths, nodes.len(), Resource::Records)?;
	depths.resize(nodes.len(), 0usize);
	let mut stack = Vec::new();
	let capacity = nodes
		.len()
		.checked_mul(2)
		.and_then(|value| value.checked_add(1))
		.ok_or(Error::Allocation {
			resource: Resource::Records,
		})?;
	reserve(&mut stack, capacity, Resource::Records)?;

	for start in 0..nodes.len() {
		work.spend(1)?;
		if state[start] == 2 {
			continue;
		}
		stack.push((start, false));
		while let Some((node, leaving)) = stack.pop() {
			work.spend(1)?;
			if leaving {
				let depth = nodes[node]
					.children
					.into_iter()
					.map(|child| match child {
						Child::Node(child) => depths[child],
						Child::Leaf(_) => 0,
					})
					.max()
					.unwrap_or(0)
					.checked_add(1)
					.ok_or(Error::Budget {
						resource: Resource::TreeDepth,
						limit: limits.tree_depth,
					})?;
				bound(depth, limits.tree_depth, Resource::TreeDepth)?;
				depths[node] = depth;
				state[node] = 2;
				continue;
			}
			match state[node] {
				1 => return Err(Invalid::Cycle { node }.into()),
				2 => continue,
				_ => {}
			}
			state[node] = 1;
			stack.push((node, true));
			for child in nodes[node].children.into_iter().rev() {
				if let Child::Node(child) = child {
					stack.push((child, false));
				}
			}
		}
	}
	Ok(depths)
}

fn vector(value: &[u8], at: usize) -> [f32; 3] {
	[0, 4, 8].map(|offset| float(value, at + offset))
}

fn word(value: &[u8], at: usize) -> u32 {
	u32::from_le_bytes([value[at], value[at + 1], value[at + 2], value[at + 3]])
}
