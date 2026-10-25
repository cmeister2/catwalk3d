//! Preview-owned traversal and collection limits for native CAD geometry.

use cadmpeg_core::{
    CodecError,
    decode::{DecodeArena, DecodeContext, DecodePolicy, View, WorkBudget},
};
use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap},
    hash::Hash,
    mem::size_of,
    ops::Deref,
};

/// Preserve the service traversal ceiling independently of codec internals.
const MAX_WORK: usize = 256_000_000;
/// Preserve the service collection ceiling for preview-owned indices.
const MAX_ITEMS: usize = 1_000_000;

/// Shared preview counters alongside the codec's retained-byte accounting.
pub(super) struct Context<'a> {
    /// Upstream recursion and retained-byte limits.
    decode: DecodeContext<'a>,
    /// Preview operations, including identity copies and graph visits.
    work: WorkBudget,
    /// Distinct entries admitted across all preview collections.
    items: Cell<usize>,
}

impl<'a> Context<'a> {
    /// Start a preview with the same retained-byte policy as the decoded input.
    ///
    /// # Errors
    /// The codec rejects the supplied root bytes under its input policy.
    pub(super) fn from_root_bytes(
        bytes: &'a [u8],
        arena: &'a DecodeArena,
        policy: &DecodePolicy,
    ) -> Result<(Self, View<'a>), CodecError> {
        let (decode, view) = DecodeContext::from_root_bytes(bytes, arena, policy)?;
        Ok((
            Self {
                decode,
                work: WorkBudget::new(MAX_WORK),
                items: Cell::new(0),
            },
            view,
        ))
    }

    /// Charge preview work before traversal or copying source identities.
    ///
    /// # Errors
    /// The cumulative service work ceiling is exceeded.
    pub(super) fn charge_work(
        &self,
        units: usize,
        operation: &'static str,
    ) -> Result<(), CodecError> {
        if !self.work.charge_by(units) {
            return Err(CodecError::malformed(format_args!(
                "{operation} exceeds native preview work limit"
            )));
        }
        Ok(())
    }

    /// Admit one collection slot and conservatively account node/bucket overhead.
    ///
    /// # Errors
    /// Collection, size arithmetic, or retained-byte limits are exceeded.
    fn entry<T>(&self, operation: &'static str) -> Result<(), CodecError> {
        self.charge_work(1, operation)?;
        let items = self
            .items
            .get()
            .checked_add(1)
            .filter(|&n| n <= MAX_ITEMS)
            .ok_or_else(|| {
                CodecError::malformed(format_args!(
                    "{operation} exceeds native preview collection limit"
                ))
            })?;
        let bytes = size_of::<T>()
            .checked_mul(4)
            .and_then(|n| n.checked_add(64))
            .ok_or_else(|| {
                CodecError::malformed(format_args!("{operation} storage size overflows"))
            })?;
        self.decode.retain_bytes(
            u64::try_from(bytes).map_err(|_| {
                CodecError::malformed(format_args!("{operation} storage size overflows"))
            })?,
            operation,
        )?;
        self.items.set(items);
        Ok(())
    }

    /// Copy an identity only after charging work and retained text bytes.
    ///
    /// # Errors
    /// A work, retained-byte, or allocation limit is exceeded.
    pub(super) fn copy_retained_text(
        &self,
        text: &str,
        operation: &'static str,
    ) -> Result<String, CodecError> {
        self.charge_work(text.len(), operation)?;
        self.decode.retain_bytes(
            u64::try_from(text.len()).map_err(|_| {
                CodecError::malformed(format_args!("{operation} text length overflows"))
            })?,
            operation,
        )?;
        let mut copy = String::new();
        copy.try_reserve_exact(text.len()).map_err(|_| {
            CodecError::malformed(format_args!("{operation} text allocation failed"))
        })?;
        copy.push_str(text);
        Ok(copy)
    }

