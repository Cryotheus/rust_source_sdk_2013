//! Bounded, owned records from a BSP entity lump.
//!
//! This is the flat map entity grammar, not hierarchical KeyValues. Records and
//! repeated pairs retain their order and original bytes, including non-UTF8
//! values and output separators. Quoted strings have no backslash escapes.
//! Whitespace and // comments are accepted outside tokens. The parser rejects
//! data after a NUL terminator other than NUL/ASCII-whitespace padding.
//!
//! Successful parsing does not establish how a game interprets duplicate keys,
//! whether entities exist at runtime, or any geometry or gameplay role.

#[cfg(test)]
mod tests;

use std::ops::Range;

/// More than one spelling of a requested singleton key was present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("entity {record} repeats a singleton field at bytes {first} and {second}")]
pub struct DuplicateField {
	pub record: usize,
	pub first: usize,
	pub second: usize,
}

/// One immutable owned payload and its checked record/pair ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entities {
	payload: Vec<u8>,
	records: Vec<RecordRange>,
	pairs: Vec<PairRange>,
}

impl Entities {
	/// Parses a complete payload without requiring UTF8 or changing its bytes.
	///
	/// Limits apply to every record and pair, including ones the caller will not
	/// use. Source spans index the original payload and exclude token quotes.
	pub fn parse(payload: &[u8], limits: EntityLimits) -> Result<Self, EntityError> {
		check(
			payload.len(),
			limits.input_bytes,
			EntityResource::InputBytes,
		)?;
		let mut lexer = Lexer {
			payload,
			position: 0,
			remaining: limits.work,
			limits,
		};
		let mut records = Vec::new();
		let mut pairs = Vec::new();

		loop {
			let start = match lexer.next()? {
				Token::End => break,
				Token::Open(at) => at,
				Token::Close(at) => return Err(syntax(at, EntitySyntax::UnexpectedClose)),

				Token::Value(value) => {
					return Err(syntax(value.start, EntitySyntax::ExpectedEntity));
				}
			};
			check_next(records.len(), limits.records, EntityResource::Records)?;
			let first_pair = pairs.len();
			let end = loop {
				let key = match lexer.next()? {
					Token::Close(at) => break at + 1,
					Token::Open(at) => return Err(syntax(at, EntitySyntax::NestedEntity)),

					Token::End => {
						return Err(syntax(lexer.position, EntitySyntax::UnterminatedEntity));
					}

					Token::Value(key) => key,
				};
				let value = match lexer.next()? {
					Token::Value(value) => value,
					Token::Open(at) => return Err(syntax(at, EntitySyntax::NestedEntity)),
					_ => return Err(syntax(key.start, EntitySyntax::MissingValue)),
				};
				check_next(pairs.len(), limits.pairs, EntityResource::Pairs)?;
				reserve(&mut pairs, 1, EntityResource::Pairs)?;
				pairs.push(PairRange { key, value });
			};
			reserve(&mut records, 1, EntityResource::Records)?;
			records.push(RecordRange {
				source: start..end,
				pairs: first_pair..pairs.len(),
			});
		}

		let mut owned = Vec::new();
		reserve(&mut owned, payload.len(), EntityResource::InputBytes)?;
		owned.extend_from_slice(payload);
		Ok(Self {
			payload: owned,
			records,
			pairs,
		})
	}

	pub fn is_empty(&self) -> bool {
		self.records.is_empty()
	}

	pub fn len(&self) -> usize {
		self.records.len()
	}

	pub fn pair_count(&self) -> usize {
		self.pairs.len()
	}

	/// Original bytes, including accepted terminators and padding.
	pub fn payload(&self) -> &[u8] {
		&self.payload
	}

	/// A view borrows only this owned snapshot, never an engine allocation.
	pub fn record(&self, index: usize) -> Option<Record<'_>> {
		self.records.get(index).map(|_| Record {
			entities: self,
			index,
		})
	}

	pub fn records(&self) -> impl ExactSizeIterator<Item = Record<'_>> + DoubleEndedIterator {
		(0..self.records.len()).map(|index| Record {
			entities: self,
			index,
		})
	}
}

/// Parsing never returns a silently truncated or partially accepted snapshot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntityError {
	#[error("entity syntax at byte {offset}: {kind:?}")]
	Syntax { offset: usize, kind: EntitySyntax },
	#[error("entity {resource:?} budget exceeded (limit {limit})")]
	Budget {
		resource: EntityResource,
		limit: usize,
	},
	#[error("could not allocate entity {resource:?}")]
	Allocation { resource: EntityResource },
}

/// Bounds on parsing one payload. These bound counts, not exact allocator RSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityLimits {
	/// Maximum input bytes, including terminators and padding.
	pub input_bytes: usize,
	/// Maximum entity records, including empty records.
	pub records: usize,
	/// Maximum key/value pairs across all records.
	pub pairs: usize,
	/// Maximum bytes in a single key or value, excluding quotes.
	pub token_bytes: usize,
	/// Maximum consumed input bytes during tokenization.
	pub work: usize,
}

impl Default for EntityLimits {
	fn default() -> Self {
		Self {
			input_bytes: 8 << 20,
			records: 65_536,
			pairs: 524_288,
			token_bytes: 65_536,
			work: 16 << 20,
		}
	}
}

/// The resource whose bound could not be satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityResource {
	InputBytes,
	Records,
	Pairs,
	TokenBytes,
	Work,
}

/// A malformed flat entity grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntitySyntax {
	ExpectedEntity,
	UnexpectedClose,
	NestedEntity,
	MissingValue,
	UnterminatedEntity,
	UnterminatedQuote,
	InteriorNul,
}

struct Lexer<'a> {
	payload: &'a [u8],
	position: usize,
	remaining: usize,
	limits: EntityLimits,
}

