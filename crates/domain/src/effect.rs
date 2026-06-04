/// A value type describing a desired execution outcome.
///
/// Effects are returned from execution handlers and interpreted by runtime crates.
/// Handlers do not execute effects directly — they describe what should happen.
///
/// # Type parameters
///
/// - `E`: Event type (model-agnostic, no DomainEvent bound required)
/// - `R`: Reply type
/// - `S`: State type
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Effect<E, R, S> {
    /// No side effects. The handler completed without producing outcomes.
    NoEffect,
    /// A state mutation carrying the new state value.
    StateMutation(S),
    /// One or more events to persist.
    EventEmission(Vec<E>),
    /// A reply to send back to the caller.
    Reply(R),
    /// Multiple effects composed together. Composition is recursive —
    /// children may themselves be `Composed`.
    Composed(Vec<Effect<E, R, S>>),
}

impl<E, R, S> Effect<E, R, S> {
    /// No side effects.
    pub fn no() -> Self {
        Effect::NoEffect
    }

    /// A state mutation carrying the new state.
    pub fn state(state: S) -> Self {
        Effect::StateMutation(state)
    }

    /// One or more events to persist.
    pub fn emit(events: Vec<E>) -> Self {
        Effect::EventEmission(events)
    }

    /// A reply to send back to the caller.
    pub fn reply(reply: R) -> Self {
        Effect::Reply(reply)
    }

    /// Multiple effects composed together.
    pub fn compose(children: Vec<Effect<E, R, S>>) -> Self {
        Effect::Composed(children)
    }

    /// Combine this effect with another, returning a `Composed` containing both.
    ///
    /// Nested `Composed` children are recursively flattened — the result
    /// contains only non-Composed leaf effects in insertion order.
    ///
    /// If either effect is `NoEffect`, the other is returned unchanged.
    pub fn and_then(self, other: Effect<E, R, S>) -> Self {
        match (&self, &other) {
            (Effect::NoEffect, _) => return other,
            (_, Effect::NoEffect) => return self,
            _ => {}
        }
        let mut children = Vec::with_capacity(2);
        self.collect_children(&mut children);
        other.collect_children(&mut children);
        Effect::Composed(children)
    }

    /// Collect non-Composed children into the given vector.
    fn collect_children(self, acc: &mut Vec<Effect<E, R, S>>) {
        match self {
            Effect::Composed(children) => {
                for child in children {
                    child.collect_children(acc);
                }
            }
            Effect::NoEffect => {}
            other => acc.push(other),
        }
    }
}

/// Convenience alias for handler return types.
///
/// Execution handlers return `HandlerResult<E, R, S>` synchronously.
/// The runtime interprets the returned Effect and executes the described outcomes.
pub type HandlerResult<E, R, S> = Effect<E, R, S>;

#[cfg(test)]
mod tests {
    use super::*;

    type TestEffect = Effect<String, String, String>;

    #[test]
    fn no_effect_constructs() {
        let effect = TestEffect::no();
        assert_eq!(effect, Effect::NoEffect);
    }

    #[test]
    fn state_mutation_constructs() {
        let effect = TestEffect::state("new_state".to_string());
        assert_eq!(effect, Effect::StateMutation("new_state".to_string()));
    }

    #[test]
    fn event_emission_constructs() {
        let effect = TestEffect::emit(vec!["evt1".to_string(), "evt2".to_string()]);
        assert_eq!(
            effect,
            Effect::EventEmission(vec!["evt1".to_string(), "evt2".to_string()])
        );
    }

    #[test]
    fn reply_constructs() {
        let effect = TestEffect::reply("response".to_string());
        assert_eq!(effect, Effect::Reply("response".to_string()));
    }

    #[test]
    fn compose_constructs() {
        let inner = vec![TestEffect::reply("r".to_string()), TestEffect::no()];
        let effect = TestEffect::compose(inner.clone());
        assert_eq!(effect, Effect::Composed(inner));
    }

    #[test]
    fn and_then_combines_two_effects() {
        let a = TestEffect::reply("r".to_string());
        let b = TestEffect::emit(vec!["e".to_string()]);
        let combined = a.and_then(b);
        assert_eq!(
            combined,
            Effect::Composed(vec![
                Effect::Reply("r".to_string()),
                Effect::EventEmission(vec!["e".to_string()]),
            ])
        );
    }

