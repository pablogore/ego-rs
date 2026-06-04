use ego_domain::effect::Effect;
use thiserror::Error;

/// Errors that can occur during effect interpretation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InterpretationError {
    /// The runtime cannot interpret the given effect variant.
    #[error("unsupported effect: {0}")]
    UnsupportedEffect(String),
    /// A transient failure occurred (e.g., retryable I/O error).
    #[error("transient interpretation failure: {0}")]
    Transient(String),
    /// A fatal failure occurred (cannot recover).
    #[error("fatal interpretation failure: {0}")]
    Fatal(String),
}

/// Interprets [`Effect`] values by executing the described outcomes.
///
/// Runtime implementations (e.g., Tokio-backed actor runtimes) implement this
/// trait to handle each `Effect` variant: sending replies, persisting events,
/// and mutating state.
///
/// # Contract
///
/// - Implementations MUST match all `Effect` variants exhaustively.
/// - `Composed` effects MUST process child effects in order.
/// - Implementations SHOULD return
///   [`InterpretationError::UnsupportedEffect`] for effects they cannot
///   handle.
/// - Implementations MUST be [`Send`] + [`Sync`] (may be shared across
///   threads).
#[async_trait::async_trait]
pub trait EffectInterpreter<E, R, S>: Send + Sync {
    /// Interpret and execute the given effect.
    ///
    /// Returns `Ok(())` if the effect was fully handled, or an
    /// [`InterpretationError`] describing the failure.
    async fn interpret(&self, effect: Effect<E, R, S>) -> Result<(), InterpretationError>;
}

/// Default interpretation for composed effects.
///
/// Processes each child effect in order, returning the first error if any
/// child fails. This is the standard semantics required by the
/// [`EffectInterpreter`] contract.
pub async fn interpret_composed<E, R, S>(
    interpreter: &dyn EffectInterpreter<E, R, S>,
    children: Vec<Effect<E, R, S>>,
) -> Result<(), InterpretationError> {
    for child in children {
        interpreter.interpret(child).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock interpreter that records dispatched effects for assertion.
    struct RecordingInterpreter {
        dispatched: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingInterpreter {
        fn new() -> Self {
            Self {
                dispatched: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn take_effects(&self) -> Vec<String> {
            std::mem::take(&mut *self.dispatched.lock().unwrap())
        }
    }

    #[async_trait::async_trait]
    impl EffectInterpreter<String, String, String> for RecordingInterpreter {
        async fn interpret(
            &self,
            effect: Effect<String, String, String>,
        ) -> Result<(), InterpretationError> {
            match &effect {
                Effect::NoEffect => {
                    self.dispatched.lock().unwrap().push("NoEffect".into());
                    Ok(())
                }
                Effect::Reply(r) => {
                    self.dispatched
                        .lock()
                        .unwrap()
                        .push(format!("Reply({})", r));
                    Ok(())
                }
                Effect::EventEmission(events) => {
                    self.dispatched
                        .lock()
                        .unwrap()
                        .push(format!("EventEmission({})", events.join(",")));
                    Ok(())
                }
                Effect::StateMutation(s) => {
                    self.dispatched
                        .lock()
                        .unwrap()
                        .push(format!("StateMutation({})", s));
                    Ok(())
                }
                Effect::Composed(children) => {
                    self.dispatched.lock().unwrap().push("Composed".into());
                    interpret_composed(self, children.clone()).await
                }
            }
        }
    }

    #[tokio::test]
    async fn test_no_effect_returns_ok() {
        let interp = RecordingInterpreter::new();
        let result = interp
            .interpret(Effect::<String, String, String>::NoEffect)
            .await;
        assert!(result.is_ok());
        assert_eq!(interp.take_effects(), vec!["NoEffect"]);
    }

    #[tokio::test]
    async fn test_reply_effect_returns_ok() {
        let interp = RecordingInterpreter::new();
        let result = interp.interpret(Effect::reply("hello".to_string())).await;
        assert!(result.is_ok());
        assert_eq!(interp.take_effects(), vec!["Reply(hello)"]);
    }

    #[tokio::test]
    async fn test_event_emission_returns_ok() {
        let interp = RecordingInterpreter::new();
        let result = interp
            .interpret(Effect::emit(vec!["evt1".to_string(), "evt2".to_string()]))
            .await;
        assert!(result.is_ok());
        assert_eq!(interp.take_effects(), vec!["EventEmission(evt1,evt2)"]);
    }

    #[tokio::test]
    async fn test_state_mutation_returns_ok() {
        let interp = RecordingInterpreter::new();
        let result = interp.interpret(Effect::state("s1".to_string())).await;
        assert!(result.is_ok());
        assert_eq!(interp.take_effects(), vec!["StateMutation(s1)"]);
    }

    #[tokio::test]
    async fn test_composed_processes_children_in_order() {
        let interp = RecordingInterpreter::new();
        let effect = Effect::compose(vec![
            Effect::reply("r1".to_string()),
            Effect::emit(vec!["e1".to_string()]),
            Effect::state("s1".to_string()),
            Effect::<String, String, String>::NoEffect,
        ]);
        let result = interp.interpret(effect).await;
        assert!(result.is_ok());
        assert_eq!(
            interp.take_effects(),
            vec![
                "Composed",
                "Reply(r1)",
                "EventEmission(e1)",
                "StateMutation(s1)",
                "NoEffect",
            ]
        );
    }

    #[tokio::test]
    async fn test_nested_composed_flattens_in_order() {
        let interp = RecordingInterpreter::new();
        let inner = Effect::compose(vec![
            Effect::emit(vec!["inner".to_string()]),
            Effect::reply("inner_r".to_string()),
        ]);
        let outer = Effect::compose(vec![
            Effect::reply("outer_r".to_string()),
            inner,
            Effect::state("final".to_string()),
        ]);
        let result = interp.interpret(outer).await;
        assert!(result.is_ok());
        assert_eq!(
            interp.take_effects(),
            vec![
                "Composed",
                "Reply(outer_r)",
                "Composed",
                "EventEmission(inner)",
                "Reply(inner_r)",
                "StateMutation(final)",
            ]
        );
    }

    #[tokio::test]
    async fn test_send_sync_bounds() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RecordingInterpreter>();
    }

    #[tokio::test]
    async fn test_trait_object_compatible() {
        let interp = RecordingInterpreter::new();
        let trait_obj: &dyn EffectInterpreter<String, String, String> = &interp;
        let result = trait_obj
            .interpret(Effect::reply("via_trait".to_string()))
            .await;
        assert!(result.is_ok());
    }
}