    /// Insert a distinct ordered reference after accounting for its storage.
    ///
    /// # Errors
    /// A preview collection or retained-byte limit is exceeded.
    pub(super) fn insert_btree_set<T: Ord>(
        &self,
        values: &mut BTreeSet<T>,
        value: T,
        operation: &'static str,
    ) -> Result<bool, CodecError> {
        if values.contains(&value) {
            return Ok(false);
        }
        self.entry::<T>(operation)?;
        Ok(values.insert(value))
    }

    /// Insert an index entry with fallible hash-table growth.
    ///
    /// # Errors
    /// A preview collection, retained-byte, or allocation limit is exceeded.
    pub(super) fn insert_hash_map<K: Eq + Hash, V>(
        &self,
        values: &mut HashMap<K, V>,
        key: K,
        value: V,
        operation: &'static str,
    ) -> Result<Option<V>, CodecError> {
        if !values.contains_key(&key) {
            self.entry::<(K, V)>(operation)?;
            values.try_reserve(1).map_err(|_| {
                CodecError::malformed(format_args!("{operation} index allocation failed"))
            })?;
        }
        Ok(values.insert(key, value))
    }

    /// Build a preview index without bypassing shared collection accounting.
    ///
    /// # Errors
    /// A preview collection, retained-byte, or allocation limit is exceeded.
    pub(super) fn collect_hash_map<K: Eq + Hash, V>(
        &self,
        values: impl IntoIterator<Item = (K, V)>,
        operation: &'static str,
    ) -> Result<HashMap<K, V>, CodecError> {
        let mut out = HashMap::new();
        for (key, value) in values {
            self.insert_hash_map(&mut out, key, value, operation)?;
        }
        Ok(out)
    }
}

impl<'a> Deref for Context<'a> {
    type Target = DecodeContext<'a>;

    fn deref(&self) -> &Self::Target {
        &self.decode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_work_stays_refused() {
        let arena = DecodeArena::new();
        let policy = DecodePolicy::service();
        let (ctx, _) = Context::from_root_bytes(&[], &arena, &policy).unwrap();
        ctx.charge_work(MAX_WORK, "test traversal").unwrap();
        assert!(ctx.charge_work(1, "test traversal").is_err());
        assert!(ctx.copy_retained_text("identity", "test copy").is_err());
    }

    #[test]
    fn retained_text_obeys_the_codec_policy() {
        let arena = DecodeArena::new();
        let mut policy = DecodePolicy::service();
        policy.limits.max_retained_bytes = 3;
        let (ctx, _) = Context::from_root_bytes(&[], &arena, &policy).unwrap();
        assert_eq!(ctx.copy_retained_text("abc", "test copy").unwrap(), "abc");
        assert!(ctx.copy_retained_text("d", "test copy").is_err());
    }

    #[test]
    fn collection_limit_refuses_new_entries_without_dropping_existing_values() {
        let arena = DecodeArena::new();
        let policy = DecodePolicy::service();
        let (ctx, _) = Context::from_root_bytes(&[], &arena, &policy).unwrap();
        let mut values = BTreeSet::new();
        ctx.insert_btree_set(&mut values, 1, "test index").unwrap();
        ctx.items.set(MAX_ITEMS);
        assert!(!ctx.insert_btree_set(&mut values, 1, "test index").unwrap());
        assert!(ctx.insert_btree_set(&mut values, 2, "test index").is_err());
        assert_eq!(values, BTreeSet::from([1]));
    }

    #[test]
    fn hash_index_limit_allows_replacement_but_refuses_new_keys() {
        let arena = DecodeArena::new();
        let policy = DecodePolicy::service();
        let (ctx, _) = Context::from_root_bytes(&[], &arena, &policy).unwrap();
        let mut values = ctx.collect_hash_map([(1, 2)], "test index").unwrap();
        ctx.items.set(MAX_ITEMS);
        assert_eq!(
            ctx.insert_hash_map(&mut values, 1, 3, "test index")
                .unwrap(),
            Some(2)
        );
        assert!(
            ctx.insert_hash_map(&mut values, 2, 4, "test index")
                .is_err()
        );
        assert_eq!(values, HashMap::from([(1, 3)]));
    }
}
