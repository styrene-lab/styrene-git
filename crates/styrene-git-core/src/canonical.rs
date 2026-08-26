//! Canonical default-branch selection from verified delegate namespaces.

use std::collections::BTreeMap;

use crate::{GitObjectId, IdentityDocument, RefState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalDecision {
    Advance(GitObjectId),
    Retain,
    NoAgreement,
    Diverged(GitObjectId),
}

pub fn derive_canonical_head<F>(
    identity: &IdentityDocument,
    states: &BTreeMap<crate::StyreneIdentity, RefState>,
    previous: Option<&GitObjectId>,
    is_descendant: F,
) -> CanonicalDecision
where
    F: Fn(&GitObjectId, &GitObjectId) -> bool,
{
    let branch = format!("refs/heads/{}", identity.default_branch);
    let mut votes: BTreeMap<GitObjectId, u16> = BTreeMap::new();
    for delegate in &identity.delegates {
        if let Some(target) = states.get(delegate).and_then(|state| state.target(&branch)) {
            *votes.entry(target.clone()).or_default() += 1;
        }
    }

    let qualifying: Vec<_> = votes
        .into_iter()
        .filter_map(|(target, votes)| (votes >= identity.threshold).then_some(target))
        .collect();
    let [candidate] = qualifying.as_slice() else {
        return if previous.is_some() {
            CanonicalDecision::Retain
        } else {
            CanonicalDecision::NoAgreement
        };
    };
    if previous == Some(candidate) {
        return CanonicalDecision::Retain;
    }
    if let Some(previous) = previous {
        if !is_descendant(previous, candidate) {
            return CanonicalDecision::Diverged(candidate.clone());
        }
    }
    CanonicalDecision::Advance(candidate.clone())
}
