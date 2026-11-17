//! Thinning out a series of copies over time (backups, revisions): recent
//! ones are all kept, older ones only one per period.

use std::collections::HashSet;
use std::hash::Hash;

pub(crate) const HOUR: i64 = 60 * 60;
pub(crate) const DAY: i64 = 24 * HOUR;

/// What to do with one copy of a series.
pub(crate) enum Keep<B> {
    Always,
    /// Only the first copy (in the order given) of each bucket is kept.
    OnePer(B),
    Never,
}

/// Which copies to delete: indices into `keeps`, ascending. Give the
/// newest first to keep the newest of each bucket.
pub(crate) fn thin_out<B: Eq + Hash>(keeps: impl IntoIterator<Item = Keep<B>>) -> Vec<usize> {
    let mut buckets = HashSet::new();
    let mut prune = Vec::new();
    for (i, keep) in keeps.into_iter().enumerate() {
        let kept = match keep {
            Keep::Always => true,
            Keep::OnePer(bucket) => buckets.insert(bucket),
            Keep::Never => false,
        };
        if !kept {
            prune.push(i);
        }
    }
    prune
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_per_bucket() {
        let keeps = [
            Keep::Always,
            Keep::OnePer(1),
            Keep::OnePer(1),
            Keep::Never,
            Keep::OnePer(2),
            Keep::Always,
            Keep::OnePer(2),
        ];
        assert_eq!(thin_out(keeps), [2, 3, 6]);
    }
}