    #[test]
    fn and_then_with_noeffect_returns_other() {
        let b = TestEffect::reply("r".to_string());
        assert_eq!(TestEffect::no().and_then(b.clone()), b);
        assert_eq!(b.clone().and_then(TestEffect::no()), b);
    }

    #[test]
    fn and_then_flattens_nested_composed() {
        let a = TestEffect::reply("r".to_string());
        let b = TestEffect::compose(vec![
            TestEffect::emit(vec!["e".to_string()]),
            TestEffect::state("s".to_string()),
        ]);
        let combined = a.and_then(b);
        assert_eq!(
            combined,
            Effect::Composed(vec![
                Effect::Reply("r".to_string()),
                Effect::EventEmission(vec!["e".to_string()]),
                Effect::StateMutation("s".to_string()),
            ])
        );
    }

    #[test]
    fn effects_equal_by_value() {
        let a = TestEffect::reply("x".to_string());
        let b = TestEffect::reply("x".to_string());
        let c = TestEffect::reply("y".to_string());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn noeffect_not_equal_to_reply() {
        let a = TestEffect::no();
        let b = TestEffect::reply("x".to_string());
        assert_ne!(a, b);
    }

    // --- User Story 1: Reply ---

    fn handle_reply() -> TestEffect {
        TestEffect::reply("ok".to_string())
    }

    #[test]
    fn handler_returns_reply_effect() {
        let result = handle_reply();
        assert_eq!(result, Effect::reply("ok".to_string()));
    }

    // --- User Story 2: Event Emission ---

    fn handle_emit() -> TestEffect {
        TestEffect::emit(vec!["user_created".to_string()])
    }

    fn handle_emit_multi() -> TestEffect {
        TestEffect::emit(vec!["evt1".to_string(), "evt2".to_string()])
    }

    #[test]
    fn handler_returns_event_emission() {
        let result = handle_emit();
        assert_eq!(result, Effect::emit(vec!["user_created".to_string()]));
    }

    #[test]
    fn handler_returns_multi_event_emission() {
        let result = handle_emit_multi();
        assert_eq!(
            result,
            Effect::emit(vec!["evt1".to_string(), "evt2".to_string()])
        );
    }

    // --- User Story 3: Composition ---

    fn handle_compose_events_and_reply() -> TestEffect {
        Effect::reply("done".to_string()).and_then(Effect::emit(vec!["evt".to_string()]))
    }

    fn handle_complex_compose() -> TestEffect {
        Effect::state("new_state".to_string())
            .and_then(Effect::emit(vec!["evt".to_string()]))
            .and_then(Effect::reply("ok".to_string()))
    }

    #[test]
    fn handler_returns_composed_events_and_reply() {
        let result = handle_compose_events_and_reply();
        let expected =
            Effect::reply("done".to_string()).and_then(Effect::emit(vec!["evt".to_string()]));
        assert_eq!(result, expected);
    }

    #[test]
    fn handler_returns_complex_composition() {
        let result = handle_complex_compose();
        let expected = Effect::state("new_state".to_string())
            .and_then(Effect::emit(vec!["evt".to_string()]))
            .and_then(Effect::reply("ok".to_string()));
        assert_eq!(result, expected);
    }

    // --- Exhaustiveness ---

    /// Verify that a match on Effect covers all variants.
    /// If a new variant is added, this test will fail to compile,
    /// forcing the developer to handle the new variant.
    #[test]
    fn effect_match_is_exhaustive() {
        let cases: Vec<TestEffect> = vec![
            TestEffect::no(),
            TestEffect::state("s".to_string()),
            TestEffect::emit(vec!["e".to_string()]),
            TestEffect::reply("r".to_string()),
            TestEffect::compose(vec![TestEffect::no()]),
        ];
        for effect in cases {
            match effect {
                Effect::NoEffect => {}
                Effect::StateMutation(_) => {}
                Effect::EventEmission(_) => {}
                Effect::Reply(_) => {}
                Effect::Composed(_) => {}
            }
        }
    }
}
