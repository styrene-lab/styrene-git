use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use styrene_git_core::{
    CoreError, IdentityDocument, IdentityState, IdentityTransition, PublicKey, SignerBinding,
    StyreneIdentity, Visibility,
};

struct Actor {
    repository_key: SigningKey,
    binding: SignerBinding,
}

impl Actor {
    fn seeded(identity_seed: u8, repository_seed: u8) -> Self {
        let identity_key = SigningKey::from_bytes(&[identity_seed; 32]);
        let repository_key = SigningKey::from_bytes(&[repository_seed; 32]);
        let binding = SignerBinding::issue(
            &identity_key,
            PublicKey::new(repository_key.verifying_key().to_bytes()),
            0,
        )
        .expect("fixed keys should create a binding");
        Self {
            repository_key,
            binding,
        }
    }

    fn id(&self) -> StyreneIdentity {
        self.binding.identity()
    }
}

fn document(
    name: String,
    actors: &[Actor],
    delegate_mask: u8,
    threshold_seed: u8,
) -> IdentityDocument {
    let effective_mask = if delegate_mask & 0x0f == 0 {
        1
    } else {
        delegate_mask & 0x0f
    };
    let delegates: Vec<_> = actors
        .iter()
        .enumerate()
        .filter_map(|(index, actor)| (effective_mask & (1 << index) != 0).then_some(actor.id()))
        .collect();
    let threshold = 1 + u16::from(threshold_seed) % delegates.len() as u16;
    IdentityDocument::new(
        name,
        "generated Heartwood parity history",
        "main",
        Visibility::Public,
        delegates,
        threshold,
    )
    .expect("generated delegate policy should be valid")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn parity_property_identity_histories_preserve_prior_policy_and_lineage(
        operations in prop::collection::vec((any::<u8>(), any::<u8>(), any::<u8>(), any::<bool>()), 1..24)
    ) {
        let actors = [
            Actor::seeded(1, 11),
            Actor::seeded(2, 12),
            Actor::seeded(3, 13),
            Actor::seeded(4, 14),
        ];
        let bindings: BTreeMap<_, _> = actors
            .iter()
            .map(|actor| {
                (
                    actor.id(),
                    actor
                        .binding
                        .selection()
                        .expect("fixture binding should produce a selection"),
                )
            })
            .collect();
        let initial_document = IdentityDocument::new(
            "generated-root",
            "generated Heartwood parity history",
            "main",
            Visibility::Public,
            actors[..3].iter().map(Actor::id).collect(),
            2,
        )
        .expect("initial policy");
        let mut current = IdentityState::initial(initial_document).expect("initial state");
        let repository = current.repository;
        let mut accepted_ids = BTreeSet::from([current.transition]);

        for (step, (delegate_mask, threshold_seed, approval_mask, duplicate)) in
            operations.into_iter().enumerate()
        {
            let previous = current.clone();
            let next_document = document(
                format!("generated-{step}"),
                &actors,
                delegate_mask,
                threshold_seed,
            );
            let mut transition =
                IdentityTransition::proposed(&previous, next_document.clone())
                    .expect("generated transition proposal");
            for (index, actor) in actors.iter().enumerate() {
                if approval_mask & (1 << index) != 0 {
                    transition
                        .approve(&actor.binding, &actor.repository_key)
                        .expect("generated approval");
                    if duplicate {
                        transition
                            .approve(&actor.binding, &actor.repository_key)
                            .expect("duplicate approval replaces the first");
                    }
                }
            }

            let authorized_approvals = actors
                .iter()
                .enumerate()
                .filter(|(index, actor)| {
                    approval_mask & (1 << index) != 0
                        && previous.document.delegates.contains(&actor.id())
                })
                .count() as u16;
            let result = transition.verify(&previous, &bindings);
            if authorized_approvals < previous.document.threshold {
                prop_assert!(matches!(
                    result,
                    Err(CoreError::InvalidIdentityTransition(message))
                        if message == "delegate threshold not satisfied"
                ));
                prop_assert_eq!(&current, &previous);
                continue;
            }

            let accepted = result.expect("the prior delegate threshold is satisfied");
            prop_assert_eq!(accepted.repository, repository);
            prop_assert_eq!(accepted.sequence, previous.sequence + 1);
            prop_assert_eq!(&accepted.document, &next_document);
            prop_assert!(accepted_ids.insert(accepted.transition));
            prop_assert!(transition.verify(&accepted, &bindings).is_err());

            let sibling_document = document(
                format!("generated-{step}-sibling"),
                &actors,
                delegate_mask ^ 0x0f,
                threshold_seed.wrapping_add(1),
            );
            let mut sibling = IdentityTransition::proposed(&previous, sibling_document)
                .expect("generated sibling proposal");
            for delegate in &previous.document.delegates {
                let actor = actors
                    .iter()
                    .find(|actor| actor.id() == *delegate)
                    .expect("every delegate has a generated actor");
                sibling
                    .approve(&actor.binding, &actor.repository_key)
                    .expect("sibling approval");
            }
            let sibling_state = sibling
                .verify(&previous, &bindings)
                .expect("all prior delegates approved the sibling");
            prop_assert_ne!(sibling_state.transition, accepted.transition);
            prop_assert!(sibling.verify(&accepted, &bindings).is_err());
            current = accepted;
        }
    }
}