impl Lexer<'_> {
	fn consume(&mut self) -> Result<(), EntityError> {
		self.remaining = self.remaining.checked_sub(1).ok_or(EntityError::Budget {
			resource: EntityResource::Work,
			limit: self.limits.work,
		})?;
		self.position += 1;
		Ok(())
	}

	fn next(&mut self) -> Result<Token, EntityError> {
		loop {
			let Some(&byte) = self.payload.get(self.position) else {
				return Ok(Token::End);
			};
			if byte == 0 {
				while let Some(&padding) = self.payload.get(self.position) {
					if padding != 0 && !padding.is_ascii_whitespace() {
						return Err(syntax(self.position, EntitySyntax::InteriorNul));
					}
					self.consume()?;
				}
				return Ok(Token::End);
			}
			if byte <= b' ' {
				self.consume()?;
				continue;
			}
			if byte == b'/' && self.payload.get(self.position + 1) == Some(&b'/') {
				while self
					.payload
					.get(self.position)
					.is_some_and(|&value| value != 0 && value != b'\n')
				{
					self.consume()?;
				}
				continue;
			}
			break;
		}

		let start = self.position;
		match self.payload[start] {
			b'{' => {
				self.consume()?;
				Ok(Token::Open(start))
			}

			b'}' => {
				self.consume()?;
				Ok(Token::Close(start))
			}

			b'"' => {
				self.consume()?;
				let start = self.position;
				loop {
					match self.payload.get(self.position).copied() {
						None => {
							return Err(syntax(
								start.saturating_sub(1),
								EntitySyntax::UnterminatedQuote,
							));
						}

						Some(0) => return Err(syntax(self.position, EntitySyntax::InteriorNul)),

						Some(b'"') => {
							let end = self.position;
							self.consume()?;
							return Ok(Token::Value(start..end));
						}

						Some(_) => {
							check_next(
								self.position - start,
								self.limits.token_bytes,
								EntityResource::TokenBytes,
							)?;
							self.consume()?;
						}
					}
				}
			}

			b'(' | b')' | b'\'' => {
				check(1, self.limits.token_bytes, EntityResource::TokenBytes)?;
				self.consume()?;
				Ok(Token::Value(start..self.position))
			}

			_ => {
				while self.payload.get(self.position).is_some_and(|&byte| {
					byte > b' ' && !matches!(byte, b'{' | b'}' | b'(' | b')' | b'\'')
				}) {
					check_next(
						self.position - start,
						self.limits.token_bytes,
						EntityResource::TokenBytes,
					)?;
					self.consume()?;
				}
				Ok(Token::Value(start..self.position))
			}
		}
	}
}

/// Original token bytes and their quote-excluding positions in the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair<'a> {
	pub key: &'a [u8],
	pub value: &'a [u8],
	pub key_source: Range<usize>,
	pub value_source: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PairRange {
	key: Range<usize>,
	value: Range<usize>,
}

/// A record view, tied to the snapshot that owns all of its ranges.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
	entities: &'a Entities,
	index: usize,
}

impl<'a> Record<'a> {
	pub fn index(self) -> usize {
		self.index
	}

	pub fn pairs(self) -> impl ExactSizeIterator<Item = Pair<'a>> + DoubleEndedIterator {
		let record = &self.entities.records[self.index];
		self.entities.pairs[record.pairs.clone()]
			.iter()
			.map(move |pair| Pair {
				key: &self.entities.payload[pair.key.clone()],
				value: &self.entities.payload[pair.value.clone()],
				key_source: pair.key.clone(),
				value_source: pair.value.clone(),
			})
	}

	/// A semantic consumer can require exactly one occurrence of a field.
	///
	/// Even identical repeated values are reported as duplicates. This does not
	/// change the underlying records or assume the game's first/last-key policy.
	pub fn singleton_ascii(self, key: &[u8]) -> Result<Option<&'a [u8]>, DuplicateField> {
		let mut found: Option<(&'a [u8], usize)> = None;
		for pair in self.pairs() {
			if !pair.key.eq_ignore_ascii_case(key) {
				continue;
			}
			if let Some((_, first)) = found {
				return Err(DuplicateField {
					record: self.index,
					first,
					second: pair.key_source.start,
				});
			}
			found = Some((pair.value, pair.key_source.start));
		}
		Ok(found.map(|(value, _)| value))
	}

	/// The complete entity text, including its opening and closing braces.
	pub fn source(self) -> Range<usize> {
		self.entities.records[self.index].source.clone()
	}

	/// Returns all matching values in their original order, without allocation.
	pub fn values_ascii(self, key: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
		self.pairs()
			.filter(move |pair| pair.key.eq_ignore_ascii_case(key))
			.map(|pair| pair.value)
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordRange {
	source: Range<usize>,
	pairs: Range<usize>,
}

enum Token {
	Open(usize),
	Close(usize),
	Value(Range<usize>),
	End,
}

fn check(value: usize, limit: usize, resource: EntityResource) -> Result<(), EntityError> {
	if value > limit {
		return Err(EntityError::Budget { resource, limit });
	}
	Ok(())
}

fn check_next(value: usize, limit: usize, resource: EntityResource) -> Result<(), EntityError> {
	if value >= limit {
		return Err(EntityError::Budget { resource, limit });
	}
	Ok(())
}

fn reserve<T>(
	values: &mut Vec<T>,
	additional: usize,
	resource: EntityResource,
) -> Result<(), EntityError> {
	values
		.try_reserve(additional)
		.map_err(|_| EntityError::Allocation { resource })
}

fn syntax(offset: usize, kind: EntitySyntax) -> EntityError {
	EntityError::Syntax { offset, kind }
}
